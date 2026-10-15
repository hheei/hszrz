use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Terminal;
use serde::Deserialize;
use std::collections::HashMap;
use std::io::{self, stdout};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct Peer {
    pub hostname: String,
    pub os: String,
    pub online: bool,
    pub dns: String,
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
            });
        }
    }

    peers.sort_by(|a, b| a.hostname.to_lowercase().cmp(&b.hostname.to_lowercase()));
    peers
}

pub fn is_fuzzy_match(query: &str, target: &str) -> bool {
    let q = query.to_lowercase();
    let t = target.to_lowercase();
    if t.contains(&q) {
        return true;
    }
    let mut t_chars = t.chars();
    for qc in q.chars() {
        if !t_chars.any(|tc| tc == qc) {
            return false;
        }
    }
    true
}

// Helper to center a rect
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
    let all_peers = get_tailscale_peers();
    if all_peers.is_empty() {
        eprintln!("❌ No reachable online Tailscale peers found.");
        return None;
    }

    // Setup terminal
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
    let mut search_buf = String::new();
    let mut chosen_peer: Option<Peer> = None;

    loop {
        // Filter candidates based on search buffer
        let filtered: Vec<&Peer> = all_peers
            .iter()
            .filter(|p| {
                if search_buf.is_empty() {
                    true
                } else {
                    is_fuzzy_match(&search_buf, &p.hostname) || is_fuzzy_match(&search_buf, &p.os)
                }
            })
            .collect();

        if selected_idx >= filtered.len() && !filtered.is_empty() {
            selected_idx = filtered.len() - 1;
        }

        let num_items = filtered.len();
        let num_cols = if num_items > 5 { 2 } else { 1 };
        let num_rows = if num_cols > 0 && num_items > 0 {
            (num_items + num_cols - 1) / num_cols
        } else {
            1
        };

        let draw_res = terminal.draw(|f| {
            let size = f.area();
            let popup_w = if num_cols == 2 { 68 } else { 45 }.min(size.width.saturating_sub(4));
            let popup_h = (num_rows as u16 + 6).min(size.height.saturating_sub(4)).max(8);

            let percent_x = ((popup_w as f32 / size.width as f32) * 100.0).clamp(40.0, 95.0) as u16;
            let percent_y = ((popup_h as f32 / size.height as f32) * 100.0).clamp(30.0, 90.0) as u16;

            let area = centered_rect(percent_x, percent_y, size);
            f.render_widget(Clear, area);

            let title = format!(" 📡 {} ", action_label);
            let block = Block::default()
                .title(title)
                .title_alignment(Alignment::Center)
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan));

            let inner = block.inner(area);
            f.render_widget(block, area);

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),              // Search prompt
                    Constraint::Min(2),                 // Grid
                    Constraint::Length(1),              // Footer
                ])
                .split(inner);

            // Search bar
            let search_text = if search_buf.is_empty() {
                "  🔍 Type to filter, numbers to jump...".to_string()
            } else {
                format!("  🔍 Search: {}█", search_buf)
            };
            let search_style = if search_buf.is_empty() {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            };
            f.render_widget(Paragraph::new(search_text).style(search_style), chunks[0]);

            // Grid items
            if num_items == 0 {
                let empty_msg = Paragraph::new("No matching online machines.")
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(Color::Red));
                f.render_widget(empty_msg, chunks[1]);
            } else {
                let grid_chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints(if num_cols == 2 {
                        vec![Constraint::Percentage(50), Constraint::Percentage(50)]
                    } else {
                        vec![Constraint::Percentage(100)]
                    })
                    .split(chunks[1]);

                let pad_width = all_peers.len().to_string().len();

                for c in 0..num_cols {
                    let mut col_lines = Vec::new();
                    for r in 0..num_rows {
                        let idx = r + c * num_rows;
                        if idx < num_items {
                            let p = filtered[idx];
                            // Find absolute index in all_peers for display
                            let abs_idx = all_peers.iter().position(|x| x.hostname == p.hostname).unwrap_or(idx) + 1;
                            let is_selected = idx == selected_idx;
                            let prefix = if is_selected { "❯ " } else { "  " };
                            let label = format!("{}[{:>width$}] {} ({})", prefix, abs_idx, p.hostname, p.os, width = pad_width);

                            let style = if is_selected {
                                Style::default()
                                    .fg(Color::Black)
                                    .bg(Color::Cyan)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(Color::White)
                            };
                            col_lines.push(ratatui::text::Line::styled(label, style));
                        }
                    }
                    f.render_widget(Paragraph::new(col_lines), grid_chunks[c]);
                }
            }

            // Footer
            let footer = Paragraph::new(" [↑↓←→] Move  [Enter] Select  [Esc/q] Quit")
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::DarkGray));
            f.render_widget(footer, chunks[2]);
        });

        if draw_res.is_err() {
            break;
        }

        // Handle keys
        if let Ok(Event::Key(key)) = event::read() {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    break;
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    break;
                }
                KeyCode::Enter => {
                    if !filtered.is_empty() {
                        chosen_peer = Some(filtered[selected_idx].clone());
                    }
                    break;
                }
                KeyCode::Up => {
                    if selected_idx > 0 {
                        selected_idx -= 1;
                    } else if !filtered.is_empty() {
                        selected_idx = filtered.len() - 1;
                    }
                }
                KeyCode::Down => {
                    if !filtered.is_empty() && selected_idx + 1 < filtered.len() {
                        selected_idx += 1;
                    } else {
                        selected_idx = 0;
                    }
                }
                KeyCode::Left => {
                    if num_cols == 2 && selected_idx >= num_rows {
                        selected_idx -= num_rows;
                    }
                }
                KeyCode::Right => {
                    if num_cols == 2 && selected_idx + num_rows < filtered.len() {
                        selected_idx += num_rows;
                    }
                }
                KeyCode::Backspace => {
                    if !search_buf.is_empty() {
                        search_buf.pop();
                        selected_idx = 0;
                    }
                }
                KeyCode::Char(c) if c.is_digit(10) => {
                    let digit = c.to_digit(10).unwrap_or(0) as usize;
                    if digit > 0 && digit <= all_peers.len() {
                        let target_peer = &all_peers[digit - 1];
                        if let Some(pos) = filtered.iter().position(|p| p.hostname == target_peer.hostname) {
                            selected_idx = pos;
                        }
                    }
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    search_buf.push(c);
                    selected_idx = 0;
                }
                _ => {}
            }
        }
    }

    // Clean terminal
    let _ = disable_raw_mode();
    let _ = crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    chosen_peer
}
