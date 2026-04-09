mod capture;
mod clipboard;
mod commands;
mod db;
#[cfg(target_os = "linux")]
mod x11_grab;

use std::sync::Mutex;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    Manager,
};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

pub const DEFAULT_CAPTURE_SHORTCUT: &str = "Ctrl+Shift+S";

pub struct AppState {
    pub db: Mutex<rusqlite::Connection>,
    /// Display server detectado al inicio (X11 o Wayland). Inmutable después del setup.
    pub display_server: capture::DisplayServer,
    /// Origen del virtual desktop (min_x, min_y de todos los monitores).
    pub overlay_offset: Mutex<(i32, i32)>,
    /// Monitores capturados como JPEG individuales para el overlay (sin compositing en Rust).
    pub desktop_background: Mutex<Option<Vec<capture::MonitorCapture>>>,
    /// XID de la ventana capture-overlay (se guarda en la primera captura para reusar).
    pub overlay_xid: Mutex<Option<u32>>,
    /// Imágenes de capturas pinadas pendientes de ser leídas por su ventana.
    pub pin_images: Mutex<std::collections::HashMap<String, String>>,
    /// Shortcut activo para captura (ej. "Ctrl+Shift+S"). Fuente de verdad en memoria.
    pub current_shortcut: Mutex<String>,
    /// Captura Wayland pendiente de ser leída por el overlay para anotación.
    pub wayland_pending_capture: Mutex<Option<capture::CaptureResult>>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let db_path = data_dir.join("aurora-screenshots.db");

            let conn = rusqlite::Connection::open(&db_path)
                .map_err(|e| format!("No se pudo abrir la base de datos: {e}"))?;
            db::init_db(&conn)
                .map_err(|e| format!("No se pudo inicializar la base de datos: {e}"))?;

            // Leer el shortcut guardado o usar el default.
            let saved_shortcut = db::get_setting(&conn, "capture_shortcut")
                .ok()
                .flatten()
                .unwrap_or_else(|| DEFAULT_CAPTURE_SHORTCUT.to_string());

            let display_server = capture::detect_display_server()
                .unwrap_or(capture::DisplayServer::X11);
            eprintln!("[init] display server: {:?}", display_server);

            app.manage(AppState {
                db: Mutex::new(conn),
                display_server,
                overlay_offset: Mutex::new((0, 0)),
                desktop_background: Mutex::new(None),
                overlay_xid: Mutex::new(None),
                pin_images: Mutex::new(std::collections::HashMap::new()),
                current_shortcut: Mutex::new(saved_shortcut.clone()),
                wayland_pending_capture: Mutex::new(None),
            });

            setup_tray(app)?;
            register_shortcut(app, &saved_shortcut)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_history,
            commands::delete_history_item,
            commands::clear_history,
            commands::start_area_capture,
            commands::finalize_area_capture,
            commands::copy_history_item,
            commands::get_desktop_background,
            commands::finalize_annotated_capture,
            commands::hide_capture_overlay,
            commands::close_capture_overlay,
            commands::show_main_window,
            commands::hide_main_window,
            commands::get_capture_shortcut,
            commands::update_capture_shortcut,
            commands::pin_screenshot,
            commands::get_pin_image,
            commands::copy_png_to_clipboard,
            commands::write_screenshot_file,
            commands::open_screenshots_folder,
            commands::get_autostart,
            commands::set_autostart,
            commands::overlay_ready,
            commands::get_wayland_pending_capture,
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|_app, event| {
            // Sin esta guard, Tauri termina el proceso cuando se cierra la última ventana.
            // La app vive en el tray: solo debe salir cuando el usuario elige "Quit".
            // app.exit(0) (desde el menú) funciona igual — bypass prevent_exit.
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                api.prevent_exit();
            }
        });
}

/// SVG embebido en el binario — no depende de ningún archivo externo en runtime.
const TRAY_ICON_SVG: &[u8] = include_bytes!("../../public/aurora-screenshots-icon.svg");

/// Renderiza bytes SVG a RGBA y los convierte en un icono de Tauri.
/// Usa resvg (pure-Rust) para no depender de herramientas externas.
fn load_svg_icon(data: &[u8], size: u32) -> Option<tauri::image::Image<'static>> {
    let opt = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_data(data, &opt).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size)?;
    let svg_w = tree.size().width();
    let svg_h = tree.size().height();
    let scale = (size as f32 / svg_w).min(size as f32 / svg_h);
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let png = pixmap.encode_png().ok()?;
    tauri::image::Image::from_bytes(&png).ok()
}

fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let capture_item = MenuItem::with_id(app, "capture", "New capture", true, None::<&str>)?;
    let history_item =
        MenuItem::with_id(app, "history", "Open history", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

    let menu = Menu::with_items(app, &[&capture_item, &history_item, &separator, &quit_item])?;

    // Ícono embebido en el binario — funciona independientemente de cómo se instale la app
    let icon = load_svg_icon(TRAY_ICON_SVG, 64)
        .or_else(|| app.default_window_icon().cloned())
        .ok_or("No se encontró el ícono de la app")?;

    TrayIconBuilder::new()
        .icon(icon)
        .menu(&menu)
        .tooltip("Aurora Screenshots")
        .on_menu_event(|app, event| match event.id.as_ref() {
            "capture" => commands::show_capture_overlay(app, true),
            "history" => show_history_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    Ok(())
}

pub(crate) fn parse_shortcut(s: &str) -> Result<Shortcut, String> {
    let parts: Vec<&str> = s.split('+').collect();
    if parts.len() < 2 {
        return Err("El shortcut debe tener al menos un modificador y una tecla".into());
    }

    let mut mods = Modifiers::empty();
    let key_token = parts.last().unwrap();

    for token in &parts[..parts.len() - 1] {
        match *token {
            "Ctrl"  => mods |= Modifiers::CONTROL,
            "Shift" => mods |= Modifiers::SHIFT,
            "Alt"   => mods |= Modifiers::ALT,
            "Super" => mods |= Modifiers::SUPER,
            other   => return Err(format!("Modificador desconocido: {other}")),
        }
    }

    let code = match *key_token {
        "A" => Code::KeyA, "B" => Code::KeyB, "C" => Code::KeyC, "D" => Code::KeyD,
        "E" => Code::KeyE, "F" => Code::KeyF, "G" => Code::KeyG, "H" => Code::KeyH,
        "I" => Code::KeyI, "J" => Code::KeyJ, "K" => Code::KeyK, "L" => Code::KeyL,
        "M" => Code::KeyM, "N" => Code::KeyN, "O" => Code::KeyO, "P" => Code::KeyP,
        "Q" => Code::KeyQ, "R" => Code::KeyR, "S" => Code::KeyS, "T" => Code::KeyT,
        "U" => Code::KeyU, "V" => Code::KeyV, "W" => Code::KeyW, "X" => Code::KeyX,
        "Y" => Code::KeyY, "Z" => Code::KeyZ,
        "0" => Code::Digit0, "1" => Code::Digit1, "2" => Code::Digit2,
        "3" => Code::Digit3, "4" => Code::Digit4, "5" => Code::Digit5,
        "6" => Code::Digit6, "7" => Code::Digit7, "8" => Code::Digit8,
        "9" => Code::Digit9,
        "F1"  => Code::F1,  "F2"  => Code::F2,  "F3"  => Code::F3,  "F4"  => Code::F4,
        "F5"  => Code::F5,  "F6"  => Code::F6,  "F7"  => Code::F7,  "F8"  => Code::F8,
        "F9"  => Code::F9,  "F10" => Code::F10, "F11" => Code::F11, "F12" => Code::F12,
        // Navegación
        "Home" => Code::Home, "End" => Code::End,
        "PageUp" => Code::PageUp, "PageDown" => Code::PageDown,
        "Insert" => Code::Insert, "Delete" => Code::Delete,
        // Flechas (nombre corto para mostrar en UI)
        "Up" => Code::ArrowUp, "Down" => Code::ArrowDown,
        "Left" => Code::ArrowLeft, "Right" => Code::ArrowRight,
        // Otras
        "Space" => Code::Space, "Tab" => Code::Tab,
        "Enter" => Code::Enter, "Backspace" => Code::Backspace,
        // Símbolos
        "-" => Code::Minus,       "=" => Code::Equal,
        "[" => Code::BracketLeft, "]" => Code::BracketRight,
        "\\" => Code::Backslash,  ";" => Code::Semicolon,
        "'" => Code::Quote,       "`" => Code::Backquote,
        "," => Code::Comma,       "." => Code::Period,
        "/" => Code::Slash,
        other => return Err(format!("Tecla desconocida: {other}")),
    };

    Ok(Shortcut::new(Some(mods), code))
}

fn register_shortcut(app: &tauri::App, shortcut_str: &str) -> Result<(), Box<dyn std::error::Error>> {
    let shortcut = parse_shortcut(shortcut_str).map_err(|e| e)?;

    app.global_shortcut()
        .on_shortcut(shortcut, |app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                commands::show_capture_overlay(app, false);
            }
        })?;

    Ok(())
}

pub(crate) fn show_history_window(app: &tauri::AppHandle) {
    let window = match app.get_webview_window("main") {
        Some(w) => w,
        None => {
            match tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("Aurora Screenshots")
            .inner_size(420.0, 650.0)
            .visible(false)
            .skip_taskbar(true)
            .resizable(false)
            .decorations(false)
            .build()
            {
                Ok(w) => w,
                Err(e) => {
                    eprintln!("[show_history] failed to create window: {e}");
                    return;
                }
            }
        }
    };

    position_main_window(&window);
    let _ = window.set_always_on_top(true);
    let _ = window.show();
    let _ = window.set_focus();
    let _ = window.set_always_on_top(false);
}

/// Posiciona la ventana principal en la esquina superior derecha del monitor primario.
/// Usa la posición física del monitor para soportar configuraciones multi-monitor.
fn position_main_window(window: &tauri::WebviewWindow) {
    if let Ok(Some(monitor)) = window.primary_monitor() {
        let pos = monitor.position();
        let size = monitor.size();
        let scale = monitor.scale_factor();
        // Convertir posición y dimensiones físicas → lógicas
        let mon_x = pos.x as f64 / scale;
        let mon_y = pos.y as f64 / scale;
        let mon_w = size.width as f64 / scale;
        let win_w = 420.0f64;
        let margin = 16.0f64;
        let x = mon_x + mon_w - win_w - margin;
        let y = mon_y + margin;
        let _ = window.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(x, y)));
    }
}
