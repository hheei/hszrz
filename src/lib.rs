use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use serde::Deserialize;
use std::collections::HashMap;
use std::io::{self, Write};
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

pub fn read_line_or_esc(prompt: &str) -> Option<String> {
    print!("{}", prompt);
    let _ = io::stdout().flush();

    if enable_raw_mode().is_err() {
        let mut line = String::new();
        if io::stdin().read_line(&mut line).is_ok() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed == "q" {
                return None;
            }
            return Some(trimmed.to_string());
        }
        return None;
    }

    let mut input = String::new();
    loop {
        if let Ok(Event::Key(key_event)) = event::read() {
            match key_event.code {
                KeyCode::Esc => {
                    let _ = disable_raw_mode();
                    print!("\r\n");
                    let _ = io::stdout().flush();
                    return None;
                }
                KeyCode::Char('c') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                    let _ = disable_raw_mode();
                    print!("\r\n");
                    let _ = io::stdout().flush();
                    return None;
                }
                KeyCode::Enter => {
                    let _ = disable_raw_mode();
                    print!("\r\n");
                    let _ = io::stdout().flush();
                    break;
                }
                KeyCode::Backspace => {
                    if !input.is_empty() {
                        input.pop();
                        print!("\x08 \x08");
                        let _ = io::stdout().flush();
                    }
                }
                KeyCode::Char(c) => {
                    input.push(c);
                    print!("{}", c);
                    let _ = io::stdout().flush();
                }
                _ => {}
            }
        }
    }

    let trimmed = input.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

pub fn format_multi_column(items: &[String]) -> Vec<String> {
    let n = items.len();
    if n <= 5 {
        return items.iter().map(|it| format!("  {}", it)).collect();
    }

    let (term_width, _) = crossterm::terminal::size().unwrap_or((80, 24));
    let term_width = term_width as usize;
    let max_len = items.iter().map(|it| it.len()).max().unwrap_or(20);
    let col_width = max_len + 4;
    let num_cols = (term_width / col_width).clamp(2, 4);
    let num_rows = (n + num_cols - 1) / num_cols;

    let mut lines = Vec::new();
    for r in 0..num_rows {
        let mut row_str = String::new();
        for c in 0..num_cols {
            let idx = r + c * num_rows;
            if idx < n {
                row_str.push_str(&format!("{:<width$}", items[idx], width = col_width));
            }
        }
        lines.push(format!("  {}", row_str.trim_end()));
    }
    lines
}

pub fn pick_host_interactive(action_label: &str) -> Option<Peer> {
    let all_peers = get_tailscale_peers();
    if all_peers.is_empty() {
        eprintln!("❌ No reachable online Tailscale peers found.");
        return None;
    }

    let mut candidates = all_peers.clone();
    let pad_width = all_peers.len().to_string().len();

    loop {
        let labels: Vec<String> = candidates
            .iter()
            .enumerate()
            .map(|(idx, p)| {
                format!(
                    "[{:>width$}] {} ({})",
                    idx + 1,
                    p.hostname,
                    p.os,
                    width = pad_width
                )
            })
            .collect();

        println!("\n📡 {} ({} available):", action_label, candidates.len());
        for line in format_multi_column(&labels) {
            println!("{}", line);
        }

        let input = read_line_or_esc("\n❯ Enter # or name (Esc to exit): ");
        let val = match input {
            Some(v) => v,
            None => {
                println!("Operation cancelled.");
                return None;
            }
        };

        if let Ok(num) = val.parse::<usize>() {
            if num >= 1 && num <= candidates.len() {
                return Some(candidates[num - 1].clone());
            }
            eprintln!("⚠️ Please enter a number between 1 and {}.", candidates.len());
            continue;
        }

        let matches: Vec<Peer> = all_peers
            .iter()
            .filter(|p| is_fuzzy_match(&val, &p.hostname))
            .cloned()
            .collect();

        if matches.len() == 1 {
            return Some(matches[0].clone());
        } else if matches.len() > 1 {
            candidates = matches;
        } else {
            eprintln!("⚠️ No hosts matching '{}'. Showing all available hosts.", val);
            candidates = all_peers.clone();
        }
    }
}
