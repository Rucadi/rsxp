// SPDX-License-Identifier: GPL-2.0-or-later
//! World-space tile placements and a shared camera transform. No Ruby values.
//!
//! A `WorldTile` is a stable placement in a group, projected on demand through
//! that group's `MapCamera`: screen `(x, y)` is the tile minus the camera, and
//! `z` follows RGSS priority rules — reflections are `-100`, tiles at priority
//! 0 (or on the active bridge layer) are `0`, otherwise
//! `((row - camera.row + priority + 1) * source_height + 1)` clamped to `i32`.
//! `bind_world_tiles` attaches placements to sprites; `clear_world_tile` and
//! `clear_map_camera` bake the current projection back into the sprite state.
//! Camera changes that affect depth (`row`, `bridge`, `source_height`)
//! invalidate the retained scene, but pixel `x`/`y` scrolling does not.
use super::Runtime;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MapCamera {
    pub x: i64,
    pub y: i64,
    pub row: i64,
    pub source_height: i64,
    pub bridge: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct WorldTile {
    pub group: u64,
    pub x: i64,
    pub y: i64,
    pub row: i64,
    pub priority: i64,
    pub reflection: bool,
    pub bridge: bool,
}
impl WorldTile {
    pub fn project(self, camera: MapCamera) -> (i64, i64, i32) {
        let z = if self.reflection {
            -100
        } else if (self.bridge && camera.bridge) || self.priority == 0 {
            0
        } else {
            (self
                .row
                .saturating_sub(camera.row)
                .saturating_add(self.priority)
                .saturating_add(1))
            .saturating_mul(camera.source_height)
            .saturating_add(1)
            .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
        };
        (
            self.x.saturating_sub(camera.x),
            self.y.saturating_sub(camera.y),
            z,
        )
    }
}
impl Runtime {
    pub fn set_map_camera(&mut self, group: u64, camera: MapCamera) {
        let previous = self.map_cameras.insert(group, camera);
        // Pixel scrolling changes geometry, not RGSS creation/z ordering.
        if previous.is_none_or(|old| {
            old.row != camera.row
                || old.bridge != camera.bridge
                || old.source_height != camera.source_height
        }) {
            self.invalidate_scene();
        }
    }
    pub fn bind_world_tiles(&mut self, placements: &[(u64, WorldTile)]) {
        for &(id, placement) in placements {
            if let Some(sprite) = self.sprites.get_mut(&id) {
                sprite.world = Some(placement);
            }
        }
        if !placements.is_empty() {
            self.invalidate_scene();
        }
    }
    pub fn clear_world_tile(&mut self, id: u64) {
        if let Some(sprite) = self.sprites.get_mut(&id)
            && let Some(world) = sprite.world.take()
            && let Some(&camera) = self.map_cameras.get(&world.group)
        {
            let (x, y, z) = world.project(camera);
            sprite.state.x = x as f64;
            sprite.state.y = y as f64;
            sprite.state.z = z;
            self.invalidate_scene();
        }
    }
    pub fn clear_map_camera(&mut self, group: u64) {
        for sprite in self.sprites.values_mut() {
            if let Some(world) = sprite.world.filter(|world| world.group == group) {
                if let Some(&camera) = self.map_cameras.get(&group) {
                    let (x, y, z) = world.project(camera);
                    sprite.state.x = x as f64;
                    sprite.state.y = y as f64;
                    sprite.state.z = z;
                }
                sprite.world = None;
            }
        }
        self.map_cameras.remove(&group);
        self.invalidate_scene();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn camera_moves_world_tiles_without_rewriting_them_and_preserves_priority_rules() {
        let tile = WorldTile {
            group: 1,
            x: 320,
            y: 640,
            row: 20,
            priority: 2,
            reflection: false,
            bridge: false,
        };
        let camera = MapCamera {
            x: 256,
            y: 512,
            row: 16,
            source_height: 32,
            bridge: false,
        };
        assert_eq!(tile.project(camera), (64, 128, 225));
        assert_eq!(
            tile.project(MapCamera {
                x: 260,
                y: 520,
                ..camera
            }),
            (60, 120, 225)
        );
        assert_eq!(
            tile.project(MapCamera { row: 17, ..camera }),
            (64, 128, 193)
        );
        assert_eq!(
            WorldTile {
                bridge: true,
                ..tile
            }
            .project(MapCamera {
                bridge: true,
                ..camera
            })
            .2,
            0
        );
        assert_eq!(
            WorldTile {
                reflection: true,
                bridge: true,
                ..tile
            }
            .project(MapCamera {
                bridge: true,
                ..camera
            })
            .2,
            -100
        );
    }
}
