// SPDX-License-Identifier: GPL-2.0-or-later
//! A small owned sprite scene used by the standalone demo, not the RGSS runtime.
//! RGSS runtime registries and viewport ordering live in `crate::engine`.
//!
//! [`Sprite`] owns its [`Bitmap`], exposes a source rectangle plus x/y/z,
//! width/height, opacity and visibility, and starts sized to the bitmap.
//! [`Scene::render`] clears the target to a background colour, keeps only visible
//! sprites, sorts them by `z`, and draws each with `stretch_blt`. The sort is
//! stable, so equal-`z` sprites keep creation order (a y tie-breaker would
//! reorder same-z overlays and windows).
use super::{Bitmap, Color, Rect};

pub struct Sprite {
    pub bitmap: Bitmap,
    pub source: Rect,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub width: u32,
    pub height: u32,
    pub opacity: u8,
    pub visible: bool,
}
impl Sprite {
    pub fn new(bitmap: Bitmap) -> Self {
        let source = bitmap.rect();
        Self {
            bitmap,
            source,
            x: 0,
            y: 0,
            z: 0,
            width: source.width,
            height: source.height,
            opacity: 255,
            visible: true,
        }
    }
}

#[derive(Default)]
pub struct Scene {
    pub sprites: Vec<Sprite>,
}
impl Scene {
    pub fn render(&self, target: &mut Bitmap, background: Color) {
        target.clear(background);
        let mut sprites: Vec<_> = self.sprites.iter().filter(|s| s.visible).collect();
        // RGSS orders by z only; the stable sort preserves creation order for
        // equal z (a y tie-breaker would reorder same-z overlays and windows).
        sprites.sort_by_key(|sprite| sprite.z);
        for sprite in sprites {
            target.stretch_blt(
                Rect::new(sprite.x, sprite.y, sprite.width, sprite.height),
                &sprite.bitmap,
                sprite.source,
                sprite.opacity,
            );
        }
    }
}
