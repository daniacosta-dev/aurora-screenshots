use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use tauri::{Emitter, Manager, State};

use crate::capture;
use crate::clipboard;
use crate::db::{self, HistoryEntry};
use crate::AppState;

// ─── Historial ────────────────────────────────────────────────────────────

#[tauri::command]
pub fn get_history(state: State<AppState>, limit: Option<i64>) -> Result<Vec<HistoryEntry>, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db::get_entries(&db, limit.unwrap_or(100)).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_history_item(state: State<AppState>, id: i64) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db::delete_entry(&db, id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn clear_history(state: State<AppState>) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db::clear_entries(&db).map_err(|e| e.to_string())
}

// ─── Captura de área seleccionable ────────────────────────────────────────

/// Lógica compartida para iniciar la captura de área.
/// Llamada tanto desde el shortcut global como desde el botón del frontend.
pub(crate) fn show_capture_overlay(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.hide();
    }

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        eprintln!("[show_overlay] start");
        let display_server = match capture::detect_display_server() {
            Ok(ds) => ds,
            Err(e) => {
                eprintln!("[show_overlay] detect_display_server error: {e}");
                return;
            }
        };

        match display_server {
            capture::DisplayServer::X11 => {
                eprintln!("[show_overlay] X11 branch");
                let (min_x, min_y, total_w, total_h) =
                    match capture::all_monitors_bounds_x11() {
                        Ok(bounds) => bounds,
                        Err(e) => {
                            eprintln!("[show_overlay] all_monitors_bounds_x11 error: {e}");
                            return;
                        }
                    };

                {
                    let state = app.state::<AppState>();
                    if let Ok(mut offset) = state.overlay_offset.lock() {
                        *offset = (min_x, min_y);
                    };
                }

                // Esperar a que el menú del tray (o la ventana principal) se cierre
                // completamente antes de capturar el escritorio.
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;

                let bg_task = tokio::task::spawn_blocking(move || {
                    capture::capture_full_desktop_x11(min_x, min_y, total_w, total_h)
                });

                tokio::time::sleep(std::time::Duration::from_millis(80)).await;

                let bg_data = bg_task.await.ok().and_then(|r| r.ok());
                eprintln!("[show_overlay] bg capture done, has_bg={}", bg_data.is_some());

                if let Some(ref bg) = bg_data {
                    let state = app.state::<AppState>();
                    if let Ok(mut bg_lock) = state.desktop_background.lock() {
                        *bg_lock = Some(bg.clone());
                    };
                }

                if let Some(overlay) = app.get_webview_window("capture-overlay") {
                    let _ = overlay.set_position(tauri::Position::Physical(
                        tauri::PhysicalPosition::new(min_x, min_y),
                    ));
                    let _ = overlay.set_size(tauri::Size::Physical(
                        tauri::PhysicalSize::new(total_w, total_h),
                    ));

                    // Intentar obtener el XID antes del show() para poder aplicar
                    // override_redirect mientras la ventana está desmapeada.
                    // En la primera captura window_handle() falla (ventana no realizada aún)
                    // → usamos el XID guardado de capturas previas si existe.
                    #[cfg(target_os = "linux")]
                    let xid_pre: Option<u32> = {
                        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
                        let fresh = match overlay.window_handle() {
                            Ok(handle) => match handle.as_raw() {
                                RawWindowHandle::Xcb(h) => Some(h.window.get()),
                                RawWindowHandle::Xlib(h) => Some(h.window as u32),
                                _ => None,
                            },
                            Err(_) => None,
                        };
                        let state = app.state::<AppState>();
                        if let Some(id) = fresh {
                            if let Ok(mut stored) = state.overlay_xid.lock() { *stored = Some(id); }
                            Some(id)
                        } else {
                            state.overlay_xid.lock().ok().and_then(|g| *g)
                        }
                    };

                    // override_redirect ANTES del show() — solo funciona en ventana desmapeada.
                    // En la primera captura xid_pre puede ser None (primera vez que se muestra).
                    #[cfg(target_os = "linux")]
                    if let Some(xid) = xid_pre {
                        crate::x11_grab::set_override_redirect(xid);
                    }

                    eprintln!("[show_overlay] calling overlay.show()");
                    let _ = overlay.show();

                    if bg_data.is_some() {
                        let _ = overlay.emit("background-ready", ());
                    }

                    // Esperar a que X11 mapee la ventana. Después del show() GTK realiza
                    // el X11 window, así que window_handle() funciona aunque haya fallado antes.
                    tokio::time::sleep(std::time::Duration::from_millis(60)).await;

                    // Si en la primera captura no teníamos XID, intentar obtenerlo ahora.
                    #[cfg(target_os = "linux")]
                    let xid: Option<u32> = if xid_pre.is_some() {
                        xid_pre
                    } else {
                        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
                        let post = match overlay.window_handle() {
                            Ok(handle) => match handle.as_raw() {
                                RawWindowHandle::Xcb(h) => Some(h.window.get()),
                                RawWindowHandle::Xlib(h) => Some(h.window as u32),
                                _ => None,
                            },
                            Err(e) => { eprintln!("[show_overlay] post-show window_handle error: {e}"); None }
                        };
                        if let Some(id) = post {
                            let state = app.state::<AppState>();
                            if let Ok(mut stored) = state.overlay_xid.lock() { *stored = Some(id); }
                            eprintln!("[show_overlay] xid stored post-show: {id}");
                        }
                        post
                    };

                    eprintln!("[show_overlay] xid={xid:?}");

                    // set_focus() fuerza a GTK a enfocar la WebView internamente.
                    let _ = overlay.set_focus();
                    let _ = overlay.set_ignore_cursor_events(false);
                    eprintln!("[show_overlay] set_focus + set_ignore_cursor_events done");

                    #[cfg(target_os = "linux")]
                    if let Some(xid) = xid {
                        tokio::task::spawn_blocking(move || {
                            if let Err(e) = crate::x11_grab::setup_and_grab(xid) {
                                eprintln!("[show_overlay] setup_and_grab error: {e}");
                            }
                        });
                    }
                } else {
                    eprintln!("[show_overlay] could not get capture-overlay window");
                }
            }

            capture::DisplayServer::Wayland => {
                // Para Wayland el portal maneja la selección interactiva
                match capture::capture_area_wayland_interactive().await {
                    Ok(result) => {
                        if let Err(e) = clipboard::copy_png_b64_to_clipboard(&result.content) {
                            eprintln!("Error copiando al clipboard (Wayland): {e}");
                        }
                        let state = app.state::<AppState>();
                        if let Ok(db) = state.db.lock() {
                            let _ = db::insert_entry(
                                &db,
                                "image",
                                &result.content,
                                Some(&result.thumbnail),
                            );
                        }
                        let _ = app.emit("history-updated", ());
                    }
                    Err(e) => {
                        eprintln!("Captura Wayland fallida: {e}");
                    }
                }
            }
        }
    });
}

/// Comando Tauri para iniciar captura (llamado desde el botón del frontend).
#[tauri::command]
pub fn start_area_capture(app: tauri::AppHandle) -> Result<(), String> {
    show_capture_overlay(&app);
    Ok(())
}

/// El frontend llama a este comando cuando el usuario presiona Ctrl+C con una
/// selección activa. Oculta el overlay, espera, captura la región real.
///
/// `x`, `y`, `width`, `height`: coordenadas en CSS px relativas al origen del overlay.
/// `dpr`: devicePixelRatio del monitor donde está el overlay.
#[tauri::command]
pub fn finalize_area_capture(
    state: State<AppState>,
    app: tauri::AppHandle,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    dpr: f64,
) -> Result<(), String> {
    // Obtener el offset del virtual desktop (min_x, min_y de todos los monitores)
    let (offset_x, offset_y) = {
        let offset = state.overlay_offset.lock().map_err(|e| e.to_string())?;
        *offset
    };

    // Convertir CSS px → píxeles físicos del virtual desktop
    let abs_x = (x as f64 * dpr).round() as i32 + offset_x;
    let abs_y = (y as f64 * dpr).round() as i32 + offset_y;
    let abs_w = (width as f64 * dpr).round() as u32;
    let abs_h = (height as f64 * dpr).round() as u32;

    // Ocultar overlay antes de capturar para que no aparezca en el resultado
    #[cfg(target_os = "linux")]
    if let Err(e) = crate::x11_grab::ungrab_input() {
        eprintln!("ungrab_input: {e}");
    }
    if let Some(overlay) = app.get_webview_window("capture-overlay") {
        let _ = overlay.hide();
    }

    // Esperar a que el overlay desaparezca realmente en pantalla
    std::thread::sleep(std::time::Duration::from_millis(200));

    // Capturar la región real del escritorio (X11)
    let result = capture::capture_region_x11(abs_x, abs_y, abs_w, abs_h)
        .map_err(|e| e.to_string())?;

    // Copiar al portapapeles del sistema
    clipboard::copy_png_b64_to_clipboard(&result.content)?;

    // Guardar en DB
    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db::insert_entry(&db, "image", &result.content, Some(&result.thumbnail))
            .map_err(|e| e.to_string())?;
    }

    let _ = app.emit("history-updated", ());
    Ok(())
}

// ─── Copiar item al portapapeles ──────────────────────────────────────────

#[tauri::command]
pub fn copy_history_item(state: State<AppState>, id: i64) -> Result<(), String> {
    let entry = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db::get_entry_by_id(&db, id).map_err(|e| e.to_string())?
    };

    match entry.entry_type.as_str() {
        "text" => clipboard::copy_text_to_clipboard(&entry.content),
        "image" => clipboard::copy_png_b64_to_clipboard(&entry.content),
        other => Err(format!("Tipo de item desconocido: {other}")),
    }
}

// ─── Captura con anotaciones ──────────────────────────────────────────────

/// Oculta el overlay de captura desde el frontend (más confiable que la API JS en Tauri).
#[tauri::command]
pub fn hide_capture_overlay(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    if let Err(e) = crate::x11_grab::ungrab_input() {
        eprintln!("ungrab_input: {e}");
    }
    if let Some(overlay) = app.get_webview_window("capture-overlay") {
        let _ = overlay.hide();
    }
    Ok(())
}

/// Oculta la ventana principal desde el frontend.
#[tauri::command]
pub fn hide_main_window(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.hide();
    }
    Ok(())
}

/// Retorna el screenshot del escritorio capturado antes de mostrar el overlay.
#[tauri::command]
pub fn get_desktop_background(state: State<AppState>) -> Result<Option<String>, String> {
    let bg = state.desktop_background.lock().map_err(|e| e.to_string())?;
    Ok(bg.clone())
}

/// El frontend llama a este comando con el PNG anotado (base64) para guardarlo y
/// copiarlo al portapapeles. No requiere coordenadas — el canvas ya tiene el recorte.
#[tauri::command]
pub fn finalize_annotated_capture(
    state: State<AppState>,
    app: tauri::AppHandle,
    image_data: String,
) -> Result<(), String> {
    // Ocultar overlay
    #[cfg(target_os = "linux")]
    if let Err(e) = crate::x11_grab::ungrab_input() {
        eprintln!("ungrab_input: {e}");
    }
    if let Some(overlay) = app.get_webview_window("capture-overlay") {
        let _ = overlay.hide();
    }

    // Copiar al portapapeles
    clipboard::copy_png_b64_to_clipboard(&image_data)?;

    // Generar miniatura
    let png_bytes = STANDARD.decode(&image_data).map_err(|e| e.to_string())?;
    let thumbnail = capture::generate_thumbnail_b64(&png_bytes).map_err(|e| e.to_string())?;

    // Guardar en DB
    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db::insert_entry(&db, "image", &image_data, Some(&thumbnail))
            .map_err(|e| e.to_string())?;
    }

    let _ = app.emit("history-updated", ());

    // Notificación del sistema via notify-send
    tauri::async_runtime::spawn(async {
        let _ = tokio::process::Command::new("notify-send")
            .args([
                "--app-name=Aurora Screenshots",
                "--icon=aurora-screenshots",
                "--expire-time=3000",
                "Aurora Screenshots",
                "Screenshot copied to clipboard",
            ])
            .status()
            .await;
    });

    Ok(())
}

// ─── Pin screenshot ───────────────────────────────────────────────────────

/// Guarda la imagen en AppState, crea una ventana flotante always-on-top y le
/// transfiere el ID para que la lea vía `get_pin_image`.
#[tauri::command]
pub fn pin_screenshot(
    state: State<AppState>,
    app: tauri::AppHandle,
    image_data: String,
    width: u32,
    height: u32,
) -> Result<(), String> {
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string();

    {
        let mut pins = state.pin_images.lock().map_err(|e| e.to_string())?;
        pins.insert(id.clone(), image_data);
    }

    // Calcular tamaño de ventana (máx 800×600), manteniendo aspect ratio
    let max_w = 800f64;
    let max_h = 600f64;
    let scale = (max_w / width as f64).min(max_h / height as f64).min(1.0);
    let win_w = (width as f64 * scale).round();
    let win_h = (height as f64 * scale).round() + 28.0; // +28 para la barra drag

    tauri::WebviewWindowBuilder::new(
        &app,
        format!("pin-{}", id),
        tauri::WebviewUrl::App("index.html".into()),
    )
    .title("Pin — Aurora Screenshots")
    .inner_size(win_w, win_h)
    .always_on_top(true)
    .decorations(false)
    .resizable(true)
    .skip_taskbar(false)
    .build()
    .map_err(|e| e.to_string())?;

    Ok(())
}

/// Retorna y elimina la imagen pinada por ID (one-shot: la ventana la lee al montar).
#[tauri::command]
pub fn get_pin_image(state: State<AppState>, id: String) -> Result<Option<String>, String> {
    let mut pins = state.pin_images.lock().map_err(|e| e.to_string())?;
    Ok(pins.remove(&id))
}

/// Copia un PNG base64 al portapapeles (usado desde la ventana de pin).
#[tauri::command]
pub fn copy_png_to_clipboard(image_data: String) -> Result<(), String> {
    clipboard::copy_png_b64_to_clipboard(&image_data)
}

/// Escribe un PNG base64 en la ruta indicada por el usuario.
#[tauri::command]
pub fn write_screenshot_file(path: String, image_data: String) -> Result<(), String> {
    let png_bytes = STANDARD.decode(&image_data).map_err(|e| e.to_string())?;
    std::fs::write(&path, &png_bytes).map_err(|e| e.to_string())
}

// ─── Helpers privados ─────────────────────────────────────────────────────

