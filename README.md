# hszrz (Hybrid sz/rz for Tailscale & Herdr)

Fast, zero-cognitive-burden terminal file transfer across Tailscale-connected machines. Built with modern AI coding agent workspaces (such as **Herdr**, **Ghostty**, and terminal multiplexers) in mind.

---

## ✨ Features

- **Zero-default, Self-Aware Peer Discovery**: Automatically reads `tailscale status` in real-time, excludes the local host, and presents only currently reachable online nodes.
- **Adaptive Multi-Column Selection**: Auto-formats into 2–4 columns when nodes > 5 with sub-sequence fuzzy search (`okr` -> `oracle-kr`, `mac` -> `mac`).
- **Instant Cancel**: Hit `Esc` anytime to cleanly abort.
- **Auto zstd Compression**: Folders are packaged into `.tar.zst` on-the-fly and cleaned up post-transfer.
- **In-Band Native OS Pickers (`hrz`)**: Triggers native GUI file choosers (AppleScript on macOS, PowerShell on Windows, Zenity on Linux) without needing terminal-level escape sequence interception.
- **Single / Multi-Call Binary**: Run `hszrz send`, `hszrz recv`, or use standalone `hsz` and `hrz`.

---

## 🚀 Quick Install (1-Line)

```bash
curl -fsSL https://raw.githubusercontent.com/hheei/hszrz/main/install.sh | bash
```

Or compile locally with Cargo:
```bash
cargo install --path .
```

---

## 🛠 Usage

### 1. Send files / folders to another machine (`hsz`)
```bash
hsz ./project/              # Auto-packs to project.tar.zst and delivers to ~/Downloads/
hsz report.html -o          # Sends and automatically opens in target browser
hsz -t mac notes.md         # Direct delivery to 'mac' (skips interactive menu)
```

### 2. Upload / receive files from another machine (`hrz`)
```bash
hrz                         # Interactive host picker -> pops up native file dialog on target
hrz -t mac                  # Pops up Mac file chooser and downloads to current dir
```

### 3. Check online Tailscale peers
```bash
hszrz hosts
```

---

## 🔮 Future Herdr Roadmap
- [ ] **Herdr Pane RPC Hooks**: Detect active `HERDR_PANE` and drop files directly into agent staging (`/tmp/herdr-*`).
- [ ] **Modal Popup Shortcut**: Native binding for Herdr `type = "popup"` for instant overlay file management.
- [ ] **Clipboard Image Ingest**: Bridge local clipboard image to remote Herdr agent session across non-remote TTYs.
