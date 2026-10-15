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
                println!("hrz — Receive/upload file from remote machine via native GUI picker");
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
            reverse_port: None,
            is_custom: false,
        },
        None => match pick_host_interactive("Select Source Machine To Upload From") {
            Some(p) => p,
            None => exit(0),
        },
    };

    println!("Waiting for file selection on {}...", peer.hostname);

    let is_client = peer.reverse_port.is_some() || peer.hostname.starts_with(':');
    let ssh_host = if is_client { "localhost".to_string() } else { peer.hostname.clone() };
    let port_str = peer.reverse_port.map(|p| p.to_string());

    // Detect remote OS if client or unknown
    let remote_os = if is_client || peer.os == "unknown" {
        detect_remote_os(&ssh_host, port_str.as_deref())
    } else {
        peer.os.to_lowercase()
    };

    let chosen_file = if remote_os.contains("darwin") || remote_os.contains("mac") {
        ask_file_macos(&ssh_host, port_str.as_deref())
    } else if remote_os.contains("win") || remote_os.contains("msys") || remote_os.contains("mingw") {
        ask_file_windows(&ssh_host, port_str.as_deref())
    } else {
        ask_file_linux(&ssh_host, port_str.as_deref())
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

    println!("Downloading '{}' from {} -> {}", base_name, peer.hostname, dest.display());

    let mut rsync = Command::new("rsync");
    rsync.arg("-ahP");
    if let Some(ref p) = port_str {
        rsync.arg("-e").arg(format!("ssh -p {} -o StrictHostKeyChecking=no", p));
    }
    rsync.arg(format!("{}:{}", ssh_host, file_path));
    rsync.arg(&dest);

    let success = match rsync.status() {
        Ok(s) if s.success() => true,
        _ => {
            let mut scp = Command::new("scp");
            scp.arg("-p");
            if let Some(ref p) = port_str {
                scp.args(["-P", p, "-o", "StrictHostKeyChecking=no"]);
            }
            scp.arg(format!("{}:{}", ssh_host, file_path));
            scp.arg(&dest);
            scp.status().map(|s| s.success()).unwrap_or(false)
        }
    };

    if success {
        println!("Received '{}'!", base_name);
    } else {
        eprintln!("Error: Download failed.");
        exit(1);
    }
}

fn detect_remote_os(host: &str, port: Option<&str>) -> String {
    let mut ssh = Command::new("ssh");
    if let Some(p) = port {
        ssh.args(["-p", p, "-o", "StrictHostKeyChecking=no"]);
    }
    ssh.arg(host).arg("uname -s");
    if let Ok(output) = ssh.output() {
        String::from_utf8_lossy(&output.stdout).trim().to_lowercase()
    } else {
        "unknown".to_string()
    }
}

fn ask_file_macos(host: &str, port: Option<&str>) -> Option<String> {
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
    let mut ssh = Command::new("ssh");
    if let Some(p) = port {
        ssh.args(["-p", p, "-o", "StrictHostKeyChecking=no"]);
    }
    ssh.arg(host).arg("python3").arg("-c").arg(py);
    let output = ssh.output().ok()?;
    let out = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if out.is_empty() || out == "CANCELED" {
        None
    } else {
        Some(out)
    }
}

fn ask_file_windows(host: &str, port: Option<&str>) -> Option<String> {
    let ps = r#"Add-Type -AssemblyName System.Windows.Forms; $f = New-Object System.Windows.Forms.OpenFileDialog; $f.Title = 'Select file to upload'; if ($f.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { Write-Output $f.FileName } else { Write-Output 'CANCELED' }"#;
    let mut ssh = Command::new("ssh");
    if let Some(p) = port {
        ssh.args(["-p", p, "-o", "StrictHostKeyChecking=no"]);
    }
    ssh.arg(host).arg(format!("powershell -Command \"{}\"", ps));
    let output = ssh.output().ok()?;
    let out = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if out.is_empty() || out.contains("CANCELED") {
        None
    } else {
        Some(out)
    }
}

fn ask_file_linux(host: &str, port: Option<&str>) -> Option<String> {
    let check = r#"if [ -n "$DISPLAY" ] || [ -n "$WAYLAND_DISPLAY" ]; then if command -v zenity >/dev/null; then zenity --file-selection 2>/dev/null; elif command -v kdialog >/dev/null; then kdialog --getopenfilename 2>/dev/null; else echo NO_PICKER; fi; else echo NO_DISPLAY; fi"#;
    let mut ssh = Command::new("ssh");
    if let Some(p) = port {
        ssh.args(["-p", p, "-o", "StrictHostKeyChecking=no"]);
    }
    ssh.arg(host).arg(check);
    let output = ssh.output().ok()?;
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

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    run_hrz(&args);
}
