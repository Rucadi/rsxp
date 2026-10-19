// SPDX-License-Identifier: GPL-2.0-or-later
//! Compose RGSS sprites, planes and viewport groups through CPU bitmaps.
//!
//! `compose_scene` resolves world tiles to current camera coordinates, buckets
//! visible sprites/planes by viewport, sorts by `(z, order)` — never `y`, which
//! would push full-screen overlays behind windows — and draws each element into
//! `target`. Viewports render into a reusable off-screen buffer, get their
//! color/tone applied, then blit back. `render_software` wraps that with the
//! frozen-transition overlay, brightness and FPS counter, then presents;
//! `snapshot` recomposes and clones for `Graphics.snap_to_bitmap`.
use super::{DrawItem, Element, Runtime, RuntimePlane, RuntimeSprite};
use crate::graphics::{Bitmap, Color, Rect, SpriteFx, SpriteTransform};
use std::collections::HashMap;

impl Runtime {
    /// Compose and present one frame using the CPU reference renderer.
    pub(super) fn render_software(&mut self) -> Result<(), String> {
        self.compose_scene()?;
        if let Some(frozen) = &self.frozen {
            let opacity = if let Some((step, total)) = &mut self.transitioning {
                *step += 1;
                (255u32.saturating_mul(total.saturating_sub(*step)) / (*total).max(1)) as u8
            } else {
                255
            };
            self.target
                .stretch_blt(self.target.rect(), frozen, frozen.rect(), opacity);
            if self
                .transitioning
                .is_some_and(|(step, total)| step >= total)
            {
                self.frozen = None;
                self.transitioning = None;
            }
        }
        if self.brightness != 255 {
            self.target.brightness(self.brightness);
        }
        if self.show_counter
            && !self.host.has_performance_ui()
            && let Some(overlay) = self.counter_bitmap()
        {
            self.target.draw_sprite(
                overlay.rect(),
                &overlay,
                overlay.rect(),
                self.target.rect(),
                &SpriteFx::default(),
            );
        }
        if !self.headless {
            self.host.present(&self.target)?;
        }
        Ok(())
    }

    /// Composite the current scene (sprites, planes and viewport groups) into
    /// `target` without advancing transitions/brightness, drawing the counter or
    /// presenting. This backs `Graphics.snap_to_bitmap`, matching mkxp-z's
    /// `Graphics::snapToBitmap` -> `screen.composite()`: a snapshot must reflect
    /// the current sprite state, not the previously presented frame.
    pub(super) fn compose_scene(&mut self) -> Result<(), String> {
        self.gpu_frame = false;
        self.target.clear(Color::rgb(0, 0, 0));
        let sprites: Vec<RuntimeSprite> = self
            .sprites
            .values()
            .filter(|sprite| sprite.visible)
            .map(|sprite| sprite.resolved(&self.map_cameras))
            .collect();
        let planes: Vec<RuntimePlane> = self
            .planes
            .values()
            .filter(|plane| plane.visible)
            .cloned()
            .collect();
        let mut elements: Vec<(i32, u64, Element)> = Vec::new();
        let mut viewport_sprites: HashMap<u64, Vec<RuntimeSprite>> = HashMap::new();
        let mut viewport_planes: HashMap<u64, Vec<RuntimePlane>> = HashMap::new();
        for sprite in sprites {
            if sprite.viewport != 0 && self.viewports.contains_key(&sprite.viewport) {
                viewport_sprites
                    .entry(sprite.viewport)
                    .or_default()
                    .push(sprite);
            } else {
                elements.push((sprite.z, sprite.order, Element::Sprite(sprite)));
            }
        }
        for plane in planes {
            if plane.viewport != 0 && self.viewports.contains_key(&plane.viewport) {
                viewport_planes
                    .entry(plane.viewport)
                    .or_default()
                    .push(plane);
            } else {
                elements.push((plane.z, plane.order, Element::Plane(plane)));
            }
        }
        for (id, viewport) in &self.viewports {
            if viewport_sprites.contains_key(id) || viewport_planes.contains_key(id) {
                elements.push((viewport.z, viewport.order, Element::Viewport(*id)));
            }
        }
        elements.sort_by_key(|(z, order, _)| (*z, *order));
        for (_, _, element) in elements {
            match element {
                Element::Sprite(sprite) => {
                    Self::draw_sprite_into(&mut self.target, &self.bitmaps, &sprite, (0, 0));
                }
                Element::Plane(plane) => {
                    Self::draw_plane_into(&mut self.target, &self.bitmaps, &plane, (0, 0));
                }
                Element::Viewport(id) => {
                    self.draw_viewport(id, &mut viewport_sprites, &mut viewport_planes)?
                }
            }
        }
        Ok(())
    }

    /// Composite the current scene and return a copy for `Bitmap` snapshots.
    pub(super) fn snapshot(&mut self) -> Result<Bitmap, String> {
        self.compose_scene()?;
        Ok(self.target.clone())
    }

    fn draw_sprite_into(
        target: &mut Bitmap,
        bitmaps: &HashMap<u64, Bitmap>,
        sprite: &RuntimeSprite,
        offset: (i32, i32),
    ) {
        if let Some(bitmap) = bitmaps.get(&sprite.bitmap) {
            let fx = SpriteFx {
                opacity: sprite.opacity,
                blend_type: sprite.blend_type,
                color: sprite.color,
                tone: sprite.tone,
                invert: sprite.invert,
                bush_y: (sprite.bush_depth > 0).then_some(
                    i64::from(sprite.src.y) + i64::from(sprite.src.height)
                        - i64::from(sprite.bush_depth),
                ),
                bush_opacity: sprite.bush_opacity,
                mirror: sprite.mirror,
                pattern: bitmaps.get(&sprite.pattern),
                pattern_blend: sprite.pattern_blend,
                pattern_tile: sprite.pattern_tile,
                pattern_opacity: sprite.pattern_opacity,
                pattern_scroll_x: sprite.pattern_scroll_x,
                pattern_scroll_y: sprite.pattern_scroll_y,
                pattern_zoom_x: sprite.pattern_zoom_x,
                pattern_zoom_y: sprite.pattern_zoom_y,
            };
            // Negative zoom mirrors the sprite around its position; the
            // inverse-mapped transform path handles those signs correctly.
            if sprite.angle == 0.0 && sprite.zoom_x >= 0.0 && sprite.zoom_y >= 0.0 {
                let dst = Rect::new(
                    (sprite.x - sprite.ox * sprite.zoom_x) as i32 + offset.0,
                    (sprite.y - sprite.oy * sprite.zoom_y) as i32 + offset.1,
                    (f64::from(sprite.src.width) * sprite.zoom_x) as u32,
                    (f64::from(sprite.src.height) * sprite.zoom_y) as u32,
                );
                target.draw_sprite(dst, bitmap, sprite.src, sprite.clip, &fx);
            } else {
                let transform = SpriteTransform {
                    x: sprite.x,
                    y: sprite.y,
                    ox: sprite.ox,
                    oy: sprite.oy,
                    zoom_x: sprite.zoom_x,
                    zoom_y: sprite.zoom_y,
                    angle: sprite.angle,
                    mirror: sprite.mirror,
                };
                target.draw_sprite_rotated(
                    bitmap,
                    sprite.src,
                    sprite.clip,
                    &transform,
                    offset,
                    &fx,
                );
            }
        }
    }
    fn draw_plane_into(
        target: &mut Bitmap,
        bitmaps: &HashMap<u64, Bitmap>,
        plane: &RuntimePlane,
        offset: (i32, i32),
    ) {
        if let Some(bitmap) = bitmaps.get(&plane.bitmap) {
            let fx = SpriteFx {
                opacity: plane.opacity,
                blend_type: plane.blend_type,
                color: plane.color,
                tone: plane.tone,
                ..SpriteFx::default()
            };
            target.draw_plane(
                plane.clip,
                offset,
                bitmap,
                plane.zoom_x,
                plane.zoom_y,
                plane.ox,
                plane.oy,
                &fx,
            );
        }
    }
    fn draw_viewport(
        &mut self,
        id: u64,
        groups: &mut HashMap<u64, Vec<RuntimeSprite>>,
        plane_groups: &mut HashMap<u64, Vec<RuntimePlane>>,
    ) -> Result<(), String> {
        let sprites = groups.remove(&id).unwrap_or_default();
        let planes = plane_groups.remove(&id).unwrap_or_default();
        let mut viewport = match self.viewports.remove(&id) {
            Some(viewport) => viewport,
            None => return Ok(()),
        };
        let (width, height) = (self.target.width(), self.target.height());
        let mut buffer = match viewport.buffer.take() {
            Some(buffer) if buffer.width() == width && buffer.height() == height => buffer,
            _ => Bitmap::new(width, height).map_err(|e| e.to_string())?,
        };
        buffer.clear(Color::default());
        let offset = (viewport.rect.x - viewport.ox, viewport.rect.y - viewport.oy);
        let mut items: Vec<(i32, i64, u64, DrawItem)> = Vec::new();
        for (i, sprite) in sprites.iter().enumerate() {
            items.push((sprite.z, sprite.y as i64, sprite.order, DrawItem::Sprite(i)));
        }
        for (i, plane) in planes.iter().enumerate() {
            items.push((plane.z, 0, plane.order, DrawItem::Plane(i)));
        }
        // RGSS orders sprites by z only; equal z keeps creation order. Using y
        // as a tie-breaker would push full-screen overlays (y = 0) behind
        // windows and option boxes drawn lower on the screen.
        items.sort_by_key(|(z, _y, order, _)| (*z, *order));
        for (_, _, _, item) in items {
            match item {
                DrawItem::Sprite(i) => {
                    Self::draw_sprite_into(&mut buffer, &self.bitmaps, &sprites[i], offset);
                }
                DrawItem::Plane(i) => {
                    Self::draw_plane_into(&mut buffer, &self.bitmaps, &planes[i], offset);
                }
            }
        }
        buffer.apply_color_tone(viewport.color, viewport.tone);
        let rect = Rect::new(0, 0, width, height);
        self.target.stretch_blt(rect, &buffer, rect, 255);
        viewport.buffer = Some(buffer);
        self.viewports.insert(id, viewport);
        groups.insert(id, sprites);
        plane_groups.insert(id, planes);
        Ok(())
    }
}
