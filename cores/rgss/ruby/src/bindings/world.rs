// SPDX-License-Identifier: GPL-2.0-or-later
//! Lazy Ruby screen coordinates backed by world placements and one camera.
//! Persistent storage contains IDs and numbers only, never Ruby objects.
//!
//! `WORLD` is a process-wide mutex of `groups` (per-group `MapCamera`, member
//! sprite ids, and a `valid` flag) and `tiles` (sprite id → engine `WorldTile`).
//! `retained.rb` drives it through four globals: `__fanplay_world_camera` (12 args —
//! group id, tile pool, changed cells, tile offsets, pixel offsets, display and
//! source heights, bridge flag, requested reset) recomputes placements for the
//! group as a whole or just the changed cells and pushes the camera and
//! placements into the engine; `__fanplay_world_release` detaches a whole group;
//! `__fanplay_world_metadata_changed` and `__fanplay_world_unbind` detach one sprite.
//!
//! While a sprite is world-bound, its `x`/`y`/`z` getters (`install_getters`,
//! installed for `Sprite` by `retained`) project the placement through the group
//! camera instead of reading the raw ivars. `detach` bakes the projected
//! coordinates back into `@x`/`@y`/`@z` and clears the engine binding; an
//! `internal` detach (map edits) keeps the group `valid`, while a metadata change
//! invalidates it so the next camera update rebuilds placements. `bound`/`forget`
//! expose this state to `sprite.rs` and `retained.rs`. State is read fresh per
//! call; `MapCamera`/`WorldTile` come from `fanplay_rgss::engine::world_map`.
use magnus::{
    function, method,
    prelude::*,
    value::{Fixnum, LazyId},
    Error, RArray, RClass, RObject, Ruby, TryConvert, Value,
};
use fanplay_rgss::engine::{
    self,
    world_map::{MapCamera, WorldTile},
};
use std::{
    collections::{HashMap, HashSet},
    sync::{LazyLock, Mutex},
};
#[derive(Default)]
struct Group {
    camera: MapCamera,
    members: HashSet<u64>,
    valid: bool,
}
#[derive(Default)]
struct World {
    groups: HashMap<u64, Group>,
    tiles: HashMap<u64, WorldTile>,
}
static WORLD: LazyLock<Mutex<World>> = LazyLock::new(Mutex::default);
fn with_world<T>(f: impl FnOnce(&mut World) -> T) -> T {
    f(&mut WORLD.lock().unwrap_or_else(|e| e.into_inner()))
}
macro_rules! get {
    ($object:expr,$name:literal,$ty:ty) => {{
        static ID: LazyId = LazyId::new($name);
        $object.ivar_get::<_, $ty>(*ID)?
    }};
}
macro_rules! set {
    ($object:expr,$name:literal,$value:expr) => {{
        static ID: LazyId = LazyId::new($name);
        $object.ivar_set(*ID, $value)?;
    }};
}
pub(super) fn bound(id: u64) -> bool {
    with_world(|world| world.tiles.contains_key(&id))
}
pub(super) fn forget(id: u64) {
    let empty_group = with_world(|world| {
        let tile = world.tiles.remove(&id)?;
        let group = world.groups.get_mut(&tile.group)?;
        group.members.remove(&id);
        if group.members.is_empty() {
            world.groups.remove(&tile.group);
            Some(tile.group)
        } else {
            None
        }
    });
    if let Some(group) = empty_group {
        engine::with_runtime_mut(|runtime| runtime.clear_map_camera(group));
    }
}

fn position(id: u64) -> Option<(i64, i64, i32)> {
    with_world(|world| {
        world.tiles.get(&id).and_then(|tile| {
            world
                .groups
                .get(&tile.group)
                .map(|group| tile.project(group.camera))
        })
    })
}
pub(super) fn detach(object: RObject, internal: bool) -> Result<(), Error> {
    if get!(object, "@__fanplay_world_group", Option<u64>).is_none() {
        return Ok(());
    }
    let id = get!(object, "@__id", u64);
    let editing = internal || get!(object, "@__fanplay_map_edit", Value).to_bool();
    let projected = with_world(|world| {
        let tile = world.tiles.remove(&id)?;
        let group = world.groups.get_mut(&tile.group)?;
        group.members.remove(&id);
        if !editing {
            group.valid = false;
        }
        Some(tile.project(group.camera))
    });
    if let Some((x, y, z)) = projected {
        set!(object, "@x", x);
        set!(object, "@y", y);
        set!(object, "@z", z);
    }
    set!(object, "@__fanplay_world_group", ());
    engine::with_runtime_mut(|runtime| runtime.clear_world_tile(id));
    Ok(())
}
fn coordinate(ruby: &Ruby, object: RObject, axis: u8) -> Result<Value, Error> {
    if get!(object, "@__fanplay_world_group", Option<u64>).is_some() {
        let id = get!(object, "@__id", u64);
        if let Some((x, y, z)) = position(id) {
            return Ok(ruby
                .integer_from_i64(match axis {
                    0 => x,
                    1 => y,
                    _ => i64::from(z),
                })
                .as_value());
        }
    }
    Ok(match axis {
        0 => get!(object, "@x", Value),
        1 => get!(object, "@y", Value),
        _ => get!(object, "@z", Value),
    })
}
fn x(ruby: &Ruby, object: RObject) -> Result<Value, Error> {
    coordinate(ruby, object, 0)
}
fn y(ruby: &Ruby, object: RObject) -> Result<Value, Error> {
    coordinate(ruby, object, 1)
}
fn z(ruby: &Ruby, object: RObject) -> Result<Value, Error> {
    coordinate(ruby, object, 2)
}
pub(super) fn install_getters(class: RClass) -> Result<(), Error> {
    class.define_method("x", method!(x, 0))?;
    class.define_method("y", method!(y, 0))?;
    class.define_method("z", method!(z, 0))?;
    Ok(())
}
fn at(tiles: RArray, i: usize, j: usize, k: usize) -> Result<RObject, Error> {
    let column: RArray = tiles.entry(i as isize)?;
    let stack: RArray = column.entry(j as isize)?;
    stack.entry(k as isize)
}
fn indices(tiles: RArray) -> Result<Vec<(usize, usize, usize)>, Error> {
    let mut result = Vec::new();
    for i in 0..tiles.len() {
        let column: RArray = tiles.entry(i as isize)?;
        for j in 0..column.len() {
            let stack: RArray = column.entry(j as isize)?;
            for k in 0..stack.len() {
                result.push((i, j, k));
            }
        }
    }
    Ok(result)
}
fn update(ruby: &Ruby, args: &[Value]) -> Result<bool, Error> {
    if args.len() != 12 {
        return Err(Error::new(
            ruby.exception_arg_error(),
            "invalid world camera arguments",
        ));
    }
    let id = u64::try_convert(args[0])?;
    let tiles = RArray::try_convert(args[1])?;
    let changes = RArray::try_convert(args[2])?;
    let tx = i64::try_convert(args[3])?;
    let ty = i64::try_convert(args[4])?;
    let ox = i64::try_convert(args[5])?;
    let oy = i64::try_convert(args[6])?;
    let width = i64::try_convert(args[7])?;
    let height = i64::try_convert(args[8])?;
    let source_height = i64::try_convert(args[9])?;
    let bridge = args[10].to_bool();
    let requested_reset = args[11].to_bool();
    let (exists, valid) = with_world(|world| {
        world
            .groups
            .get(&id)
            .map_or((false, true), |group| (true, group.valid))
    });
    if exists && !valid && !requested_reset {
        return Ok(false);
    }
    let reset = requested_reset || !exists;
    let camera = MapCamera {
        x: tx
            .checked_mul(width)
            .and_then(|n| n.checked_add(ox))
            .ok_or_else(|| Error::new(ruby.exception_range_error(), "world camera overflow"))?,
        y: ty
            .checked_mul(height)
            .and_then(|n| n.checked_add(oy))
            .ok_or_else(|| Error::new(ruby.exception_range_error(), "world camera overflow"))?,
        row: ty,
        source_height,
        bridge,
    };
    let cells = if reset {
        indices(tiles)?
    } else {
        let mut cells = Vec::with_capacity(changes.len());
        for index in 0..changes.len() {
            let change: RArray = changes.entry(index as isize)?;
            cells.push((change.entry(0)?, change.entry(1)?, change.entry(2)?));
        }
        cells
    };
    let mut placements = Vec::with_capacity(cells.len());
    let mut removed = Vec::new();
    // Preflight before changing group state. No persistent Ruby references.
    for &(i, j, k) in &cells {
        let tile = at(tiles, i, j, k)?;
        if !get!(tile, "@__fanplay_retained", Value).to_bool() || tile.is_frozen() {
            return Ok(false);
        }
        let tile_id = get!(tile, "@__id", u64);
        if !get!(tile, "@__fanplay_map_covered", Value).to_bool() {
            removed.push(tile_id);
            continue;
        }
        let Some(priority) = Fixnum::from_value(get!(tile, "@priority", Value)) else {
            return Ok(false);
        };
        let row = ty
            .checked_add(j as i64)
            .ok_or_else(|| Error::new(ruby.exception_range_error(), "world tile overflow"))?;
        let x = tx
            .checked_add(i as i64)
            .and_then(|n| n.checked_mul(width))
            .ok_or_else(|| Error::new(ruby.exception_range_error(), "world tile overflow"))?;
        let y = row
            .checked_mul(height)
            .ok_or_else(|| Error::new(ruby.exception_range_error(), "world tile overflow"))?;
        placements.push((
            tile_id,
            WorldTile {
                group: id,
                x,
                y,
                row,
                priority: priority.to_i64(),
                reflection: get!(tile, "@shows_reflection", Value).to_bool(),
                bridge: get!(tile, "@bridge", Value).to_bool(),
            },
        ));
    }
    for &(i, j, k) in &cells {
        let tile = at(tiles, i, j, k)?;
        if get!(tile, "@__fanplay_map_covered", Value).to_bool() {
            set!(tile, "@__fanplay_world_group", id);
        } else {
            detach(tile, true)?;
        }
    }
    with_world(|world| {
        if reset && let Some(old) = world.groups.remove(&id) {
            for member in old.members {
                world.tiles.remove(&member);
            }
        }
        for removed in &removed {
            if let Some(tile) = world.tiles.remove(removed)
                && let Some(group) = world.groups.get_mut(&tile.group)
            {
                group.members.remove(removed);
            }
        }
        let group = world.groups.entry(id).or_insert_with(|| Group {
            valid: true,
            ..Default::default()
        });
        group.camera = camera;
        for &(tile, _) in &placements {
            group.members.insert(tile);
        }
        for &(tile, placement) in &placements {
            world.tiles.insert(tile, placement);
        }
    });
    engine::with_runtime_mut(|runtime| {
        if reset {
            runtime.clear_map_camera(id);
        }
        for &tile in &removed {
            runtime.clear_world_tile(tile);
        }
        runtime.set_map_camera(id, camera);
        runtime.bind_world_tiles(&placements);
    });
    Ok(true)
}
fn release(group: u64, tiles: RArray) -> Result<(), Error> {
    for (i, j, k) in indices(tiles)? {
        detach(at(tiles, i, j, k)?, true)?;
    }
    with_world(|world| {
        if let Some(group) = world.groups.remove(&group) {
            for member in group.members {
                world.tiles.remove(&member);
            }
        }
    });
    engine::with_runtime_mut(|runtime| runtime.clear_map_camera(group));
    Ok(())
}
fn metadata_changed(object: RObject) -> Result<(), Error> {
    detach(object, false)
}
fn unbind(object: RObject) -> Result<(), Error> {
    detach(object, true)
}
pub(super) fn bind(ruby: &Ruby) -> Result<(), Error> {
    with_world(|world| *world = World::default());
    ruby.define_global_function("__fanplay_world_camera", function!(update, -1))?;
    ruby.define_global_function("__fanplay_world_release", function!(release, 2))?;
    ruby.define_global_function(
        "__fanplay_world_metadata_changed",
        function!(metadata_changed, 1),
    )?;
    ruby.define_global_function("__fanplay_world_unbind", function!(unbind, 1))?;
    Ok(())
}
