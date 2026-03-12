use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::borrow::Cow;

pub fn copy_png_b64_to_clipboard(content_b64: &str) -> Result<(), String> {
    let png_bytes = STANDARD.decode(content_b64).map_err(|e| e.to_string())?;
    copy_png_bytes_to_clipboard(&png_bytes)
}

pub fn copy_png_bytes_to_clipboard(png_bytes: &[u8]) -> Result<(), String> {
    let img = image::load_from_memory(png_bytes).map_err(|e| e.to_string())?;
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    let bytes = rgba.into_raw();

    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    clipboard
        .set_image(arboard::ImageData {
            width: width as usize,
            height: height as usize,
            bytes: Cow::Owned(bytes),
        })
        .map_err(|e| e.to_string())
}

pub fn copy_text_to_clipboard(text: &str) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    clipboard.set_text(text).map_err(|e| e.to_string())
}
