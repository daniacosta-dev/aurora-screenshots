# Aurora Screenshots — Linux clipboard manager & screenshot tool

A lightweight desktop app for Linux that unifies clipboard history and annotated screenshot capture, built with Tauri v2 + Rust + React.

---

## Features

- **Area screenshot capture** — press `Ctrl+Shift+S` to open a fullscreen overlay, drag to select any region across one or multiple monitors, and confirm with `Ctrl+C`. The screenshot is automatically copied to your clipboard.
- **Annotation editor** — before finalizing a capture, annotate it directly on canvas: arrows, rectangles, text, blur regions, highlight, and color inversion tools.
- **Clipboard history** — every screenshot you take is saved with a thumbnail and timestamp. Re-copy any item with one click.
- **Pin screenshots** — keep any screenshot floating always-on-top in its own resizable window while you work.
- **Save to file** — export any capture as a PNG to your filesystem.
- **System tray** — the app lives in your tray. Open history or start a new capture from the tray menu without keeping a window open.
- **Global shortcut** — `Ctrl+Shift+S` works system-wide, even when the app window is hidden.
- **X11 and Wayland support** — automatically detects your display server. Wayland uses the XDG portal for interactive selection.
- **Multi-monitor support** — the capture overlay spans all connected monitors correctly.

---

## Requirements

- Linux (X11 or Wayland)
- `libwebkit2gtk-4.1` (usually pre-installed on Ubuntu 22.04+)

---

## Installation

Download the latest `.tar.gz` from [Releases](../../releases), extract and run the installer:

```bash
tar -xzf aurora-screenshots-*-linux-x86_64.tar.gz
cd aurora-screenshots-*
sudo ./install.sh
```

This installs the binary to `/usr/local/bin/aurora-screenshots` and registers a `.desktop` entry so the app appears in your application launcher.

### Uninstall

```bash
sudo ./uninstall.sh
```

---

## Usage

Launch `aurora-screenshots` from your app menu or terminal. The app will appear in your system tray.

| Action | How |
|---|---|
| Capture area | `Ctrl+Shift+S` or tray → *New capture* |
| Confirm capture | `Ctrl+C` after drawing a selection |
| Cancel capture | `Escape` |
| Open history | Tray → *Open history* |
| Re-copy an item | Click *Copy* on any history entry |

---

## Build from source

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install system dependencies (Ubuntu/Debian)
sudo apt install libwebkit2gtk-4.1-dev build-essential libxdo-dev \
  libssl-dev libayatana-appindicator3-dev librsvg2-dev libgtk-3-dev

# Clone and run
git clone https://github.com/your-username/aurora-screenshots
cd aurora-screenshots
npm install
npm run tauri dev
```

To generate a release package:

```bash
./scripts/package-release.sh
```
