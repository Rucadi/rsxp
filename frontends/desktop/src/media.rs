// SPDX-License-Identifier: GPL-2.0-or-later
//! Pure-Rust image decoding and font rasterization for the desktop frontend.
//!
//! [`image`] decodes arbitrary encoded bytes (PNG/JPEG/etc. via the `image`
//! crate) into an RGBA [`Bitmap`] for the RGSS surface.
//!
//! [`text`] rasterizes a string with `fontdue`: it measures the box from line
//! ascent plus descenders and per-glyph advance widths (matching desktop RGSS
//! sizing), then, when `render` is true, composites glyph coverage into a
//! [`Bitmap`] using the requested [`Color`]'s RGB and alpha-scaled coverage.
//! With `render == false` it only returns the measured `(width, height, None)`,
//! which the engine uses for layout. Font size is validated to `1..=512`; the
//! `_bold`/`_italic` parameters are accepted but currently ignored. A missing
//! font falls back to the bundled `assets/liberation.ttf`.
//!
//! Decoded fonts are cached in a thread-local list keyed by exact font bytes,
//! capped at 16 entries (oldest evicted), so the host must call these
//! functions on the thread that owns the cache.
use rsxp::graphics::{Bitmap, Color};
use std::cell::RefCell;
use std::rc::Rc;

thread_local! {
    static FONTS: RefCell<Vec<(Vec<u8>, Rc<fontdue::Font>)>> = const { RefCell::new(Vec::new()) };
}

pub fn image(bytes: &[u8]) -> Result<Bitmap, String> {
    let image = image::load_from_memory(bytes).map_err(|error| error.to_string())?;
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    Bitmap::from_rgba(width, height, rgba.as_raw()).map_err(|error| error.to_string())
}

fn font(bytes: &[u8]) -> Result<Rc<fontdue::Font>, String> {
    FONTS.with(|fonts| {
        let mut fonts = fonts.borrow_mut();
        if let Some((_, font)) = fonts.iter().find(|(cached, _)| cached.as_slice() == bytes) {
            return Ok(Rc::clone(font));
        }
        let font = Rc::new(
            fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default())
                .map_err(|error| error.to_string())?,
        );
        if fonts.len() == 16 {
            fonts.remove(0);
        }
        fonts.push((bytes.to_vec(), Rc::clone(&font)));
        Ok(font)
    })
}

pub fn text(
    font_bytes: Option<&[u8]>,
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
    let bytes = font_bytes.unwrap_or(include_bytes!(
        "../../../crates/rsxp-rgss/assets/liberation.ttf"
    ));
    let font = font(bytes)?;
    let scale = size as f32;
    // Anchor the text box at the line ascent and include descenders, matching
    // the desktop RGSS sizing behavior.
    let ascent = font
        .horizontal_line_metrics(scale)
        .map(|metrics| metrics.ascent)
        .unwrap_or(scale * 0.8);
    let mut advance = 0.0f32;
    let mut bottom = ascent;
    for character in text.chars() {
        let (metrics, _) = font.rasterize(character, scale);
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

    let mut bitmap = Bitmap::new(width.max(1), height.max(1)).map_err(|error| error.to_string())?;
    let mut pen = 0.0f32;
    for character in text.chars() {
        let (metrics, coverage) = font.rasterize(character, scale);
        let gx = pen + metrics.xmin as f32;
        let gy = ascent - (metrics.height as f32 + metrics.ymin as f32);
        for row in 0..metrics.height {
            for column in 0..metrics.width {
                let coverage = coverage[row * metrics.width + column];
                if coverage == 0 {
                    continue;
                }
                let alpha = (u32::from(coverage) * u32::from(color.alpha) / 255) as u8;
                bitmap.set_pixel(
                    gx as i32 + column as i32,
                    gy as i32 + row as i32,
                    Color::rgba(color.red, color.green, color.blue, alpha),
                );
            }
        }
        pen += metrics.advance_width;
    }
    Ok((width, height, Some(bitmap)))
}
