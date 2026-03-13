#!/bin/bash
set -e

BINARY_NAME="aurora-screenshots"
VERSION=$(grep '^version' "$(dirname "$0")/../src-tauri/Cargo.toml" | head -1 | sed 's/.*= *"\(.*\)"/\1/')
DIST_NAME="${BINARY_NAME}-${VERSION}-linux-x86_64"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
DIST_DIR="$PROJECT_DIR/dist"
STAGING="$DIST_DIR/$DIST_NAME"

echo "Building Aurora Screenshots v$VERSION..."
cargo build --release --manifest-path "$PROJECT_DIR/src-tauri/Cargo.toml"

echo "Staging files..."
rm -rf "$STAGING"
mkdir -p "$STAGING/icons"

cp "$PROJECT_DIR/src-tauri/target/release/$BINARY_NAME"       "$STAGING/"
cp "$PROJECT_DIR/aurora-screenshots.desktop"                   "$STAGING/"
cp "$PROJECT_DIR/src-tauri/icons/32x32.png"                    "$STAGING/icons/32x32.png"
cp "$PROJECT_DIR/src-tauri/icons/128x128.png"                  "$STAGING/icons/128x128.png"
cp "$PROJECT_DIR/src-tauri/icons/icon.png"                     "$STAGING/icons/256x256.png"
cp "$PROJECT_DIR/public/aurora-screenshots-icon.svg"           "$STAGING/icons/aurora-screenshots.svg"

# Installer embebido en el tar
cat > "$STAGING/install.sh" << 'EOF'
#!/bin/bash
set -e

BINARY_NAME="aurora-screenshots"
INSTALL_DIR="$HOME/.local/bin"
ICON_DIR="$HOME/.local/share/icons/hicolor"
DESKTOP_DIR="$HOME/.local/share/applications"
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "Installing Aurora Screenshots..."

mkdir -p "$INSTALL_DIR"
cp "$DIR/$BINARY_NAME" "$INSTALL_DIR/"
chmod +x "$INSTALL_DIR/$BINARY_NAME"

mkdir -p "$ICON_DIR/32x32/apps" "$ICON_DIR/128x128/apps" "$ICON_DIR/256x256/apps" "$ICON_DIR/scalable/apps"
cp "$DIR/icons/32x32.png"             "$ICON_DIR/32x32/apps/$BINARY_NAME.png"
cp "$DIR/icons/128x128.png"           "$ICON_DIR/128x128/apps/$BINARY_NAME.png"
cp "$DIR/icons/256x256.png"           "$ICON_DIR/256x256/apps/$BINARY_NAME.png"
cp "$DIR/icons/aurora-screenshots.svg" "$ICON_DIR/scalable/apps/$BINARY_NAME.svg"

mkdir -p "$DESKTOP_DIR"
cp "$DIR/aurora-screenshots.desktop" "$DESKTOP_DIR/"

update-desktop-database "$DESKTOP_DIR" 2>/dev/null || true
gtk-update-icon-cache -f -t "$ICON_DIR" 2>/dev/null || true

echo ""
echo "✓ Aurora Screenshots installed"
echo "  Run: aurora-screenshots"
echo ""

if [[ ":$PATH:" != *":$HOME/.local/bin:"* ]]; then
  echo "⚠ Add ~/.local/bin to your PATH:"
  echo "  export PATH=\"\$HOME/.local/bin:\$PATH\""
fi
EOF
chmod +x "$STAGING/install.sh"

echo "Creating tarball..."
cd "$DIST_DIR"
tar -czf "${DIST_NAME}.tar.gz" "$DIST_NAME"
rm -rf "$STAGING"

echo ""
echo "✓ dist/${DIST_NAME}.tar.gz ready"
