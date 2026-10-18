// SPDX-License-Identifier: GPL-2.0-or-later
//! Translate RGSS scene resources into GPU draws, with compatibility fallback.
//!
//! `render_gpu` returns `Ok(false)` (so the caller runs the software renderer)
//! when no GPU host is present, or when a feature cannot be matched exactly:
//! fractional plane tiling or pattern sprites with zero pattern zoom. It draws
//! the retained `scene_plan` from `prepare_scene`, translating viewport groups
//! into scissor offsets, then overlays the frozen transition frame and the FPS
//! counter (sentinel bitmap ids `u64::MAX-1`/`u64::MAX-2`), applies brightness
//! and submits. Sprites use the tiled path only when flagged `native_tile`.
use super::{Runtime, RuntimePlane, RuntimeSprite, SceneDraw};
use crate::graphics::{Bitmap, SpriteFx, SpriteTransform};
use std::{collections::HashMap, time::Instant};

impl Runtime {
    pub(super) fn render_gpu(&mut self) -> Result<bool, String> {
        if self.host.gpu().is_none() {
            return Ok(false);
        }
        // Fractional plane tiling in the reference renderer rounds individual
        // tile edges; preserve that behavior through the software fallback.
        if self.planes.values().filter(|p| p.visible).any(|p| {
            self.bitmaps.get(&p.bitmap).is_some_and(|b| {
                let w = b.width() as f64 * p.zoom_x;
                let h = b.height() as f64 * p.zoom_y;
                !w.is_finite() || !h.is_finite() || w.fract() != 0.0 || h.fract() != 0.0
            })
        }) || self
            .sprites
            .values()
            .filter(|s| s.visible && s.pattern != 0)
            .any(|s| s.pattern_zoom_x == 0.0 || s.pattern_zoom_y == 0.0)
        {
            return Ok(false);
        }
        let counter = if self.show_counter && !self.host.has_profiler_ui() {
            self.counter_bitmap()
        } else {
            None
        };
        self.prepare_scene();
        let gpu = self.host.gpu().unwrap();
        let (width, height) = (self.target.width(), self.target.height());
        gpu.begin(width, height)?;
        let mut viewport = None;
        for draw in &self.scene_plan {
            match *draw {
                SceneDraw::Sprite(id) => {
                    let offset = viewport.map_or((0, 0), |id| {
                        let v = &self.viewports[&id];
                        (v.rect.x - v.ox, v.rect.y - v.oy)
                    });
                    Self::gpu_sprite(
                        gpu,
                        &self.bitmaps,
                        &self.sprites[&id],
                        &self.map_cameras,
                        offset,
                        viewport.is_some(),
                    )?;
                }
                SceneDraw::Plane(id) => {
                    let offset = viewport.map_or((0, 0), |id| {
                        let v = &self.viewports[&id];
                        (v.rect.x - v.ox, v.rect.y - v.oy)
                    });
                    Self::gpu_plane(
                        gpu,
                        &self.bitmaps,
                        &self.planes[&id],
                        offset,
                        viewport.is_some(),
                    )?;
                }
                SceneDraw::BeginViewport(id) => {
                    viewport = Some(id);
                    gpu.clear(true, 0);
                }
                SceneDraw::EndViewport(id) => {
                    let v = &self.viewports[&id];
                    gpu.viewport(v.color, v.tone);
                    viewport = None;
                }
            }
        }
        if let Some(frozen) = &self.frozen {
            let opacity = if let Some((step, total)) = &mut self.transitioning {
                *step += 1;
                (255u32.saturating_mul(total.saturating_sub(*step)) / (*total).max(1)) as u8
            } else {
                255
            };
            gpu.sprite(
                u64::MAX - 1,
                frozen,
                frozen.rect(),
                self.target.rect(),
                SpriteTransform::default(),
                (0, 0),
                &SpriteFx {
                    opacity,
                    ..Default::default()
                },
                0,
                false,
            )?;
            if self
                .transitioning
                .is_some_and(|(step, total)| step >= total)
            {
                self.frozen = None;
                self.transitioning = None;
            }
        }
        gpu.brightness(self.brightness);
        if let Some(overlay) = counter {
            gpu.sprite(
                u64::MAX - 2,
                &overlay,
                overlay.rect(),
                self.target.rect(),
                SpriteTransform::default(),
                (0, 0),
                &SpriteFx::default(),
                0,
                false,
            )?;
        }
        gpu.submit()?;
        self.gpu_frame = true;
        if !self.headless {
            let present_start = self.show_counter.then(Instant::now);
            self.host.present_gpu()?;
            if let Some(start) = present_start {
                self.profile_present_ms = start.elapsed().as_secs_f64() * 1000.0;
            }
        }
        Ok(true)
    }
    fn gpu_sprite(
        gpu: &mut crate::gpu::GpuRenderer,
        bitmaps: &HashMap<u64, Bitmap>,
        s: &RuntimeSprite,
        cameras: &HashMap<u64, super::world_map::MapCamera>,
        offset: (i32, i32),
        scratch: bool,
    ) -> Result<(), String> {
        let Some(bitmap) = bitmaps.get(&s.bitmap) else {
            return Ok(());
        };
        let fx = SpriteFx {
            opacity: s.opacity,
            blend_type: s.blend_type,
            color: s.color,
            tone: s.tone,
            invert: s.invert,
            bush_y: (s.bush_depth > 0)
                .then_some(i64::from(s.src.y) + i64::from(s.src.height) - i64::from(s.bush_depth)),
            bush_opacity: s.bush_opacity,
            mirror: s.mirror,
            pattern: bitmaps.get(&s.pattern),
            pattern_blend: s.pattern_blend,
            pattern_tile: s.pattern_tile,
            pattern_opacity: s.pattern_opacity,
            pattern_scroll_x: s.pattern_scroll_x,
            pattern_scroll_y: s.pattern_scroll_y,
            pattern_zoom_x: s.pattern_zoom_x,
            pattern_zoom_y: s.pattern_zoom_y,
        };
        let (x, y, z) = s.position(cameras);
        let tr = SpriteTransform {
            x,
            y,
            ox: s.ox,
            oy: s.oy,
            zoom_x: s.zoom_x,
            zoom_y: s.zoom_y,
            angle: s.angle,
            mirror: s.mirror,
        };
        if s.native_tile && s.pattern == 0 {
            gpu.tile(
                z, s.bitmap, bitmap, s.src, s.clip, tr, offset, &fx, s.pattern, scratch,
            )
        } else {
            gpu.sprite(
                s.bitmap, bitmap, s.src, s.clip, tr, offset, &fx, s.pattern, scratch,
            )
        }
    }
    fn gpu_plane(
        gpu: &mut crate::gpu::GpuRenderer,
        bitmaps: &HashMap<u64, Bitmap>,
        p: &RuntimePlane,
        offset: (i32, i32),
        scratch: bool,
    ) -> Result<(), String> {
        let Some(bitmap) = bitmaps.get(&p.bitmap) else {
            return Ok(());
        };
        let fx = SpriteFx {
            opacity: p.opacity,
            blend_type: p.blend_type,
            color: p.color,
            tone: p.tone,
            ..Default::default()
        };
        gpu.plane(
            p.bitmap,
            bitmap,
            p.clip,
            offset,
            (p.zoom_x, p.zoom_y),
            (p.ox, p.oy),
            &fx,
            scratch,
        )
    }
}
