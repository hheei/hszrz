use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Terminal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::stdout;
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Peer {
    pub hostname: String,
    pub os: String,
    pub online: bool,
    pub dns: String,
    pub reverse_port: Option<u16>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Config {
    pub order: Option<Vec<String>>,
    pub reverse_port: Option<u16>,
}

pub fn config_path() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".config/hrzsz.toml")
    } else {
        PathBuf::from(".hrzsz.toml")
    }
}

pub fn load_config() -> Config {
    let p = config_path();
    if let Ok(content) = std::fs::read_to_string(&p) {
        toml::from_str(&content).unwrap_or_default()
    } else {
        Config::default()
    }
}

pub fn get_reverse_port() -> u16 {
    let cfg = load_config();
    if let Some(p) = cfg.reverse_port {
        return p;
    }
    if let Ok(env_p) = std::env::var("HSZRZ_REVERSE_PORT") {
        if let Ok(p) = env_p.parse::<u16>() {
            return p;
        }
    }
    18901
}

pub fn check_reverse_tunnel(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(50)).is_ok()
}

pub fn save_config_order(order: &[String]) {
    let p = config_path();
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut cfg = load_config();
    // Exclude "client" from persisted order
    let clean_order: Vec<String> = order
        .iter()
        .filter(|h| !h.to_lowercase().starts_with("client"))
        .cloned()
        .collect();
    cfg.order = Some(clean_order);

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

pub fn get_tailscale_peers() -> Vec<Peer> {
    let output = Command::new("tailscale")
        .args(["status", "--json"])
        .output();

    let output = match output {
        Ok(o) if o.status.success() => o.stdout,
        _ => return Vec::new(),
    };

    let status: TsStatus = match serde_json::from_slice(&output) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let self_host = status
        .self_node
        .as_ref()
        .and_then(|s| s.hostname.clone())
        .unwrap_or_default()
        .to_lowercase();
    let self_dns = status
        .self_node
        .as_ref()
        .and_then(|s| s.dns_name.clone())
        .map(|s| s.split('.').next().unwrap_or("").to_lowercase())
        .unwrap_or_default();

    let mut peers = Vec::new();
    if let Some(peer_map) = status.peers {
        for (_, p) in peer_map {
            let hname = p.hostname.unwrap_or_default().trim().to_string();
            let dns = p
                .dns_name
                .map(|d| d.split('.').next().unwrap_or("").trim().to_string())
                .unwrap_or_default();
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

            peers.push(Peer {
                hostname: hname,
                os,
                online,
                dns,
                reverse_port: None,
            });
        }
    }

    let cfg = load_config();
    if let Some(order) = cfg.order {
        peers.sort_by(|a, b| {
            let pos_a = order.iter().position(|h| h.eq_ignore_ascii_case(&a.hostname));
            let pos_b = order.iter().position(|h| h.eq_ignore_ascii_case(&b.hostname));

            match (pos_a, pos_b) {
                (Some(ia), Some(ib)) => ia.cmp(&ib),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.hostname.to_lowercase().cmp(&b.hostname.to_lowercase()),
            }
        });
    } else {
        peers.sort_by(|a, b| a.hostname.to_lowercase().cmp(&b.hostname.to_lowercase()));
    }

    peers
}

pub fn get_available_peers() -> Vec<Peer> {
    let mut peers = get_tailscale_peers();
    let rev_port = get_reverse_port();

    if check_reverse_tunnel(rev_port) {
        peers.insert(
            0,
            Peer {
                hostname: format!("client {}", rev_port),
                os: "ssh".to_string(),
                online: true,
                dns: "127.0.0.1".to_string(),
                reverse_port: Some(rev_port),
            },
        );
    }
    peers
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

pub fn pick_host_interactive(action_label: &str) -> Option<Peer> {
    let mut peers = get_available_peers();
    if peers.is_empty() {
        eprintln!("Error: No reachable Tailscale peers or SSH reverse tunnels found.");
        return None;
    }

    let has_fixed_client = peers.first().map(|p| p.reverse_port.is_some()).unwrap_or(false);

    let _ = enable_raw_mode();
    let mut stdout = stdout();
    let _ = crossterm::execute!(stdout, EnterAlternateScreen);
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = match Terminal::new(backend) {
        Ok(t) => t,
        Err(_) => {
            let _ = disable_raw_mode();
            return None;
        }
    };

    let mut selected_idx = 0;
    let mut chosen_peer: Option<Peer> = None;

    let num_items = peers.len();
    let num_cols = if num_items > 5 { 2 } else { 1 };
    let num_rows = (num_items + num_cols - 1) / num_cols;

    loop {
        let draw_res = terminal.draw(|f| {
            let size = f.area();
            let popup_w = if num_cols == 2 { 52 } else { 32 }.min(size.width.saturating_sub(4));
            let popup_h = (num_rows as u16 + 4).min(size.height.saturating_sub(4)).max(6);

            let percent_x = ((popup_w as f32 / size.width as f32) * 100.0).clamp(35.0, 95.0) as u16;
            let percent_y = ((popup_h as f32 / size.height as f32) * 100.0).clamp(25.0, 90.0) as u16;

            let area = centered_rect(percent_x, percent_y, size);
            f.render_widget(Clear, area);

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
                .constraints([
                    Constraint::Min(num_rows as u16),
                    Constraint::Length(1),
                ])
                .split(inner);

            let grid_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(if num_cols == 2 {
                    vec![Constraint::Percentage(50), Constraint::Percentage(50)]
                } else {
                    vec![Constraint::Percentage(100)]
                })
                .split(chunks[0]);

            let pad_width = peers.len().to_string().len();

            for c in 0..num_cols {
                let mut col_lines = Vec::new();
                let col_width = (grid_chunks[c].width as usize).saturating_sub(1);
                for r in 0..num_rows {
                    let idx = r + c * num_rows;
                    if idx < num_items {
                        let p = &peers[idx];
                        let is_selected = idx == selected_idx;
                        let prefix = if is_selected { "> " } else { "  " };
                        let raw_label = format!("{}{:>width$}. {}", prefix, idx + 1, p.hostname, width = pad_width);
                        let padded_label = format!("{:<width$}", raw_label, width = col_width);

                        let style = if is_selected {
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::Reset)
                        };
                        col_lines.push(ratatui::text::Line::styled(padded_label, style));
                    }
                }
                f.render_widget(Paragraph::new(col_lines).alignment(Alignment::Left), grid_chunks[c]);
            }

            let key_style = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);
            let dim_style = Style::default().add_modifier(Modifier::DIM);

            let footer = Paragraph::new(Line::from(vec![
                Span::raw("  "),
                Span::styled("arrows", key_style),
                Span::styled(" navigate ", dim_style),
                Span::styled("‧ ", dim_style),
                Span::styled("[ ]", key_style),
                Span::styled(" reorder ", dim_style),
                Span::styled("‧ ", dim_style),
                Span::styled("enter", key_style),
                Span::styled(" select ", dim_style),
                Span::styled("‧ ", dim_style),
                Span::styled("esc", key_style),
                Span::styled(" quit", dim_style),
            ])).alignment(Alignment::Left);
            f.render_widget(footer, chunks[1]);
        });

        if draw_res.is_err() {
            break;
        }

        if let Ok(Event::Key(key)) = event::read() {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    break;
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    break;
                }
                KeyCode::Enter => {
                    chosen_peer = Some(peers[selected_idx].clone());
                    break;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if selected_idx > 0 {
                        selected_idx -= 1;
                    } else {
                        selected_idx = num_items - 1;
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if selected_idx + 1 < num_items {
                        selected_idx += 1;
                    } else {
                        selected_idx = 0;
                    }
                }
                KeyCode::Left | KeyCode::Char('h') => {
                    if num_cols == 2 && selected_idx >= num_rows {
                        selected_idx -= num_rows;
                    }
                }
                KeyCode::Right | KeyCode::Char('l') => {
                    if num_cols == 2 && selected_idx + num_rows < num_items {
                        selected_idx += num_rows;
                    }
                }
                KeyCode::Char('[') => {
                    // Cannot reorder client, and cannot move items above client
                    let min_reorder_idx = if has_fixed_client { 2 } else { 1 };
                    if selected_idx >= min_reorder_idx {
                        peers.swap(selected_idx, selected_idx - 1);
                        selected_idx -= 1;
                        let order: Vec<String> = peers.iter().map(|p| p.hostname.clone()).collect();
                        save_config_order(&order);
                    }
                }
                KeyCode::Char(']') => {
                    // Cannot reorder client
                    let min_reorder_idx = if has_fixed_client { 1 } else { 0 };
                    if selected_idx >= min_reorder_idx && selected_idx + 1 < num_items {
                        peers.swap(selected_idx, selected_idx + 1);
                        selected_idx += 1;
                        let order: Vec<String> = peers.iter().map(|p| p.hostname.clone()).collect();
                        save_config_order(&order);
                    }
                }
                _ => {}
            }
        }
    }

    let _ = disable_raw_mode();
    let _ = crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    chosen_peer
}
