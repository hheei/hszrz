use hszrz::{
    get_next_available_name, pick_host_interactive, prompt_conflict_resolution, resolve_target,
    split_stem_ext, ssh_cmd, ConflictMode,
};
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

fn check_remote_exists(host: &str, port: Option<u16>, remote_dir: &str, filename: &str) -> bool {
    let base_dir = if let Some(stripped) = remote_dir.strip_prefix("~/") {
        format!(r#""$HOME/{}""#, stripped.trim_end_matches('/'))
    } else {
        format!(r#""{}""#, remote_dir.trim_end_matches('/'))
    };
    let test_cmd = format!(r#"[ -e "{}/{}" ]"#, base_dir, filename);
    ssh_cmd(host, port)
        .arg(test_cmd)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn get_remote_available_name(host: &str, port: Option<u16>, remote_dir: &str, filename: &str) -> String {
    let (stem, ext) = split_stem_ext(filename);
    let base_dir = if let Some(stripped) = remote_dir.strip_prefix("~/") {
        format!(r#""$HOME/{}""#, stripped.trim_end_matches('/'))
    } else {
        format!(r#""{}""#, remote_dir.trim_end_matches('/'))
    };
    let script = format!(
        r#"d={}; s="{}"; e="{}"; seq=1; if [ -z "$e" ]; then cand="$s.$seq"; while [ -e "$d/$cand" ]; do seq=$((seq+1)); cand="$s.$seq"; done; else cand="$s.$seq.$e"; while [ -e "$d/$cand" ]; do seq=$((seq+1)); cand="$s.$seq.$e"; done; fi; echo "$cand""#,
        base_dir, stem, ext
    );
    ssh_cmd(host, port)
        .arg(script)
        .output()
        .ok()
        .and_then(|o| {
            let out = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if out.is_empty() { None } else { Some(out) }
        })
        .unwrap_or_else(|| get_next_available_name(filename, |_| false))
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
        None => match pick_host_interactive("Destination") {
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

    // Concurrent compression threads capped at 6
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .clamp(1, 6);

    for p in paths {
        let abs = p.canonicalize().unwrap_or(p.clone());
        let fname = abs.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "transfer".to_string());
        let is_dir = abs.is_dir();

        let (src_path, orig_name, file_size) = if is_dir {
            println!("Packaging directory '{}' with zstd (threads: {})...", fname, threads);
            let pid = std::process::id();
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            let tmp_archive = env::temp_dir().join(format!("{}_{}_{}.tar.zst", fname, pid, now));
            let parent = abs.parent().unwrap_or(Path::new("."));

            let compress_prog = format!("zstd -T{} -3", threads);
            let status = Command::new("tar")
                .arg("-C")
                .arg(parent)
                .arg("-I")
                .arg(&compress_prog)
                .arg("-cf")
                .arg(&tmp_archive)
                .arg(&fname)
                .status()
                .or_else(|_| {
                    Command::new("tar")
                        .arg("-C")
                        .arg(parent)
                        .arg("--zstd")
                        .arg("-cf")
                        .arg(&tmp_archive)
                        .arg(&fname)
                        .status()
                });

            if status.map(|s| s.success()).unwrap_or(false) {
                temp_guard.0.push(tmp_archive.clone());
                let sz = tmp_archive.metadata().map(|m| m.len()).unwrap_or(0);
                (tmp_archive, format!("{}.tar.zst", fname), sz)
            } else {
                eprintln!("Failed to compress directory '{}'", fname);
                exit(1);
            }
        } else {
            let sz = abs.metadata().map(|m| m.len()).unwrap_or(0);
            (abs, fname, sz)
        };

        let remote_name = if check_remote_exists(&ssh_host, target_port, &dest_dir, &orig_name) {
            let mode = prompt_conflict_resolution(&orig_name, file_size, is_dir);
            match mode {
                ConflictMode::Overwrite => orig_name,
                ConflictMode::Rename => get_remote_available_name(&ssh_host, target_port, &dest_dir, &orig_name),
            }
        } else {
            orig_name
        };

        // If filename changed for transfer, create temporary link/copy with desired name
        let final_src = if remote_name != src_path.file_name().unwrap_or_default().to_string_lossy() {
            let pid = std::process::id();
            let renamed_tmp = env::temp_dir().join(format!("{}_{}", pid, remote_name));
            let _ = std::fs::copy(&src_path, &renamed_tmp);
            temp_guard.0.push(renamed_tmp.clone());
            renamed_tmp
        } else {
            src_path
        };

        transfer_files.push((final_src, remote_name));
    }

    println!("Sending to {}:{}", raw_host, dest_dir);
    let mut rsync = Command::new("rsync");
    rsync.arg("-ahPs"); // -s / --protect-args to handle spaces safely
    if let Some(p) = target_port {
        rsync.arg("-e").arg(format!(
            "ssh -p {} -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR",
            p
        ));
    }
    for (src, _) in &transfer_files {
        rsync.arg(src);
    }
    let rsync_dest = dest_dir.strip_prefix("~/").unwrap_or(&dest_dir);
    rsync.arg(format!("{}:{}", ssh_host, rsync_dest));

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
            for (src, _) in &transfer_files {
                scp.arg(src);
            }
            scp.arg(format!("{}:{}", ssh_host, dest_dir));
            scp.status().map(|s| s.success()).unwrap_or(false)
        }
    };

    if success {
        let base_dir = if let Some(stripped) = dest_dir.strip_prefix("~/") {
            format!(r#""$HOME/{}""#, stripped.trim_end_matches('/'))
        } else {
            format!(r#""{}""#, dest_dir.trim_end_matches('/'))
        };

        // Always update Date Added / modification date on target
        for (_, remote_name) in &transfer_files {
            let escaped_name = remote_name.replace('"', "\\\"");
            let touch_cmd = format!(
                r#"target={}/"{}"; touch "$target" 2>/dev/null || powershell -Command "(Get-Item '$target').LastWriteTime = Get-Date" 2>/dev/null"#,
                base_dir, escaped_name
            );
            let _ = ssh_cmd(&ssh_host, target_port).arg(touch_cmd).status();
        }

        println!("Done: Transfer complete.");
        if auto_open || auto_reveal {
            for (_, remote_name) in transfer_files {
                let escaped_name = remote_name.replace('"', "\\\"");
                let remote_target = format!(r#"{}/"{}""#, base_dir, escaped_name);
                let flag = if auto_reveal { "-R" } else { "" };
                let remote_script = format!(
                    r#"target={}; open {} "$target" 2>/dev/null || xdg-open "$target" 2>/dev/null"#,
                    remote_target, flag
                );
                let _ = ssh_cmd(&ssh_host, target_port).arg(remote_script).status();
            }
        }
    } else {
        eprintln!("Error: Transfer failed.");
        exit(1);
    }
}
