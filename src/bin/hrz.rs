use hszrz::{pick_host_interactive, Peer};
use std::env;
use std::path::Path;
use std::process::{exit, Command};

pub fn run_hrz(args: &[String]) {
    let mut target_host: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-t" | "--from" | "--host" => {
                if i + 1 < args.len() {
                    target_host = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "-h" | "--help" => {
                println!("hrz — Receive/upload file from remote node via native GUI picker");
                println!("Usage: hrz [options]");
                println!("Options:");
                println!("  -t, --from <host>   Source host (skips selection menu)");
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
        },
        None => match pick_host_interactive("Select Source Machine To Upload From") {
            Some(p) => p,
            None => exit(0),
        },
    };

    println!("⏳ Waiting for file selection on {} ({})...", peer.hostname, peer.os);
    let os_low = peer.os.to_lowercase();
    let host_low = peer.hostname.to_lowercase();

    let chosen_file = if os_low.contains("mac") || host_low.contains("mac") {
        ask_file_macos(&peer.hostname)
    } else if os_low.contains("win") {
        ask_file_windows(&peer.hostname)
    } else if os_low.contains("linux") {
        ask_file_linux(&peer.hostname)
    } else {
        ask_file_macos(&peer.hostname)
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

    let dest = env::current_dir()
        .map(|d| d.join(&base_name))
        .unwrap_or_else(|_| Path::new(&base_name).to_path_buf());

    println!("🚀 Downloading '{}' from {} -> {}", base_name, peer.hostname, dest.display());

    let rsync_status = Command::new("rsync")
        .arg("-ahP")
        .arg(format!("{}:{}", peer.hostname, file_path))
        .arg(&dest)
        .status();

    let success = match rsync_status {
        Ok(s) if s.success() => true,
        _ => {
            Command::new("scp")
                .arg("-p")
                .arg(format!("{}:{}", peer.hostname, file_path))
                .arg(&dest)
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        }
    };

    if success {
        println!("✅ Received '{}'!", base_name);
    } else {
        eprintln!("❌ Download failed.");
        exit(1);
    }
}

fn ask_file_macos(host: &str) -> Option<String> {
    let script = r#"
try
    tell application "System Events"
        activate
        set f to choose file with prompt "Select file to upload:" default location (path to downloads folder)
        return POSIX path of f
    end tell
on error number -128
    return "CANCELED"
end try
"#;
    let py = format!("import subprocess; print(subprocess.run(['osascript', '-e', '''{}'''], capture_output=True, text=True).stdout.strip())", script);
    let output = Command::new("ssh")
        .arg(host)
        .arg("python3")
        .arg("-c")
        .arg(py)
        .output()
        .ok()?;
    let out = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if out.is_empty() || out == "CANCELED" {
        None
    } else {
        Some(out)
    }
}

fn ask_file_windows(host: &str) -> Option<String> {
    let ps = r#"Add-Type -AssemblyName System.Windows.Forms; $f = New-Object System.Windows.Forms.OpenFileDialog; $f.Title = 'Select file to upload'; if ($f.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { Write-Output $f.FileName } else { Write-Output 'CANCELED' }"#;
    let output = Command::new("ssh")
        .arg(host)
        .arg(format!("powershell -Command \"{}\"", ps))
        .output()
        .ok()?;
    let out = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if out.is_empty() || out.contains("CANCELED") {
        None
    } else {
        Some(out)
    }
}

fn ask_file_linux(host: &str) -> Option<String> {
    let check = r#"if [ -n "$DISPLAY" ] || [ -n "$WAYLAND_DISPLAY" ]; then if command -v zenity >/dev/null; then zenity --file-selection 2>/dev/null; elif command -v kdialog >/dev/null; then kdialog --getopenfilename 2>/dev/null; else echo NO_PICKER; fi; else echo NO_DISPLAY; fi"#;
    let output = Command::new("ssh").arg(host).arg(check).output().ok()?;
    let out = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if out == "NO_DISPLAY" {
        eprintln!("❌ Host '{}' (Linux) is a headless server without a desktop GUI environment.", host);
        None
    } else if out == "NO_PICKER" {
        eprintln!("❌ Host '{}' (Linux) has a desktop session, but no GUI picker (zenity/kdialog) is installed.", host);
        None
    } else if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    run_hrz(&args);
}
