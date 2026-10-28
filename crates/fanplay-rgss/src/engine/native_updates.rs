// SPDX-License-Identifier: GPL-2.0-or-later
//! Converted native binding updates, transported between the two web VMs.
use super::world_map::{MapCamera, WorldTile};
use super::{NativeTileUpdate, Runtime, SpriteMotion, SpriteState};

pub fn encode(updates: &[NativeUpdate]) -> Result<Vec<u8>, String> {
    bincode::serde::encode_to_vec(updates, bincode::config::standard())
        .map_err(|error| error.to_string())
}

pub fn decode(bytes: &[u8]) -> Result<Vec<NativeUpdate>, String> {
    let (updates, consumed) = bincode::serde::decode_from_slice(
        bytes,
        bincode::config::standard().with_limit::<67108864>(),
    )
    .map_err(|error| error.to_string())?;
    if consumed != bytes.len() {
        return Err("trailing native update bytes".into());
    }
    Ok(updates)
}

#[derive(serde::Serialize, serde::Deserialize)]
pub enum NativeUpdate {
    Sprite {
        id: u64,
        state: SpriteState,
        world_bound: bool,
    },
    Motion(Vec<SpriteMotion>),
    Tiles {
        updates: Vec<NativeTileUpdate>,
        hidden: Vec<u64>,
    },
    ClearCamera(u64),
    ClearTile(u64),
    World {
        id: u64,
        camera: MapCamera,
        placements: Vec<(u64, WorldTile)>,
        removed: Vec<u64>,
        reset: bool,
    },
}

impl NativeUpdate {
    pub fn apply(self, runtime: &mut Runtime) -> Result<(), String> {
        match self {
            Self::Sprite {
                id,
                state,
                world_bound,
            } => {
                if !world_bound {
                    runtime.clear_world_tile(id);
                }
                runtime.sync_sprite(id, state)?;
            }
            Self::Motion(motions) => runtime.move_sprites(&motions),
            Self::Tiles { updates, hidden } => runtime.update_native_tiles(&updates, &hidden),
            Self::ClearCamera(id) => runtime.clear_map_camera(id),
            Self::ClearTile(id) => runtime.clear_world_tile(id),
            Self::World {
                id,
                camera,
                placements,
                removed,
                reset,
            } => {
                if reset {
                    runtime.clear_map_camera(id);
                }
                for tile in removed {
                    runtime.clear_world_tile(tile);
                }
                runtime.set_map_camera(id, camera);
                runtime.bind_world_tiles(&placements);
            }
        }
        Ok(())
    }
}
