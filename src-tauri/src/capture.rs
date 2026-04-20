use base64::{engine::general_purpose::STANDARD, Engine as _};
use image::{
    codecs::jpeg::JpegEncoder,
    DynamicImage, ImageBuffer, ImageFormat, Rgba, RgbaImage,
};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use std::sync::{Arc, Mutex};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("Captura fallida: {0}")]
    Capture(String),
    #[error("Error procesando imagen: {0}")]
    ImageProcessing(String),
    #[error("Display server desconocido o no disponible")]
    NoDisplay,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CaptureResult {
    pub content: String,   // base64 PNG completo
    pub thumbnail: String, // base64 PNG reducido (240x160 max)
    pub width: u32,
    pub height: u32,
}

/// Un monitor capturado como JPEG con su posición en el desktop virtual.
/// El frontend dibuja cada uno en el canvas en su posición exacta — sin compositing en Rust.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MonitorCapture {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub data: String, // base64 JPEG
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DisplayServer {
    X11,
    Wayland,
}

/// Detecta el display server leyendo variables de entorno.
/// Wayland tiene precedencia sobre X11 cuando ambos están presentes.
pub fn detect_display_server() -> Result<DisplayServer, CaptureError> {
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        return Ok(DisplayServer::Wayland);
    }
    if std::env::var("DISPLAY").is_ok() {
        return Ok(DisplayServer::X11);
    }
    Err(CaptureError::NoDisplay)
}

/// Devuelve el bounding box que abarca todos los monitores en X11.
/// Retorna (min_x, min_y, total_width, total_height) en píxeles físicos.
pub fn all_monitors_bounds_x11() -> Result<(i32, i32, u32, u32), CaptureError> {
    let screens = screenshots::Screen::all()
        .map_err(|e| CaptureError::Capture(format!("No se pudo enumerar pantallas: {e}")))?;

    if screens.is_empty() {
        return Err(CaptureError::Capture("No se encontraron pantallas".to_string()));
    }

    let min_x = screens.iter().map(|s| s.display_info.x).min().unwrap_or(0);
    let min_y = screens.iter().map(|s| s.display_info.y).min().unwrap_or(0);
    let max_x = screens
        .iter()
        .map(|s| s.display_info.x + s.display_info.width as i32)
        .max()
        .unwrap_or(1920);
    let max_y = screens
        .iter()
        .map(|s| s.display_info.y + s.display_info.height as i32)
        .max()
        .unwrap_or(1080);

    Ok((min_x, min_y, (max_x - min_x) as u32, (max_y - min_y) as u32))
}

/// Captura una región específica del escritorio virtual X11 (multi-monitor).
/// `abs_x` y `abs_y` son coordenadas absolutas del escritorio virtual (píxeles físicos).
pub fn capture_region_x11(abs_x: i32, abs_y: i32, width: u32, height: u32) -> Result<CaptureResult, CaptureError> {
    // Screen::from_point encuentra el monitor que contiene ese punto virtual
    let screen = screenshots::Screen::from_point(abs_x, abs_y)
        .map_err(|e| CaptureError::Capture(format!("Monitor no encontrado en ({abs_x},{abs_y}): {e}")))?;

    // Las coordenadas de capture_area son relativas al origen del monitor
    let rel_x = abs_x - screen.display_info.x;
    let rel_y = abs_y - screen.display_info.y;

    let image = screen
        .capture_area(rel_x, rel_y, width, height)
        .map_err(|e| CaptureError::Capture(format!("capture_area falló: {e}")))?;

    let raw_pixels = image.into_raw();
    let rgba_image = RgbaImage::from_raw(width, height, raw_pixels)
        .ok_or_else(|| CaptureError::ImageProcessing("Buffer RGBA inválido".to_string()))?;

    process_image(DynamicImage::ImageRgba8(rgba_image), width, height)
}

/// Captura Wayland con selección interactiva de área (el portal muestra su propia UI).
pub async fn capture_area_wayland_interactive() -> Result<CaptureResult, CaptureError> {
    let response = ashpd::desktop::screenshot::Screenshot::request()
        .interactive(true)
        .send()
        .await
        .map_err(|e| CaptureError::Capture(format!("XDG Portal error: {e}")))?
        .response()
        .map_err(|e| CaptureError::Capture(format!("Portal response error: {e}")))?;

    let uri = response.uri();
    eprintln!("[wayland] portal URI: {uri}");

    let path = uri.to_file_path()
        .map_err(|_| CaptureError::Capture(format!("URI no es un path local válido: {uri}")))?;

    let data = std::fs::read(&path)
        .map_err(|e| CaptureError::Capture(format!("No se pudo leer la captura: {e}")))?;
    let _ = std::fs::remove_file(&path);

    let img = image::load_from_memory(&data)
        .map_err(|e| CaptureError::ImageProcessing(format!("Imagen inválida: {e}")))?;

    let width = img.width();
    let height = img.height();
    process_image(img, width, height)
}

/// Captura la pantalla completa en Wayland.
/// Intenta el portal XDG primero (universal); si falla, usa GNOME Shell D-Bus como fallback.
/// Devuelve Vec<MonitorCapture> compatible con el flujo X11 del frontend.
pub async fn capture_monitors_wayland() -> Result<Vec<MonitorCapture>, CaptureError> {
    // Intento 1: GNOME Shell D-Bus (sin UI, sin permisos extra)
    match capture_via_gnome_shell().await {
        Ok(monitors) => return Ok(monitors),
        Err(e) => eprintln!("[wayland] GNOME Shell D-Bus failed ({e}), trying wlroots fallback..."),
    }

    // Intento 2: grim (wlroots: Hyprland, Sway, etc.)
    match capture_via_grim().await {
        Ok(monitors) => return Ok(monitors),
        Err(e) => eprintln!("[wayland] grim failed ({e}), trying XDG portal..."),
    }

    // Intento 3: portal XDG (puede mostrar diálogo de permisos)
    match capture_via_portal_noninteractive().await {
        Ok(monitors) => return Ok(monitors),
        Err(e) => eprintln!("[wayland] portal non-interactive failed: {e}"),
    }

    Err(CaptureError::Capture(
        "No se pudo capturar la pantalla. Instalá 'grim' (wlroots) o verificá permisos en Configuración → Privacidad → Pantalla.".to_string(),
    ))
}

async fn capture_via_portal_noninteractive() -> Result<Vec<MonitorCapture>, CaptureError> {
    let response = ashpd::desktop::screenshot::Screenshot::request()
        .interactive(false)
        .send()
        .await
        .map_err(|e| CaptureError::Capture(format!("XDG Portal error: {e}")))?
        .response()
        .map_err(|e| CaptureError::Capture(format!("Portal response error: {e}")))?;

    let uri = response.uri();
    let path = uri.to_file_path()
        .map_err(|_| CaptureError::Capture(format!("URI inválido: {uri}")))?;

    let data = std::fs::read(&path)
        .map_err(|e| CaptureError::Capture(format!("No se pudo leer la captura: {e}")))?;
    let _ = std::fs::remove_file(&path);

    image_bytes_to_monitor_captures(&data)
}

/// Usa la API D-Bus de GNOME Shell para capturar la pantalla completa.
/// No requiere permisos del portal — funciona en cualquier sesión GNOME Wayland.
async fn capture_via_gnome_shell() -> Result<Vec<MonitorCapture>, CaptureError> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let path = format!("/tmp/aurora-shot-{ts}.png");

    // org.gnome.Shell.Screenshot.Screenshot(include_cursor, flash, filename)
    let output = tokio::process::Command::new("gdbus")
        .args([
            "call", "--session",
            "--dest", "org.gnome.Shell.Screenshot",
            "--object-path", "/org/gnome/Shell/Screenshot",
            "--method", "org.gnome.Shell.Screenshot.Screenshot",
            "false", "false", &path,
        ])
        .output()
        .await
        .map_err(|e| CaptureError::Capture(format!("gdbus not found: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CaptureError::Capture(format!("GNOME Shell screenshot failed: {stderr}")));
    }

    // gdbus devuelve "(true, 'path')" — verificar el booleano
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim_start().starts_with("(false") {
        return Err(CaptureError::Capture("GNOME Shell Screenshot returned false".to_string()));
    }

    let data = std::fs::read(&path)
        .map_err(|e| CaptureError::Capture(format!("No se pudo leer screenshot: {e}")))?;
    let _ = std::fs::remove_file(&path);

    image_bytes_to_monitor_captures(&data)
}

/// Usa `grim` para capturar en compositors wlroots (Hyprland, Sway, river, etc.).
/// Requiere que `grim` esté instalado: `sudo pacman -S grim` o `sudo apt install grim`.
async fn capture_via_grim() -> Result<Vec<MonitorCapture>, CaptureError> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let path = format!("/tmp/aurora-shot-{ts}.png");

    let output = tokio::process::Command::new("grim")
        .arg(&path)
        .output()
        .await
        .map_err(|e| CaptureError::Capture(format!("grim not found: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CaptureError::Capture(format!("grim failed: {stderr}")));
    }

    let data = std::fs::read(&path)
        .map_err(|e| CaptureError::Capture(format!("No se pudo leer screenshot de grim: {e}")))?;
    let _ = std::fs::remove_file(&path);

    image_bytes_to_monitor_captures(&data)
}

/// Captura el escritorio completo como bytes PNG crudos.
/// Intenta GNOME Shell D-Bus primero (GNOME Wayland), luego `grim` (wlroots).
/// No composita ni convierte — devuelve bytes para que el caller los divida por monitor.
pub async fn capture_full_desktop_wayland_bytes() -> Result<Vec<u8>, CaptureError> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    // Intento 1: GNOME Shell D-Bus
    let path = format!("/tmp/aurora-desk-{ts}.png");
    let out = tokio::process::Command::new("gdbus")
        .args([
            "call", "--session",
            "--dest", "org.gnome.Shell.Screenshot",
            "--object-path", "/org/gnome/Shell/Screenshot",
            "--method", "org.gnome.Shell.Screenshot.Screenshot",
            "false", "false", &path,
        ])
        .output()
        .await;
    if let Ok(o) = out {
        if o.status.success() && !String::from_utf8_lossy(&o.stdout).trim_start().starts_with("(false") {
            if let Ok(data) = std::fs::read(&path) {
                let _ = std::fs::remove_file(&path);
                eprintln!("[wayland_bytes] GNOME Shell D-Bus ok: {}x? ({} bytes)", "?", data.len());
                return Ok(data);
            }
        }
    }
    let _ = std::fs::remove_file(&path);

    // Intento 2: grim (wlroots: Hyprland, Sway, etc.)
    let path2 = format!("/tmp/aurora-desk-{ts}b.png");
    let out2 = tokio::process::Command::new("grim")
        .arg(&path2)
        .output()
        .await;
    if let Ok(o2) = out2 {
        if o2.status.success() {
            if let Ok(data) = std::fs::read(&path2) {
                let _ = std::fs::remove_file(&path2);
                eprintln!("[wayland_bytes] grim ok ({} bytes)", data.len());
                return Ok(data);
            }
        } else {
            eprintln!("[wayland_bytes] grim failed: {}", String::from_utf8_lossy(&o2.stderr));
        }
    }
    let _ = std::fs::remove_file(&path2);

    Err(CaptureError::Capture(
        "No se pudo capturar el escritorio (gdbus y grim fallaron). \
         Instalá grim o verificá permisos de GNOME Shell Screenshot.".to_string(),
    ))
}

/// Divide una imagen del escritorio completo en un `Vec<MonitorCapture>` recortando
/// la región de cada monitor. `monitors` usa coordenadas en píxeles físicos.
pub fn split_image_by_monitors(
    data: &[u8],
    monitors: &[(i32, i32, u32, u32)],
) -> Result<Vec<MonitorCapture>, CaptureError> {
    let img = image::load_from_memory(data)
        .map_err(|e| CaptureError::ImageProcessing(format!("Imagen inválida: {e}")))?;

    let img_w = img.width();
    let img_h = img.height();
    eprintln!("[split] imagen completa: {img_w}x{img_h}, {} monitores", monitors.len());

    let mut result = Vec::with_capacity(monitors.len());

    for &(x, y, w, h) in monitors {
        let cx = (x.max(0) as u32).min(img_w);
        let cy = (y.max(0) as u32).min(img_h);
        let cw = w.min(img_w.saturating_sub(cx));
        let ch = h.min(img_h.saturating_sub(cy));

        if cw == 0 || ch == 0 {
            eprintln!("[split] monitor ({x},{y}) {w}x{h} fuera de imagen — skip");
            continue;
        }

        let cropped = img.crop_imm(cx, cy, cw, ch);
        let rgb = cropped.to_rgb8();
        let mut buf = std::io::Cursor::new(Vec::new());
        JpegEncoder::new_with_quality(&mut buf, 95)
            .encode_image(&DynamicImage::ImageRgb8(rgb))
            .map_err(|e| CaptureError::ImageProcessing(format!("JPEG encode: {e}")))?;

        eprintln!("[split] monitor ({x},{y}) {cw}x{ch} ok");
        result.push(MonitorCapture { x, y, width: cw, height: ch, data: STANDARD.encode(buf.into_inner()) });
    }

    if result.is_empty() {
        return Err(CaptureError::Capture(
            "Ningún monitor quedó dentro de la imagen capturada".to_string(),
        ));
    }

    Ok(result)
}

/// Convierte bytes PNG/imagen en Vec<MonitorCapture> (un solo monitor, posición 0,0).
fn image_bytes_to_monitor_captures(data: &[u8]) -> Result<Vec<MonitorCapture>, CaptureError> {
    let img = image::load_from_memory(data)
        .map_err(|e| CaptureError::ImageProcessing(format!("Imagen inválida: {e}")))?;

    let width = img.width();
    let height = img.height();
    eprintln!("[wayland] captured: {width}x{height}");

    let rgb = img.to_rgb8();
    let mut buf = std::io::Cursor::new(Vec::new());
    JpegEncoder::new_with_quality(&mut buf, 95)
        .encode_image(&DynamicImage::ImageRgb8(rgb))
        .map_err(|e| CaptureError::ImageProcessing(format!("JPEG encode error: {e}")))?;

    Ok(vec![MonitorCapture {
        x: 0,
        y: 0,
        width,
        height,
        data: STANDARD.encode(buf.into_inner()),
    }])
}

/// Captura todos los monitores en paralelo (rayon) y devuelve cada uno como JPEG.
/// El frontend dibuja cada monitor en el canvas en su posición — sin compositing en Rust.
pub fn capture_monitors_x11(min_x: i32, min_y: i32) -> Result<Vec<MonitorCapture>, CaptureError> {
    let t0 = std::time::Instant::now();

    let screens = screenshots::Screen::all()
        .map_err(|e| CaptureError::Capture(format!("Error enumerando pantallas: {e}")))?;

    let results: Result<Vec<MonitorCapture>, CaptureError> = screens
        .par_iter()
        .map(|screen| {
            let info = screen.display_info;
            let t_cap = std::time::Instant::now();

            let shot = screen
                .capture()
                .map_err(|e| CaptureError::Capture(format!("Error capturando monitor: {e}")))?;
            eprintln!("[timing] monitor {}x{} captured: {}ms", info.width, info.height, t_cap.elapsed().as_millis());

            let raw = shot.into_raw();
            let rgba = ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(info.width, info.height, raw)
                .ok_or_else(|| CaptureError::ImageProcessing("Buffer RGBA inválido".into()))?;

            // RGBA → RGB → JPEG (sin compositing global)
            let rgb = DynamicImage::ImageRgba8(rgba).to_rgb8();
            let mut buf = Cursor::new(Vec::new());
            JpegEncoder::new_with_quality(&mut buf, 95)
                .encode_image(&DynamicImage::ImageRgb8(rgb))
                .map_err(|e| CaptureError::ImageProcessing(format!("JPEG encode error: {e}")))?;

            Ok(MonitorCapture {
                x: info.x - min_x,
                y: info.y - min_y,
                width: info.width,
                height: info.height,
                data: STANDARD.encode(buf.into_inner()),
            })
        })
        .collect();

    eprintln!("[timing] capture_monitors_x11 TOTAL: {}ms", t0.elapsed().as_millis());
    results
}

/// Genera una miniatura base64 a partir de bytes PNG crudos.
pub fn generate_thumbnail_b64(png_bytes: &[u8]) -> Result<String, CaptureError> {
    let img = image::load_from_memory(png_bytes)
        .map_err(|e| CaptureError::ImageProcessing(format!("Imagen inválida: {e}")))?;
    let thumb = img.thumbnail(240, 160);
    let mut buf = Cursor::new(Vec::new());
    thumb
        .write_to(&mut buf, ImageFormat::Png)
        .map_err(|e| CaptureError::ImageProcessing(format!("Thumbnail encode error: {e}")))?;
    Ok(STANDARD.encode(buf.into_inner()))
}

fn process_image(img: DynamicImage, width: u32, height: u32) -> Result<CaptureResult, CaptureError> {
    // Generar miniatura primero (antes de liberar img).
    let thumb = img.thumbnail(240, 160);

    // Encodear imagen completa a PNG y liberar los píxeles RGBA crudos antes de crear
    // el string base64. Sin el drop, img + PNG buffer + base64 string están vivos juntos.
    let mut full_buf = Cursor::new(Vec::new());
    img.write_to(&mut full_buf, ImageFormat::Png)
        .map_err(|e| CaptureError::ImageProcessing(format!("PNG encode error: {e}")))?;
    drop(img);
    let content = STANDARD.encode(full_buf.into_inner());

    // Mismo patrón para el thumbnail.
    let mut thumb_buf = Cursor::new(Vec::new());
    thumb
        .write_to(&mut thumb_buf, ImageFormat::Png)
        .map_err(|e| CaptureError::ImageProcessing(format!("Thumbnail encode error: {e}")))?;
    drop(thumb);
    let thumbnail = STANDARD.encode(thumb_buf.into_inner());

    Ok(CaptureResult { content, thumbnail, width, height })
}

// ─── PipeWire ScreenCast ──────────────────────────────────────────────────

/// Captura la pantalla vía XDG ScreenCast + PipeWire.
/// La primera vez muestra el diálogo de selección de GNOME; después usa el
/// restore_token para saltar ese paso y capturar silenciosamente.
/// Retorna (monitores, nuevo_token) — el token debe persistirse en la DB.
///
/// Si el restore_token era stale (algún stream no entregó frames, lo cual suele
/// ocurrir cuando los node IDs de PipeWire cambiaron tras un reinicio), se descarta
/// el token y se reintenta con diálogo fresco — así el usuario obtiene los 3 monitores.
pub async fn capture_via_screencast(
    restore_token: Option<String>,
) -> Result<(Vec<MonitorCapture>, Option<String>), CaptureError> {
    let had_token = restore_token.is_some();
    let (monitors, new_token, uncaptured) = capture_via_screencast_raw(restore_token).await?;

    // Token stale detectado: el portal entregó streams que no produjeron frames.
    // Esto ocurre cuando los PipeWire node IDs cambian (ej. reinicio) y el token
    // apunta a nodos que ya no existen o tienen configuración incorrecta.
    // Solución: reintentar sin token → portal muestra diálogo de selección fresco.
    if !uncaptured.is_empty() && had_token {
        eprintln!(
            "[screencast] {} stream(s) sin frame con restore token — token stale, reintentando sin token",
            uncaptured.len()
        );
        match capture_via_screencast_raw(None).await {
            Ok((monitors2, new_token2, uncaptured2)) => {
                if uncaptured2.is_empty() {
                    eprintln!("[screencast] reintento sin token exitoso, {} monitor(s)", monitors2.len());
                    return Ok((monitors2, new_token2));
                }
                eprintln!("[screencast] reintento sin token también tuvo {} uncaptured, usando gnome-crop", uncaptured2.len());
                return apply_gnome_crop_fallback(monitors2, uncaptured2).await.map(|m| (m, new_token2));
            }
            Err(e) => {
                eprintln!("[screencast] reintento sin token falló: {e}, usando resultado parcial con gnome-crop");
            }
        }
    }

    apply_gnome_crop_fallback(monitors, uncaptured).await.map(|m| (m, new_token))
}

/// Intento único de captura vía screencast. Retorna (monitores_capturados, token, uncaptured).
/// No aplica fallback — permite al caller decidir si reintentar o usar gnome-crop.
async fn capture_via_screencast_raw(
    restore_token: Option<String>,
) -> Result<(Vec<MonitorCapture>, Option<String>, Vec<(i32, i32, u32, u32)>), CaptureError> {
    use ashpd::desktop::screencast::{CursorMode, Screencast, SourceType};
    use ashpd::desktop::PersistMode;

    let proxy = Screencast::new()
        .await
        .map_err(|e| CaptureError::Capture(format!("ScreenCast portal: {e}")))?;

    let session = proxy
        .create_session()
        .await
        .map_err(|e| CaptureError::Capture(format!("create_session: {e}")))?;

    proxy
        .select_sources(
            &session,
            CursorMode::Hidden.into(),
            SourceType::Monitor.into(),
            true, // multiple monitors
            restore_token.as_deref(),
            PersistMode::ExplicitlyRevoked,
        )
        .await
        .map_err(|e| CaptureError::Capture(format!("select_sources: {e}")))?;

    let response = proxy
        .start(&session, None)
        .await
        .map_err(|e| CaptureError::Capture(format!("start: {e}")))?
        .response()
        .map_err(|e| CaptureError::Capture(format!("start response: {e}")))?;

    let new_token = response.restore_token().map(|t| t.to_string());
    let streams: Vec<_> = response.streams().to_vec();
    eprintln!("[screencast] {} stream(s), new_token={:?}", streams.len(), new_token);

    let fd = proxy
        .open_pipe_wire_remote(&session)
        .await
        .map_err(|e| CaptureError::Capture(format!("open_pipe_wire_remote: {e}")))?;

    // (node_id, x, y) — position() devuelve Option<(i32, i32)>, default (0,0)
    let stream_info: Vec<(u32, i32, i32)> = streams
        .iter()
        .map(|s| {
            let pos = s.position().unwrap_or((0, 0));
            eprintln!("[screencast] node={} pos=({},{})", s.pipe_wire_node_id(), pos.0, pos.1);
            (s.pipe_wire_node_id(), pos.0, pos.1)
        })
        .collect();

    let (monitors, uncaptured) = tokio::task::spawn_blocking(move || pipewire_capture_frames(fd, stream_info))
        .await
        .map_err(|_| CaptureError::Capture("PipeWire thread panicked".to_string()))?
        .map_err(|e| CaptureError::Capture(format!("PipeWire capture: {e}")))?;

    let _ = session.close().await;
    Ok((monitors, new_token, uncaptured))
}

/// Aplica gnome-crop como fallback para streams que PipeWire no capturó.
async fn apply_gnome_crop_fallback(
    mut monitors: Vec<MonitorCapture>,
    uncaptured: Vec<(i32, i32, u32, u32)>,
) -> Result<Vec<MonitorCapture>, CaptureError> {
    for (x, y, w, h) in uncaptured {
        eprintln!("[screencast] fallback gnome-crop para monitor en ({x},{y})");
        match capture_area_via_gnome_crop(x, y, w, h).await {
            Ok(monitor) => {
                eprintln!("[screencast] gnome-crop ok para ({x},{y})");
                monitors.push(monitor);
            }
            Err(e) => eprintln!("[screencast] gnome-crop falló para ({x},{y}): {e}"),
        }
    }
    Ok(monitors)
}

/// Fallback: captura el escritorio completo con gdbus Screenshot() y recorta la región
/// del monitor indicado. Usado cuando PipeWire no entrega frame para ese monitor.
///
/// `ScreenshotArea()` está prohibida para apps no-privilegiadas en GNOME moderno;
/// `Screenshot()` (pantalla completa) sí está permitida, y recortamos en memoria.
async fn capture_area_via_gnome_crop(x: i32, y: i32, width: u32, height: u32) -> Result<MonitorCapture, CaptureError> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let path = format!("/tmp/aurora-full-{ts}.png");

    let output = tokio::process::Command::new("gdbus")
        .args([
            "call", "--session",
            "--dest", "org.gnome.Shell.Screenshot",
            "--object-path", "/org/gnome/Shell/Screenshot",
            "--method", "org.gnome.Shell.Screenshot.Screenshot",
            "false", "false", &path,
        ])
        .output()
        .await
        .map_err(|e| CaptureError::Capture(format!("gdbus no encontrado: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CaptureError::Capture(format!("gdbus Screenshot falló: {stderr}")));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim_start().starts_with("(false") {
        return Err(CaptureError::Capture("Screenshot devolvió false".to_string()));
    }

    let data = std::fs::read(&path)
        .map_err(|e| CaptureError::Capture(format!("No se pudo leer la captura: {e}")))?;
    let _ = std::fs::remove_file(&path);

    let img = image::load_from_memory(&data)
        .map_err(|e| CaptureError::ImageProcessing(format!("Imagen inválida: {e}")))?;

    // Recortar la región del monitor faltante del screenshot completo.
    // crop_imm clampea automáticamente al borde de la imagen.
    let cropped = img.crop_imm(x as u32, y as u32, width, height);
    let actual_w = cropped.width();
    let actual_h = cropped.height();

    let rgb = cropped.to_rgb8();
    let mut buf = std::io::Cursor::new(Vec::new());
    JpegEncoder::new_with_quality(&mut buf, 95)
        .encode_image(&DynamicImage::ImageRgb8(rgb))
        .map_err(|e| CaptureError::ImageProcessing(format!("JPEG encode error: {e}")))?;

    Ok(MonitorCapture {
        x,
        y,
        width: actual_w,
        height: actual_h,
        data: STANDARD.encode(buf.into_inner()),
    })
}

/// Construye el pod SPA para negociación de formato video/raw.
fn make_video_format_pod() -> Vec<u8> {
    use pipewire::spa::{
        param::{
            format::{FormatProperties, MediaSubtype, MediaType},
            video::VideoFormat,
            ParamType,
        },
        pod::{serialize::PodSerializer, Object, Property, PropertyFlags, Value},
        pod::ChoiceValue,
        utils::{Choice, ChoiceEnum, ChoiceFlags, Fraction, Id, Rectangle, SpaTypes},
    };

    PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &Value::Object(Object {
            type_: SpaTypes::ObjectParamFormat.as_raw(),
            id: ParamType::EnumFormat.as_raw(),
            properties: vec![
                Property {
                    key: FormatProperties::MediaType.as_raw(),
                    flags: PropertyFlags::empty(),
                    value: Value::Id(Id(MediaType::Video.as_raw())),
                },
                Property {
                    key: FormatProperties::MediaSubtype.as_raw(),
                    flags: PropertyFlags::empty(),
                    value: Value::Id(Id(MediaSubtype::Raw.as_raw())),
                },
                Property {
                    key: FormatProperties::VideoFormat.as_raw(),
                    flags: PropertyFlags::empty(),
                    value: Value::Choice(ChoiceValue::Id(Choice(
                        ChoiceFlags::empty(),
                        ChoiceEnum::Enum {
                            default: Id(VideoFormat::BGRA.as_raw()),
                            alternatives: vec![
                                Id(VideoFormat::BGRx.as_raw()),
                                Id(VideoFormat::RGBA.as_raw()),
                                Id(VideoFormat::RGBx.as_raw()),
                            ],
                        },
                    ))),
                },
                Property {
                    key: FormatProperties::VideoSize.as_raw(),
                    flags: PropertyFlags::empty(),
                    value: Value::Choice(ChoiceValue::Rectangle(Choice(
                        ChoiceFlags::empty(),
                        ChoiceEnum::Range {
                            default: Rectangle { width: 1920, height: 1080 },
                            min: Rectangle { width: 1, height: 1 },
                            max: Rectangle { width: 16384, height: 16384 },
                        },
                    ))),
                },
                Property {
                    key: FormatProperties::VideoFramerate.as_raw(),
                    flags: PropertyFlags::empty(),
                    value: Value::Choice(ChoiceValue::Fraction(Choice(
                        ChoiceFlags::empty(),
                        ChoiceEnum::Range {
                            default: Fraction { num: 25, denom: 1 },
                            min: Fraction { num: 0, denom: 1 },
                            max: Fraction { num: 1000, denom: 1 },
                        },
                    ))),
                },
            ],
        }),
    )
    .expect("Failed to serialize video format pod")
    .0
    .into_inner()
}

#[derive(Default)]
struct StreamCaptureState {
    format: Option<pipewire::spa::param::video::VideoFormat>,
    width: u32,
    height: u32,
}

/// Captura un frame por cada stream usando el PipeWire remote fd del portal.
/// `streams` es Vec<(node_id, x, y)> con las coordenadas de cada monitor.
/// Retorna (monitores_capturados, no_capturados:(x,y,w,h)).
/// Los "no capturados" son streams que negociaron formato pero no entregaron frame;
/// el caller puede usar un fallback (ej. gdbus ScreenshotArea) para completarlos.
/// Función síncrona — debe ejecutarse en spawn_blocking.
fn pipewire_capture_frames(
    fd: std::os::fd::OwnedFd,
    streams: Vec<(u32, i32, i32)>,
) -> Result<(Vec<MonitorCapture>, Vec<(i32, i32, u32, u32)>), String> {
    use pipewire::{
        context::Context,
        main_loop::MainLoop,
        spa::{
            param::{
                format::{MediaSubtype, MediaType},
                format_utils::parse_format,
                video::VideoInfoRaw,
                ParamType,
            },
            pod::Pod,
            utils::Direction,
        },
        stream::StreamFlags,
    };

    pipewire::init();

    let mainloop = MainLoop::new(None).map_err(|e| e.to_string())?;
    let context = Context::new(&mainloop).map_err(|e| e.to_string())?;
    let core = context.connect_fd(fd, None).map_err(|e| e.to_string())?;

    // Guardar frames crudos en el callback (NO hacer encoding ahí — bloquea el event loop).
    // El encoding se hace después de mainloop.run().
    //
    // Usamos HashMap<node_id, RawFrame> en lugar de Vec + contador:
    // garantiza exactamente UN frame por node, aunque el mismo stream dispare
    // múltiples callbacks antes de que los demás streams envíen su primer frame.
    struct RawFrame {
        data: Vec<u8>,
        width: u32,
        height: u32,
        format: pipewire::spa::param::video::VideoFormat,
        x: i32,
        y: i32,
    }

    let needed = streams.len();
    let raw_frames: Arc<Mutex<std::collections::HashMap<u32, RawFrame>>> =
        Arc::new(Mutex::new(std::collections::HashMap::new()));
    // Registra el tamaño negociado por formato para cada stream.
    // Si el stream negocia formato pero no entrega frame (timeout), lo usamos
    // para el fallback gdbus ScreenshotArea.
    let stream_sizes: Arc<Mutex<std::collections::HashMap<u32, (u32, u32)>>> =
        Arc::new(Mutex::new(std::collections::HashMap::new()));

    let format_pod_bytes = make_video_format_pod();

    // Guardamos streams y listeners para que no sean dropeados antes de mainloop.run().
    let mut _alive: Vec<Box<dyn std::any::Any>> = Vec::new();
    // Punteros a los streams para el timer de retry. Son válidos mientras _alive viva.
    let mut stream_ptrs: Vec<(usize, u32)> = Vec::new(); // (ptr as usize, node_id)
    // Nodos que ya entraron en Streaming — el retry solo actúa sobre estos.
    let streaming_nodes: Arc<Mutex<std::collections::HashSet<u32>>> =
        Arc::new(Mutex::new(std::collections::HashSet::new()));

    for (node_id, stream_x, stream_y) in streams.iter().copied() {
        let state: Arc<Mutex<StreamCaptureState>> =
            Arc::new(Mutex::new(StreamCaptureState::default()));
        let state_param = Arc::clone(&state);
        let state_proc = Arc::clone(&state);
        let frames_clone = Arc::clone(&raw_frames);
        let sizes_clone = Arc::clone(&stream_sizes);
        let mainloop_clone = mainloop.clone();
        let streaming_nodes_clone = Arc::clone(&streaming_nodes);

        let mut props = pipewire::properties::Properties::new();
        props.insert("media.type", "Video");
        props.insert("media.category", "Capture");
        props.insert("media.role", "Screen");

        let stream = pipewire::stream::Stream::new(&core, "aurora-screenshot", props)
            .map_err(|e| e.to_string())?;

        let listener = stream
            .add_local_listener_with_user_data(())
            .state_changed(move |stream, _data, old, new| {
                use pipewire::stream::StreamState;
                eprintln!("[pw] node={node_id} state: {old:?} → {new:?}");
                // trigger_process() solicita el primer frame. Debe llamarse cuando el
                // stream ya está en STREAMING — si se llama antes (ej. en param_changed
                // justo tras set_active(true)), la transición es asíncrona y el request
                // llega antes de que el stream esté listo, resultando en frames perdidos.
                if new == StreamState::Streaming {
                    streaming_nodes_clone.lock().unwrap().insert(node_id);
                    eprintln!("[pw] node={node_id} STREAMING → trigger_process()");
                    let _ = stream.trigger_process();
                }
            })
            .param_changed(move |stream, _data, id, pod| {
                if id != ParamType::Format.as_raw() {
                    return;
                }
                let Some(pod) = pod else { return };
                if let Ok((mt, mst)) = parse_format(pod) {
                    if mt == MediaType::Video && mst == MediaSubtype::Raw {
                        let mut info = VideoInfoRaw::default();
                        if info.parse(pod).is_ok() {
                            let mut s = state_param.lock().unwrap();
                            s.format = Some(info.format());
                            s.width = info.size().width;
                            s.height = info.size().height;
                            eprintln!(
                                "[pw] node={node_id} format: {:?} {}x{}",
                                info.format(),
                                info.size().width,
                                info.size().height
                            );
                            sizes_clone.lock().unwrap().insert(node_id, (info.size().width, info.size().height));
                        }
                        // set_active(true) inicia la transición PAUSED → STREAMING.
                        // El trigger_process() real va en state_changed cuando llega a STREAMING.
                        let _ = stream.set_active(true);
                    }
                }
            })
            .process(move |stream, _| {
                eprintln!("[pw] process node={node_id}");

                // Si ya tenemos frame para este node, desencolar y descartar el buffer
                // para que PipeWire no siga re-disparando el callback con el mismo buffer
                // no drenado — que era la causa del spam de 50+ callbacks.
                // NO llamamos set_active(false): modificar el grafo PipeWire mientras otros
                // streams están esperando su primer frame cancela sus triggers pendientes.
                {
                    let frames = frames_clone.lock().unwrap();
                    if frames.contains_key(&node_id) {
                        let _ = stream.dequeue_buffer(); // descartar — drop automático
                        return;
                    }
                }

                let (fmt, width, height) = {
                    let s = state_proc.lock().unwrap();
                    let Some(fmt) = s.format else { return };
                    if s.width == 0 || s.height == 0 { return; }
                    (fmt, s.width, s.height)
                };

                let Some(mut buffer) = stream.dequeue_buffer() else { return };
                let datas = buffer.datas_mut();
                let Some(d) = datas.first_mut() else { return };
                let offset = d.chunk().offset() as usize;
                let size = d.chunk().size() as usize;
                let Some(slice) = d.data() else { return };
                if size == 0 || slice.len() < offset + size { return; }

                // Copiar los bytes ANTES de adquirir el lock: el copy de ~8MB
                // bloquea el event loop de PipeWire si se hace dentro del lock,
                // retrasando los callbacks de los otros streams.
                let frame_bytes = slice[offset..offset + size].to_vec();

                let mut frames = frames_clone.lock().unwrap();
                frames.insert(node_id, RawFrame {
                    data: frame_bytes,
                    width,
                    height,
                    format: fmt,
                    x: stream_x,
                    y: stream_y,
                });
                let count = frames.len();
                eprintln!("[pw] node={node_id} frame captured ({count}/{needed})");
                if count >= needed {
                    mainloop_clone.quit();
                }
            })
            .register()
            .map_err(|e| e.to_string())?;

        let pod = unsafe { Pod::from_bytes(&format_pod_bytes) }
            .ok_or("Invalid format pod")?;

        stream
            .connect(
                Direction::Input,
                Some(node_id),
                StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS,
                &mut [pod],
            )
            .map_err(|e| e.to_string())?;

        // Capturar el puntero al stream ANTES de moverlo al Box.
        // El heap address del Box no cambia al moverse al Vec — solo se mueve el fat ptr.
        // SAFETY: el Box vive en _alive durante todo mainloop.run().
        let boxed = Box::new((stream, listener));
        let stream_ptr = &boxed.0 as *const pipewire::stream::Stream as usize;
        stream_ptrs.push((stream_ptr, node_id));
        _alive.push(boxed);
    }

    // Timer de retry: cada 200ms reintenta trigger_process() SOLO para nodos que:
    //   1. Ya entraron en Streaming (set_active(true) completó la transición)
    //   2. Aún no entregaron un frame
    // Algunos nodos de PipeWire ignoran el primer trigger y responden al segundo.
    // IMPORTANTE: NO llamar trigger_process en nodos Paused — puede enviarlos a Error.
    let frames_for_retry = Arc::clone(&raw_frames);
    let streaming_for_retry = Arc::clone(&streaming_nodes);
    let ptrs_for_retry = stream_ptrs.clone();
    let retry_timer = mainloop.loop_().add_timer(move |_| {
        let frames = frames_for_retry.lock().unwrap();
        let streaming = streaming_for_retry.lock().unwrap();
        let to_retry: Vec<(usize, u32)> = ptrs_for_retry.iter()
            .filter(|&&(_, nid)| streaming.contains(&nid) && !frames.contains_key(&nid))
            .copied()
            .collect();
        drop(frames);
        drop(streaming);
        for (ptr, nid) in to_retry {
            // SAFETY: el stream vive en _alive durante todo mainloop.run()
            let stream = unsafe { &*(ptr as *const pipewire::stream::Stream) };
            eprintln!("[pw] retry trigger_process() for node={nid}");
            let _ = stream.trigger_process();
        }
    });
    let _ = retry_timer.update_timer(
        Some(std::time::Duration::from_millis(200)),
        Some(std::time::Duration::from_millis(200)),
    );

    // Timeout total: si después de 3s algún stream sigue sin entregar, salir igualmente.
    let ml_timeout = mainloop.clone();
    let timer = mainloop.loop_().add_timer(move |_| {
        eprintln!("[pw] capture timeout — quitting");
        ml_timeout.quit();
    });
    let _ = timer.update_timer(Some(std::time::Duration::from_secs(3)), None);

    mainloop.run();

    // El event loop terminó. Ahora hacemos el encoding JPEG fuera del callback.
    let frames_map = raw_frames.lock().unwrap();
    if frames_map.is_empty() {
        return Err("No se recibieron frames de PipeWire (timeout o error)".to_string());
    }

    // Ordenar por node_id para output determinista
    let mut frames: Vec<(u32, &RawFrame)> = frames_map.iter().map(|(k, v)| (*k, v)).collect();
    frames.sort_by_key(|(id, _)| *id);

    let mut monitors = Vec::with_capacity(frames.len());
    for (node_id, frame) in &frames {
        eprintln!("[pw] encoding node={node_id} ({}x{}) at ({},{})", frame.width, frame.height, frame.x, frame.y);
        match raw_pixels_to_monitor_capture(frame.data.clone(), frame.width, frame.height, frame.format, frame.x, frame.y) {
            Ok(monitor) => monitors.push(monitor),
            Err(e) => eprintln!("[pw] encode error for node {node_id}: {e}"),
        }
    }

    if monitors.is_empty() {
        return Err("Fallo al encodear frames de PipeWire".to_string());
    }

    eprintln!("[pw] done: {} monitor(s) encoded", monitors.len());

    // Construir lista de streams que negociaron formato pero no entregaron frame.
    let sizes_map = stream_sizes.lock().unwrap();
    let uncaptured: Vec<(i32, i32, u32, u32)> = streams
        .iter()
        .filter(|(node_id, _, _)| !frames_map.contains_key(node_id))
        .filter_map(|(node_id, x, y)| {
            sizes_map.get(node_id).map(|&(w, h)| (*x, *y, w, h))
        })
        .collect();
    for (x, y, w, h) in &uncaptured {
        eprintln!("[pw] uncaptured stream at ({x},{y}) {w}x{h} — needs fallback");
    }

    Ok((monitors, uncaptured))
}

/// Convierte píxeles crudos del frame PipeWire a MonitorCapture (JPEG base64).
fn raw_pixels_to_monitor_capture(
    data: Vec<u8>,
    width: u32,
    height: u32,
    format: pipewire::spa::param::video::VideoFormat,
    x: i32,
    y: i32,
) -> Result<MonitorCapture, String> {
    use image::{ImageBuffer, Rgb, Rgba};
    use pipewire::spa::param::video::VideoFormat;

    let img: DynamicImage = match format {
        VideoFormat::RGBA => ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(width, height, data)
            .map(DynamicImage::ImageRgba8)
            .ok_or("Buffer RGBA inválido")?,

        VideoFormat::RGBx => {
            let rgba: Vec<u8> = data.chunks_exact(4).flat_map(|c| [c[0], c[1], c[2], 255]).collect();
            ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(width, height, rgba)
                .map(DynamicImage::ImageRgba8)
                .ok_or("Buffer RGBx inválido")?
        }

        VideoFormat::BGRA => {
            let rgba: Vec<u8> = data.chunks_exact(4).flat_map(|c| [c[2], c[1], c[0], c[3]]).collect();
            ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(width, height, rgba)
                .map(DynamicImage::ImageRgba8)
                .ok_or("Buffer BGRA inválido")?
        }

        VideoFormat::BGRx => {
            let rgba: Vec<u8> = data.chunks_exact(4).flat_map(|c| [c[2], c[1], c[0], 255]).collect();
            ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(width, height, rgba)
                .map(DynamicImage::ImageRgba8)
                .ok_or("Buffer BGRx inválido")?
        }

        VideoFormat::RGB => ImageBuffer::<Rgb<u8>, Vec<u8>>::from_raw(width, height, data)
            .map(DynamicImage::ImageRgb8)
            .ok_or("Buffer RGB inválido")?,

        other => return Err(format!("Formato de video no soportado: {other:?}")),
    };

    let rgb = img.to_rgb8();
    let mut buf = Cursor::new(Vec::new());
    JpegEncoder::new_with_quality(&mut buf, 95)
        .encode_image(&DynamicImage::ImageRgb8(rgb))
        .map_err(|e| e.to_string())?;

    Ok(MonitorCapture {
        x,
        y,
        width,
        height,
        data: STANDARD.encode(buf.into_inner()),
    })
}
