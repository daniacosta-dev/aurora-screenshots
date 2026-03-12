use base64::{engine::general_purpose::STANDARD, Engine as _};
use image::{
    codecs::png::{CompressionType, FilterType, PngEncoder},
    imageops, DynamicImage, ImageBuffer, ImageEncoder, ImageFormat, Rgba, RgbaImage,
};
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
    let path = std::path::PathBuf::from(uri.path());

    let data = std::fs::read(&path)
        .map_err(|e| CaptureError::Capture(format!("No se pudo leer la captura: {e}")))?;

    let _ = std::fs::remove_file(&path);

    let img = image::load_from_memory(&data)
        .map_err(|e| CaptureError::ImageProcessing(format!("Imagen inválida: {e}")))?;

    let width = img.width();
    let height = img.height();
    process_image(img, width, height)
}

/// Captura el escritorio completo (todos los monitores) en X11 y retorna base64 PNG.
/// Se usa como fondo "congelado" para el overlay de captura.
pub fn capture_full_desktop_x11(
    min_x: i32,
    min_y: i32,
    total_w: u32,
    total_h: u32,
) -> Result<String, CaptureError> {
    let screens = screenshots::Screen::all()
        .map_err(|e| CaptureError::Capture(format!("Error enumerando pantallas: {e}")))?;

    let mut canvas: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::new(total_w, total_h);

    for screen in screens {
        let info = screen.display_info;
        let shot = screen
            .capture()
            .map_err(|e| CaptureError::Capture(format!("Error capturando monitor: {e}")))?;
        let raw = shot.into_raw();
        let monitor_img =
            ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(info.width, info.height, raw)
                .ok_or_else(|| CaptureError::ImageProcessing("Buffer RGBA inválido".into()))?;

        let dest_x = (info.x - min_x) as i64;
        let dest_y = (info.y - min_y) as i64;
        imageops::replace(&mut canvas, &monitor_img, dest_x, dest_y);
    }

    // Comprimir con nivel Fast (deflate 1) en lugar del default (deflate 6).
    // Para el fondo del overlay no necesitamos máxima compresión, sí máxima velocidad.
    let mut buf = Cursor::new(Vec::new());
    PngEncoder::new_with_quality(&mut buf, CompressionType::Fast, FilterType::Sub)
        .write_image(
            canvas.as_raw(),
            total_w,
            total_h,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| CaptureError::ImageProcessing(format!("PNG fast encode error: {e}")))?;

    Ok(STANDARD.encode(buf.into_inner()))
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
    let mut full_buf = Cursor::new(Vec::new());
    img.write_to(&mut full_buf, ImageFormat::Png)
        .map_err(|e| CaptureError::ImageProcessing(format!("PNG encode error: {e}")))?;
    let content = STANDARD.encode(full_buf.into_inner());

    let thumb = img.thumbnail(240, 160);
    let mut thumb_buf = Cursor::new(Vec::new());
    thumb
        .write_to(&mut thumb_buf, ImageFormat::Png)
        .map_err(|e| CaptureError::ImageProcessing(format!("Thumbnail encode error: {e}")))?;
    let thumbnail = STANDARD.encode(thumb_buf.into_inner());

    Ok(CaptureResult { content, thumbnail, width, height })
}
