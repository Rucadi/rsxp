// SPDX-License-Identifier: GPL-2.0-or-later
//! Pixel math: hue/brightness, viewport tint and the sprite shader equivalent.
use super::{Bitmap, Color, SpriteFx, Tone};

impl Bitmap {
    pub fn brightness(&mut self, value: u8) {
        self.touch();
        for pixel in &mut self.pixels {
            let c = Color::unpack(*pixel);
            let dim = |v: u8| (u32::from(v) * u32::from(value) / 255) as u8;
            *pixel = Color::rgba(dim(c.red), dim(c.green), dim(c.blue), c.alpha).packed();
        }
    }
    /// In-place HSL hue rotation, implemented by the hue shader.
    pub fn hue_change(&mut self, hue: f32) {
        self.touch();
        let adjust = hue.rem_euclid(360.0) / 360.0;
        if adjust == 0.0 {
            return;
        }
        for pixel in &mut self.pixels {
            let color = Color::unpack(*pixel);
            let (h, s, v) = rgb_to_hsv(color.red, color.green, color.blue);
            let (red, green, blue) = hsv_to_rgb(h + adjust, s, v);
            *pixel = Color::rgba(red, green, blue, color.alpha).packed();
        }
    }
    /// Applies a viewport's tone and color to every pixel, matching the engine's
    /// `plane.frag` (gray, then tone rgb, then color mix). Alpha is preserved.
    pub fn apply_color_tone(&mut self, color: Color, tone: Tone) {
        self.touch();
        let gray = tone.gray.clamp(0, 255) as f32 / 255.0;
        let (tr, tg, tb) = (
            tone.red.clamp(-255, 255) as f32 / 255.0,
            tone.green.clamp(-255, 255) as f32 / 255.0,
            tone.blue.clamp(-255, 255) as f32 / 255.0,
        );
        let ca = f32::from(color.alpha) / 255.0;
        if gray == 0.0 && tr == 0.0 && tg == 0.0 && tb == 0.0 && ca == 0.0 {
            return;
        }
        let (cr, cg, cb) = (
            f32::from(color.red) / 255.0,
            f32::from(color.green) / 255.0,
            f32::from(color.blue) / 255.0,
        );
        for pixel in &mut self.pixels {
            let c = Color::unpack(*pixel);
            let mut r = f32::from(c.red) / 255.0;
            let mut g = f32::from(c.green) / 255.0;
            let mut b = f32::from(c.blue) / 255.0;
            if gray > 0.0 {
                let luma = 0.299 * r + 0.587 * g + 0.114 * b;
                r += (luma - r) * gray;
                g += (luma - g) * gray;
                b += (luma - b) * gray;
            }
            r += tr;
            g += tg;
            b += tb;
            r += (cr - r) * ca;
            g += (cg - g) * ca;
            b += (cb - b) * ca;
            let to_u8 = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
            *pixel = Color::rgba(to_u8(r), to_u8(g), to_u8(b), c.alpha).packed();
        }
    }
}

/// GLSL `step(edge, x)`: 0.0 when `x < edge`, otherwise 1.0.
fn step(edge: f32, x: f32) -> f32 {
    if x < edge {
        0.0
    } else {
        1.0
    }
}
/// GLSL `fract(x)`.
fn fract(x: f32) -> f32 {
    x - x.floor()
}
/// RGB to HSV, transliterated from the branchless conversion in `hue.frag`.
fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let p = [
        b + step(b, g) * (g - b),
        g + step(b, g) * (b - g),
        -1.0 + step(b, g) * (0.0 - -1.0),
        2.0 / 3.0 + step(b, g) * (-1.0 / 3.0 - 2.0 / 3.0),
    ];
    let t = step(p[0], r);
    let (q0, q1, q2, q3) = (
        p[0] + t * (r - p[0]),
        p[1],
        p[3] + t * (p[2] - p[3]),
        r + t * (p[0] - r),
    );
    let d = q0 - q1.min(q3);
    let eps = 1.0e-10;
    ((q2 + (q3 - q1) / (6.0 * d + eps)).abs(), d / (q0 + eps), q0)
}
/// HSV to RGB, transliterated from `hsv2rgb` in `hue.frag`.
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let k = [1.0, 2.0 / 3.0, 1.0 / 3.0];
    let channel = |ki: f32| {
        let p = (fract(h + ki) * 6.0 - 3.0).abs();
        let clamped = (p - 1.0).clamp(0.0, 1.0);
        let mixed = 1.0 + s * (clamped - 1.0);
        (mixed * v * 255.0).round().clamp(0.0, 255.0) as u8
    };
    (channel(k[0]), channel(k[1]), channel(k[2]))
}

/// Applies the sprite shader (pattern, tone, opacity, color, invert) to
/// one sampled source pixel.
pub(super) fn shade_pixel(tex: &Bitmap, source: Color, sx: i64, sy: i64, fx: &SpriteFx) -> Color {
    let color_alpha = f32::from(fx.color.alpha) / 255.0;
    let (cr, cg, cb) = (
        f32::from(fx.color.red) / 255.0,
        f32::from(fx.color.green) / 255.0,
        f32::from(fx.color.blue) / 255.0,
    );
    let gray = fx.tone.gray.clamp(0, 255) as f32 / 255.0;
    let (tr, tg, tb) = (
        fx.tone.red.clamp(-255, 255) as f32 / 255.0,
        fx.tone.green.clamp(-255, 255) as f32 / 255.0,
        fx.tone.blue.clamp(-255, 255) as f32 / 255.0,
    );
    let opacity = f32::from(fx.opacity) / 255.0;
    let mut r = f32::from(source.red) / 255.0;
    let mut g = f32::from(source.green) / 255.0;
    let mut b = f32::from(source.blue) / 255.0;
    if let Some(pattern) = fx.pattern
        && fx.pattern_opacity > 0
        && let Some(pc) = pattern_sample(pattern, tex, sx, sy, fx)
    {
        let po = f32::from(pc.alpha) / 255.0 * f32::from(fx.pattern_opacity) / 255.0;
        let (pr, pg, pb) = (
            f32::from(pc.red) / 255.0,
            f32::from(pc.green) / 255.0,
            f32::from(pc.blue) / 255.0,
        );
        match fx.pattern_blend {
            1 => {
                r = (r + pr).min(1.0) * po + r * (1.0 - po);
                g = (g + pg).min(1.0) * po + g * (1.0 - po);
                b = (b + pb).min(1.0) * po + b * (1.0 - po);
            }
            2 => {
                r = (r - pr).max(0.0) * po + r * (1.0 - po);
                g = (g - pg).max(0.0) * po + g * (1.0 - po);
                b = (b - pb).max(0.0) * po + b * (1.0 - po);
            }
            _ => {
                r = pr * po + r * (1.0 - po);
                g = pg * po + g * (1.0 - po);
                b = pb * po + b * (1.0 - po);
            }
        }
    }
    if gray > 0.0 {
        let luma = 0.299 * r + 0.587 * g + 0.114 * b;
        r += (luma - r) * gray;
        g += (luma - g) * gray;
        b += (luma - b) * gray;
    }
    r += tr;
    g += tg;
    b += tb;
    let bush = if fx.bush_y.is_some_and(|row| sy >= row) {
        f32::from(fx.bush_opacity) / 255.0
    } else {
        1.0
    };
    let alpha = f32::from(source.alpha) / 255.0 * opacity * bush;
    r += (cr - r) * color_alpha;
    g += (cg - g) * color_alpha;
    b += (cb - b) * color_alpha;
    if fx.invert {
        r = 1.0 - r;
        g = 1.0 - g;
        b = 1.0 - b;
    }
    let to_u8 = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    Color::rgba(to_u8(r), to_u8(g), to_u8(b), to_u8(alpha.max(0.0)))
}

/// Samples the pattern overlay for a source bitmap pixel, mirroring the
/// coordinate math in the engine's `sprite.vert`/`sprite.frag`.
fn pattern_sample(pat: &Bitmap, tex: &Bitmap, sx: i64, sy: i64, fx: &SpriteFx) -> Option<Color> {
    let (pw, ph) = (f64::from(pat.width()), f64::from(pat.height()));
    if pw == 0.0 || ph == 0.0 {
        return None;
    }
    let (zx, zy) = (fx.pattern_zoom_x, fx.pattern_zoom_y);
    let (vx, vy) = if fx.pattern_tile {
        (
            sx as f64 / (pw * zx) - fx.pattern_scroll_x * f64::from(tex.width()) / (pw * pw),
            sy as f64 / (ph * zy) - fx.pattern_scroll_y * f64::from(tex.height()) / (ph * ph),
        )
    } else {
        (
            sx as f64 / (f64::from(tex.width()) * zx) - fx.pattern_scroll_x / pw,
            sy as f64 / (f64::from(tex.height()) * zy) - fx.pattern_scroll_y / ph,
        )
    };
    let px = (vx.rem_euclid(1.0) * pw).floor() as i64;
    let py = (vy.rem_euclid(1.0) * ph).floor() as i64;
    pat.get_pixel(px as i32, py as i32)
}

/// Composites one source pixel over a destination pixel using the engine's blend
/// modes: 0 normal (source-over), 1 addition, 2 subtraction.
pub(super) fn compose(dst: Color, src: Color, blend_type: i32) -> Color {
    match blend_type {
        1 => {
            let channel = |s: u8, d: u8| {
                (u32::from(s) * u32::from(src.alpha) / 255 + u32::from(d)).min(255) as u8
            };
            Color::rgba(
                channel(src.red, dst.red),
                channel(src.green, dst.green),
                channel(src.blue, dst.blue),
                (u32::from(src.alpha) + u32::from(dst.alpha)).min(255) as u8,
            )
        }
        2 => {
            let channel = |s: u8, d: u8| {
                u32::from(d).saturating_sub(u32::from(s) * u32::from(src.alpha) / 255) as u8
            };
            Color::rgba(
                channel(src.red, dst.red),
                channel(src.green, dst.green),
                channel(src.blue, dst.blue),
                (u32::from(src.alpha) + u32::from(dst.alpha)).min(255) as u8,
            )
        }
        _ => blend(src, dst, 255),
    }
}

fn blend(src: Color, dst: Color, opacity: u8) -> Color {
    let a = u32::from(src.alpha) * u32::from(opacity) / 255;
    let back = u32::from(dst.alpha) * (255 - a) / 255;
    let alpha = a + back;
    if alpha == 0 {
        return Color::default();
    }
    let channel = |s: u8, d: u8| ((u32::from(s) * a + u32::from(d) * back) / alpha) as u8;
    Color::rgba(
        channel(src.red, dst.red),
        channel(src.green, dst.green),
        channel(src.blue, dst.blue),
        alpha as u8,
    )
}
