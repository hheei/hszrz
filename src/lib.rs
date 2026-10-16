
pub fn parse_ssh_config_hosts() -> Vec<String> {
    let home = std::env::var("HOME").ok().or_else(|| std::env::var("USERPROFILE").ok());
    let home_path = match home {
        Some(h) => PathBuf::from(h),
        None => return Vec::new(),
    };

    let main_config = home_path.join(".ssh").join("config");
    let mut files_to_read = vec![main_config];
    let mut hosts = Vec::new();
    let mut seen = HashSet::new();

    while let Some(config_path) = files_to_read.pop() {
        let content = match std::fs::read_to_string(&config_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('#') || trimmed.is_empty() {
                continue;
            }

            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.is_empty() {
                continue;
            }

            if parts[0].eq_ignore_ascii_case("Include") && parts.len() > 1 {
                for pattern in &parts[1..] {
                    let expanded = if let Some(stripped) = pattern.strip_prefix("~/") {
                        home_path.join(stripped)
                    } else if !pattern.starts_with('/') {
                        home_path.join(".ssh").join(pattern)
                    } else {
                        PathBuf::from(pattern)
                    };
                    if expanded.exists() && expanded.is_file() {
                        files_to_read.push(expanded);
                    }
                }
            } else if parts[0].eq_ignore_ascii_case("Host") {
                for pattern in &parts[1..] {
                    if pattern.contains('*') || pattern.contains('?') || pattern.starts_with('!') {
                        continue;
                    }
                    let host = pattern.trim();
                    let host_lower = host.to_lowercase();
                    if host_lower == "github.com"
                        || host_lower == "gitlab.com"
                        || host_lower == "bitbucket.org"
                        || host_lower == "gitee.com"
                    {
                        continue;
                    }
                    if !host.is_empty() && seen.insert(host_lower) {
                        hosts.push(host.to_string());
                    }
                }
            }
        }
    }

    hosts
}

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::{Terminal, TerminalOptions, Viewport};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::stdout;
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

pub const KEY_STYLE: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
pub const DIM_STYLE: Style = Style::new().add_modifier(Modifier::DIM);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum HostSource {
    Tailscale, // lowest (1)
    Ssh,       // middle (2)
    User,      // highest (3)
}

impl HostSource {
    pub fn priority(&self) -> u8 {
        match self {
            HostSource::User => 3,
            HostSource::Ssh => 2,
            HostSource::Tailscale => 1,
        }
    }

    pub fn letter(&self) -> char {
        match self {
            HostSource::User => 'U',
            HostSource::Ssh => 'S',
            HostSource::Tailscale => 'T',
        }
    }

    pub fn badge_style(&self) -> Style {
        match self {
            HostSource::User => Style::default().fg(Color::Black).bg(Color::Magenta).add_modifier(Modifier::BOLD),
            HostSource::Ssh => Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD),
            HostSource::Tailscale => Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Peer {
    pub hostname: String,
    pub os: String,
    pub online: bool,
    pub dns: String,
    pub reverse_port: Option<u16>,
    pub is_custom: bool,
    pub source: HostSource,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Config {
    pub hosts: Option<Vec<String>>,
    pub order: Option<Vec<String>>,
    pub reverse_port: Option<u16>,
}

impl Config {
    pub fn host_list(&self) -> Vec<String> {
        self.hosts.clone().or_else(|| self.order.clone()).unwrap_or_default()
    }
}

pub fn config_path() -> PathBuf {
    std::env::var("HOME")
        .map(|h| PathBuf::from(h).join(".config/hrzsz.toml"))
        .unwrap_or_else(|_| PathBuf::from(".hrzsz.toml"))
}

pub fn load_config() -> Config {
    std::fs::read_to_string(config_path())
        .ok()
        .and_then(|c| toml::from_str(&c).ok())
        .unwrap_or_default()
}

pub fn get_reverse_port() -> u16 {
    load_config().reverse_port.unwrap_or_else(|| {
        std::env::var("HSZRZ_REVERSE_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(18901)
    })
}

#[inline]
pub fn check_reverse_tunnel(port: u16) -> bool {
    let timeout = Duration::from_millis(30);
    let addr_v4 = SocketAddr::from(([127, 0, 0, 1], port));
    if TcpStream::connect_timeout(&addr_v4, timeout).is_ok() {
        return true;
    }
    let addr_v6 = SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr_v6, timeout).is_ok()
}

pub fn resolve_target(name: &str) -> (String, Option<u16>) {
    if let Some(port_str) = name.strip_prefix(':') {
        if let Ok(p) = port_str.parse::<u16>() {
            return ("localhost".to_string(), Some(p));
        }
    }
    if let Some((host, port_str)) = name.split_once(':') {
        if let Ok(p) = port_str.parse::<u16>() {
            return (host.to_string(), Some(p));
        }
    }
    (name.to_string(), None)
}

pub fn ssh_cmd(host: &str, port: Option<u16>) -> Command {
    let mut cmd = Command::new("ssh");
    if let Some(p) = port {
        cmd.args([
            "-p",
            &p.to_string(),
            "-o",
            "StrictHostKeyChecking=no",
            "-o",
            "UserKnownHostsFile=/dev/null",
            "-o",
            "LogLevel=ERROR",
        ]);
    }
    cmd.arg(host);
    cmd
}

pub fn save_config_hosts(visible_peers: &[Peer], removed_set: &HashSet<String>) {
    let p = config_path();
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut cfg = load_config();

    let mut list = Vec::new();
    for peer in visible_peers {
        if peer.reverse_port.is_some() || peer.hostname.starts_with(':') {
            continue;
        }
        if peer.is_custom {
            list.push(format!("+{}", peer.hostname));
        } else {
            list.push(peer.hostname.clone());
        }
    }
    for r in removed_set {
        list.push(format!("-{}", r));
    }

    cfg.hosts = Some(list);
    cfg.order = None;

    if let Ok(s) = toml::to_string_pretty(&cfg) {
        let _ = std::fs::write(p, s);
    }
}

#[derive(Deserialize)]
struct TsStatus {
    #[serde(rename = "Self")]
    self_node: Option<TsSelf>,
    #[serde(rename = "Peer")]
    peers: Option<HashMap<String, TsPeer>>,
}

#[derive(Deserialize)]
struct TsSelf {
    #[serde(rename = "HostName")]
    hostname: Option<String>,
    #[serde(rename = "DNSName")]
    dns_name: Option<String>,
}

#[derive(Deserialize)]
struct TsPeer {
    #[serde(rename = "HostName")]
    hostname: Option<String>,
    #[serde(rename = "OS")]
    os: Option<String>,
    #[serde(rename = "Online")]
    online: Option<bool>,
    #[serde(rename = "DNSName")]
    dns_name: Option<String>,
}

pub fn get_tailscale_peers() -> (Vec<Peer>, HashSet<String>) {
    let output = Command::new("tailscale").args(["status", "--json"]).output();
    let stdout = match output {
        Ok(o) if o.status.success() => o.stdout,
        _ => return (Vec::new(), HashSet::new()),
    };

    let status: TsStatus = match serde_json::from_slice(&stdout) {
        Ok(s) => s,
        Err(_) => return (Vec::new(), HashSet::new()),
    };

    let self_host = status.self_node.as_ref().and_then(|s| s.hostname.clone()).unwrap_or_default().to_lowercase();
    let self_dns = status.self_node.as_ref().and_then(|s| s.dns_name.clone()).map(|s| s.split('.').next().unwrap_or("").to_lowercase()).unwrap_or_default();

    let mut ts_peers = Vec::new();
    if let Some(peer_map) = status.peers {
        for (_, p) in peer_map {
            let hname = p.hostname.unwrap_or_default().trim().to_string();
            let dns = p.dns_name.map(|d| d.split('.').next().unwrap_or("").trim().to_string()).unwrap_or_default();
            let online = p.online.unwrap_or(false);
            let os = p.os.unwrap_or_else(|| "unknown".to_string());

            if !online || hname.is_empty() {
                continue;
            }

            let h_low = hname.to_lowercase();
            let d_low = dns.to_lowercase();
            if h_low == self_host || (!self_dns.is_empty() && (h_low == self_dns || d_low == self_dns)) {
                continue;
            }

            ts_peers.push(Peer {
                hostname: hname,
                os,
                online,
                dns,
                reverse_port: None,
                is_custom: false,
                source: HostSource::Tailscale,
            });
        }
    }

    let cfg = load_config();
    let mut removed_set = HashSet::new();
    let mut custom_added = Vec::new();
    let mut order_map = HashMap::new();

    for (idx, entry) in cfg.host_list().iter().enumerate() {
        let trimmed = entry.trim();
        if let Some(stripped) = trimmed.strip_prefix('-') {
            removed_set.insert(stripped.to_lowercase());
        } else if let Some(stripped) = trimmed.strip_prefix('+') {
            custom_added.push((idx, stripped.to_string()));
            order_map.insert(stripped.to_lowercase(), idx);
        } else {
            order_map.insert(trimmed.to_lowercase(), idx);
        }
    }

    let user_set: HashSet<String> = custom_added.iter().map(|(_, name)| name.to_lowercase()).collect();
    let ssh_hosts = parse_ssh_config_hosts();
    let ssh_set: HashSet<String> = ssh_hosts.iter().map(|h| h.to_lowercase()).collect();

    let mut peers: Vec<Peer> = Vec::new();

    // 1. Process Tailscale peers with U > S > T source precedence
    for mut p in ts_peers {
        let h_low = p.hostname.to_lowercase();
        let d_low = p.dns.to_lowercase();
        if removed_set.contains(&h_low) {
            continue;
        }

        if user_set.contains(&h_low) || (!d_low.is_empty() && user_set.contains(&d_low)) {
            p.source = HostSource::User;
            p.is_custom = true;
        } else if ssh_set.contains(&h_low) || (!d_low.is_empty() && ssh_set.contains(&d_low)) {
            p.source = HostSource::Ssh;
        }
        peers.push(p);
    }

    // 2. Add explicit User hosts not in Tailscale
    for (_idx, custom_name) in custom_added {
        if !peers.iter().any(|p| p.hostname.eq_ignore_ascii_case(&custom_name)) {
            peers.push(Peer {
                hostname: custom_name,
                os: "ssh".to_string(),
                online: true,
                dns: String::new(),
                reverse_port: None,
                is_custom: true,
                source: HostSource::User,
            });
        }
    }

    // 3. Add SSH config hosts not in Tailscale or User
    for ssh_name in ssh_hosts {
        let low = ssh_name.to_lowercase();
        if !removed_set.contains(&low)
            && !peers.iter().any(|p| {
                p.hostname.eq_ignore_ascii_case(&ssh_name) || (!p.dns.is_empty() && p.dns.eq_ignore_ascii_case(&ssh_name))
            })
        {
            let src = if user_set.contains(&low) { HostSource::User } else { HostSource::Ssh };
            peers.push(Peer {
                hostname: ssh_name,
                os: "ssh".to_string(),
                online: true,
                dns: String::new(),
                reverse_port: None,
                is_custom: true,
                source: src,
            });
        }
    }

    peers.sort_by(|a, b| {
        let pos_a = order_map.get(&a.hostname.to_lowercase());
        let pos_b = order_map.get(&b.hostname.to_lowercase());
        match (pos_a, pos_b) {
            (Some(ia), Some(ib)) => ia.cmp(ib),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => match b.source.priority().cmp(&a.source.priority()) {
                std::cmp::Ordering::Equal => a.hostname.to_lowercase().cmp(&b.hostname.to_lowercase()),
                other => other,
            },
        }
    });

    (peers, removed_set)
}

pub fn get_available_peers() -> (Vec<Peer>, HashSet<String>) {
    let (mut peers, removed_set) = get_tailscale_peers();
    let rev_port = get_reverse_port();

    if check_reverse_tunnel(rev_port) {
        peers.insert(
            0,
            Peer {
                hostname: format!(":{}", rev_port),
                os: "ssh".to_string(),
                online: true,
                dns: "127.0.0.1".to_string(),
                reverse_port: Some(rev_port),
                is_custom: false,
                source: HostSource::User,
            },
        );
    }
    (peers, removed_set)
}

fn centered_rect_fixed(width: u16, height: u16, r: Rect) -> Rect {
    let w = width.min(r.width.saturating_sub(2));
    let h = height.min(r.height);
    let x = r.x + (r.width.saturating_sub(w)) / 2;
    let y = r.y + (r.height.saturating_sub(h)) / 2;
    Rect { x, y, width: w, height: h }
}

enum AppMode {
    Normal,
    AddHost,
}

struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = crossterm::execute!(stdout(), crossterm::cursor::Show);
    }
}

pub fn pick_host_interactive(action_label: &str) -> Option<Peer> {
    let (mut peers, mut removed_set) = get_available_peers();
    if peers.is_empty() {
        eprintln!("Error: No reachable Tailscale peers or SSH reverse tunnels found.");
        return None;
    }

    if enable_raw_mode().is_err() {
        return None;
    }
    let stdout = stdout();
    let _guard = TerminalGuard;

    let inline_h = (peers.len().min(7) as u16 + 5).max(6);
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = match Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(inline_h),
        },
    ) {
        Ok(t) => t,
        Err(_) => match Terminal::new(CrosstermBackend::new(std::io::stdout())) {
            Ok(t) => t,
            Err(_) => return None,
        },
    };

    let mut selected_idx = 0;
    let mut chosen_peer: Option<Peer> = None;
    let mut mode = AppMode::Normal;
    let mut add_buffer = String::new();
    let mut last_reorder: Option<(char, Instant)> = None;
    let mut is_dirty = false;

    const VISIBLE_LIMIT: usize = 7;

    loop {
        let has_fixed_client = peers.first().map(|p| p.reverse_port.is_some()).unwrap_or(false);
        let num_items = peers.len();
        let visible_count = num_items.min(VISIBLE_LIMIT);

        if selected_idx >= num_items && num_items > 0 {
            selected_idx = num_items - 1;
        }

        // Center-pinned scrolling viewport: keep cursor in middle unless near start or end
        let half = VISIBLE_LIMIT / 2;
        let start_idx = (selected_idx.saturating_sub(half)).min(num_items.saturating_sub(visible_count));
        let end_idx = (start_idx + visible_count).min(num_items);

        let draw_res = terminal.draw(|f| {
            let size = f.area();
            let max_name_len = peers.iter().map(|p| p.hostname.len()).max().unwrap_or(12);
            let popup_w = (34.max(max_name_len as u16 + 14)).min(size.width.saturating_sub(4));
            let extra_lines = match mode {
                AppMode::AddHost => 5, // 1 input + 1 counter + 1 footer + 2 border
                AppMode::Normal => 4,  // 1 counter + 1 footer + 2 border
            };
            // Exact height without any extra line padding / blank rows
            let popup_h = (visible_count as u16 + extra_lines).min(size.height);
            let area = centered_rect_fixed(popup_w, popup_h, size);
            f.render_widget(Clear, area);

            // Clean title with action label only (no file/count info in title)
            let title = format!(" {} ", action_label);
            let block = Block::default()
                .title(title)
                .title_alignment(Alignment::Left)
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan));

            let inner = block.inner(area);
            f.render_widget(block, area);

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(match mode {
                    AppMode::AddHost => vec![
                        Constraint::Length(1),
                        Constraint::Length(visible_count as u16),
                        Constraint::Length(1),
                        Constraint::Length(1),
                    ],
                    AppMode::Normal => vec![
                        Constraint::Length(visible_count as u16),
                        Constraint::Length(1),
                        Constraint::Length(1),
                    ],
                })
                .split(inner);

            let (list_chunk, counter_chunk, footer_chunk) = match mode {
                AppMode::AddHost => {
                    let input_text = format!("  Add host: {}█", add_buffer);
                    let input_para = Paragraph::new(input_text).style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
                    f.render_widget(input_para, chunks[0]);
                    (chunks[1], chunks[2], chunks[3])
                }
                AppMode::Normal => (chunks[0], chunks[1], chunks[2]),
            };

            let mut col_lines = Vec::new();
            if peers.is_empty() {
                col_lines.push(Line::styled("  (No hosts. Press 'a' to add)", DIM_STYLE));
            } else {
                for (offset, p) in peers[start_idx..end_idx].iter().enumerate() {
                    let idx = start_idx + offset;
                    let is_selected = idx == selected_idx;
                    let prefix_span = if is_selected {
                        Span::styled("> ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
                    } else {
                        Span::raw("  ")
                    };

                    let badge_span = Span::styled(p.source.letter().to_string(), p.source.badge_style());
                    let space_span = Span::raw(" ");

                    let name_style = if is_selected {
                        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::Reset)
                    };
                    let name_span = Span::styled(&p.hostname, name_style);

                    col_lines.push(Line::from(vec![prefix_span, badge_span, space_span, name_span]));
                }
            }
            f.render_widget(Paragraph::new(col_lines).alignment(Alignment::Left), list_chunk);

            // Count display directly above key hints with parentheses, no internal digit padding
            let counter_line = Line::styled(
                format!("  ({}/{})", selected_idx + 1, num_items),
                Style::default().fg(Color::DarkGray),
            );
            f.render_widget(Paragraph::new(counter_line).alignment(Alignment::Left), counter_chunk);

            let footer_spans = match mode {
                AppMode::AddHost => vec![
                    Span::raw("  "),
                    Span::styled("enter", KEY_STYLE),
                    Span::styled(" confirm ", DIM_STYLE),
                    Span::styled("‧ ", DIM_STYLE),
                    Span::styled("esc", KEY_STYLE),
                    Span::styled(" cancel", DIM_STYLE),
                ],
                AppMode::Normal => vec![
                    Span::raw("  "),
                    Span::styled("[]", KEY_STYLE),
                    Span::styled(" reorder ", DIM_STYLE),
                    Span::styled("‧ ", DIM_STYLE),
                    Span::styled("a", KEY_STYLE),
                    Span::styled(" add ", DIM_STYLE),
                    Span::styled("‧ ", DIM_STYLE),
                    Span::styled("x", KEY_STYLE),
                    Span::styled(" delete", DIM_STYLE),
                ],
            };

            let footer = Paragraph::new(Line::from(footer_spans)).alignment(Alignment::Left);
            f.render_widget(footer, footer_chunk);
        });

        if draw_res.is_err() {
            break;
        }

        if let Ok(Event::Key(key)) = event::read() {
            if key.kind == KeyEventKind::Release {
                continue;
            }
            match mode {
                AppMode::AddHost => match key.code {
                    KeyCode::Esc => {
                        mode = AppMode::Normal;
                        add_buffer.clear();
                    }
                    KeyCode::Enter => {
                        let trimmed = add_buffer.trim().to_string();
                        if !trimmed.is_empty() {
                            removed_set.remove(&trimmed.to_lowercase());
                            peers.push(Peer {
                                hostname: trimmed,
                                os: "ssh".to_string(),
                                online: true,
                                dns: String::new(),
                                reverse_port: None,
                                is_custom: true,
                                source: HostSource::User,
                            });
                            selected_idx = peers.len() - 1;
                            is_dirty = true;
                        }
                        mode = AppMode::Normal;
                        add_buffer.clear();
                    }
                    KeyCode::Backspace => {
                        add_buffer.pop();
                    }
                    KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        add_buffer.push(c);
                    }
                    _ => {}
                },
                AppMode::Normal => match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => break,
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                    KeyCode::Enter => {
                        if !peers.is_empty() {
                            chosen_peer = Some(peers[selected_idx].clone());
                        }
                        break;
                    }
                    KeyCode::Char('a') => {
                        mode = AppMode::AddHost;
                        add_buffer.clear();
                    }
                    KeyCode::Char('x') => {
                        let is_client = peers.get(selected_idx).map(|p| p.reverse_port.is_some()).unwrap_or(false);
                        if !is_client && !peers.is_empty() {
                            let removed_peer = peers.remove(selected_idx);
                            removed_set.insert(removed_peer.hostname.to_lowercase());
                            if selected_idx >= peers.len() && !peers.is_empty() {
                                selected_idx = peers.len() - 1;
                            }
                            is_dirty = true;
                        }
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        selected_idx = if selected_idx > 0 { selected_idx - 1 } else { num_items.saturating_sub(1) };
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        selected_idx = if selected_idx + 1 < num_items { selected_idx + 1 } else { 0 };
                    }

                    KeyCode::Char('[') => {
                        let now = Instant::now();
                        let allow = match last_reorder {
                            Some(('[', prev)) => now.duration_since(prev) >= Duration::from_millis(200),
                            _ => true,
                        };
                        if allow {
                            let min_idx = if has_fixed_client { 2 } else { 1 };
                            if selected_idx >= min_idx {
                                peers.swap(selected_idx, selected_idx - 1);
                                selected_idx -= 1;
                                is_dirty = true;
                                last_reorder = Some(('[', now));
                            }
                        }
                    }
                    KeyCode::Char(']') => {
                        let now = Instant::now();
                        let allow = match last_reorder {
                            Some((']', prev)) => now.duration_since(prev) >= Duration::from_millis(200),
                            _ => true,
                        };
                        if allow {
                            let min_idx = if has_fixed_client { 1 } else { 0 };
                            if selected_idx >= min_idx && selected_idx + 1 < num_items {
                                peers.swap(selected_idx, selected_idx + 1);
                                selected_idx += 1;
                                is_dirty = true;
                                last_reorder = Some((']', now));
                            }
                        }
                    }
                    _ => {
                        last_reorder = None;
                    }
                },
            }
        }
    }

    let _ = terminal.clear();

    if is_dirty {
        save_config_hosts(&peers, &removed_set);
    }

    chosen_peer
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictMode {
    Overwrite,
    Rename,
}

pub fn split_stem_ext(filename: &str) -> (&str, &str) {
    if let Some(stem) = filename.strip_suffix(".tar.zst") {
        (stem, "tar.zst")
    } else if let Some(dot_idx) = filename.rfind('.') {
        if dot_idx > 0 {
            (&filename[..dot_idx], &filename[dot_idx + 1..])
        } else {
            (filename, "")
        }
    } else {
        (filename, "")
    }
}

pub fn get_next_available_name<F>(filename: &str, exists_check: F) -> String
where
    F: Fn(&str) -> bool,
{
    if !exists_check(filename) {
        return filename.to_string();
    }
    let (stem, ext) = split_stem_ext(filename);
    let mut seq = 1;
    loop {
        let candidate = if ext.is_empty() {
            format!("{}.{}", stem, seq)
        } else {
            format!("{}.{}.{}", stem, seq, ext)
        };
        if !exists_check(&candidate) {
            return candidate;
        }
        seq += 1;
    }
}

pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

pub fn prompt_conflict_resolution(filename: &str, size_bytes: u64, default_rename: bool) -> ConflictMode {
    if enable_raw_mode().is_err() {
        return if default_rename { ConflictMode::Rename } else { ConflictMode::Overwrite };
    }
    let stdout = stdout();
    let _guard = TerminalGuard;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = match Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(6),
        },
    ) {
        Ok(t) => t,
        Err(_) => match Terminal::new(CrosstermBackend::new(std::io::stdout())) {
            Ok(t) => t,
            Err(_) => return if default_rename { ConflictMode::Rename } else { ConflictMode::Overwrite },
        },
    };

    let (stem, ext) = split_stem_ext(filename);
    let preview_rename = if ext.is_empty() {
        format!("{}.1", stem)
    } else {
        format!("{}.1.{}", stem, ext)
    };

    let options = [
        "1. Overwrite".to_string(),
        format!("2. Rename (save as {})", preview_rename),
    ];

    let mut selected_idx: usize = if default_rename { 1 } else { 0 };

    loop {
        let draw_res = terminal.draw(|f| {
            let size = f.area();
            let popup_w = (54.max(filename.len() as u16 + 26)).min(size.width.saturating_sub(4));
            let popup_h = 6.min(size.height); // exactly 1 info + 2 options + 1 footer + 2 borders
            let area = centered_rect_fixed(popup_w, popup_h, size);
            f.render_widget(Clear, area);

            // Clean title: no file info in title
            let block = Block::default()
                .title(" File Conflict ")
                .title_alignment(Alignment::Left)
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Yellow));

            let inner = block.inner(area);
            f.render_widget(block, area);

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Length(2),
                    Constraint::Length(1),
                ])
                .split(inner);

            // File info inside body
            let info_text = if size_bytes > 0 {
                format!("  File exists: {} ({})", filename, format_size(size_bytes))
            } else {
                format!("  File exists: {}", filename)
            };
            f.render_widget(Paragraph::new(Line::styled(info_text, Style::default().fg(Color::White))), chunks[0]);

            let mut lines = Vec::new();
            for (idx, opt) in options.iter().enumerate() {
                let is_sel = idx == selected_idx;
                let prefix = if is_sel { "> " } else { "  " };
                let style = if is_sel {
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Reset)
                };
                lines.push(Line::styled(format!("{}{}", prefix, opt), style));
            }
            f.render_widget(Paragraph::new(lines), chunks[1]);

            let footer_spans = vec![
                Span::raw("  "),
                Span::styled("jk", KEY_STYLE),
                Span::styled(" select ", DIM_STYLE),
                Span::styled("‧ ", DIM_STYLE),
                Span::styled("enter", KEY_STYLE),
                Span::styled(" confirm", DIM_STYLE),
            ];
            f.render_widget(Paragraph::new(Line::from(footer_spans)), chunks[2]);
        });

        if draw_res.is_err() {
            break;
        }

        if let Ok(Event::Key(key)) = event::read() {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Up | KeyCode::Char('k') | KeyCode::Down | KeyCode::Char('j') => {
                    selected_idx = if selected_idx == 0 { 1 } else { 0 };
                }
                KeyCode::Enter => {
                    return if selected_idx == 0 {
                        ConflictMode::Overwrite
                    } else {
                        ConflictMode::Rename
                    };
                }
                KeyCode::Esc | KeyCode::Char('q') => {
                    return if default_rename { ConflictMode::Rename } else { ConflictMode::Overwrite };
                }
                _ => {}
            }
        }
    }

    if selected_idx == 0 {
        ConflictMode::Overwrite
    } else {
        ConflictMode::Rename
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_stem_ext() {
        assert_eq!(split_stem_ext("photo.png"), ("photo", "png"));
        assert_eq!(split_stem_ext("project.tar.zst"), ("project", "tar.zst"));
        assert_eq!(split_stem_ext("Makefile"), ("Makefile", ""));
        assert_eq!(split_stem_ext("archive.tar.gz"), ("archive.tar", "gz"));
    }


    #[test]
    fn test_parse_ssh_config_syntax() {
        let sample = "Host *\n  ServerAliveInterval 60\n\nHost nscc\n  HostName 10.0.0.1\n\nHost dev-1 dev-2\n  User root\n\nHost *.wildcard\n  Port 22\n";
        let mut hosts = Vec::new();
        for line in sample.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if !parts.is_empty() && parts[0].eq_ignore_ascii_case("Host") {
                for pat in &parts[1..] {
                    if !pat.contains('*') && !pat.contains('?') && !pat.starts_with('!') {
                        hosts.push(pat.to_string());
                    }
                }
            }
        }
        assert_eq!(hosts, vec!["nscc", "dev-1", "dev-2"]);
    }

    #[test]
    fn test_sequence_naming() {
        let mut existing = HashSet::new();
        existing.insert("photo.png".to_string());
        let next = get_next_available_name("photo.png", |f| existing.contains(f));
        assert_eq!(next, "photo.1.png");

        existing.insert("photo.1.png".to_string());
        let next2 = get_next_available_name("photo.png", |f| existing.contains(f));
        assert_eq!(next2, "photo.2.png");

        let mut existing_dir = HashSet::new();
        existing_dir.insert("project.tar.zst".to_string());
        let next_dir = get_next_available_name("project.tar.zst", |f| existing_dir.contains(f));
        assert_eq!(next_dir, "project.1.tar.zst");
    }
}
