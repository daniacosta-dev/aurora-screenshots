use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use tauri::{Emitter, Manager, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

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
pub(crate) fn show_capture_overlay(app: &tauri::AppHandle, from_tray: bool) {
    // Cerrar la ventana de historial al iniciar captura: libera el proceso WebKit
    // para que el overlay no tenga que competir con él (evita 2 procesos simultáneos).
    // El historial se mantiene caliente solo entre aperturas del tray (hide_main_window).
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.close();
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

                // El menú del tray tarda ~30ms en cerrarse antes de capturar.
                // Si viene de shortcut, no hay menú abierto — sin sleep.
                if from_tray {
                    tokio::time::sleep(std::time::Duration::from_millis(40)).await;
                }

                // Arrancar la captura del fondo en paralelo con la preparación de la ventana.
                // Así no pagamos ambos costos en secuencia.
                let bg_task = tokio::task::spawn_blocking(move || {
                    capture::capture_monitors_x11(min_x, min_y)
                });

                // Preparar la ventana overlay MIENTRAS la captura corre en background.
                // En el caso reuse (ventana ya existe) esto toma ~5ms.
                // En el caso create (ESC previo) toma ~100ms — pero la captura también tarda
                // ~100-200ms, así que ambas terminan aproximadamente al mismo tiempo.
                let overlay = match app.get_webview_window("capture-overlay") {
                    Some(w) => w,
                    None => {
                        // La ventana fue cerrada — el XID almacenado ya no es válido.
                        {
                            let state = app.state::<AppState>();
                            if let Ok(mut xid) = state.overlay_xid.lock() { *xid = None; };
                        }
                        match tauri::WebviewWindowBuilder::new(
                            &app,
                            "capture-overlay",
                            tauri::WebviewUrl::App("index.html".into()),
                        )
                        .transparent(true)
                        .decorations(false)
                        .always_on_top(true)
                        .skip_taskbar(true)
                        .visible(false)
                        .resizable(false)
                        .inner_size(1920.0, 1080.0)
                        .build() {
                            Ok(w) => {
                                // Esperar a que GTK realice la ventana antes de usarla.
                                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                                w
                            }
                            Err(e) => {
                                eprintln!("[show_overlay] failed to create overlay window: {e}");
                                return;
                            }
                        }
                    }
                };

                let _ = overlay.set_position(tauri::Position::Physical(
                    tauri::PhysicalPosition::new(min_x, min_y),
                ));
                let _ = overlay.set_size(tauri::Size::Physical(
                    tauri::PhysicalSize::new(total_w, total_h),
                ));

                // Obtener XID e intentar override_redirect mientras la captura aún corre.
                // override_redirect debe aplicarse con la ventana desmapeada (antes del show).
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

                #[cfg(target_os = "linux")]
                if let Some(xid) = xid_pre {
                    crate::x11_grab::set_override_redirect(xid);
                }

                // Ahora sí esperamos el fondo. Como la preparación de la ventana ya consumió
                // ~5-100ms, la captura probablemente terminó o está por terminar.
                // Sin el sleep de 80ms que había antes (era tiempo muerto).
                let bg_data = bg_task.await.ok().and_then(|r| r.ok());
                eprintln!("[show_overlay] bg capture done, has_bg={}", bg_data.is_some());

                let has_bg = bg_data.is_some();
                if let Some(bg) = bg_data {
                    let state = app.state::<AppState>();
                    if let Ok(mut bg_lock) = state.desktop_background.lock() {
                        *bg_lock = Some(bg);
                    };
                }

                if has_bg {
                    // El fondo ya está en AppState. El frontend lo cargará en canvas y,
                    // cuando termine de dibujar, llamará overlay_ready para hacer show().
                    // Así el overlay aparece directamente con la imagen, sin flash negro.
                    eprintln!("[show_overlay] emitting background-ready (show deferred to overlay_ready)");
                    let _ = overlay.emit("background-ready", ());
                } else {
                    // Sin fondo: mostrar de inmediato (fallback, no debería ocurrir normalmente).
                    eprintln!("[show_overlay] no bg, showing immediately");
                    let _ = overlay.show();
                    tokio::time::sleep(std::time::Duration::from_millis(15)).await;
                    let _ = overlay.set_focus();
                    let _ = overlay.set_ignore_cursor_events(false);
                    #[cfg(target_os = "linux")]
                    if let Some(xid) = xid_pre {
                        let overlay_grab = overlay.clone();
                        tokio::task::spawn_blocking(move || {
                            if let Err(e) = crate::x11_grab::setup_and_grab(xid) {
                                eprintln!("[show_overlay] setup_and_grab error: {e}");
                            }
                            let _ = overlay_grab.set_focus();
                            let _ = overlay_grab.emit("grab-ready", ());
                        });
                    }
                }
            }

            capture::DisplayServer::Wayland => {
                // El portal XDG maneja la selección interactiva de región.
                // Después de capturar, abrimos el overlay de anotación con la imagen.
                eprintln!("[show_overlay wayland] calling portal (interactive)...");
                match capture::capture_area_wayland_interactive().await {
                    Ok(result) => {
                        let img_w = result.width;
                        let img_h = result.height;
                        eprintln!("[show_overlay wayland] portal OK: {img_w}x{img_h}");

                        {
                            let state = app.state::<AppState>();
                            if let Ok(mut pending) = state.wayland_pending_capture.lock() {
                                *pending = Some(result);
                            };
                        }

                        let overlay = match app.get_webview_window("capture-overlay") {
                            Some(w) => w,
                            None => {
                                match tauri::WebviewWindowBuilder::new(
                                    &app,
                                    "capture-overlay",
                                    tauri::WebviewUrl::App("index.html".into()),
                                )
                                .transparent(false)
                                .decorations(false)
                                .always_on_top(true)
                                .skip_taskbar(true)
                                .visible(false)
                                .resizable(true)
                                .inner_size(800.0, 600.0)
                                .build() {
                                    Ok(w) => {
                                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                                        w
                                    }
                                    Err(e) => {
                                        eprintln!("[show_overlay wayland] failed to create overlay: {e}");
                                        return;
                                    }
                                }
                            }
                        };

                        // Fullscreen antes de show() para que el compositor asigne las
                        // dimensiones correctas desde el primer map — sin flash de tamaño intermedio.
                        let _ = overlay.set_fullscreen(true);
                        let _ = overlay.show();
                        let _ = overlay.set_focus();

                        // Esperar a que el compositor mapee la ventana y React monte.
                        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                        eprintln!("[show_overlay wayland] emitting wayland-capture-ready");
                        let _ = overlay.emit("wayland-capture-ready", ());
                    }
                    Err(e) => {
                        eprintln!("[show_overlay wayland] capture FAILED: {e}");
                    }
                }
            }
        }
    });
}

/// El frontend llama a este comando cuando ha terminado de renderizar el fondo en canvas.
/// Hace show() del overlay y establece el grab de input — garantiza que el overlay
/// aparezca directamente con la imagen cargada, sin flash negro previo.
/// Solo relevante en X11; en Wayland el overlay ya está visible antes de este llamado.
#[tauri::command]
pub async fn overlay_ready(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let overlay = match app.get_webview_window("capture-overlay") {
        Some(w) => w,
        None => {
            eprintln!("[overlay_ready] no overlay window");
            return Ok(());
        }
    };

    eprintln!("[overlay_ready] showing overlay");
    let _ = overlay.show();

    tokio::time::sleep(std::time::Duration::from_millis(15)).await;

    let _ = overlay.set_focus();
    let _ = overlay.set_ignore_cursor_events(false);

    // Input grab: solo en X11. En Wayland no hay XID ni grab_pointer/keyboard.
    #[cfg(target_os = "linux")]
    if state.display_server == capture::DisplayServer::X11 {
        let xid: Option<u32> = {
            let stored = state.overlay_xid.lock().ok().and_then(|g| *g);
            if stored.is_some() {
                stored
            } else {
                use raw_window_handle::{HasWindowHandle, RawWindowHandle};
                let post = match overlay.window_handle() {
                    Ok(handle) => match handle.as_raw() {
                        RawWindowHandle::Xcb(h) => Some(h.window.get()),
                        RawWindowHandle::Xlib(h) => Some(h.window as u32),
                        _ => None,
                    },
                    Err(e) => { eprintln!("[overlay_ready] window_handle error: {e}"); None }
                };
                if let Some(id) = post {
                    if let Ok(mut s) = state.overlay_xid.lock() { *s = Some(id); }
                    eprintln!("[overlay_ready] xid obtained post-show: {id}");
                }
                post
            }
        };
        eprintln!("[overlay_ready] xid={xid:?}");
        if let Some(xid) = xid {
            let overlay_grab = overlay.clone();
            tokio::task::spawn_blocking(move || {
                if let Err(e) = crate::x11_grab::setup_and_grab(xid) {
                    eprintln!("[overlay_ready] setup_and_grab error: {e}");
                }
                let _ = overlay_grab.set_focus();
                let _ = overlay_grab.emit("grab-ready", ());
            });
        }
    }

    Ok(())
}

/// Retorna (y consume) la captura Wayland pendiente de anotación.
/// Llamado por el frontend al recibir el evento "wayland-capture-ready".
#[tauri::command]
pub fn get_wayland_pending_capture(state: State<AppState>) -> Result<Option<capture::CaptureResult>, String> {
    let mut pending = state.wayland_pending_capture.lock().map_err(|e| e.to_string())?;
    Ok(pending.take())
}

/// Comando Tauri para iniciar captura (llamado desde el botón del frontend).
#[tauri::command]
pub fn start_area_capture(app: tauri::AppHandle) -> Result<(), String> {
    show_capture_overlay(&app, false);
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

    // finalize_area_capture solo es válido en X11 (el overlay X11 envía coordenadas).
    // En Wayland el portal maneja la selección y el resultado va por finalize_annotated_capture.
    if state.display_server == capture::DisplayServer::Wayland {
        return Err("finalize_area_capture no está disponible en Wayland".to_string());
    }

    // Ungrab inmediato para liberar el input antes de cualquier otra cosa.
    #[cfg(target_os = "linux")]
    if let Err(e) = crate::x11_grab::ungrab_input() {
        eprintln!("ungrab_input: {e}");
    }

    // Destruir el overlay después del export: la captura terminó, liberar memoria.
    // El overlay se mantiene vivo (hide) solo después de ESC para retry inmediato.
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        if let Some(overlay) = app2.get_webview_window("capture-overlay") {
            let _ = overlay.close();
        }
    });

    // Esperar a que el overlay desaparezca realmente en pantalla (200ms > 50ms del spawn).
    std::thread::sleep(std::time::Duration::from_millis(200));

    // Capturar la región real del escritorio (X11)
    let result = capture::capture_region_x11(abs_x, abs_y, abs_w, abs_h)
        .map_err(|e| e.to_string())?;

    // Decodificar una sola vez: sirve para clipboard y para escribir a disco.
    let png_bytes = STANDARD.decode(&result.content).map_err(|e| e.to_string())?;

    // Copiar al portapapeles del sistema
    clipboard::copy_png_bytes_to_clipboard(&png_bytes)?;

    // Guardar PNG en ~/Pictures/screenshots/aurora-screenshots/
    let path = save_png_bytes_to_disk(&png_bytes)?;

    // Guardar path en DB (ya no el base64 completo)
    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db::insert_entry(&db, "image", &path, Some(&result.thumbnail))
            .map_err(|e| e.to_string())?;
    }

    // Liberar el screenshot del escritorio: ya no hace falta en AppState.
    if let Ok(mut bg) = state.desktop_background.lock() {
        *bg = None;
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
        "image" => {
            if entry.content.starts_with('/') {
                // Nuevo formato: content es un path en disco
                let png_bytes = std::fs::read(&entry.content).map_err(|e| e.to_string())?;
                clipboard::copy_png_bytes_to_clipboard(&png_bytes)
            } else {
                // Formato legacy: content es base64
                clipboard::copy_png_b64_to_clipboard(&entry.content)
            }
        }
        other => Err(format!("Tipo de item desconocido: {other}")),
    }
}

// ─── Captura con anotaciones ──────────────────────────────────────────────

/// Oculta el overlay de captura desde el frontend (más confiable que la API JS en Tauri).
#[tauri::command]
pub fn hide_capture_overlay(state: State<AppState>, app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    if state.display_server == capture::DisplayServer::X11 {
        if let Err(e) = crate::x11_grab::ungrab_input() {
            eprintln!("ungrab_input: {e}");
        }
    }
    if let Some(overlay) = app.get_webview_window("capture-overlay") {
        let _ = overlay.hide();
    }
    Ok(())
}

/// Cierra (destruye) el overlay de captura desde el frontend.
/// Úsalo en ESC para que la próxima captura arranque con una ventana WebKit limpia,
/// evitando el bug de "primer teclazo da foco, segundo ejecuta la acción".
#[tauri::command]
pub fn close_capture_overlay(state: State<AppState>, app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    if state.display_server == capture::DisplayServer::X11 {
        if let Err(e) = crate::x11_grab::ungrab_input() {
            eprintln!("ungrab_input: {e}");
        }
        // Invalida el XID almacenado — la nueva ventana tendrá uno distinto.
        if let Ok(mut xid) = state.overlay_xid.lock() {
            *xid = None;
        }
    }
    // Liberar datos de captura al cancelar con ESC.
    if let Ok(mut bg) = state.desktop_background.lock() {
        *bg = None;
    }
    if let Ok(mut pending) = state.wayland_pending_capture.lock() {
        *pending = None;
    }
    if let Some(overlay) = app.get_webview_window("capture-overlay") {
        let _ = overlay.close();
    }
    Ok(())
}

/// Devuelve el shortcut activo para captura.
#[tauri::command]
pub fn get_capture_shortcut(state: State<AppState>) -> Result<String, String> {
    state.current_shortcut.lock().map_err(|e| e.to_string()).map(|s| s.clone())
}

/// Registra un nuevo shortcut para captura, reemplazando el anterior. Persiste en DB.
#[tauri::command]
pub fn update_capture_shortcut(
    state: State<AppState>,
    app: tauri::AppHandle,
    shortcut: String,
) -> Result<(), String> {
    let new_sc = crate::parse_shortcut(&shortcut)?;

    // Desregistrar el shortcut actual.
    let old_str = state.current_shortcut.lock().map_err(|e| e.to_string())?.clone();
    if let Ok(old_sc) = crate::parse_shortcut(&old_str) {
        let _ = app.global_shortcut().unregister(old_sc);
    }

    // Registrar el nuevo con el mismo handler.
    app.global_shortcut()
        .on_shortcut(new_sc, |app, _sc, event| {
            if event.state() == ShortcutState::Pressed {
                show_capture_overlay(app, false);
            }
        })
        .map_err(|e| e.to_string())?;

    // Actualizar estado en memoria.
    *state.current_shortcut.lock().map_err(|e| e.to_string())? = shortcut.clone();

    // Persistir en DB.
    let db = state.db.lock().map_err(|e| e.to_string())?;
    crate::db::set_setting(&db, "capture_shortcut", &shortcut).map_err(|e| e.to_string())
}

// ─── Autoarranque ─────────────────────────────────────────────────────────

fn autostart_desktop_path() -> Result<std::path::PathBuf, String> {
    let home = std::env::var("HOME").map_err(|_| "No se encontró $HOME".to_string())?;
    Ok(std::path::PathBuf::from(home)
        .join(".config")
        .join("autostart")
        .join("aurora-screenshots.desktop"))
}

/// Verifica si el autoarranque al inicio del sistema está habilitado.
#[tauri::command]
pub fn get_autostart() -> Result<bool, String> {
    let path = autostart_desktop_path()?;
    Ok(path.exists())
}

/// Habilita o deshabilita el autoarranque al inicio del sistema.
#[tauri::command]
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let path = autostart_desktop_path()?;
    if enabled {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let desktop = format!(
            "[Desktop Entry]\nType=Application\nName=Aurora Screenshots\nExec={}\nHidden=false\nNoDisplay=false\nX-GNOME-Autostart-enabled=true\n",
            exe.display()
        );
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, desktop).map_err(|e| e.to_string())?;
    } else if path.exists() {
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Muestra la ventana principal. Llamado por el frontend cuando React ya está montado,
/// para evitar el flash de pantalla blanca al abrir el historial por primera vez.
#[tauri::command]
pub fn show_main_window(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.set_always_on_top(true);
        let _ = win.show();
        let _ = win.set_focus();
        let _ = win.set_always_on_top(false);
    }
    Ok(())
}

/// Cierra (destruye) la ventana principal desde el frontend.
/// Con ventanas lazy no hay razón para ocultarla — el tray la recreará cuando haga falta.
#[tauri::command]
pub fn hide_main_window(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.close();
    }
    Ok(())
}

/// Retorna el screenshot del escritorio capturado antes de mostrar el overlay.
/// Usa take() en lugar de clone() para mover el String fuera del AppState:
/// evita tener dos copias de ~15-20 MB vivas durante el IPC.
/// Retorna los monitores capturados como JPEG individuales con su posición.
/// Después de esta llamada desktop_background queda en None — el frontend ya tiene el dato.
#[tauri::command]
pub fn get_desktop_background(state: State<AppState>) -> Result<Option<Vec<capture::MonitorCapture>>, String> {
    let mut bg = state.desktop_background.lock().map_err(|e| e.to_string())?;
    Ok(bg.take())
}

/// El frontend llama a este comando con el PNG anotado (base64) para guardarlo y
/// copiarlo al portapapeles. No requiere coordenadas — el canvas ya tiene el recorte.
#[tauri::command]
pub fn finalize_annotated_capture(
    state: State<AppState>,
    app: tauri::AppHandle,
    image_data: String,
) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    if state.display_server == capture::DisplayServer::X11 {
        if let Err(e) = crate::x11_grab::ungrab_input() {
            eprintln!("ungrab_input: {e}");
        }
    }

    // Destruir el overlay después del export: la captura terminó, liberar memoria.
    // El overlay se mantiene vivo (hide) solo después de ESC para retry inmediato.
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        if let Some(overlay) = app2.get_webview_window("capture-overlay") {
            let _ = overlay.close();
        }
    });

    // Decodificar una sola vez: clipboard, disco y thumbnail.
    let png_bytes = STANDARD.decode(&image_data).map_err(|e| e.to_string())?;

    // Copiar al portapapeles
    clipboard::copy_png_bytes_to_clipboard(&png_bytes)?;

    // Guardar PNG en ~/Pictures/screenshots/aurora-screenshots/
    let path = save_png_bytes_to_disk(&png_bytes)?;

    // Generar miniatura
    let thumbnail = capture::generate_thumbnail_b64(&png_bytes).map_err(|e| e.to_string())?;

    // Guardar path en DB (ya no el base64 completo)
    {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        db::insert_entry(&db, "image", &path, Some(&thumbnail))
            .map_err(|e| e.to_string())?;
    }

    // Liberar el screenshot del escritorio: ya no hace falta en AppState.
    if let Ok(mut bg) = state.desktop_background.lock() {
        *bg = None;
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
/// Al terminar cierra el overlay (en background para que el invoke response llegue primero).
#[tauri::command]
pub fn write_screenshot_file(app: tauri::AppHandle, path: String, image_data: String) -> Result<(), String> {
    let png_bytes = STANDARD.decode(&image_data).map_err(|e| e.to_string())?;
    std::fs::write(&path, &png_bytes).map_err(|e| e.to_string())?;

    // Destruir el overlay después de guardar el archivo: captura terminada, liberar memoria.
    // El XID se puede invalidar aquí porque el overlay se va a recrear en la próxima captura.
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        #[cfg(target_os = "linux")]
        {
            let state = app.state::<AppState>();
            if let Ok(mut xid) = state.overlay_xid.lock() {
                *xid = None;
            };
        }
        if let Some(overlay) = app.get_webview_window("capture-overlay") {
            let _ = overlay.close();
        }
    });

    Ok(())
}

/// Abre ~/Pictures/aurora-screenshots en el gestor de archivos del sistema.
#[tauri::command]
pub fn open_screenshots_folder() -> Result<(), String> {
    let dir = get_screenshots_dir()?;
    let path_str = dir.to_str().ok_or("Ruta no válida UTF-8")?;
    tauri_plugin_opener::open_path(path_str, None::<&str>).map_err(|e| e.to_string())
}

// ─── Helpers privados ─────────────────────────────────────────────────────

/// Retorna (creando si no existe) ~/Pictures/screenshots/aurora-screenshots.
fn get_screenshots_dir() -> Result<std::path::PathBuf, String> {
    let home = std::env::var("HOME").map_err(|e| e.to_string())?;
    let dir = std::path::Path::new(&home)
        .join("Pictures")
        .join("aurora-screenshots");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Escribe bytes PNG en ~/Pictures/screenshots/aurora-screenshots/aurora_<ms>.png
/// y retorna la ruta absoluta como String.
fn save_png_bytes_to_disk(png_bytes: &[u8]) -> Result<String, String> {
    let dir = get_screenshots_dir()?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let path = dir.join(format!("aurora_{ts}.png"));
    std::fs::write(&path, png_bytes).map_err(|e| e.to_string())?;
    path.to_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "Ruta no válida UTF-8".to_string())
}

