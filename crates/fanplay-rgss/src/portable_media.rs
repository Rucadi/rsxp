// SPDX-License-Identifier: GPL-2.0-or-later
//! Shared image and font services for mobile hosts.
use crate::graphics::{Bitmap, Color};
pub fn image(bytes: &[u8]) -> Result<Bitmap, String> {
    let image = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    Bitmap::from_rgba(width, height, rgba.as_raw()).map_err(|e| e.to_string())
}
#[allow(clippy::too_many_arguments)]
pub fn text(
    font: Option<&[u8]>,
    size: i32,
    _bold: bool,
    _italic: bool,
    text: &str,
    color: Color,
    render: bool,
) -> Result<(u32, u32, Option<Bitmap>), String> {
    if !(1..=512).contains(&size) {
        return Err("font size must be 1..512".into());
    }
    let bytes = font.unwrap_or(include_bytes!("../assets/liberation.ttf"));
    let font = fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default())
        .map_err(|e| e.to_string())?;
    let scale = size as f32;
    // Anchor the text box at the top of the line and include descenders.
    // Games size their windows using these measurements.
    let ascent = font
        .horizontal_line_metrics(scale)
        .map(|m| m.ascent)
        .unwrap_or(scale * 0.8);
    let mut advance = 0.0f32;
    let mut bottom = ascent;
    for ch in text.chars() {
        let (metrics, _) = font.rasterize(ch, scale);
        advance += metrics.advance_width;
        if metrics.width > 0 && metrics.height > 0 {
            bottom = bottom.max(ascent - metrics.ymin as f32);
        }
    }
    let width = advance.ceil().max(0.0) as u32;
    let height = bottom.ceil().max(0.0) as u32;
    if !render {
        return Ok((width, height, None));
    }
    let mut bitmap = Bitmap::new(width.max(1), height.max(1)).map_err(|e| e.to_string())?;
    let mut pen = 0.0f32;
    for ch in text.chars() {
        let (metrics, coverage) = font.rasterize(ch, scale);
        let gx = pen + metrics.xmin as f32;
        let gy = ascent - (metrics.height as f32 + metrics.ymin as f32);
        for row in 0..metrics.height {
            for col in 0..metrics.width {
                let a = coverage[row * metrics.width + col];
                if a == 0 {
                    continue;
                }
                let alpha = (u32::from(a) * u32::from(color.alpha) / 255) as u8;
                bitmap.set_pixel(
                    gx as i32 + col as i32,
                    gy as i32 + row as i32,
                    Color::rgba(color.red, color.green, color.blue, alpha),
                );
            }
        }
        pen += metrics.advance_width;
    }
    Ok((width, height, Some(bitmap)))
}
