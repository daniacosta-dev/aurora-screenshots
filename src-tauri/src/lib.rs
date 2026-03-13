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

pub struct AppState {
    pub db: Mutex<rusqlite::Connection>,
    /// Origen del virtual desktop (min_x, min_y de todos los monitores).
    pub overlay_offset: Mutex<(i32, i32)>,
    /// Screenshot del escritorio capturado justo antes de mostrar el overlay (base64 PNG).
    pub desktop_background: Mutex<Option<String>>,
    /// XID de la ventana capture-overlay (se guarda en la primera captura para reusar).
    pub overlay_xid: Mutex<Option<u32>>,
    /// Imágenes de capturas pinadas pendientes de ser leídas por su ventana.
    pub pin_images: Mutex<std::collections::HashMap<String, String>>,
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

            app.manage(AppState {
                db: Mutex::new(conn),
                overlay_offset: Mutex::new((0, 0)),
                desktop_background: Mutex::new(None),
                overlay_xid: Mutex::new(None),
                pin_images: Mutex::new(std::collections::HashMap::new()),
            });

            setup_tray(app)?;
            register_shortcuts(app)?;

            Ok(())
        })
        .on_window_event(|window, event| {
            // Ocultar ventana principal en lugar de cerrarla — la app vive en el tray
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
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
            commands::hide_main_window,
            commands::pin_screenshot,
            commands::get_pin_image,
            commands::copy_png_to_clipboard,
            commands::write_screenshot_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
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
            "capture" => commands::show_capture_overlay(app),
            "history" => show_history_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    Ok(())
}

fn register_shortcuts(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let shortcut = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyS);

    app.global_shortcut()
        .on_shortcut(shortcut, |app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                commands::show_capture_overlay(app);
            }
        })?;

    Ok(())
}

fn show_history_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        position_main_window(&window);
        // always_on_top(true) fuerza al WM a levantar la ventana por encima de todo,
        // luego lo desactivamos para que se comporte como ventana normal.
        let _ = window.set_always_on_top(true);
        let _ = window.show();
        let _ = window.set_focus();
        let _ = window.set_always_on_top(false);
    }
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
