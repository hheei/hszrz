use hszrz::{pick_host_interactive, resolve_target, ssh_cmd};
use std::env;
use std::path::{Path, PathBuf};
use std::process::{exit, Command};

struct TempCleanup(Vec<PathBuf>);
impl Drop for TempCleanup {
    fn drop(&mut self) {
        for p in &self.0 {
            let _ = std::fs::remove_file(p);
        }
    }
}

pub fn run(args: &[String]) {
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut target_host: Option<String> = None;
    let mut dest_dir = "~/Downloads/".to_string();
    let mut auto_open = false;
    let mut auto_reveal = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-t" | "--to" | "--host" => {
                if i + 1 < args.len() {
                    target_host = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "-d" | "--dest" => {
                if i + 1 < args.len() {
                    dest_dir = args[i + 1].clone();
                    i += 1;
                }
            }
            "-o" | "--open" => auto_open = true,
            "-r" | "--reveal" => auto_reveal = true,
            "-h" | "--help" => {
                println!("hsz — Send files/directories to target machine\n\nUsage: hsz [options] <files/dirs...>\n\nOptions:\n  -t, --to <host>   Target host (skips selection menu)\n  -d, --dest <dir>  Destination folder (default: ~/Downloads/)\n  -o, --open        Open file(s) on target after transfer\n  -r, --reveal      Reveal file(s) in target file manager");
                return;
            }
            arg if !arg.starts_with('-') => paths.push(PathBuf::from(arg)),
            _ => {}
        }
        i += 1;
    }

    if paths.is_empty() {
        eprintln!("Error: No files or directories specified.");
        exit(1);
    }

    for p in &paths {
        if !p.exists() {
            eprintln!("Error: '{}' not found.", p.display());
            exit(1);
        }
    }

    let (raw_host, reverse_port) = match target_host {
        Some(h) => (h, None),
        None => match pick_host_interactive("Select Destination Machine") {
            Some(p) => (p.hostname, p.reverse_port),
            None => exit(0),
        },
    };

    let (ssh_host, target_port) = if let Some(p) = reverse_port {
        ("localhost".to_string(), Some(p))
    } else {
        resolve_target(&raw_host)
    };

    let mut transfer_files: Vec<(PathBuf, String)> = Vec::new();
    let mut temp_guard = TempCleanup(Vec::new());

    for p in paths {
        let abs = p.canonicalize().unwrap_or(p.clone());
        let fname = abs.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "transfer".to_string());

        if abs.is_dir() {
            println!("Packaging directory '{}' with zstd...", fname);
            let tmp_archive = env::temp_dir().join(format!("{}.tar.zst", fname));
            let parent = abs.parent().unwrap_or(Path::new("."));

            let status = Command::new("tar")
                .arg("-C")
                .arg(parent)
                .arg("--zstd")
                .arg("-cf")
                .arg(&tmp_archive)
                .arg(&fname)
                .status();

            if status.map(|s| s.success()).unwrap_or(false) {
                temp_guard.0.push(tmp_archive.clone());
                transfer_files.push((tmp_archive, format!("{}.tar.zst", fname)));
            } else {
                eprintln!("Failed to compress directory '{}'", fname);
                exit(1);
            }
        } else {
            transfer_files.push((abs, fname));
        }
    }

    println!("Sending to {}:{}", raw_host, dest_dir);
    let mut rsync = Command::new("rsync");
    rsync.arg("-ahP");
    if let Some(p) = target_port {
        rsync.arg("-e").arg(format!("ssh -p {} -o StrictHostKeyChecking=no", p));
    }
    for (src, _) in &transfer_files {
        rsync.arg(src);
    }
    rsync.arg(format!("{}:{}", ssh_host, dest_dir));

    let success = match rsync.status() {
        Ok(s) if s.success() => true,
        _ => {
            let mut scp = Command::new("scp");
            scp.arg("-p");
            if let Some(p) = target_port {
                scp.args(["-P", &p.to_string(), "-o", "StrictHostKeyChecking=no"]);
            }
            for (src, _) in &transfer_files {
                scp.arg(src);
            }
            scp.arg(format!("{}:{}", ssh_host, dest_dir));
            scp.status().map(|s| s.success()).unwrap_or(false)
        }
    };

    if success {
        println!("Done: Transfer complete.");
        if auto_open || auto_reveal {
            for (_, remote_name) in transfer_files {
                let full = format!("{}/{}", dest_dir.trim_end_matches('/'), remote_name);
                let flag = if auto_reveal { "-R" } else { "" };
                let remote_script = format!("open {} '{}' 2>/dev/null || xdg-open '{}' 2>/dev/null", flag, full, full);
                let _ = ssh_cmd(&ssh_host, target_port).arg(remote_script).status();
            }
        }
    } else {
        eprintln!("Error: Transfer failed.");
        exit(1);
    }
}
