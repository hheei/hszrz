use std::env;

mod send;
mod recv;

fn main() {
    let args: Vec<String> = env::args().collect();
    let raw_bin = env::args()
        .next()
        .map(|s| {
            std::path::Path::new(&s)
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or(s)
        })
        .unwrap_or_else(|| "hszrz".to_string());

    let bin_name = raw_bin
        .strip_suffix(".exe")
        .unwrap_or(&raw_bin);

    if bin_name == "hsz" {
        send::run(&args[1..]);
        return;
    }
    if bin_name == "hrz" {
        recv::run(&args[1..]);
        return;
    }

    if args.len() < 2 {
        print_help();
        return;
    }

    let subcmd = &args[1];
    let rest = &args[2..];

    match subcmd.as_str() {
        "send" | "s" | "sz" => send::run(rest),
        "recv" | "r" | "rz" => recv::run(rest),
        "hosts" | "list" | "ls" => {
            let (peers, _) = hszrz::get_tailscale_peers();
            println!("Available Hosts ({} total, Tailscale + ~/.ssh/config):", peers.len());
            for p in peers {
                let src_desc = match p.source {
                    hszrz::HostSource::User => "user",
                    hszrz::HostSource::Ssh => "ssh",
                    hszrz::HostSource::Tailscale => "tailscale",
                };
                println!("  - [{}] {:<18} ({}, {})", p.source.letter(), p.hostname, p.os, src_desc);
            }
        }
        "help" | "-h" | "--help" => print_help(),
        _ => {
            if subcmd.starts_with('-') || std::path::Path::new(subcmd).exists() {
                send::run(&args[1..]);
            } else {
                eprintln!("Unknown subcommand: '{}'", subcmd);
                print_help();
                std::process::exit(1);
            }
        }
    }
}

fn print_help() {
    println!("hszrz — Tailscale-native terminal file transfer for modern workspaces");
    println!("\nUsage:");
    println!("  hszrz send [options] <files...>   (alias: hsz)");
    println!("  hszrz recv [options]              (alias: hrz)");
    println!("  hszrz hosts                       List online Tailscale machines");
    println!("\nExamples:");
    println!("  hsz ./my-dir/                     Package dir with zstd & send");
    println!("  hsz file.html -o                  Send to Mac Downloads and open in browser");
    println!("  hrz                               Pop up native file picker on selected machine");
}
