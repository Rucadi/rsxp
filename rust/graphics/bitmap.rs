// SPDX-License-Identifier: GPL-2.0-or-later
//! Bitmap allocation, copy-on-write pixel storage, revisions and import/export.
use super::{Color, Rect};
use std::{
    error::Error,
    fmt,
    io::{self, Write},
};
use std::{
    ops::{Deref, DerefMut},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

static NEXT_REVISION: AtomicU64 = AtomicU64::new(1);
fn next_revision() -> u64 {
    NEXT_REVISION.fetch_add(1, Ordering::Relaxed)
}
/// Bitmap copies share immutable pixels until the first write. This keeps
/// snapshots and blit source copies cheap without aliasing mutable game data.
#[derive(Clone)]
pub(super) struct Pixels(Arc<Vec<u32>>);
impl Deref for Pixels {
    type Target = Vec<u32>;
    fn deref(&self) -> &Vec<u32> {
        &self.0
    }
}
impl DerefMut for Pixels {
    fn deref_mut(&mut self) -> &mut Vec<u32> {
        Arc::make_mut(&mut self.0)
    }
}
impl<'a> IntoIterator for &'a mut Pixels {
    type Item = &'a mut u32;
    type IntoIter = std::slice::IterMut<'a, u32>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}
impl<'a> IntoIterator for &'a Pixels {
    type Item = &'a u32;
    type IntoIter = std::slice::Iter<'a, u32>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[derive(Debug)]
pub struct InvalidBitmap;
impl fmt::Display for InvalidBitmap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("bitmap dimensions exceed the Rust renderer's 16 million pixel limit")
    }
}
impl Error for InvalidBitmap {}

#[derive(Clone)]
pub struct Bitmap {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) pixels: Pixels,
    pub(super) revision: u64,
    pub(super) animation: Option<BitmapAnimation>,
}

/// Frame-by-frame animation backing an animated GIF bitmap. Frames share the
/// bitmap's canvas size, so advancing only swaps a cheap shared pixel buffer.
#[derive(Clone)]
pub(super) struct BitmapAnimation {
    frames: Vec<Pixels>,
    /// Seconds each frame is shown, index-aligned with `frames`.
    delays: Vec<f64>,
    /// 0 (or negative) means loop forever; n>0 is a finite repeat count.
    loop_count: i32,
    loops: i32,
    index: usize,
    elapsed: f64,
    playing: bool,
    /// Explicit frames-per-second override set through `frame_rate=`.
    manual_fps: Option<f64>,
}
impl Bitmap {
    pub fn new(width: u32, height: u32) -> Result<Self, InvalidBitmap> {
        let count = u64::from(width) * u64::from(height);
        if width == 0 || height == 0 || count > 16_777_216 {
            return Err(InvalidBitmap);
        }
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(count as usize)
            .map_err(|_| InvalidBitmap)?;
        pixels.resize(count as usize, 0);
        Ok(Self {
            width,
            height,
            pixels: Pixels(Arc::new(pixels)),
            revision: next_revision(),
            animation: None,
        })
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    /// Builds a bitmap from tightly packed RGBA bytes (row-major).
    pub fn from_rgba(width: u32, height: u32, data: &[u8]) -> Result<Self, InvalidBitmap> {
        if data.len() < width as usize * height as usize * 4 {
            return Err(InvalidBitmap);
        }
        let mut bitmap = Self::new(width, height)?;
        for (pixel, rgba) in bitmap.pixels.iter_mut().zip(data.chunks_exact(4)) {
            *pixel = Color::rgba(rgba[0], rgba[1], rgba[2], rgba[3]).packed();
        }
        Ok(bitmap)
    }
    /// Returns tightly packed RGBA bytes (row-major), for host presentation.
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.write_rgba(&mut out);
        out
    }
    /// Writes tightly packed RGBA bytes into a reusable host-owned buffer.
    pub fn write_rgba(&self, out: &mut Vec<u8>) {
        out.resize(self.pixels.len() * 4, 0);
        for (&pixel, rgba) in self.pixels.iter().zip(out.chunks_exact_mut(4)) {
            let color = Color::unpack(pixel);
            rgba.copy_from_slice(&[color.red, color.green, color.blue, color.alpha]);
        }
    }
    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub(super) fn touch(&mut self) {
        self.revision = next_revision();
    }
    pub fn from_argb(width: u32, height: u32, pixels: &[u32]) -> Result<Self, InvalidBitmap> {
        Self::from_argb_owned(width, height, pixels.to_vec())
    }
    pub fn from_argb_owned(
        width: u32,
        height: u32,
        pixels: Vec<u32>,
    ) -> Result<Self, InvalidBitmap> {
        let count = u64::from(width) * u64::from(height);
        if width == 0 || height == 0 || count > 16_777_216 || pixels.len() as u64 != count {
            return Err(InvalidBitmap);
        }
        Ok(Self {
            width,
            height,
            pixels: Pixels(Arc::new(pixels)),
            revision: next_revision(),
            animation: None,
        })
    }
    pub fn rect(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }
    pub fn clear(&mut self, color: Color) {
        self.touch();
        self.pixels.fill(color.packed());
    }
    pub fn get_pixel(&self, x: i32, y: i32) -> Option<Color> {
        if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height {
            return None;
        }
        Some(Color::unpack(
            self.pixels[y as usize * self.width as usize + x as usize],
        ))
    }
    pub fn set_pixel(&mut self, x: i32, y: i32, color: Color) {
        self.touch();
        if x >= 0 && y >= 0 && (x as u32) < self.width && (y as u32) < self.height {
            self.pixels[y as usize * self.width as usize + x as usize] = color.packed();
        }
    }
    pub(super) fn clipped(&self, rect: Rect) -> (i64, i64, i64, i64) {
        (
            i64::from(rect.x).max(0).min(self.width as i64),
            i64::from(rect.y).max(0).min(self.height as i64),
            (i64::from(rect.x) + i64::from(rect.width))
                .max(0)
                .min(self.width as i64),
            (i64::from(rect.y) + i64::from(rect.height))
                .max(0)
                .min(self.height as i64),
        )
    }
    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        self.touch();
        let (x0, y0, x1, y1) = self.clipped(rect);
        for y in y0..y1 {
            let start = y as usize * self.width as usize;
            self.pixels[start + x0 as usize..start + x1 as usize].fill(color.packed());
        }
    }
    /// Installs composited GIF frames (index-aligned with `delays` in seconds).
    /// The first frame becomes the visible one.
    pub fn set_gif_frames(
        &mut self,
        frames: Vec<Vec<u32>>,
        delays: Vec<f64>,
        loop_count: i32,
        frames_per_second: Option<f64>,
    ) {
        if frames.is_empty() {
            return;
        }
        let delays = if delays.len() == frames.len() {
            delays
        } else {
            vec![0.1; frames.len()]
        };
        let frames: Vec<Pixels> = frames.into_iter().map(|f| Pixels(Arc::new(f))).collect();
        self.pixels = frames[0].clone();
        self.animation = Some(BitmapAnimation {
            frames,
            delays,
            loop_count,
            loops: 0,
            index: 0,
            elapsed: 0.0,
            playing: true,
            manual_fps: frames_per_second,
        });
        self.revision = next_revision();
    }
    /// Whether this bitmap has more than one frame and can animate.
    pub fn animated(&self) -> bool {
        self.animation
            .as_ref()
            .is_some_and(|a| a.frames.len() > 1)
    }
    pub fn frame_count(&self) -> u32 {
        self.animation
            .as_ref()
            .map_or(1, |a| a.frames.len() as u32)
    }
    pub fn current_frame(&self) -> u32 {
        self.animation.as_ref().map_or(0, |a| a.index as u32)
    }
    /// Frames per second of the current frame, or the explicit override.
    pub fn frame_rate(&self) -> f64 {
        match self.animation.as_ref() {
            Some(animation) => animation
                .manual_fps
                .unwrap_or_else(|| 1.0 / animation.delays[animation.index]),
            None => 0.0,
        }
    }
    pub fn set_frame_rate(&mut self, fps: f64) {
        if !fps.is_finite() || fps <= 0.0 {
            return;
        }
        if let Some(animation) = self.animation.as_mut() {
            animation.manual_fps = Some(fps);
        }
    }
    pub fn playing(&self) -> bool {
        self.animation.as_ref().is_some_and(|a| a.playing)
    }
    pub fn set_playing(&mut self, playing: bool) {
        if let Some(animation) = self.animation.as_mut() {
            animation.playing = playing;
        }
    }
    /// Shows `frame` and continues animating from it.
    pub fn goto_and_play(&mut self, frame: u32) {
        self.show_frame(frame, true);
    }
    /// Shows `frame` and pauses there.
    pub fn goto_and_stop(&mut self, frame: u32) {
        self.show_frame(frame, false);
    }
    fn show_frame(&mut self, frame: u32, playing: bool) {
        let (index, pixels) = match self.animation.as_ref() {
            Some(animation) if animation.frames.len() > 1 => {
                let index = (frame as usize).min(animation.frames.len() - 1);
                (index, animation.frames[index].clone())
            }
            _ => return,
        };
        self.pixels = pixels;
        self.revision = next_revision();
        if let Some(animation) = self.animation.as_mut() {
            animation.index = index;
            animation.elapsed = 0.0;
            animation.playing = playing;
        }
    }
    /// Advances an active animation by `dt` seconds, switching frames as delays
    /// elapse. The visible pixels always track the current frame.
    pub fn advance_animation(&mut self, dt: f64) {
        if !(dt.is_finite() && dt > 0.0) {
            return;
        }
        let (changed, index) = {
            let Some(animation) = self.animation.as_mut() else {
                return;
            };
            if !animation.playing || animation.frames.len() < 2 {
                return;
            }
            animation.elapsed += dt;
            let mut changed = false;
            loop {
                let delay = animation
                    .manual_fps
                    .map(|fps| 1.0 / fps)
                    .unwrap_or(animation.delays[animation.index]);
                if animation.elapsed < delay {
                    break;
                }
                animation.elapsed -= delay;
                animation.index += 1;
                changed = true;
                if animation.index >= animation.frames.len() {
                    animation.index = 0;
                    if animation.loop_count > 0 {
                        animation.loops += 1;
                        if animation.loops >= animation.loop_count {
                            animation.index = animation.frames.len() - 1;
                            animation.playing = false;
                            break;
                        }
                    }
                }
            }
            (changed, animation.index)
        };
        if changed {
            if let Some(animation) = self.animation.as_ref() {
                self.pixels = animation.frames[index].clone();
            }
            self.revision = next_revision();
        }
    }
    /// Source-over alpha composition, with nearest-neighbor scaling and clipping.
    pub fn write_ppm(&self, mut writer: impl Write) -> io::Result<()> {
        write!(writer, "P6\n{} {}\n255\n", self.width, self.height)?;
        let mut row = Vec::with_capacity(self.width as usize * 3);
        for pixels in self.pixels.chunks(self.width as usize) {
            row.clear();
            for &pixel in pixels {
                let color = Color::unpack(pixel);
                row.extend_from_slice(&[color.red, color.green, color.blue]);
            }
            writer.write_all(&row)?;
        }
        Ok(())
    }
}

/// KGL2 compatibility operations (see `scripts/preload/kgl2_wrap.rb` in mkxp-z).
/// These back the `KGL2.klib` Win32API functions some Pokémon fangames call for
/// light blending and cast shadows.
impl Bitmap {
    #[inline]
    fn pixel_index(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height {
            return None;
        }
        Some(y as usize * self.width as usize + x as usize)
    }
    fn scale_pixel_rgb(&mut self, x: i32, y: i32, factor: f64) {
        if let Some(index) = self.pixel_index(x, y) {
            let color = Color::unpack(self.pixels[index]);
            self.pixels[index] = Color::rgba(
                (f64::from(color.red) * factor).round() as u8,
                (f64::from(color.green) * factor).round() as u8,
                (f64::from(color.blue) * factor).round() as u8,
                color.alpha,
            )
            .packed();
        }
    }
    /// Inverts the red, green and blue components, leaving alpha unchanged.
    pub fn kgl_invert(&mut self) {
        self.touch();
        for pixel in self.pixels.iter_mut() {
            let color = Color::unpack(*pixel);
            *pixel = Color::rgba(
                255 - color.red,
                255 - color.green,
                255 - color.blue,
                color.alpha,
            )
            .packed();
        }
    }
    /// Multiplies RGB by alpha/255 and clears alpha to zero.
    pub fn kgl_compress_alpha(&mut self) {
        self.touch();
        for pixel in self.pixels.iter_mut() {
            let color = Color::unpack(*pixel);
            let factor = f64::from(color.alpha) / 255.0;
            *pixel = Color::rgba(
                (f64::from(color.red) * factor).round() as u8,
                (f64::from(color.green) * factor).round() as u8,
                (f64::from(color.blue) * factor).round() as u8,
                0,
            )
            .packed();
        }
    }
    /// KGL2 `KglLightShader`: subtracts `src * opacity` from this bitmap's RGB
    /// and forces alpha opaque, matching the original fragment shader.
    pub fn kgl_subtract_rect(&mut self, x: i32, y: i32, src: &Bitmap, src_rect: Rect, opacity: u8) {
        self.touch();
        let norm = if opacity >= 255 {
            1.0
        } else {
            f64::from(opacity) / 256.0
        };
        let subtract = |destination: u8, source: u8| {
            (f64::from(destination) - norm * f64::from(source))
                .round()
                .clamp(0.0, 255.0) as u8
        };
        for dy in 0..src_rect.height as i32 {
            let Some(sy) = src_rect.y.checked_add(dy) else {
                continue;
            };
            for dx in 0..src_rect.width as i32 {
                let (Some(sx), Some(dx_dest), Some(dy_dest)) = (
                    src_rect.x.checked_add(dx),
                    x.checked_add(dx),
                    y.checked_add(dy),
                ) else {
                    continue;
                };
                let Some(source_index) = src.pixel_index(sx, sy) else {
                    continue;
                };
                let Some(destination_index) = self.pixel_index(dx_dest, dy_dest) else {
                    continue;
                };
                let destination = Color::unpack(self.pixels[destination_index]);
                let source = Color::unpack(src.pixels[source_index]);
                self.pixels[destination_index] = Color::rgba(
                    subtract(destination.red, source.red),
                    subtract(destination.green, source.green),
                    subtract(destination.blue, source.blue),
                    255,
                )
                .packed();
            }
        }
    }
    /// KGL2 `KglShadowShaderH`: casts a shadow radially from a horizontal line.
    pub fn kgl_shadow_shader_h(&mut self, x1: i32, x2: i32, y: i32, soft: bool) -> i32 {
        self.touch();
        let w = self.width as i32;
        let h = self.height as i32;
        if y < 0 || y >= h || y == h / 2 {
            return 111;
        }
        if w <= 0 || h <= 0 {
            return 1;
        }
        let x1 = x1.clamp(0, w - 1);
        let x2 = x2.clamp(0, w - 1);
        let x_center = w / 2;
        let y_center = h / 2;
        let slope1 = f64::from(x1 - x_center) / f64::from(y - y_center);
        let slope2 = f64::from(x2 - x_center) / f64::from(y - y_center);
        let (y_start, y_end) = if y < y_center { (0, y - 1) } else { (y, h - 1) };
        for i in y_start..=y_end {
            let x_start_raw = (slope1 * f64::from(i - y_center) + f64::from(x_center)).round();
            let x_end_raw = (slope2 * f64::from(i - y_center) + f64::from(x_center) + 0.2).round();
            let x_start = x_start_raw.clamp(0.0, f64::from(w + 3)) as i32;
            let x_end = x_end_raw.clamp(
                -4.0,
                if x2 < x_center {
                    f64::from(x2 - 1)
                } else {
                    f64::from(w - 1)
                },
            ) as i32;
            if x_start <= x_end {
                for px in x_start..=x_end {
                    if let Some(index) = self.pixel_index(px, i) {
                        self.pixels[index] = 0;
                    }
                }
            }
            if soft {
                for j in (1..=3).rev() {
                    if (if x1 < x_center {
                        x_start - j < 0
                    } else {
                        x_start - j < x1
                    }) || (if x2 < x_center {
                        x_start - j >= x2
                    } else {
                        x_start - j >= w
                    }) {
                        continue;
                    }
                    self.scale_pixel_rgb(x_start - j, i, f64::from(j) / 4.0);
                }
                if x2 != x_center {
                    for j in 1..=3 {
                        if (if x1 < x_center {
                            x_end < -j
                        } else {
                            x_end < x1 - j
                        }) || (if x2 < x_center {
                            x_end >= x2 - j
                        } else {
                            x_end >= w - j
                        }) {
                            continue;
                        }
                        self.scale_pixel_rgb(x_end + j, i, f64::from(j) / 4.0);
                    }
                }
            }
        }
        1
    }
    /// KGL2 `KglShadowShaderV`/`W`: casts a shadow radially from a vertical line.
    /// `wall` uses the line as a wall instead of casting past its lower half.
    pub fn kgl_shadow_shader_v(&mut self, y1: i32, y2: i32, x: i32, wall: bool, soft: bool) -> i32 {
        self.touch();
        let w = self.width as i32;
        let h = self.height as i32;
        if x < 0 || x >= h || x == w / 2 {
            return 111;
        }
        if w <= 0 || h <= 0 {
            return 1;
        }
        let y1 = y1.clamp(0, h - 1);
        let y2 = y2.clamp(0, h - 1);
        let x_center = w / 2;
        let y_center = h / 2;
        let slope1 = f64::from(y1 - y_center) / f64::from(x - x_center);
        let slope2 = f64::from(y2 - y_center) / f64::from(x - x_center);
        let (x_start, x_end) = if x < x_center { (0, x - 1) } else { (x, w - 1) };
        for i in x_start..=x_end {
            let y_start_raw = (slope1 * f64::from(i - x_center) + f64::from(y_center)).round();
            let y_end_raw = (slope2 * f64::from(i - x_center) + f64::from(y_center) + 0.2).round();
            let y_start = if wall && y1 >= y_center {
                y1
            } else {
                y_start_raw.clamp(0.0, f64::from(h + 3)) as i32
            };
            let y_end = if wall && y2 >= y_center {
                y2 - 1
            } else {
                y_end_raw.clamp(
                    -4.0,
                    if y2 < y_center {
                        f64::from(y2 - 1)
                    } else {
                        f64::from(h - 1)
                    },
                ) as i32
            };
            if y_start <= y_end {
                for py in y_start..=y_end {
                    if let Some(index) = self.pixel_index(i, py) {
                        self.pixels[index] = 0;
                    }
                }
            }
            if soft {
                if !wall || y1 < y_center {
                    for j in (1..=3).rev() {
                        if (if y1 < y_center {
                            y_start - j < 0
                        } else {
                            y_start - j < y1
                        }) || (if y2 < y_center {
                            y_start - j >= y2
                        } else {
                            y_start - j >= h
                        }) {
                            continue;
                        }
                        self.scale_pixel_rgb(i, y_start - j, f64::from(j) / 4.0);
                    }
                }
                if y2 != y_center && (!wall || y2 < y_center) {
                    for j in 1..=3 {
                        if (if y1 < y_center {
                            y_end < -j
                        } else {
                            y_end < y1 - j
                        }) || (if y2 < y_center {
                            y_end >= y2 - j
                        } else {
                            y_end >= h - j
                        }) {
                            continue;
                        }
                        self.scale_pixel_rgb(i, y_end + j, f64::from(j) / 4.0);
                    }
                }
            }
        }
        1
    }
    /// Replaces the pixels from tightly packed RGBA bytes. Returns false when the
    /// length does not match the bitmap size.
    pub fn replace_rgba(&mut self, data: &[u8]) -> bool {
        if data.len() != self.width as usize * self.height as usize * 4 {
            return false;
        }
        self.touch();
        for (pixel, rgba) in self.pixels.iter_mut().zip(data.chunks_exact(4)) {
            *pixel = Color::rgba(rgba[0], rgba[1], rgba[2], rgba[3]).packed();
        }
        true
    }
}

/// RGBA source-over composition, matching the blit pipeline's channel math.
fn source_over(destination: u32, source: Color) -> u32 {
    if source.alpha == 0 {
        return destination;
    }
    if source.alpha == 255 {
        return source.packed();
    }
    let destination = Color::unpack(destination);
    let source_alpha = u32::from(source.alpha);
    let back = u32::from(destination.alpha) * (255 - source_alpha) / 255;
    let alpha = source_alpha + back;
    if alpha == 0 {
        return 0;
    }
    let channel = |s: u8, d: u8| ((u32::from(s) * source_alpha + u32::from(d) * back) / alpha) as u8;
    Color::rgba(
        channel(source.red, destination.red),
        channel(source.green, destination.green),
        channel(source.blue, destination.blue),
        alpha as u8,
    )
    .packed()
}

/// Linear gradient fill, box blur and radial blur (mkxp-z Bitmap effects).
impl Bitmap {
    fn lerp_channel(a: u8, b: u8, t: f64) -> u8 {
        (f64::from(a) + (f64::from(b) - f64::from(a)) * t).round() as u8
    }
    pub fn gradient_fill_rect(
        &mut self,
        rect: Rect,
        color1: Color,
        color2: Color,
        vertical: bool,
    ) {
        self.touch();
        let (x0, y0, x1, y1) = self.clipped(rect);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let span = if vertical {
            (rect.height.max(2) - 1) as f64
        } else {
            (rect.width.max(2) - 1) as f64
        };
        for y in y0..y1 {
            for x in x0..x1 {
                let t = if vertical {
                    (y - i64::from(rect.y)) as f64 / span
                } else {
                    (x - i64::from(rect.x)) as f64 / span
                }
                .clamp(0.0, 1.0);
                let color = Color::rgba(
                    Self::lerp_channel(color1.red, color2.red, t),
                    Self::lerp_channel(color1.green, color2.green, t),
                    Self::lerp_channel(color1.blue, color2.blue, t),
                    Self::lerp_channel(color1.alpha, color2.alpha, t),
                );
                let index = y as usize * self.width as usize + x as usize;
                self.pixels[index] = source_over(self.pixels[index], color);
            }
        }
    }
    /// 3x3 alpha-aware box blur.
    pub fn blur(&mut self) {
        if self.width < 3 || self.height < 3 {
            return;
        }
        let source = self.pixels.clone();
        let w = self.width as i64;
        let h = self.height as i64;
        self.touch();
        for y in 0..h {
            for x in 0..w {
                let (mut r, mut g, mut b, mut a, mut n) = (0u32, 0u32, 0u32, 0u32, 0u32);
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let (sx, sy) = (x + dx, y + dy);
                        if sx < 0 || sy < 0 || sx >= w || sy >= h {
                            continue;
                        }
                        let color = Color::unpack(source[(sy * w + sx) as usize]);
                        r += u32::from(color.red);
                        g += u32::from(color.green);
                        b += u32::from(color.blue);
                        a += u32::from(color.alpha);
                        n += 1;
                    }
                }
                self.pixels[(y * w + x) as usize] =
                    Color::rgba((r / n) as u8, (g / n) as u8, (b / n) as u8, (a / n) as u8).packed();
            }
        }
    }
    /// Rotational blur around the bitmap centre.
    pub fn radial_blur(&mut self, angle: i32, divisions: i32) {
        let divisions = divisions.clamp(2, 100);
        let angle = angle.clamp(0, 359);
        if self.width == 0 || self.height == 0 || angle == 0 {
            return;
        }
        let source = self.pixels.clone();
        let w = self.width as i64;
        let h = self.height as i64;
        let cx = (w as f64 - 1.0) / 2.0;
        let cy = (h as f64 - 1.0) / 2.0;
        let step = f64::from(angle) / (f64::from(divisions) - 1.0);
        let base = -(f64::from(angle)) / 2.0;
        self.touch();
        for y in 0..h {
            for x in 0..w {
                let (mut r, mut g, mut b, mut a) = (0u32, 0u32, 0u32, 0u32);
                for i in 0..divisions {
                    let (sin, cos) = (base + step * f64::from(i)).to_radians().sin_cos();
                    let dx = x as f64 - cx;
                    let dy = y as f64 - cy;
                    let sx = (cos * dx - sin * dy + cx).round() as i64;
                    let sy = (sin * dx + cos * dy + cy).round() as i64;
                    if sx < 0 || sy < 0 || sx >= w || sy >= h {
                        continue;
                    }
                    let color = Color::unpack(source[(sy * w + sx) as usize]);
                    r += u32::from(color.red);
                    g += u32::from(color.green);
                    b += u32::from(color.blue);
                    a += u32::from(color.alpha);
                }
                let n = divisions as u32;
                self.pixels[(y * w + x) as usize] =
                    Color::rgba((r / n) as u8, (g / n) as u8, (b / n) as u8, (a / n) as u8).packed();
            }
        }
    }
    /// A static copy of the current frame, dropping any GIF animation.
    pub fn static_copy(&self) -> Bitmap {
        Bitmap {
            width: self.width,
            height: self.height,
            pixels: self.pixels.clone(),
            revision: next_revision(),
            animation: None,
        }
    }
}
