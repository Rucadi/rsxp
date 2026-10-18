// SPDX-License-Identifier: GPL-2.0-or-later
//! CPU blits, nearest-neighbor sampling and sprite/plane rasterization.
use super::effects::{compose, shade_pixel};
use super::{Bitmap, Color, Rect, SpriteFx, SpriteTransform};

impl Bitmap {
    pub fn stretch_blt(&mut self, dst: Rect, src: &Bitmap, area: Rect, opacity: u8) {
        self.stretch_blt_clipped(dst, src, area, opacity, self.rect());
    }
    pub fn stretch_blt_clipped(
        &mut self,
        dst: Rect,
        src: &Bitmap,
        area: Rect,
        opacity: u8,
        clip: Rect,
    ) {
        self.touch();
        if dst.width == 0 || dst.height == 0 || opacity == 0 {
            return;
        }
        self.blit_simple(dst, src, area, clip, opacity, false);
    }
    /// Fast path for opacity-only sprite blits: integer nearest-neighbor
    /// sampling with source-over composition (no f32 effect pipeline).
    fn blit_simple(
        &mut self,
        dst: Rect,
        src: &Bitmap,
        area: Rect,
        clip: Rect,
        opacity: u8,
        mirror: bool,
    ) {
        let (x0, y0, x1, y1) = self.clipped(dst);
        let (cx0, cy0, cx1, cy1) = self.clipped(clip);
        let (x0, y0, x1, y1) = (x0.max(cx0), y0.max(cy0), x1.min(cx1), y1.min(cy1));
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        // Unscaled, unmirrored draws (including full-screen viewport composition)
        // have contiguous source rows. Clip once, then avoid the column Vec,
        // coordinate division and bounds checks for every sampled pixel.
        if !mirror && dst.width == area.width && dst.height == area.height {
            let dx = i64::from(dst.x) - i64::from(area.x);
            let dy = i64::from(dst.y) - i64::from(area.y);
            let left = x0.max(dx);
            let right = x1.min(dx + i64::from(src.width));
            let top = y0.max(dy);
            let bottom = y1.min(dy + i64::from(src.height));
            if left >= right || top >= bottom {
                return;
            }
            let count = (right - left) as usize;
            let pixels = &mut *self.pixels;
            for y in top..bottom {
                let source_start = (y - dy) as usize * src.width as usize + (left - dx) as usize;
                let target_start = y as usize * self.width as usize + left as usize;
                let source = &src.pixels[source_start..source_start + count];
                let target = &mut pixels[target_start..target_start + count];
                for (destination, &source) in target.iter_mut().zip(source) {
                    *destination = blend_pixel(*destination, source, u32::from(opacity));
                }
            }
            return;
        }
        let dw = i64::from(dst.width);
        let dh = i64::from(dst.height);
        let aw = i64::from(area.width);
        let ah = i64::from(area.height);
        let sw = src.width as usize;
        let src_w = src.width as i64;
        let src_h = src.height as i64;
        let op = u32::from(opacity);
        let mirror = mirror && area.width > 0;
        let origin = i64::from(area.x);
        let dst_x = i64::from(dst.x);
        // Source columns depend only on x, so compute them once per blit.
        let xs: Vec<i64> = (x0..x1)
            .map(|x| {
                let mut sx = origin + (x - dst_x) * aw / dw;
                if mirror {
                    sx = origin + aw - 1 - (sx - origin);
                }
                sx
            })
            .collect();
        let pixels = &mut *self.pixels;
        for y in y0..y1 {
            let sy = i64::from(area.y) + (y - i64::from(dst.y)) * ah / dh;
            if sy < 0 || sy >= src_h {
                continue;
            }
            let srow = sy as usize * sw;
            let drow = y as usize * self.width as usize;
            for (i, &sx) in xs.iter().enumerate() {
                if sx < 0 || sx >= src_w {
                    continue;
                }
                let sp = src.pixels[srow + sx as usize];
                let index = drow + (x0 as usize + i);
                pixels[index] = blend_pixel(pixels[index], sp, op);
            }
        }
    }
    /// Renders one sprite including the engine's effect pipeline: pattern overlay,
    /// tone (gray + rgb), opacity, color blend, inversion, mirroring, and the
    /// per-sprite blend type. `src`/`area` describe the source bitmap and source
    /// rectangle; `dst` is the destination rectangle in this bitmap.
    pub fn draw_sprite(&mut self, dst: Rect, src: &Bitmap, area: Rect, clip: Rect, fx: &SpriteFx) {
        self.touch();
        if dst.width == 0 || dst.height == 0 || fx.opacity == 0 {
            return;
        }
        // Common case: opacity-only (optionally mirrored) with no color, tone,
        // pattern, inversion or blend mode. Use an integer path instead of the
        // f32 effect pipeline, which dominates rendering time.
        let simple = fx.bush_y.is_none()
            && fx.pattern.is_none()
            && !fx.invert
            && fx.blend_type == 0
            && fx.color.alpha == 0
            && fx.tone.red == 0
            && fx.tone.green == 0
            && fx.tone.blue == 0
            && fx.tone.gray == 0;
        if simple {
            self.blit_simple(dst, src, area, clip, fx.opacity, fx.mirror);
            return;
        }
        let (x0, y0, x1, y1) = self.clipped(dst);
        let (cx0, cy0, cx1, cy1) = self.clipped(clip);
        let (x0, y0, x1, y1) = (x0.max(cx0), y0.max(cy0), x1.min(cx1), y1.min(cy1));
        let pixels = &mut *self.pixels;
        for y in y0..y1 {
            for x in x0..x1 {
                let mut sx = i64::from(area.x)
                    + (x - i64::from(dst.x)) * i64::from(area.width) / i64::from(dst.width);
                let sy = i64::from(area.y)
                    + (y - i64::from(dst.y)) * i64::from(area.height) / i64::from(dst.height);
                if fx.mirror && area.width > 0 {
                    sx = i64::from(area.x) + i64::from(area.width) - 1 - (sx - i64::from(area.x));
                }
                if sx < 0 || sy < 0 || sx >= src.width as i64 || sy >= src.height as i64 {
                    continue;
                }
                let source =
                    Color::unpack(src.pixels[sy as usize * src.width as usize + sx as usize]);
                let out = shade_pixel(src, source, sx, sy, fx);
                let index = y as usize * self.width as usize + x as usize;
                pixels[index] = compose(Color::unpack(pixels[index]), out, fx.blend_type).packed();
            }
        }
    }
    /// Renders a rotated and/or scaled sprite by inverse-mapping destination
    /// pixels into the sprite quad, matching the engine's transform matrix.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_sprite_rotated(
        &mut self,
        src: &Bitmap,
        area: Rect,
        clip: Rect,
        tr: &SpriteTransform,
        offset: (i32, i32),
        fx: &SpriteFx,
    ) {
        self.touch();
        if area.width == 0 || area.height == 0 || fx.opacity == 0 {
            return;
        }
        let (zx, zy) = (tr.zoom_x, tr.zoom_y);
        if zx == 0.0 || zy == 0.0 {
            return;
        }
        let angle = tr.angle.to_radians();
        let (sin, cos) = angle.sin_cos();
        let src_ox = f64::from(area.x.min(0));
        let src_oy = f64::from(area.y.min(0));
        let adj_ox = tr.ox + src_ox;
        let adj_oy = tr.oy + src_oy;
        let (sxc, sys) = (zx * cos, zy * sin);
        let (sxs, syc) = (zx * sin, zy * cos);
        let tx = -adj_ox * sxc - adj_oy * sys + tr.x + f64::from(offset.0);
        let ty = adj_ox * sxs - adj_oy * syc + tr.y + f64::from(offset.1);
        let det = sxc * syc + sys * sxs;
        if det == 0.0 {
            return;
        }
        let (w, h) = (f64::from(area.width), f64::from(area.height));
        let map = |lx: f64, ly: f64| (sxc * lx + sys * ly + tx, -sxs * lx + syc * ly + ty);
        let corners = [map(0.0, 0.0), map(w, 0.0), map(0.0, h), map(w, h)];
        let min_x = corners.iter().map(|p| p.0).fold(f64::INFINITY, f64::min) as i64;
        let max_x = corners
            .iter()
            .map(|p| p.0)
            .fold(f64::NEG_INFINITY, f64::max)
            .ceil() as i64;
        let min_y = corners.iter().map(|p| p.1).fold(f64::INFINITY, f64::min) as i64;
        let max_y = corners
            .iter()
            .map(|p| p.1)
            .fold(f64::NEG_INFINITY, f64::max)
            .ceil() as i64;
        let (cx0, cy0, cx1, cy1) = self.clipped(clip);
        let x0 = min_x.max(0).max(cx0);
        let y0 = min_y.max(0).max(cy0);
        let x1 = max_x.min(self.width as i64).min(cx1);
        let y1 = max_y.min(self.height as i64).min(cy1);
        let pixels = &mut *self.pixels;
        for y in y0..y1 {
            for x in x0..x1 {
                let dx = x as f64 + 0.5 - tx;
                let dy = y as f64 + 0.5 - ty;
                let lx = (syc * dx - sys * dy) / det;
                let ly = (sxs * dx + sxc * dy) / det;
                if lx < 0.0 || ly < 0.0 || lx >= w || ly >= h {
                    continue;
                }
                let cell = lx.floor() as i32;
                let row = ly.floor() as i32;
                let sx = if tr.mirror {
                    i64::from(area.x) + i64::from(area.width) - 1 - i64::from(cell)
                } else {
                    i64::from(area.x) + i64::from(cell)
                };
                let sy = i64::from(area.y) + i64::from(row);
                if sx < 0 || sy < 0 || sx >= src.width as i64 || sy >= src.height as i64 {
                    continue;
                }
                let source =
                    Color::unpack(src.pixels[sy as usize * src.width as usize + sx as usize]);
                let out = shade_pixel(src, source, sx, sy, fx);
                let index = y as usize * self.width as usize + x as usize;
                pixels[index] = compose(Color::unpack(pixels[index]), out, fx.blend_type).packed();
            }
        }
    }
    /// Draws a Plane: the bitmap tiled across `clip`, scrolled by `ox`/`oy`
    /// and scaled by `zoom`, matching the engine's plane tiling.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_plane(
        &mut self,
        clip: Rect,
        offset: (i32, i32),
        src: &Bitmap,
        zoom_x: f64,
        zoom_y: f64,
        ox: i32,
        oy: i32,
        fx: &SpriteFx,
    ) {
        if fx.opacity == 0 || zoom_x <= 0.0 || zoom_y <= 0.0 {
            return;
        }
        let sw = f64::from(src.width()) * zoom_x;
        let sh = f64::from(src.height()) * zoom_y;
        if sw < 1.0 || sh < 1.0 {
            return;
        }
        let (cx0, cy0, cx1, cy1) = self.clipped(clip);
        let origin_x = f64::from(offset.0) - (ox as f64).rem_euclid(sw);
        let origin_y = f64::from(offset.1) - (oy as f64).rem_euclid(sh);
        let i0 = ((cx0 as f64 - origin_x) / sw).floor() as i64;
        let i1 = ((cx1 as f64 - origin_x) / sw).ceil() as i64;
        let j0 = ((cy0 as f64 - origin_y) / sh).floor() as i64;
        let j1 = ((cy1 as f64 - origin_y) / sh).ceil() as i64;
        let area = src.rect();
        let budget = (cx1 - cx0).max(1) * (cy1 - cy0).max(1) + 4;
        let mut drawn = 0i64;
        for j in j0..j1 {
            for i in i0..i1 {
                drawn += 1;
                if drawn > budget {
                    return;
                }
                let dst = Rect::new(
                    (origin_x + i as f64 * sw) as i32,
                    (origin_y + j as f64 * sh) as i32,
                    sw as u32,
                    sh as u32,
                );
                self.draw_sprite(dst, src, area, clip, fx);
            }
        }
    }
}

/// Integer source-over, shared by contiguous and scaled blits to keep rounding
/// identical for transparent destinations and partially opaque sources.
#[inline]
fn blend_pixel(destination: u32, source: u32, opacity: u32) -> u32 {
    let a = ((source >> 24) & 0xff) * opacity / 255;
    if a == 0 {
        return destination;
    }
    if a == 255 {
        return source;
    }
    let back = ((destination >> 24) & 0xff) * (255 - a) / 255;
    let alpha = a + back;
    let channel = |s: u32, d: u32| (s * a + d * back) / alpha;
    let r = channel((source >> 16) & 0xff, (destination >> 16) & 0xff);
    let g = channel((source >> 8) & 0xff, (destination >> 8) & 0xff);
    let b = channel(source & 0xff, destination & 0xff);
    (alpha << 24) | (r << 16) | (g << 8) | b
}
