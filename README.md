# hszrz (Hybrid sz/rz for Tailscale & Herdr)

Fast, zero-cognitive-burden terminal file transfer across Tailscale-connected machines. Built with modern AI coding agent workspaces (such as **Herdr**, **Ghostty**, and terminal multiplexers) in mind.

---

## ✨ Features

- **Zero-default Peer & Host Discovery**: Automatically discovers active nodes from `tailscale status` in real-time AND auto-detects valid host aliases from `~/.ssh/config`, with local host exclusions and smart deduplication.
- **Centered Scrollable Picker with Source Badges**: Clean single-column 7-item window with center-pinned scrolling, wrap-around navigation, (x/total) progress, and source badges ('U' user, 'S' ssh, 'T' tailscale).
- **Custom Peers & Reverse SSH Support**: Add (`a`) or remove (`x`) persistent custom nodes; automatically prioritizes SSH reverse tunnel endpoints (`:18901`).
- **Instant Cancel & Safe Terminal Recovery**: Hit `Esc` or `q` anytime to cleanly abort with guaranteed terminal raw mode restoration.
- **Multi-Threaded zstd Compression**: Folders are packaged into `.tar.zst` on-the-fly using up to 6 parallel threads and safely cleaned up post-transfer.
- **In-Band Native OS Pickers (`hrz`)**: Triggers native GUI file choosers (direct AppleScript on macOS, PowerShell on Windows, Zenity/KDialog on Linux) without external runtime dependencies like Python.
- **Single / Multi-Call Binary**: Run `hszrz send`, `hszrz recv`, or standalone `hsz` and `hrz`.

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
hsz ./project/              # Auto-packs to project_*.tar.zst with multi-threaded zstd & delivers to ~/Downloads/
hsz report.html -o          # Sends and automatically opens in target browser
hsz report.html -r          # Sends and reveals in target file manager
hsz -t mac notes.md         # Direct delivery to 'mac' (skips interactive menu)
```

### 2. Upload / receive files from another machine (`hrz`)
```bash
hrz                         # Interactive host picker -> pops up native file dialog on target
hrz -t mac                  # Pops up Mac file chooser and downloads to current dir
hrz -t mac -d ~/Desktop/    # Download to specified directory
```

### 3. Check online Tailscale peers
```bash
hszrz hosts
```

### 4. Yazi File Manager Integration
Add to `~/.config/yazi/keymap.toml` (see [examples/yazi/keymap.toml](examples/yazi/keymap.toml)):

```toml
[manager]
prepend_keymap = [
  # Ctrl+u: Upload/Send selected or hovered file(s) via hsz
  { on = "<C-u>", run = 'shell "hsz \"$@\"" --block', desc = "Upload (hsz send)" },

  # Ctrl+d: Download/Receive file into current directory via hrz
  { on = "<C-d>", run = 'shell "hrz" --block',        desc = "Download (hrz receive)" },
]
```

---

## 🔮 Future Herdr Roadmap
- [ ] **Herdr Pane RPC Hooks**: Detect active `HERDR_PANE` and drop files directly into agent staging (`/tmp/herdr-*`).
- [ ] **Modal Popup Shortcut**: Native binding for Herdr `type = "popup"` for instant overlay file management.
- [ ] **Clipboard Image Ingest**: Bridge local clipboard image to remote Herdr agent session across non-remote TTYs.
