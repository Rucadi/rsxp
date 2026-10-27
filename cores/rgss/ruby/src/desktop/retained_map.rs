// SPDX-License-Identifier: GPL-2.0-or-later
//! Native `FANPLAYRetainedMap` for SabiRuby.
//!
//! Port of the Magnus binding in `bindings/retained_map.rs`. The class is the
//! Ruby-visible handle to `fanplay_rgss::engine::retained_map::RetainedMap`,
//! the engine's retained Essentials visible-cell cache. `retained.rb` calls
//! `update` with one descriptor per connected map, `shift` when the game
//! rotates its reusable sprite pool, and reads `chunk_stats` for diagnostics.
//!
//! Each handle is a host-data object (tag [`TAG`]) whose handle is a key in the
//! module-local map below. The integrator gives this module the native `Table`
//! registry at install time, so `update` borrows the engine tables directly;
//! no table data is copied into Ruby values. Engine borrows and the retained
//! cache lock are held only while the engine computes changes.
use fanplay_rgss::{
    engine::retained_map::{MapView, RetainedMap},
    table::Table,
};
use sabiruby::{object::Aspec, Value, Vm, VmError};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};

use super::table::{Slot, Tables};

/// Host-data tag for a retained-map handle in `Vm::data_of`.
pub(super) const TAG: u32 = 5;

static MAPS: LazyLock<Mutex<HashMap<u64, RetainedMap>>> = LazyLock::new(Mutex::default);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Installs `FANPLAYRetainedMap`. `tables` is the same registry
/// `table::install` populated, so `update` can borrow each descriptor's Table.
pub(super) fn install(vm: &mut Vm, tables: Tables) -> Result<(), String> {
    let class = vm.define_class("FANPLAYRetainedMap", vm.core.object);
    let sc = vm
        .singleton_class(Value::Obj(class))
        .map_err(|e| vm.describe_error(&e))?;

    // `Class#new` dispatches `allocate`, as it does for the native Table.
    vm.define_closure_args(sc, "allocate", Aspec::none(), move |vm, _class, _args, _blk| {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let mut map = RetainedMap::default();
        map.set_chunks_enabled(std::env::var("FANPLAY_MAP_CHUNKS").as_deref() != Ok("0"));
        lock_maps().insert(id, map);
        Ok(vm.data_new(class, TAG, id))
    });

    vm.define_closure_args(class, "shift", Aspec::req(2), |vm, this, args, _blk| {
        let x = count(vm, args[0], "shift x")?;
        let y = count(vm, args[1], "shift y")?;
        let id = handle(vm, this)?;
        with_map(vm, id, |map| map.shift(x, y))?;
        Ok(Value::Nil)
    });

    vm.define_closure_args(class, "chunk_stats", Aspec::none(), |vm, this, _args, _blk| {
        let id = handle(vm, this)?;
        let stats = with_map(vm, id, |map| map.chunk_stats())?;
        Ok(vm.ary_new(vec![
            Value::Int(stats.builds as i64),
            Value::Int(stats.hits as i64),
            Value::Int(stats.checks as i64),
            Value::Int(stats.evictions as i64),
            Value::Int(stats.resident as i64),
        ]))
    });

    vm.define_closure_args(class, "update", Aspec::req(4), move |vm, this, args, _blk| {
        update(&tables, vm, this, args)
    });
    Ok(())
}

/// Drops a collected handle's cache. The VM's `on_free` hook calls this for
/// objects carrying [`TAG`]; it is also safe to call for an unknown id.
pub(super) fn forget(id: u64) {
    lock_maps().remove(&id);
}

struct Input {
    identity: u64,
    table: Value,
    x: i64,
    y: i64,
    width: usize,
    height: usize,
    data_identity: u64,
}

fn update(tables: &Tables, vm: &mut Vm, this: Value, args: &[Value]) -> Result<Value, VmError> {
    let width = count(vm, args[0], "width")?;
    let height = count(vm, args[1], "height")?;
    let force = args[3].truthy();
    let entries = vm
        .ary_vals(args[2])
        .ok_or_else(|| vm.raise_type("map descriptors must be an Array"))?;

    let mut inputs = Vec::with_capacity(entries.len());
    for (index, entry) in entries.into_iter().enumerate() {
        let fields = vm
            .ary_vals(entry)
            .ok_or_else(|| vm.raise_arg("map descriptor must be an Array"))?;
        if fields.len() != 7 {
            return Err(vm.raise_arg(&format!(
                "map descriptor {index} must have 7 elements"
            )));
        }
        inputs.push(Input {
            identity: integer_id(vm, fields[0], "map identity")?,
            table: fields[1],
            x: vm.expect_int(fields[2], "map x")?,
            y: vm.expect_int(fields[3], "map y")?,
            width: count(vm, fields[4], "map width")?,
            height: count(vm, fields[5], "map height")?,
            data_identity: integer_id(vm, fields[6], "map data identity")?,
        });
    }

    let id = handle(vm, this)?;
    // Hold the Table registry across RetainedMap::update so MapView can borrow
    // the engine tables. No Ruby calls or retained-cache borrows happen while
    // collecting these views.
    let changes = {
        let registry = tables.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut views = Vec::with_capacity(inputs.len());
        for input in &inputs {
            let (data, revision) = table_slot(&registry, vm, input.table)?;
            views.push(MapView {
                identity: input.identity,
                data_identity: input.data_identity,
                revision,
                data,
                x: input.x,
                y: input.y,
                width: input.width,
                height: input.height,
            });
        }
        match with_map(vm, id, |map| map.update(width, height, &views, force)) {
            Ok(Ok(changes)) => changes,
            Ok(Err(error)) => return Err(vm.raise_arg(error)),
            Err(error) => return Err(error),
        }
    };

    // `[x, y, layer, map_index_or_nil, tile]`; `nil` marks an uncovered cell.
    let mut result = Vec::with_capacity(changes.len());
    for change in changes {
        let (map_index, tile) = match change.cell {
            Some(cell) => (Value::Int(cell.map as i64), Value::Int(cell.tile as i64)),
            None => (Value::Nil, Value::Int(0)),
        };
        result.push(vm.ary_new(vec![
            Value::Int(change.x as i64),
            Value::Int(change.y as i64),
            Value::Int(change.layer as i64),
            map_index,
            tile,
        ]));
    }
    Ok(vm.ary_new(result))
}

/// Resolves a native `Table` handle to its engine table and revision.
///
/// Kept as the single integration seam for `table.rs`: it reads
/// `Slot::table`/`Slot::revision`, which therefore need to be visible to this
/// module (`pub(super)` fields or equivalent accessors). A `with_slot_for`
/// helper that lends both values works too; the borrow must last for all views
/// of one `update`, because `RetainedMap::update` needs every table at once.
fn table_slot<'a>(
    registry: &'a HashMap<u64, Slot>,
    vm: &mut Vm,
    value: Value,
) -> Result<(&'a Table, u64), VmError> {
    let (tag, id) = vm
        .data_of(value)
        .ok_or_else(|| vm.raise(vm.core.runtime_error, "uninitialized map Table"))?;
    if tag != super::table::TAG {
        return Err(vm.raise(vm.core.runtime_error, "uninitialized map Table"));
    }
    let slot = registry
        .get(&id)
        .ok_or_else(|| vm.raise(vm.core.runtime_error, "uninitialized map Table"))?;
    Ok((&slot.table, slot.revision))
}

fn with_map<T>(
    vm: &mut Vm,
    id: u64,
    f: impl FnOnce(&mut RetainedMap) -> T,
) -> Result<T, VmError> {
    let mut maps = lock_maps();
    let map = maps
        .get_mut(&id)
        .ok_or_else(|| vm.raise_arg("invalid FANPLAYRetainedMap handle"))?;
    Ok(f(map))
}

fn lock_maps() -> std::sync::MutexGuard<'static, HashMap<u64, RetainedMap>> {
    MAPS.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn handle(vm: &mut Vm, this: Value) -> Result<u64, VmError> {
    match vm.data_of(this) {
        Some((TAG, id)) => Ok(id),
        _ => Err(vm.raise_arg("invalid FANPLAYRetainedMap handle")),
    }
}

fn count(vm: &mut Vm, value: Value, what: &str) -> Result<usize, VmError> {
    let n = vm.expect_int(value, what)?;
    usize::try_from(n).map_err(|_| vm.raise_arg(&format!("negative {what}")))
}

fn integer_id(vm: &mut Vm, value: Value, what: &str) -> Result<u64, VmError> {
    let n = vm.expect_int(value, what)?;
    u64::try_from(n).map_err(|_| vm.raise_arg(&format!("negative {what}")))
}
