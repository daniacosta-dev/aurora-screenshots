#!/bin/bash
set -e

BINARY_NAME="aurora-screenshots"
INSTALL_DIR="$HOME/.local/bin"
ICON_DIR="$HOME/.local/share/icons/hicolor"
DESKTOP_DIR="$HOME/.local/share/applications"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

echo "Building release binary..."
cargo build --release --manifest-path "$PROJECT_DIR/src-tauri/Cargo.toml"

echo "Installing binary..."
mkdir -p "$INSTALL_DIR"
cp "$PROJECT_DIR/src-tauri/target/release/$BINARY_NAME" "$INSTALL_DIR/"
chmod +x "$INSTALL_DIR/$BINARY_NAME"

echo "Installing icons..."
mkdir -p "$ICON_DIR/32x32/apps"
mkdir -p "$ICON_DIR/128x128/apps"
mkdir -p "$ICON_DIR/256x256/apps"
mkdir -p "$ICON_DIR/scalable/apps"
cp "$PROJECT_DIR/src-tauri/icons/32x32.png"   "$ICON_DIR/32x32/apps/$BINARY_NAME.png"
cp "$PROJECT_DIR/src-tauri/icons/128x128.png"  "$ICON_DIR/128x128/apps/$BINARY_NAME.png"
cp "$PROJECT_DIR/src-tauri/icons/icon.png"     "$ICON_DIR/256x256/apps/$BINARY_NAME.png"
cp "$PROJECT_DIR/public/aurora-screenshots-icon.svg" "$ICON_DIR/scalable/apps/$BINARY_NAME.svg"

echo "Installing launcher..."
mkdir -p "$DESKTOP_DIR"
cp "$PROJECT_DIR/aurora-screenshots.desktop" "$DESKTOP_DIR/"

echo "Updating caches..."
update-desktop-database "$DESKTOP_DIR" 2>/dev/null || true
gtk-update-icon-cache -f -t "$ICON_DIR" 2>/dev/null || true

echo ""
echo "✓ Aurora Screenshots installed successfully"
echo "  Binary : $INSTALL_DIR/$BINARY_NAME"
echo "  Launcher: $DESKTOP_DIR/aurora-screenshots.desktop"
echo ""

# Verificar que ~/.local/bin está en PATH
if [[ ":$PATH:" != *":$HOME/.local/bin:"* ]]; then
  echo "⚠ $HOME/.local/bin is not in your PATH."
  echo "  Add this line to your ~/.bashrc or ~/.zshrc:"
  echo "    export PATH=\"\$HOME/.local/bin:\$PATH\""
fi
