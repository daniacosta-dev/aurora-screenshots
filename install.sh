#!/usr/bin/env bash
set -e

APP_NAME="aurora-screenshots"
BINARY_NAME="aurora-screenshots"
INSTALL_DIR="/usr/local/bin"
DESKTOP_DIR="/usr/share/applications"
ICON_DIR="/usr/share/icons/hicolor/scalable/apps"
DESKTOP_ENTRY="/usr/share/applications/${APP_NAME}.desktop"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "Instalando ${APP_NAME}..."

# Requiere sudo
if [ "$EUID" -ne 0 ]; then
  echo "Ejecutá con sudo: sudo ./install.sh"
  exit 1
fi

# Copiar binario
install -m 755 "${SCRIPT_DIR}/${BINARY_NAME}" "${INSTALL_DIR}/${BINARY_NAME}"
echo "  ✓ Binario → ${INSTALL_DIR}/${BINARY_NAME}"

# Copiar icono SVG (scalable, mejor calidad que PNG en cualquier tamaño)
if [ -f "${SCRIPT_DIR}/icon.svg" ]; then
  mkdir -p "${ICON_DIR}"
  install -m 644 "${SCRIPT_DIR}/icon.svg" "${ICON_DIR}/${APP_NAME}.svg"
  echo "  ✓ Icono → ${ICON_DIR}/${APP_NAME}.svg"
fi

# Crear entrada .desktop
cat > "${DESKTOP_ENTRY}" <<EOF
[Desktop Entry]
Name=Aurora Screenshots
Comment=Clipboard manager and screen capture
Exec=${INSTALL_DIR}/${BINARY_NAME}
Icon=${APP_NAME}
Terminal=false
Type=Application
Categories=Utility;Graphics;
StartupNotify=false
EOF
echo "  ✓ .desktop → ${DESKTOP_ENTRY}"

update-desktop-database "${DESKTOP_DIR}" 2>/dev/null || true
gtk-update-icon-cache -f -t /usr/share/icons/hicolor 2>/dev/null || true

echo ""
echo "Instalación completa. Ejecuta 'aurora-screenshots' o búscalo en el menú de apps."
