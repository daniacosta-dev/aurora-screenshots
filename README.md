# Aurora Screenshots

A Linux desktop app for screen capture with annotation tools and clipboard history. Built with Tauri v2.

## Features

- **Area capture** — select any region of the screen with a crosshair overlay
- **Annotation editor** — draw arrows, rectangles, freehand marker, text, and blur regions on the capture before saving
- **Pin screenshot** — float a captured region as a borderless always-on-top window for quick visual comparisons
- **Clipboard history** — automatically stores captured images in a local SQLite database with thumbnail previews
- **Copy to clipboard** — one-click copy of any item from history or any pinned window
- **System tray** — lives in the tray, no taskbar clutter; accessible via tray menu or global shortcut
- **Global shortcut** — `Ctrl+Shift+S` from anywhere to open the capture overlay
- **Multi-monitor support** — overlay covers all monitors, coordinates correctly mapped across the virtual desktop
- **X11 + Wayland** — X11 via native capture with input grab; Wayland via XDG Desktop Portal

## Stack

| Layer | Technology |
|---|---|
| Desktop runtime | Tauri v2 |
| Backend | Rust |
| Frontend | React 19 + TypeScript + Vite |
| Styling | Tailwind CSS v4 |
| Database | SQLite via `rusqlite` (bundled) |
| State management | Zustand |
| X11 capture | `screenshots` crate |
| Wayland capture | `ashpd` (XDG portal) |
| Clipboard | `arboard` |
| Input grab | `x11rb` |

## Requirements

- Linux (X11 or Wayland)
- Node.js 18+
- Rust + Cargo
- [Tauri v2 system dependencies](https://v2.tauri.app/start/prerequisites/)

## Development

```bash
npm install
npm run tauri dev
```

## Build

```bash
npm run tauri build
```

## Keyboard shortcuts

| Shortcut | Action |
|---|---|
| `Ctrl+Shift+S` | Open capture overlay |
| `Ctrl+C` | Capture selected area |
| `Ctrl+Z` | Undo last annotation |
| `Escape` | Cancel / close overlay |

## Support

If AuroraWall saves you from a boring desktop, consider supporting development:

<div align="center">

[![ko-fi](https://ko-fi.com/img/githubbutton_sm.svg)](https://ko-fi.com/daniacostadev)

</div>

---

<div align="center">

Made with ❤️ and Rust · MIT License · [Ko-fi](https://ko-fi.com/daniacostadev)

</div>

Created by [@daniacosta-dev](https://github.com/daniacosta-dev)