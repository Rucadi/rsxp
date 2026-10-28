// SPDX-License-Identifier: GPL-2.0-or-later
//! Native Ruby conversions are shared; only delivery to the engine differs.
use fanplay_rgss::engine::world_map::{MapCamera, WorldTile};
use fanplay_rgss::engine::{
    native_updates::NativeUpdate, NativeTileUpdate, SpriteMotion, SpriteState,
};

#[cfg(target_os = "wasi")]
thread_local! {
    static UPDATES: std::cell::RefCell<Vec<NativeUpdate>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn submit(update: NativeUpdate) -> Result<(), String> {
    #[cfg(target_os = "wasi")]
    {
        UPDATES.with(|queue| queue.borrow_mut().push(update));
        Ok(())
    }
    #[cfg(not(target_os = "wasi"))]
    {
        fanplay_rgss::engine::with_runtime_mut(|runtime| update.apply(runtime))
            .unwrap_or_else(|| Err("runtime is closed".into()))
    }
}

pub(super) fn sync_sprite(id: u64, state: SpriteState, world_bound: bool) -> Result<(), String> {
    submit(NativeUpdate::Sprite {
        id,
        state,
        world_bound,
    })
}
pub(super) fn move_sprites(motions: Vec<SpriteMotion>) {
    let _ = submit(NativeUpdate::Motion(motions));
}
pub(super) fn update_native_tiles(updates: Vec<NativeTileUpdate>, hidden: Vec<u64>) {
    let _ = submit(NativeUpdate::Tiles { updates, hidden });
}
pub(super) fn clear_map_camera(id: u64) {
    let _ = submit(NativeUpdate::ClearCamera(id));
}
pub(super) fn clear_world_tile(id: u64) {
    let _ = submit(NativeUpdate::ClearTile(id));
}
pub(super) fn update_world(
    id: u64,
    camera: MapCamera,
    placements: Vec<(u64, WorldTile)>,
    removed: Vec<u64>,
    reset: bool,
) {
    let _ = submit(NativeUpdate::World {
        id,
        camera,
        placements,
        removed,
        reset,
    });
}

#[cfg(target_os = "wasi")]
pub(super) fn bind(ruby: &magnus::Ruby) -> Result<(), magnus::Error> {
    fn take(ruby: &magnus::Ruby) -> Result<Option<magnus::RString>, magnus::Error> {
        let updates = UPDATES.with(|queue| std::mem::take(&mut *queue.borrow_mut()));
        if updates.is_empty() {
            return Ok(None);
        }
        let bytes = fanplay_rgss::engine::native_updates::encode(&updates).map_err(|error| {
            magnus::Error::new(ruby.exception_runtime_error(), error.to_string())
        })?;
        Ok(Some(ruby.str_from_slice(&bytes)))
    }
    ruby.define_global_function("__fanplay_take_render_commands", magnus::function!(take, 0))
}
