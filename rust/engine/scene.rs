// SPDX-License-Identifier: GPL-2.0-or-later
//! Retained RGSS scene ordering and invalidation; resource IDs define draw order.
use super::Runtime;
#[cfg(feature = "gpu")]
use super::SceneDraw;
#[cfg(feature = "gpu")]
use std::collections::HashMap;

impl Runtime {
    pub(super) fn invalidate_scene(&mut self) {
        #[cfg(feature = "gpu")]
        {
            self.scene_dirty = true;
        }
    }

    #[cfg(feature = "gpu")]
    pub(super) fn prepare_scene(&mut self) {
        if self.scene_cache && !self.scene_dirty {
            return;
        }
        // Cache IDs, never references: resource disposal and replacement cannot
        // leave dangling pointers in a retained plan.
        let mut top = Vec::new();
        let mut children: HashMap<u64, Vec<(i32, i64, u64, SceneDraw)>> = HashMap::new();
        for (&id, sprite) in self.sprites.iter().filter(|(_, s)| s.visible) {
            let item = SceneDraw::Sprite(id);
            let (_, y, z) = sprite.position(&self.map_cameras);
            if sprite.viewport != 0 && self.viewports.contains_key(&sprite.viewport) {
                children.entry(sprite.viewport).or_default().push((
                    z,
                    y as i64,
                    sprite.order,
                    item,
                ));
            } else {
                top.push((z, sprite.order, item));
            }
        }
        for (&id, plane) in self.planes.iter().filter(|(_, p)| p.visible) {
            let item = SceneDraw::Plane(id);
            if plane.viewport != 0 && self.viewports.contains_key(&plane.viewport) {
                children
                    .entry(plane.viewport)
                    .or_default()
                    .push((plane.z, 0, plane.order, item));
            } else {
                top.push((plane.z, plane.order, item));
            }
        }
        for (&id, viewport) in &self.viewports {
            if children.contains_key(&id) {
                top.push((viewport.z, viewport.order, SceneDraw::BeginViewport(id)));
            }
        }
        top.sort_unstable_by_key(|(z, order, _)| (*z, *order));
        self.scene_plan.clear();
        for (_, _, draw) in top {
            self.scene_plan.push(draw);
            if let SceneDraw::BeginViewport(id) = draw {
                let mut items = children.remove(&id).unwrap_or_default();
                // RGSS orders sprites by z only; equal z keeps creation order.
                // y must not be a tie-breaker: full-screen overlays (y = 0)
                // would otherwise draw behind windows and option boxes.
                items.sort_unstable_by_key(|(z, _y, order, _)| (*z, *order));
                self.scene_plan
                    .extend(items.into_iter().map(|(_, _, _, draw)| draw));
                self.scene_plan.push(SceneDraw::EndViewport(id));
            }
        }
        self.scene_dirty = false;
    }
}
