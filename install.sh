#!/usr/bin/env bash
set -euo pipefail

# hszrz installer — Tailscale-native terminal file transfer
REPO="${HSZRZ_REPO:-hheei/hszrz}"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"

mkdir -p "$INSTALL_DIR"

OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
    Darwin)
        case "$ARCH" in
            arm64|aarch64) TARGET="aarch64-apple-darwin" ;;
            x86_64) TARGET="x86_64-apple-darwin" ;;
            *) echo "Unsupported architecture: $ARCH" >&2; exit 1 ;;
        esac
        ;;
    Linux)
        case "$ARCH" in
            x86_64) TARGET="x86_64-unknown-linux-musl" ;;
            aarch64|arm64) TARGET="aarch64-unknown-linux-musl" ;;
            *) echo "Unsupported architecture: $ARCH" >&2; exit 1 ;;
        esac
        ;;
    *)
        echo "Unsupported OS: $OS (For Windows, download zip from GitHub releases)" >&2
        exit 1
        ;;
esac

echo "📡 Fetching latest release for $TARGET..."
RELEASE_JSON="$(curl -sL "https://api.github.com/repos/$REPO/releases/latest")"
DOWNLOAD_URL="$(echo "$RELEASE_JSON" | grep "browser_download_url.*hszrz-$TARGET.tar.gz" | cut -d '"' -f 4)"

if [ -z "$DOWNLOAD_URL" ]; then
    echo "❌ Failed to find release archive for $TARGET." >&2
    exit 1
fi

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

echo "⬇️  Downloading $DOWNLOAD_URL..."
curl -sL "$DOWNLOAD_URL" -o "$TMP_DIR/hszrz.tar.gz"

echo "📦 Extracting to $INSTALL_DIR..."
tar -xzf "$TMP_DIR/hszrz.tar.gz" -C "$TMP_DIR"
install -m 755 "$TMP_DIR/hszrz" "$INSTALL_DIR/hszrz"
install -m 755 "$TMP_DIR/hsz" "$INSTALL_DIR/hsz"
install -m 755 "$TMP_DIR/hrz" "$INSTALL_DIR/hrz"

echo "✅ Installed successfully to $INSTALL_DIR:"
echo "   - hszrz (unified CLI)"
echo "   - hsz   (fast send to ~/Downloads/ with zstd)"
echo "   - hrz   (fast recv via native OS GUI file picker)"

if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
    echo "⚠️  Note: $INSTALL_DIR is not in your PATH. Add it to ~/.zshrc or ~/.bashrc."
fi
