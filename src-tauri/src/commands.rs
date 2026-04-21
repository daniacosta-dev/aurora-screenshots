use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use tauri::{Emitter, Manager, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

#[cfg(target_os = "linux")]
use gtk::prelude::{GtkWindowExt, WidgetExt};
#[cfg(target_os = "linux")]
use gdk::prelude::MonitorExt;

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

/// Fallback de captura Wayland usando PipeWire ScreenCast con restore token.
/// Solo se usa cuando la captura vía GDK+desktop falla (entornos no estándar).
async fn capture_screencast_fallback(app: &tauri::AppHandle) -> Vec<capture::MonitorCapture> {
    let restore_token = {
        let state = app.state::<AppState>();
        state.db.lock().ok()
            .and_then(|db| db::get_setting(&db, "screencast_restore_token").ok())
            .flatten()
    };
    eprintln!("[screencast_fallback] restore_token={}", restore_token.is_some());

    match capture::capture_via_screencast(restore_token).await {
        Ok((monitors, new_token)) => {
            if let Some(token) = new_token {
                let state = app.state::<AppState>();
                if let Ok(db) = state.db.lock() {
                    let _ = db::set_setting(&db, "screencast_restore_token", &token);
                    eprintln!("[screencast_fallback] restore token saved");
                };
            }
            monitors
        }
        Err(e) => {
            eprintln!("[screencast_fallback] screencast failed ({e}), trying capture_monitors_wayland");
            capture::capture_monitors_wayland().await.unwrap_or_default()
        }
    }
}

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
                eprintln!("[show_overlay wayland] capturing full screen...");

                if from_tray {
                    tokio::time::sleep(std::time::Duration::from_millis(40)).await;
                }

                // Obtener geometrías de monitores vía GDK (requiere GTK main thread).
                let (geo_tx, geo_rx) = tokio::sync::oneshot::channel::<Vec<(i32, i32, u32, u32)>>();
                #[cfg(target_os = "linux")]
                if let Err(e) = app.run_on_main_thread(move || {
                    let _ = geo_tx.send(get_gdk_monitor_geometries());
                }) {
                    eprintln!("[show_overlay wayland] run_on_main_thread geo: {e}");
                }
                let gdk_monitors = geo_rx.await.unwrap_or_default();
                eprintln!("[show_overlay wayland] GDK: {} monitor(s): {:?}", gdk_monitors.len(), gdk_monitors);

                // Estrategia primaria: captura completa + split por monitor (sin PipeWire,
                // sin tokens, sin diálogos — confiable en GNOME y wlroots).
                let monitors = if !gdk_monitors.is_empty() {
                    match capture::capture_full_desktop_wayland_bytes().await {
                        Ok(bytes) => {
                            match capture::split_image_by_monitors(&bytes, &gdk_monitors) {
                                Ok(m) => {
                                    eprintln!("[show_overlay wayland] split ok: {} monitor(s)", m.len());
                                    m
                                }
                                Err(e) => {
                                    eprintln!("[show_overlay wayland] split error: {e}, fallback screencast");
                                    capture_screencast_fallback(&app).await
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("[show_overlay wayland] full desktop failed: {e}, fallback screencast");
                            capture_screencast_fallback(&app).await
                        }
                    }
                } else {
                    // Sin info GDK (entorno no estándar): usar PipeWire como antes.
                    eprintln!("[show_overlay wayland] sin GDK monitors, usando screencast");
                    capture_screencast_fallback(&app).await
                };

                let monitors = match monitors {
                    m if !m.is_empty() => m,
                    _ => {
                        eprintln!("[show_overlay wayland] todos los métodos de captura fallaron");
                        return;
                    }
                };

                for m in &monitors {
                    eprintln!("[show_overlay wayland] monitor: pos=({},{}) size={}x{}", m.x, m.y, m.width, m.height);
                }

                // Guardar monitors en AppState para que get_monitor_background los sirva.
                {
                    let state = app.state::<AppState>();
                    if let Ok(mut bg) = state.desktop_background.lock() {
                        *bg = Some(monitors.clone());
                    };
                    if let Ok(mut offset) = state.overlay_offset.lock() {
                        *offset = (0, 0);
                    };
                }

                // Cerrar ventanas viejas de overlay que ya no corresponden a monitores actuales.
                for label in app.webview_windows().keys().cloned().collect::<Vec<_>>() {
                    if label.starts_with("capture-overlay-") {
                        if let Some(w) = app.get_webview_window(&label) {
                            let _ = w.close();
                        }
                    }
                }
                // Breve pausa para que GTK cierre las ventanas antes de crear las nuevas.
                tokio::time::sleep(std::time::Duration::from_millis(60)).await;

                // Crear UNA ventana fullscreen por monitor y posicionarla con GTK.
                //
                // Estrategia en dos fases para que todas las ventanas aparezcan simultáneamente:
                //   Fase 1: crear todas las ventanas OCULTAS en secuencia.
                //   Fase 2: una única llamada a run_on_main_thread muestra TODAS en el mismo
                //           tick del GTK main loop — el compositor las recibe juntas.

                // Fase 1: crear todas las ventanas ocultas.
                let mut window_monitors: Vec<(tauri::WebviewWindow, i32, i32, usize)> = Vec::new();
                for (i, monitor) in monitors.iter().enumerate() {
                    let label = format!("capture-overlay-{i}");
                    let mon_x = monitor.x;
                    let mon_y = monitor.y;
                    match tauri::WebviewWindowBuilder::new(
                        &app,
                        &label,
                        tauri::WebviewUrl::App("index.html".into()),
                    )
                    .transparent(true)
                    .decorations(false)
                    .always_on_top(true)
                    .skip_taskbar(true)
                    .visible(false)
                    .resizable(false)
                    .inner_size(monitor.width as f64, monitor.height as f64)
                    .build() {
                        Ok(w) => {
                            eprintln!("[show_overlay wayland] created overlay-{i} at ({mon_x},{mon_y}) {}x{}", monitor.width, monitor.height);
                            window_monitors.push((w, mon_x, mon_y, i));
                        }
                        Err(e) => eprintln!("[show_overlay wayland] failed to create overlay-{i}: {e}"),
                    }
                }

                if window_monitors.is_empty() {
                    eprintln!("[show_overlay wayland] no overlay windows created");
                    return;
                }

                // Esperar a que todos los WebViews terminen de inicializarse antes de
                // tocar GTK. Una sola pausa cubre todos los monitores a la vez.
                tokio::time::sleep(std::time::Duration::from_millis(80)).await;

                // Fase 2: mostrar TODAS las ventanas en un único tick del GTK main loop
                // para que el compositor las reciba simultáneamente.
                #[cfg(target_os = "linux")]
                {
                    let wins: Vec<(tauri::WebviewWindow, i32, i32, usize)> = window_monitors.clone();
                    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
                    if let Err(e) = app.run_on_main_thread(move || {
                        for (win, mon_x, mon_y, i) in &wins {
                            match win.gtk_window() {
                                Ok(gtk_win) => {
                                    fullscreen_on_gtk_window(&gtk_win, *mon_x, *mon_y);
                                    gtk_win.show_all();
                                    eprintln!("[show_overlay wayland] show_all() for overlay-{i}");
                                }
                                Err(e) => eprintln!("[show_overlay wayland] gtk_window() error for overlay-{i}: {e}"),
                            }
                        }
                        let _ = tx.send(());
                    }) {
                        eprintln!("[show_overlay wayland] run_on_main_thread error: {e}");
                    } else {
                        let _ = rx.await;
                    }
                }

                // Pausa para que el compositor procese el mapeo inicial de todas las ventanas.
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;

                // Notificar a todos los frontends simultáneamente.
                for (win, mon_x, mon_y, i) in &window_monitors {
                    eprintln!("[show_overlay wayland] emitting background-ready for overlay-{i} at ({mon_x},{mon_y})");
                    let _ = win.emit("background-ready", ());
                }
            }
        }
    });
}

/// Retorna las geometrías de todos los monitores GDK en píxeles físicos: (x, y, w, h).
/// Debe llamarse desde el GTK main thread.
#[cfg(target_os = "linux")]
fn get_gdk_monitor_geometries() -> Vec<(i32, i32, u32, u32)> {
    let display = match gdk::Display::default() {
        Some(d) => d,
        None => { eprintln!("[gdk_geometries] no default GDK display"); return vec![]; }
    };
    let mut result = Vec::new();
    for i in 0..display.n_monitors() {
        if let Some(monitor) = display.monitor(i) {
            let geo = monitor.geometry();
            let scale = monitor.scale_factor();
            eprintln!(
                "[gdk_geometries] monitor[{i}]: pos=({},{}) size={}x{} scale={}",
                geo.x(), geo.y(), geo.width(), geo.height(), scale
            );
            // GDK devuelve coordenadas lógicas; multiplicar por scale_factor da píxeles físicos.
            result.push((
                geo.x() * scale,
                geo.y() * scale,
                (geo.width() * scale) as u32,
                (geo.height() * scale) as u32,
            ));
        }
    }
    result
}

/// Devuelve el índice GDK del monitor cuyas coordenadas coincidan con (target_x, target_y).
/// Retorna -1 si no se encuentra (el caller debería usar fullscreen() genérico).
#[cfg(target_os = "linux")]
fn find_gdk_monitor_idx(target_x: i32, target_y: i32) -> i32 {
    let display = match gdk::Display::default() {
        Some(d) => d,
        None => return -1,
    };
    for i in 0..display.n_monitors() {
        if let Some(monitor) = display.monitor(i) {
            let geo = monitor.geometry();
            let scale = monitor.scale_factor();
            if geo.x() == target_x && geo.y() == target_y { return i; }
            if geo.x() * scale == target_x && geo.y() * scale == target_y { return i; }
        }
    }
    -1
}

/// Pone un GtkWindow en fullscreen sobre el monitor cuyas coordenadas GDK coincidan con
/// (target_x, target_y). Debe llamarse desde el GTK main thread.
/// Toma gtk::ApplicationWindow directamente — sin pasar por Tauri — para usarse
/// en el mismo tick de run_on_main_thread junto con show_all().
#[cfg(target_os = "linux")]
fn fullscreen_on_gtk_window(gtk_win: &gtk::ApplicationWindow, target_x: i32, target_y: i32) {
    let display = match gdk::Display::default() {
        Some(d) => d,
        None => { eprintln!("[fullscreen_on_monitor] no default GDK display"); return; }
    };

    let n = display.n_monitors();
    eprintln!("[fullscreen_on_monitor] target=({target_x},{target_y}), GDK n_monitors={n}");

    let mut found_idx: Option<i32> = None;
    for i in 0..n {
        if let Some(monitor) = display.monitor(i) {
            let geo = monitor.geometry();
            let scale = monitor.scale_factor();
            eprintln!("[fullscreen_on_monitor] GDK[{i}]: pos=({},{}) size={}x{} scale={scale}", geo.x(), geo.y(), geo.width(), geo.height());
            if geo.x() == target_x && geo.y() == target_y {
                found_idx = Some(i);
                break;
            }
            if geo.x() * scale == target_x && geo.y() * scale == target_y {
                found_idx = Some(i);
                break;
            }
        }
    }

    match found_idx {
        Some(idx) => {
            eprintln!("[fullscreen_on_monitor] matched GDK[{idx}]");
            if let Some(gdk_win) = WidgetExt::window(gtk_win) {
                // GdkWindow ya existe (ventana realizada) — llamada directa al backend Wayland.
                eprintln!("[fullscreen_on_monitor] GdkWindow realized → gdk_win.fullscreen_on_monitor({idx})");
                gdk_win.fullscreen_on_monitor(idx);
            } else {
                // GdkWindow aún no existe (ventana no realizada). Seteamos el estado inicial
                // en GtkWindow: cuando se realice/mapee, aplicará fullscreen en monitor {idx}.
                eprintln!("[fullscreen_on_monitor] GdkWindow not yet → gtk_win.fullscreen_on_monitor(screen, {idx})");
                let screen = match WidgetExt::screen(gtk_win) {
                    Some(s) => s,
                    None => { eprintln!("[fullscreen_on_monitor] no screen"); return; }
                };
                gtk_win.fullscreen_on_monitor(&screen, idx);
            }
        }
        None => {
            eprintln!("[fullscreen_on_monitor] no match for ({target_x},{target_y}) — fallback fullscreen()");
            gtk_win.fullscreen();
        }
    }
}

/// El frontend llama a este comando cuando ha terminado de renderizar el fondo en canvas.
/// Hace show() del overlay y establece el grab de input — garantiza que el overlay
/// aparezca directamente con la imagen cargada, sin flash negro previo.
/// Solo relevante en X11; en Wayland el overlay ya está visible antes de este llamado.
#[tauri::command]
pub async fn overlay_ready(window: tauri::WebviewWindow, state: State<'_, AppState>) -> Result<(), String> {
    let overlay = window;

    eprintln!("[overlay_ready] showing overlay");
    let _ = overlay.show();

    tokio::time::sleep(std::time::Duration::from_millis(15)).await;

    let _ = overlay.set_focus();
    let _ = overlay.set_ignore_cursor_events(false);

    // En Wayland no hay grab de input — emitir grab-ready directamente para que
    // el frontend re-solicite foco a WebKit.
    #[cfg(target_os = "linux")]
    if state.display_server == capture::DisplayServer::Wayland {
        let _ = overlay.emit("grab-ready", ());
        return Ok(());
    }

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
pub fn hide_capture_overlay(state: State<AppState>, app: tauri::AppHandle, window: tauri::WebviewWindow) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    if state.display_server == capture::DisplayServer::X11 {
        if let Err(e) = crate::x11_grab::ungrab_input() {
            eprintln!("ungrab_input: {e}");
        }
    }
    // En Wayland: ocultar todas las ventanas capture-overlay-*.
    // En X11: ocultar solo "capture-overlay".
    let label = window.label().to_string();
    if label.starts_with("capture-overlay-") {
        for (lbl, w) in app.webview_windows() {
            if lbl.starts_with("capture-overlay-") {
                let _ = w.hide();
            }
        }
    } else if let Some(overlay) = app.get_webview_window("capture-overlay") {
        let _ = overlay.hide();
    }
    Ok(())
}

/// Cierra (destruye) el overlay de captura desde el frontend.
/// Úsalo en ESC para que la próxima captura arranque con una ventana WebKit limpia,
/// evitando el bug de "primer teclazo da foco, segundo ejecuta la acción".
#[tauri::command]
pub fn close_capture_overlay(state: State<AppState>, app: tauri::AppHandle, window: tauri::WebviewWindow) -> Result<(), String> {
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
    // En Wayland: cerrar todas las ventanas capture-overlay-*.
    // En X11: cerrar solo "capture-overlay".
    let label = window.label().to_string();
    if label.starts_with("capture-overlay-") {
        for (lbl, w) in app.webview_windows() {
            if lbl.starts_with("capture-overlay-") {
                let _ = w.close();
            }
        }
    } else if let Some(overlay) = app.get_webview_window("capture-overlay") {
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

/// Oculta la ventana principal desde el frontend.
#[tauri::command]
pub fn hide_main_window(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.hide();
    }
    Ok(())
}

#[tauri::command]
pub fn get_history_window_mode(state: State<AppState>) -> &'static str {
    if state.history_fullscreen { "fullscreen" } else { "panel" }
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

/// Retorna el background del monitor específico para una ventana "capture-overlay-N".
/// A diferencia de get_desktop_background, NO consume el dato — permite que cada
/// ventana de overlay solicite su propio monitor sin que interfieran entre sí.
#[tauri::command]
pub fn get_monitor_background(
    state: State<AppState>,
    window: tauri::WebviewWindow,
) -> Result<Option<capture::MonitorCapture>, String> {
    // Extraer índice del label "capture-overlay-N"
    let label = window.label().to_string();
    let idx: usize = label
        .strip_prefix("capture-overlay-")
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("Label inválido para get_monitor_background: {label}"))?;

    let bg = state.desktop_background.lock().map_err(|e| e.to_string())?;
    Ok(bg.as_ref().and_then(|monitors| monitors.get(idx).cloned()))
}

/// Borra el restore token del ScreenCast portal de la DB.
/// El próximo screenshot mostrará el diálogo de selección de monitores,
/// permitiendo al usuario elegir qué monitores incluir.
#[tauri::command]
pub fn reset_screencast_token(state: State<AppState>) -> Result<(), String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    db::delete_setting(&db, "screencast_restore_token").map_err(|e| e.to_string())?;
    eprintln!("[reset_screencast_token] token borrado");
    Ok(())
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
    // En Wayland hay múltiples ventanas capture-overlay-*; en X11 solo "capture-overlay".
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        // Cerrar Wayland per-monitor overlays
        for (lbl, w) in app2.webview_windows() {
            if lbl.starts_with("capture-overlay-") {
                let _ = w.close();
            }
        }
        // Cerrar X11 overlay (no-op si no existe)
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

