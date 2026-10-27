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
//! expose this state to the sprite and retained ports. State is read fresh per
//! call; `MapCamera`/`WorldTile` come from `fanplay_rgss::engine::world_map`.
use fanplay_rgss::engine::{
    self,
    world_map::{MapCamera, WorldTile},
};
use sabiruby::{object::Aspec, value::ObjId, Value, Vm, VmError};
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

/// Whether `id` currently has a placement in the world registry.
pub(super) fn bound(id: u64) -> bool {
    with_world(|world| world.tiles.contains_key(&id))
}

/// Drops `id` from the world registry, clearing the engine camera of a group
/// that this leaves empty.
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
        let _ = engine::with_runtime_mut(|runtime| runtime.clear_map_camera(group));
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

/// Unbinds one sprite, baking the projected screen coordinates into `@x`/`@y`/`@z`.
///
/// An `internal` detach (a map edit) leaves the group valid; a metadata change
/// (`internal == false`) invalidates it so the next camera update rebuilds the
/// placements from the tile pool.
pub(super) fn detach(vm: &mut Vm, object: Value, internal: bool) -> Result<(), VmError> {
    if world_group(vm, object).is_none() {
        return Ok(());
    }
    let id = ivar_u64(vm, object, "@__id")?;
    let editing = internal || raw_ivar(vm, object, "@__fanplay_map_edit").truthy();
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
        if let Some(obj) = object.obj() {
            vm.ivar_set(obj, "@x", Value::Int(x));
            vm.ivar_set(obj, "@y", Value::Int(y));
            vm.ivar_set(obj, "@z", Value::Int(i64::from(z)));
        }
    }
    if let Some(obj) = object.obj() {
        vm.ivar_set(obj, "@__fanplay_world_group", Value::Nil);
    }
    let _ = engine::with_runtime_mut(|runtime| runtime.clear_world_tile(id));
    Ok(())
}

fn coordinate(vm: &mut Vm, object: Value, axis: u8) -> Result<Value, VmError> {
    if world_group(vm, object).is_some() {
        let id = ivar_u64(vm, object, "@__id")?;
        if let Some((x, y, z)) = position(id) {
            return Ok(Value::Int(match axis {
                0 => x,
                1 => y,
                _ => i64::from(z),
            }));
        }
    }
    Ok(raw_ivar(
        vm,
        object,
        match axis {
            0 => "@x",
            1 => "@y",
            _ => "@z",
        },
    ))
}

/// Installs the world-aware `x`/`y`/`z` getters on `class`.
pub(super) fn install_getters(vm: &mut Vm, class: ObjId) -> Result<(), VmError> {
    vm.define_closure_args(class, "x", Aspec::none(), |vm, this, _args, _blk| {
        coordinate(vm, this, 0)
    });
    vm.define_closure_args(class, "y", Aspec::none(), |vm, this, _args, _blk| {
        coordinate(vm, this, 1)
    });
    vm.define_closure_args(class, "z", Aspec::none(), |vm, this, _args, _blk| {
        coordinate(vm, this, 2)
    });
    Ok(())
}

fn array_vals(vm: &mut Vm, value: Value, what: &str) -> Result<Vec<Value>, VmError> {
    match vm.ary_vals(value) {
        Some(values) => Ok(values),
        None => Err(vm.raise_type(&format!("{what} cannot be converted to Array"))),
    }
}

fn array_entry(vm: &mut Vm, array: Value, index: usize, what: &str) -> Result<Value, VmError> {
    let values = array_vals(vm, array, what)?;
    values
        .get(index)
        .copied()
        .ok_or_else(|| vm.raise(vm.core.index_error, &format!("index {index} out of array")))
}

fn at(vm: &mut Vm, tiles: Value, i: usize, j: usize, k: usize) -> Result<Value, VmError> {
    let column = array_entry(vm, tiles, i, "world tile column")?;
    let stack = array_entry(vm, column, j, "world tile stack")?;
    array_entry(vm, stack, k, "world tile")
}

fn indices(vm: &mut Vm, tiles: Value) -> Result<Vec<(usize, usize, usize)>, VmError> {
    let columns = array_vals(vm, tiles, "world tiles")?;
    let mut result = Vec::new();
    for (i, column) in columns.into_iter().enumerate() {
        let stacks = array_vals(vm, column, "world tile column")?;
        for (j, stack) in stacks.into_iter().enumerate() {
            let tiles = array_vals(vm, stack, "world tile stack")?;
            for k in 0..tiles.len() {
                result.push((i, j, k));
            }
        }
    }
    Ok(result)
}

fn index(vm: &mut Vm, value: Value, what: &str) -> Result<usize, VmError> {
    let n = vm.expect_int(value, what)?;
    usize::try_from(n).map_err(|_| vm.raise_arg(&format!("negative {what}")))
}

fn overflow(vm: &mut Vm, what: &str) -> VmError {
    vm.raise(vm.core.range_error, what)
}

fn update(vm: &mut Vm, args: &[Value]) -> Result<Value, VmError> {
    if args.len() != 12 {
        return Err(vm.raise_arg("invalid world camera arguments"));
    }
    let id = expect_u64(vm, args[0], "world group")?;
    let tiles = args[1];
    let changes = args[2];
    // Both the tile pool and the change list are converted before anything else,
    // as the Magnus `RArray::try_convert` calls did.
    array_vals(vm, tiles, "world tiles")?;
    array_vals(vm, changes, "world changes")?;
    let tx = vm.expect_int(args[3], "world tile x")?;
    let ty = vm.expect_int(args[4], "world tile y")?;
    let ox = vm.expect_int(args[5], "world offset x")?;
    let oy = vm.expect_int(args[6], "world offset y")?;
    let width = vm.expect_int(args[7], "world width")?;
    let height = vm.expect_int(args[8], "world height")?;
    let source_height = vm.expect_int(args[9], "world source height")?;
    let bridge = args[10].truthy();
    let requested_reset = args[11].truthy();
    let (exists, valid) = with_world(|world| {
        world
            .groups
            .get(&id)
            .map_or((false, true), |group| (true, group.valid))
    });
    if exists && !valid && !requested_reset {
        return Ok(Value::False);
    }
    let reset = requested_reset || !exists;
    let camera = MapCamera {
        x: tx
            .checked_mul(width)
            .and_then(|n| n.checked_add(ox))
            .ok_or_else(|| overflow(vm, "world camera overflow"))?,
        y: ty
            .checked_mul(height)
            .and_then(|n| n.checked_add(oy))
            .ok_or_else(|| overflow(vm, "world camera overflow"))?,
        row: ty,
        source_height,
        bridge,
    };
    let cells = if reset {
        indices(vm, tiles)?
    } else {
        let changes = array_vals(vm, changes, "world changes")?;
        let mut cells = Vec::with_capacity(changes.len());
        for change in changes {
            let i = array_entry(vm, change, 0, "world change")?;
            let j = array_entry(vm, change, 1, "world change")?;
            let k = array_entry(vm, change, 2, "world change")?;
            cells.push((
                index(vm, i, "world cell")?,
                index(vm, j, "world cell")?,
                index(vm, k, "world cell")?,
            ));
        }
        cells
    };
    let mut placements = Vec::with_capacity(cells.len());
    let mut removed = Vec::new();
    // Preflight before changing group state. No persistent Ruby references.
    for &(i, j, k) in &cells {
        let tile = at(vm, tiles, i, j, k)?;
        if !raw_ivar(vm, tile, "@__fanplay_retained").truthy() || frozen(vm, tile) {
            return Ok(Value::False);
        }
        let tile_id = ivar_u64(vm, tile, "@__id")?;
        if !raw_ivar(vm, tile, "@__fanplay_map_covered").truthy() {
            removed.push(tile_id);
            continue;
        }
        let priority = match raw_ivar(vm, tile, "@priority") {
            Value::Int(n) => n,
            _ => return Ok(Value::False),
        };
        let row = ty
            .checked_add(j as i64)
            .ok_or_else(|| overflow(vm, "world tile overflow"))?;
        let x = tx
            .checked_add(i as i64)
            .and_then(|n| n.checked_mul(width))
            .ok_or_else(|| overflow(vm, "world tile overflow"))?;
        let y = row
            .checked_mul(height)
            .ok_or_else(|| overflow(vm, "world tile overflow"))?;
        placements.push((
            tile_id,
            WorldTile {
                group: id,
                x,
                y,
                row,
                priority,
                reflection: raw_ivar(vm, tile, "@shows_reflection").truthy(),
                bridge: raw_ivar(vm, tile, "@bridge").truthy(),
            },
        ));
    }
    for &(i, j, k) in &cells {
        let tile = at(vm, tiles, i, j, k)?;
        if raw_ivar(vm, tile, "@__fanplay_map_covered").truthy() {
            if let Some(obj) = tile.obj() {
                vm.ivar_set(obj, "@__fanplay_world_group", Value::Int(id as i64));
            }
        } else {
            detach(vm, tile, true)?;
        }
    }
    with_world(|world| {
        if reset {
            if let Some(old) = world.groups.remove(&id) {
                for member in old.members {
                    world.tiles.remove(&member);
                }
            }
        }
        for removed in &removed {
            if let Some(tile) = world.tiles.remove(removed) {
                if let Some(group) = world.groups.get_mut(&tile.group) {
                    group.members.remove(removed);
                }
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
    let _ = engine::with_runtime_mut(|runtime| {
        if reset {
            runtime.clear_map_camera(id);
        }
        for &tile in &removed {
            runtime.clear_world_tile(tile);
        }
        runtime.set_map_camera(id, camera);
        runtime.bind_world_tiles(&placements);
    });
    Ok(Value::True)
}

fn release(vm: &mut Vm, group: u64, tiles: Value) -> Result<(), VmError> {
    for (i, j, k) in indices(vm, tiles)? {
        let tile = at(vm, tiles, i, j, k)?;
        detach(vm, tile, true)?;
    }
    with_world(|world| {
        if let Some(group) = world.groups.remove(&group) {
            for member in group.members {
                world.tiles.remove(&member);
            }
        }
    });
    let _ = engine::with_runtime_mut(|runtime| runtime.clear_map_camera(group));
    Ok(())
}

fn metadata_changed(vm: &mut Vm, object: Value) -> Result<(), VmError> {
    detach(vm, object, false)
}

fn unbind(vm: &mut Vm, object: Value) -> Result<(), VmError> {
    detach(vm, object, true)
}

/// Installs the four `__fanplay_world_*` globals and resets the registry.
pub(super) fn install(vm: &mut Vm) {
    with_world(|world| *world = World::default());
    let object = vm.core.object;
    vm.define_closure_args(
        object,
        "__fanplay_world_camera",
        Aspec::any(),
        |vm, _this, args, _blk| update(vm, args),
    );
    vm.define_closure_args(
        object,
        "__fanplay_world_release",
        Aspec::req(2),
        |vm, _this, args, _blk| {
            let group = expect_u64(vm, args[0], "world group")?;
            release(vm, group, args[1])?;
            Ok(Value::Nil)
        },
    );
    vm.define_closure_args(
        object,
        "__fanplay_world_metadata_changed",
        Aspec::req(1),
        |vm, _this, args, _blk| {
            metadata_changed(vm, args[0])?;
            Ok(Value::Nil)
        },
    );
    vm.define_closure_args(
        object,
        "__fanplay_world_unbind",
        Aspec::req(1),
        |vm, _this, args, _blk| {
            unbind(vm, args[0])?;
            Ok(Value::Nil)
        },
    );
}

fn raw_ivar(vm: &Vm, object: Value, name: &str) -> Value {
    match object.obj() {
        Some(obj) => vm.ivar_get(obj, name),
        None => Value::Nil,
    }
}

fn world_group(vm: &Vm, object: Value) -> Option<u64> {
    match raw_ivar(vm, object, "@__fanplay_world_group") {
        Value::Int(n) if n >= 0 => Some(n as u64),
        _ => None,
    }
}

fn ivar_u64(vm: &mut Vm, object: Value, name: &str) -> Result<u64, VmError> {
    let obj = object
        .obj()
        .ok_or_else(|| vm.raise_type("world state needs an object"))?;
    match vm.ivar_get(obj, name) {
        Value::Int(n) => {
            u64::try_from(n).map_err(|_| vm.raise_arg(&format!("negative {name}")))
        }
        _ => Err(vm.raise_type(&format!("{name} is not an integer"))),
    }
}

fn expect_u64(vm: &mut Vm, value: Value, what: &str) -> Result<u64, VmError> {
    let n = vm.expect_int(value, what)?;
    u64::try_from(n).map_err(|_| vm.raise_arg(&format!("negative {what}")))
}

fn frozen(vm: &Vm, value: Value) -> bool {
    value
        .obj()
        .map(|obj| vm.heap.get(obj).frozen)
        .unwrap_or(true)
}
