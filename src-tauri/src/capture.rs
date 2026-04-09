use base64::{engine::general_purpose::STANDARD, Engine as _};
use image::{
    codecs::jpeg::JpegEncoder,
    codecs::png::PngEncoder,
    DynamicImage, ImageBuffer, ImageEncoder, ImageFormat, Rgba, RgbaImage,
};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::io::Cursor;
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
    // Intento 1: portal XDG con interactive(false)
    match capture_via_portal_noninteractive().await {
        Ok(monitors) => return Ok(monitors),
        Err(e) => eprintln!("[wayland] portal non-interactive failed ({e}), trying GNOME Shell fallback..."),
    }

    // Intento 2: GNOME Shell D-Bus via gdbus (no requiere permisos extra)
    match capture_via_gnome_shell().await {
        Ok(monitors) => return Ok(monitors),
        Err(e) => eprintln!("[wayland] GNOME Shell fallback failed: {e}"),
    }

    Err(CaptureError::Capture(
        "No se pudo capturar la pantalla. Intentá otorgar permisos de captura en Configuración → Privacidad → Pantalla.".to_string(),
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
