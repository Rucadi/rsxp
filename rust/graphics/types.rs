// SPDX-License-Identifier: GPL-2.0-or-later
//! Shared geometry, color, tone and sprite parameter values.
use super::Bitmap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Color {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}
impl Color {
    pub const fn rgba(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }
    pub const fn rgb(red: u8, green: u8, blue: u8) -> Self {
        Self::rgba(red, green, blue, 255)
    }
    pub(super) fn packed(self) -> u32 {
        u32::from_be_bytes([self.alpha, self.red, self.green, self.blue])
    }
    pub(super) fn unpack(pixel: u32) -> Self {
        let [alpha, red, green, blue] = pixel.to_be_bytes();
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
impl Rect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tone {
    pub red: i32,
    pub green: i32,
    pub blue: i32,
    pub gray: i32,
}

/// Per-sprite effect parameters matching the engine's sprite shader uniforms.
pub struct SpriteFx<'a> {
    pub opacity: u8,
    pub blend_type: i32,
    pub color: Color,
    pub tone: Tone,
    pub invert: bool,
    /// Source row where bush transparency begins, or None for ordinary sprites.
    pub bush_y: Option<i64>,
    pub bush_opacity: u8,
    pub mirror: bool,
    pub pattern: Option<&'a Bitmap>,
    pub pattern_blend: i32,
    pub pattern_tile: bool,
    pub pattern_opacity: u8,
    pub pattern_scroll_x: f64,
    pub pattern_scroll_y: f64,
    pub pattern_zoom_x: f64,
    pub pattern_zoom_y: f64,
}
impl Default for SpriteFx<'_> {
    fn default() -> Self {
        Self {
            opacity: 255,
            blend_type: 0,
            color: Color::default(),
            tone: Tone::default(),
            invert: false,
            bush_y: None,
            bush_opacity: 128,
            mirror: false,
            pattern: None,
            pattern_blend: 0,
            pattern_tile: true,
            pattern_opacity: 255,
            pattern_scroll_x: 0.0,
            pattern_scroll_y: 0.0,
            pattern_zoom_x: 1.0,
            pattern_zoom_y: 1.0,
        }
    }
}

/// Position/scale/rotation of a sprite, mirroring the engine's transform matrix.
#[derive(Clone, Copy, Debug)]
pub struct SpriteTransform {
    pub x: f64,
    pub y: f64,
    pub ox: f64,
    pub oy: f64,
    pub zoom_x: f64,
    pub zoom_y: f64,
    pub angle: f64,
    pub mirror: bool,
}
impl Default for SpriteTransform {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            ox: 0.0,
            oy: 0.0,
            zoom_x: 1.0,
            zoom_y: 1.0,
            angle: 0.0,
            mirror: false,
        }
    }
}
