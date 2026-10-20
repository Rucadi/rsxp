// SPDX-License-Identifier: GPL-2.0-or-later
//! CPU-side command encoding: turns RGSS scene elements into the flat `Params`
//! records the compute shader consumes.
//!
//! Each public method appends one 256-byte `Params` (16 `vec4<u32>` rows) to
//! `GpuRenderer::commands`; `shader.wgsl` decodes the same rows. All entries are
//! `u32`; floating-point fields are bit patterns written through
//! `Params::floats`. The operation code lives in `a[0][0]`:
//! - `0` clear: fill the clipped region with the ARGB color in `a[3][0]`.
//! - `1` sprite/plane: sample an atlas source (and optional pattern) and
//!   transform it into the destination.
//! - `2` viewport: apply color/tone to the scratch layer (read at byte offset
//!   `a[0][2] == w*h`) and composite it into the destination.
//! - `3` brightness: scale destination RGB by `a[3][0] / 255`.
//! - `4` native tile geometry: draw the bin/quad list encoded in `tile_data`,
//!   bounded by `a[1]`.
//!
//! Common field layout (rows below the operation tag):
//! - `a[0] = [operation, dest_offset, source_atlas, pattern_atlas]`, where
//!   `dest_offset` is `w*h` for the scratch layer and `0` for the frame.
//! - `a[1] = [x, y, x1, y1]` clipped destination bounds; empty rects are
//!   dropped before uploads so they cannot split tile runs.
//! - `a[2] = [frame_w, frame_h, source_w, source_h]`.
//! - `a[3]`: source area rect, or the clear color / brightness value.
//! - `a[4]` inverse transform, `a[5]` translation and size (floats).
//! - `a[6]` color, `a[7]` tone, `a[8]` pattern zoom/scroll (floats).
//! - `a[9] = [pattern_w, pattern_h, tile, blend]`.
//! - `a[10] = [blend, invert, mirror, mode]`; `mode` is `0` for the
//!   axis-aligned/zoom path, `1` for the rotated matrix path and `2` for a
//!   tiled plane.
//! - `a[11]` opacity floats `[opacity, pattern_opacity, 0, simple]`.
//! - `a[12] = [opacity, bush, bush_y, bush_opacity]`.
//! - `a[13]` zoomed destination size, `a[14]` tile bin origin,
//!   `a[15] = [base, bin_width, BIN_SIZE, 0]`.
//!
//! `effects` fills the shared color/tone/pattern/opacity rows. `sprite` selects
//! the axis-aligned, negative-zoom or rotated inverse-matrix path and narrows
//! the clip to the real draw. `tile` defers compact `TileQuad`s to `flush_tiles`
//! (which caches geometry through `TileGeometryCache`), routing oversized,
//! rotated, patterned or bushed cells back to `sprite`.
use super::{GpuRenderer, Params};
use crate::graphics::{Bitmap, Color, Rect, SpriteFx, SpriteTransform, Tone};
use crate::tilemap::{TileGeometry, TileGeometryCache, TileQuad};

impl GpuRenderer {
    fn params(&self, operation: u32, scratch: bool, clip: Rect) -> Params {
        let (w, h) = self.size;
        let mut p = Params { a: [[0; 4]; 16] };
        p.a[0] = [operation, if scratch { w * h } else { 0 }, 0, 0];
        let x = i64::from(clip.x).clamp(0, i64::from(w));
        let y = i64::from(clip.y).clamp(0, i64::from(h));
        let x1 = (i64::from(clip.x) + i64::from(clip.width)).clamp(x, i64::from(w));
        let y1 = (i64::from(clip.y) + i64::from(clip.height)).clamp(y, i64::from(h));
        p.a[1] = [x as u32, y as u32, x1 as u32, y1 as u32];
        p.a[2] = [w, h, 0, 0];
        p.floats(11, [1.0, 0.0, 0.0, 0.0]);
        p
    }
    pub fn clear(&mut self, scratch: bool, color: u32) {
        self.flush_tiles();
        let mut p = self.params(0, scratch, Rect::new(0, 0, self.size.0, self.size.1));
        p.a[3][0] = color;
        self.commands.push(p);
    }
    fn effects(
        &mut self,
        p: &mut Params,
        source_id: u64,
        bitmap: &Bitmap,
        fx: &SpriteFx,
    ) -> Result<(), String> {
        p.a[0][2] = self.upload(source_id, bitmap)?;
        p.a[2][2] = bitmap.width();
        p.a[2][3] = bitmap.height();
        p.floats(
            6,
            [
                fx.color.red as f32 / 255.0,
                fx.color.green as f32 / 255.0,
                fx.color.blue as f32 / 255.0,
                fx.color.alpha as f32 / 255.0,
            ],
        );
        p.floats(
            7,
            [
                fx.tone.red.clamp(-255, 255) as f32 / 255.0,
                fx.tone.green.clamp(-255, 255) as f32 / 255.0,
                fx.tone.blue.clamp(-255, 255) as f32 / 255.0,
                fx.tone.gray.clamp(0, 255) as f32 / 255.0,
            ],
        );
        p.a[10][0] = fx.blend_type as u32;
        p.a[10][1] = u32::from(fx.invert);
        p.a[10][2] = u32::from(fx.mirror);
        let simple = fx.bush_y.is_none()
            && fx.pattern.is_none()
            && !fx.invert
            && fx.blend_type == 0
            && fx.color.alpha == 0
            && fx.tone == Tone::default();
        p.floats(
            11,
            [
                fx.opacity as f32 / 255.0,
                fx.pattern_opacity as f32 / 255.0,
                0.0,
                if simple { 1.0 } else { 0.0 },
            ],
        );
        p.a[12][0] = u32::from(fx.opacity);
        p.a[12][1] = u32::from(fx.bush_y.is_some());
        p.a[12][2] = fx.bush_y.unwrap_or(0) as i32 as u32;
        p.a[12][3] = u32::from(fx.bush_opacity);
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub fn sprite(
        &mut self,
        id: u64,
        bitmap: &Bitmap,
        area: Rect,
        clip: Rect,
        tr: SpriteTransform,
        offset: (i32, i32),
        fx: &SpriteFx,
        pattern_id: u64,
        scratch: bool,
    ) -> Result<(), String> {
        if area.width == 0
            || area.height == 0
            || fx.opacity == 0
            || tr.zoom_x == 0.0
            || tr.zoom_y == 0.0
        {
            return Ok(());
        }
        let mut p = self.params(1, scratch, clip);
        p.a[3] = [area.x as u32, area.y as u32, area.width, area.height];
        p.floats(
            5,
            [
                (tr.x - tr.ox * tr.zoom_x) as f32 + offset.0 as f32,
                (tr.y - tr.oy * tr.zoom_y) as f32 + offset.1 as f32,
                area.width as f32,
                area.height as f32,
            ],
        );
        // Negative zoom mirrors the sprite around its position. The rotated
        // branch's inverse matrix already handles signed zoom, so use it.
        if tr.angle == 0.0 && tr.zoom_x > 0.0 && tr.zoom_y > 0.0 {
            let width = (area.width as f64 * tr.zoom_x) as u32;
            let height = (area.height as f64 * tr.zoom_y) as u32;
            if width == 0 || height == 0 {
                return Ok(());
            }
            p.a[13] = [width, height, 0, 0];
            let x = (tr.x - tr.ox * tr.zoom_x) as i32 + offset.0;
            let y = (tr.y - tr.oy * tr.zoom_y) as i32 + offset.1;
            p.floats(
                5,
                [x as f32, y as f32, area.width as f32, area.height as f32],
            );
            p.floats(
                4,
                [
                    area.width as f32 / width as f32,
                    0.0,
                    0.0,
                    area.height as f32 / height as f32,
                ],
            );
            let c = p.a[1];
            p.a[1] = [
                c[0].max(x.max(0) as u32),
                c[1].max(y.max(0) as u32),
                c[2].min((i64::from(x) + i64::from(width)).max(0) as u32),
                c[3].min((i64::from(y) + i64::from(height)).max(0) as u32),
            ];
        } else {
            let (sin, cos) = tr.angle.to_radians().sin_cos();
            let (a, b, c, d) = (
                tr.zoom_x * cos,
                tr.zoom_y * sin,
                tr.zoom_x * sin,
                tr.zoom_y * cos,
            );
            let det = a * d + b * c;
            let ox = tr.ox + f64::from(area.x.min(0));
            let oy = tr.oy + f64::from(area.y.min(0));
            let tx = -ox * a - oy * b + tr.x + offset.0 as f64;
            let ty = ox * c - oy * d + tr.y + offset.1 as f64;
            p.floats(
                4,
                [
                    (d / det) as f32,
                    (-b / det) as f32,
                    (c / det) as f32,
                    (a / det) as f32,
                ],
            );
            p.floats(
                5,
                [tx as f32, ty as f32, area.width as f32, area.height as f32],
            );
            p.a[10][3] = 1;
            let corners = [
                (0.0, 0.0),
                (area.width as f64, 0.0),
                (0.0, area.height as f64),
                (area.width as f64, area.height as f64),
            ]
            .map(|(x, y)| (a * x + b * y + tx, -c * x + d * y + ty));
            let c = p.a[1];
            p.a[1] = [
                c[0].max(
                    corners
                        .iter()
                        .map(|v| v.0)
                        .fold(f64::INFINITY, f64::min)
                        .max(0.0) as u32,
                ),
                c[1].max(
                    corners
                        .iter()
                        .map(|v| v.1)
                        .fold(f64::INFINITY, f64::min)
                        .max(0.0) as u32,
                ),
                c[2].min(
                    corners
                        .iter()
                        .map(|v| v.0)
                        .fold(f64::NEG_INFINITY, f64::max)
                        .ceil()
                        .max(0.0) as u32,
                ),
                c[3].min(
                    corners
                        .iter()
                        .map(|v| v.1)
                        .fold(f64::NEG_INFINITY, f64::max)
                        .ceil()
                        .max(0.0) as u32,
                ),
            ];
        }
        // Empty draws cannot change pixels. Drop them before uploads and batch
        // construction, where they would otherwise split disjoint tile runs.
        let [x, y, x1, y1] = p.a[1];
        if x1 <= x || y1 <= y {
            return Ok(());
        }
        // A culled/transparent sprite is not an ordering barrier. Flush only
        // when a draw will actually be emitted, keeping native tile runs whole.
        if !self.collecting_tile {
            self.flush_tiles();
        }
        self.effects(&mut p, id, bitmap, fx)?;
        if let Some(pattern) = fx.pattern {
            p.a[0][3] = self.upload(pattern_id, pattern)?;
            p.a[9] = [
                pattern.width(),
                pattern.height(),
                u32::from(fx.pattern_tile),
                fx.pattern_blend as u32,
            ];
            p.floats(
                8,
                [
                    fx.pattern_zoom_x as f32,
                    fx.pattern_zoom_y as f32,
                    fx.pattern_scroll_x as f32,
                    fx.pattern_scroll_y as f32,
                ],
            );
        }
        self.commands.push(p);
        Ok(())
    }
    /// Native atlas cells plus ground/priority geometry. Ordinary scene
    /// elements flush the run, allowing adjacent tile layers to share a draw
    /// while characters retain their exact z/y/creation ordering.
    #[allow(clippy::too_many_arguments)]
    pub fn tile(
        &mut self,
        z: i32,
        id: u64,
        bitmap: &Bitmap,
        area: Rect,
        clip: Rect,
        tr: SpriteTransform,
        offset: (i32, i32),
        fx: &SpriteFx,
        pattern_id: u64,
        scratch: bool,
    ) -> Result<(), String> {
        if area.width == 0 || area.height == 0 || fx.opacity == 0 {
            return Ok(());
        }
        if tr.angle == 0.0 {
            let x = (tr.x - tr.ox * tr.zoom_x) as i32 as i64 + i64::from(offset.0);
            let y = (tr.y - tr.oy * tr.zoom_y) as i32 as i64 + i64::from(offset.1);
            let w = (area.width as f64 * tr.zoom_x) as u32 as i64;
            let h = (area.height as f64 * tr.zoom_y) as u32 as i64;
            let cx = i64::from(clip.x).max(0);
            let cy = i64::from(clip.y).max(0);
            let cx1 = (i64::from(clip.x) + i64::from(clip.width)).min(i64::from(self.size.0));
            let cy1 = (i64::from(clip.y) + i64::from(clip.height)).min(i64::from(self.size.1));
            if w == 0 || h == 0 || x >= cx1 || y >= cy1 || x + w <= cx || y + h <= cy {
                return Ok(());
            }
        }
        // Complex plugin effects keep the ordinary sprite path.
        if area.width > 64
            || area.height > 64
            || tr.angle != 0.0
            || fx.pattern.is_some()
            || fx.bush_y.is_some()
        {
            return self.sprite(id, bitmap, area, clip, tr, offset, fx, pattern_id, scratch);
        }
        if self.pending_tiles.first().is_some_and(|q| {
            q.params[0][1]
                != if scratch {
                    self.size.0 * self.size.1
                } else {
                    0
                }
        }) {
            self.flush_tiles();
        }
        let (cell_id, cell) = self.tile_atlas.cell(id, bitmap, area)?;
        let cell = cell.clone();
        let count = self.commands.len();
        self.collecting_tile = true;
        let result = self.sprite(
            cell_id,
            &cell,
            cell.rect(),
            clip,
            tr,
            offset,
            fx,
            0,
            scratch,
        );
        self.collecting_tile = false;
        result?;
        if self.commands.len() > count {
            let p = self.commands.pop().unwrap();
            self.pending_tiles.push(TileQuad { z, params: p.a });
        }
        Ok(())
    }
    pub fn flush_tiles(&mut self) {
        if self.pending_tiles.is_empty() {
            return;
        }
        if self.tile_runs.len() <= self.tile_run_index {
            self.tile_runs.push(TileGeometryCache::default());
        }
        let geometry = self.tile_runs[self.tile_run_index].append(
            self.size.0,
            self.size.1,
            &self.pending_tiles,
            &mut self.tile_data,
        );
        self.tile_run_index += 1;
        let mut p = self.params(4, false, Rect::new(0, 0, self.size.0, self.size.1));
        p.a[0][1] = self.pending_tiles[0].params[0][1];
        p.a[1] = geometry.bounds;
        p.a[14] = [geometry.bin_origin[0], geometry.bin_origin[1], 0, 0];
        p.a[15] = [geometry.base, geometry.bin_width, TileGeometry::BIN_SIZE, 0];
        self.native_tile_batches += 1;
        self.native_tile_layers += geometry.layer_count as u64;
        self.native_tile_quads += self.pending_tiles.len() as u64;
        self.commands.push(p);
        self.pending_tiles.clear();
    }
    #[allow(clippy::too_many_arguments)]
    pub fn plane(
        &mut self,
        id: u64,
        bitmap: &Bitmap,
        clip: Rect,
        offset: (i32, i32),
        zoom: (f64, f64),
        scroll: (i32, i32),
        fx: &SpriteFx,
        scratch: bool,
    ) -> Result<(), String> {
        let (w, h) = (
            bitmap.width() as f64 * zoom.0,
            bitmap.height() as f64 * zoom.1,
        );
        if w < 1.0 || h < 1.0 || fx.opacity == 0 {
            return Ok(());
        }
        let mut p = self.params(1, scratch, clip);
        if p.a[1][2] <= p.a[1][0] || p.a[1][3] <= p.a[1][1] {
            return Ok(());
        }
        self.flush_tiles();
        self.effects(&mut p, id, bitmap, fx)?;
        p.a[3] = [0, 0, bitmap.width(), bitmap.height()];
        p.a[10][3] = 2;
        p.floats(
            5,
            [
                offset.0 as f32 - (scroll.0 as f64).rem_euclid(w) as f32,
                offset.1 as f32 - (scroll.1 as f64).rem_euclid(h) as f32,
                w as f32,
                h as f32,
            ],
        );
        self.commands.push(p);
        Ok(())
    }
    pub fn viewport(&mut self, color: Color, tone: Tone) {
        self.flush_tiles();
        let mut p = self.params(2, false, Rect::new(0, 0, self.size.0, self.size.1));
        p.a[0][2] = self.size.0 * self.size.1;
        p.floats(
            6,
            [
                color.red as f32 / 255.0,
                color.green as f32 / 255.0,
                color.blue as f32 / 255.0,
                color.alpha as f32 / 255.0,
            ],
        );
        p.floats(
            7,
            [
                tone.red.clamp(-255, 255) as f32 / 255.0,
                tone.green.clamp(-255, 255) as f32 / 255.0,
                tone.blue.clamp(-255, 255) as f32 / 255.0,
                tone.gray.clamp(0, 255) as f32 / 255.0,
            ],
        );
        self.commands.push(p);
    }
    pub fn brightness(&mut self, value: u8) {
        self.flush_tiles();
        if value != 255 {
            let mut p = self.params(3, false, Rect::new(0, 0, self.size.0, self.size.1));
            p.a[3][0] = value.into();
            self.commands.push(p);
        }
    }
}
