#!/usr/bin/env bash
set -e

APP_NAME="aurora-screenshots"
VERSION=$(grep '^version' src-tauri/Cargo.toml | head -1 | sed 's/.*= "\(.*\)"/\1/')
ARCH="x86_64"
RELEASE_NAME="${APP_NAME}-${VERSION}-linux-${ARCH}"
RELEASE_DIR="release/${RELEASE_NAME}"

echo "==> Compilando ${APP_NAME} v${VERSION}..."
npm run tauri build

echo ""
echo "==> Empaquetando release..."

rm -rf "release/${RELEASE_NAME}"
mkdir -p "${RELEASE_DIR}"

# Binario
cp "src-tauri/target/release/${APP_NAME}" "${RELEASE_DIR}/"

# Icono (SVG original — se instala como scalable icon para mayor calidad)
cp "public/aurora-screenshots-icon.svg" "${RELEASE_DIR}/icon.svg"

# Scripts
cp install.sh "${RELEASE_DIR}/install.sh"
chmod +x "${RELEASE_DIR}/install.sh"

# Crear uninstall.sh
cat > "${RELEASE_DIR}/uninstall.sh" <<'EOF'
#!/usr/bin/env bash
set -e
if [ "$EUID" -ne 0 ]; then
  echo "Ejecutá con sudo: sudo ./uninstall.sh"
  exit 1
fi
rm -f /usr/local/bin/aurora-screenshots
rm -f /usr/share/applications/aurora-screenshots.desktop
rm -f /usr/share/icons/hicolor/scalable/apps/aurora-screenshots.svg
update-desktop-database /usr/share/applications 2>/dev/null || true
gtk-update-icon-cache -f -t /usr/share/icons/hicolor 2>/dev/null || true
echo "aurora-screenshots desinstalado."
EOF
chmod +x "${RELEASE_DIR}/uninstall.sh"

# Empaquetar
cd release
tar -czf "${RELEASE_NAME}.tar.gz" "${RELEASE_NAME}"
cd ..

echo ""
echo "==> Release generado:"
echo "    release/${RELEASE_NAME}.tar.gz"
echo ""
echo "Para instalar:"
echo "    tar -xzf release/${RELEASE_NAME}.tar.gz"
echo "    cd release/${RELEASE_NAME}"
echo "    sudo ./install.sh"
