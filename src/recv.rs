use hszrz::{
    get_next_available_name, pick_host_interactive, prompt_conflict_resolution, resolve_target,
    ssh_cmd, ConflictMode, Peer,
};
use std::env;
use std::path::{Path, PathBuf};
use std::process::{exit, Command};

fn query_remote_size(host: &str, port: Option<u16>, remote_path: &str) -> u64 {
    let script = format!(
        r#"stat -c%s "{}" 2>/dev/null || stat -f%z "{}" 2>/dev/null || wc -c < "{}" 2>/dev/null"#,
        remote_path, remote_path, remote_path
    );
    ssh_cmd(host, port)
        .arg(script)
        .output()
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .lines()
                .next()
                .and_then(|l| l.trim().parse::<u64>().ok())
        })
        .unwrap_or(0)
}

pub fn run(args: &[String]) {
    let mut target_host: Option<String> = None;
    let mut dest_dir: Option<PathBuf> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-t" | "--from" | "--host" => {
                if i + 1 < args.len() {
                    target_host = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "-d" | "--dest" => {
                if i + 1 < args.len() {
                    dest_dir = Some(PathBuf::from(&args[i + 1]));
                    i += 1;
                }
            }
            "-h" | "--help" => {
                println!("hrz — Receive file from remote machine via native GUI picker\n\nUsage: hrz [options]\n\nOptions:\n  -t, --from <host>   Source host (skips selection menu)\n  -d, --dest <dir>    Destination directory (default: current directory)");
                return;
            }
            _ => {}
        }
        i += 1;
    }

    let peer = match target_host {
        Some(h) => Peer {
            hostname: h,
            os: "unknown".to_string(),
            online: true,
            dns: String::new(),
            reverse_port: None,
            is_custom: false,
            source: hszrz::HostSource::User,
        },
        None => match pick_host_interactive("Source") {
            Some(p) => p,
            None => exit(0),
        },
    };

    println!("Waiting for file selection on {}...", peer.hostname);

    let (ssh_host, target_port) = if let Some(p) = peer.reverse_port {
        ("localhost".to_string(), Some(p))
    } else {
        resolve_target(&peer.hostname)
    };

    let remote_os = if peer.reverse_port.is_some() || peer.os == "unknown" {
        detect_remote_os(&ssh_host, target_port)
    } else {
        peer.os.to_lowercase()
    };

    let chosen_file = if remote_os.contains("darwin") || remote_os.contains("mac") {
        ask_file_macos(&ssh_host, target_port)
    } else if remote_os.contains("win") || remote_os.contains("msys") || remote_os.contains("mingw") {
        ask_file_windows(&ssh_host, target_port)
    } else {
        ask_file_linux(&ssh_host, target_port)
    };

    let file_path = match chosen_file {
        Some(f) if !f.is_empty() && f != "CANCELED" => f,
        _ => {
            println!("Upload cancelled.");
            exit(0);
        }
    };

    let base_name = Path::new(&file_path)
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| "downloaded_file".to_string());

    let target_folder = dest_dir
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let initial_dest = target_folder.join(&base_name);
    let remote_size = query_remote_size(&ssh_host, target_port, &file_path);
    let is_dir_zstd = base_name.ends_with(".tar.zst");

    let final_dest = if initial_dest.exists() {
        let mode = prompt_conflict_resolution(&base_name, remote_size, is_dir_zstd);
        match mode {
            ConflictMode::Overwrite => initial_dest,
            ConflictMode::Rename => {
                let available_name = get_next_available_name(&base_name, |cand| target_folder.join(cand).exists());
                target_folder.join(available_name)
            }
        }
    } else {
        initial_dest
    };

    let display_name = final_dest.file_name().unwrap_or_default().to_string_lossy();
    println!("Downloading '{}' from {} -> {}", display_name, peer.hostname, final_dest.display());

    let mut rsync = Command::new("rsync");
    rsync.arg("-ahPs"); // -s / --protect-args to handle spaces safely
    if let Some(p) = target_port {
        rsync.arg("-e").arg(format!(
            "ssh -p {} -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR",
            p
        ));
    }
    rsync.arg(format!("{}:{}", ssh_host, file_path));
    rsync.arg(&final_dest);

    let success = match rsync.status() {
        Ok(s) if s.success() => true,
        _ => {
            let mut scp = Command::new("scp");
            scp.arg("-p");
            if let Some(p) = target_port {
                scp.args([
                    "-P",
                    &p.to_string(),
                    "-o",
                    "StrictHostKeyChecking=no",
                    "-o",
                    "UserKnownHostsFile=/dev/null",
                    "-o",
                    "LogLevel=ERROR",
                ]);
            }
            scp.arg(format!("{}:{}", ssh_host, file_path));
            scp.arg(&final_dest);
            scp.status().map(|s| s.success()).unwrap_or(false)
        }
    };

    if success {
        // Always update Date Added / modification time to now
        let _ = Command::new("touch").arg(&final_dest).status();
        let now = std::time::SystemTime::now();
        let _ = std::fs::File::open(&final_dest).and_then(|f| {
            f.set_times(std::fs::FileTimes::new().set_accessed(now).set_modified(now))
        });

        println!("Received '{}'!", display_name);
    } else {
        eprintln!("Error: Download failed.");
        exit(1);
    }
}

fn detect_remote_os(host: &str, port: Option<u16>) -> String {
    ssh_cmd(host, port)
        .arg("uname -s")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_lowercase())
        .unwrap_or_else(|_| "unknown".to_string())
}

fn ask_file_macos(host: &str, port: Option<u16>) -> Option<String> {
    let script = r#"try
tell application "System Events"
activate
set f to choose file with prompt "Select file to upload:" default location (path to downloads folder)
return POSIX path of f
end tell
on error number -128
return "CANCELED"
end try"#;
    let output = ssh_cmd(host, port).args(["osascript", "-e", script]).output().ok()?;
    let out = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if out.is_empty() || out == "CANCELED" { None } else { Some(out) }
}

fn ask_file_windows(host: &str, port: Option<u16>) -> Option<String> {
    let ps = r#"Add-Type -AssemblyName System.Windows.Forms; $f = New-Object System.Windows.Forms.OpenFileDialog; $f.Title = 'Select file to upload'; if ($f.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { Write-Output $f.FileName } else { Write-Output 'CANCELED' }"#;
    let output = ssh_cmd(host, port).args(["powershell", "-Sta", "-Command", ps]).output().ok()?;
    let out = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if out.is_empty() || out.contains("CANCELED") { None } else { Some(out) }
}

fn ask_file_linux(host: &str, port: Option<u16>) -> Option<String> {
    let check = r#"if [ -n "$DISPLAY" ] || [ -n "$WAYLAND_DISPLAY" ]; then if command -v zenity >/dev/null; then zenity --file-selection 2>/dev/null; elif command -v kdialog >/dev/null; then kdialog --getopenfilename 2>/dev/null; else echo NO_PICKER; fi; else echo NO_DISPLAY; fi"#;
    let output = ssh_cmd(host, port).arg(check).output().ok()?;
    let out = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if out == "NO_DISPLAY" {
        eprintln!("Error: Host '{}' (Linux) is a headless server without a desktop GUI environment.", host);
        None
    } else if out == "NO_PICKER" {
        eprintln!("Error: Host '{}' (Linux) has a desktop session, but no GUI picker (zenity/kdialog) is installed.", host);
        None
    } else if out.is_empty() {
        None
    } else {
        Some(out)
    }
}
