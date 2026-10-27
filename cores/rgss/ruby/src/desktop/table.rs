// SPDX-License-Identifier: GPL-2.0-or-later
//! Native RGSS `Table` for SabiRuby.
//!
//! The portable `rgss/table.rb` keeps a Ruby Array and is far slower for the
//! per-cell access the map renderer, compiler and tile caches do. This module
//! installs the same `__initialize`/`__get`/`__set`/`__resize`/`__dimensions`
//! surface the old Magnus binding had, backed by `fanplay_rgss::table::Table`.
//! `table.rb` detects `__initialize` and leaves its Array fallback unused.
use fanplay_rgss::table::Table;
use sabiruby::{Vm, Value, VmError};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Host-data tag for a table handle in `Vm::data_of`.
pub(super) const TAG: u32 = 4;

pub(super) struct Slot {
    pub(super) table: Table,
    pub(super) revision: u64,
}

pub(super) type Tables = Arc<Mutex<HashMap<u64, Slot>>>;

pub(super) fn install(vm: &mut Vm, tables: Tables) -> Result<(), String> {
    let class = vm.define_class("Table", vm.core.object);
    let serial = Arc::new(AtomicU64::new(1));

    // `Class#new` dispatches `allocate`, so returning a host-data object here
    // gives every Table a handle without a separate Ruby-side constructor.
    let alloc_tables = tables.clone();
    let alloc_serial = serial.clone();
    let sc = vm.singleton_class(Value::Obj(class)).map_err(|e| vm.describe_error(&e))?;
    vm.define_closure(sc, "allocate", move |vm, _class, _args, _blk| {
        let id = alloc_serial.fetch_add(1, Ordering::Relaxed);
        alloc_tables.lock().unwrap().insert(id, Slot {
            table: Table::new(0, 1, 1).map_err(|e| vm.raise_arg(&e.to_string()))?,
            revision: 0,
        });
        Ok(vm.data_new(class, TAG, id))
    });

    let initialize = tables.clone();
    vm.define_closure(class, "__initialize", move |vm, this, args, _blk| {
        let (x, y, z) = table_dims(vm, args)?;
        check_size(vm, x, y, z)?;
        let table = Table::new(x, y, z).map_err(|e| vm.raise_arg(&e.to_string()))?;
        with_slot(&initialize, vm, this, |slot| {
            slot.table = table;
            slot.revision = slot.revision.wrapping_add(1);
            Ok(())
        })?;
        Ok(Value::Nil)
    });

    let get = tables.clone();
    vm.define_closure(class, "__get", move |vm, this, args, _blk| {
        let (x, y, z) = coord_args(vm, args)?;
        let value = with_slot(&get, vm, this, |slot| Ok(slot.table.get(x, y, z)))?;
        Ok(value.map(|n| Value::Int(n as i64)).unwrap_or(Value::Nil))
    });

    let set = tables.clone();
    vm.define_closure(class, "__set", move |vm, this, args, _blk| {
        if args.len() != 4 { return Err(vm.raise_arg("wrong number of arguments")); }
        let (x, y, z) = coord_args(vm, &args[..3])?;
        let value = vm.expect_int(args[3], "value")? as i16;
        with_slot(&set, vm, this, |slot| {
            if slot.table.get(x, y, z).is_some_and(|old| old != value) {
                slot.table.set(x, y, z, value);
                slot.revision = slot.revision.wrapping_add(1);
            }
            Ok(())
        })?;
        Ok(Value::Int(value as i64))
    });

    let resize = tables.clone();
    vm.define_closure(class, "__resize", move |vm, this, args, _blk| {
        let (x, y, z) = table_dims(vm, args)?;
        check_size(vm, x, y, z)?;
        with_slot(&resize, vm, this, |slot| {
            slot.table.resize(x, y, z).map_err(|e| e.to_string())?;
            slot.revision = slot.revision.wrapping_add(1);
            Ok(())
        })?;
        Ok(Value::Nil)
    });

    let dimensions = tables.clone();
    vm.define_closure(class, "__dimensions", move |vm, this, _args, _blk| {
        let [x, y, z] = with_slot(&dimensions, vm, this, |slot| Ok(slot.table.dimensions()))?;
        Ok(vm.ary_new(vec![Value::Int(x as i64), Value::Int(y as i64), Value::Int(z as i64)]))
    });

    let revision = tables.clone();
    vm.define_closure(class, "__fanplay_revision", move |vm, this, _args, _blk| {
        let n = with_slot(&revision, vm, this, |slot| Ok(slot.revision))?;
        Ok(Value::Int(n as i64))
    });

    let dump = tables.clone();
    vm.define_closure(class, "_dump", move |vm, this, _args, _blk| {
        let bytes = with_slot(&dump, vm, this, |slot| Ok(slot.table.serialize()))?;
        Ok(vm.str_new(&bytes))
    });

    let load = tables.clone();
    let load_serial = serial.clone();
    let sc = vm.singleton_class(Value::Obj(class)).map_err(|e| vm.describe_error(&e))?;
    vm.define_closure(sc, "_load", move |vm, _class, args, _blk| {
        if args.len() != 1 { return Err(vm.raise_arg("wrong number of arguments")); }
        let bytes = vm.expect_str(args[0], "data")?;
        let table = Table::deserialize(&bytes).map_err(|e| vm.raise_arg(&e.to_string()))?;
        let id = load_serial.fetch_add(1, Ordering::Relaxed);
        load.lock().unwrap().insert(id, Slot { table, revision: 0 });
        Ok(vm.data_new(class, TAG, id))
    });

    let copy = tables.clone();
    vm.define_closure(class, "initialize_copy", move |vm, this, args, _blk| {
        if args.len() != 1 { return Err(vm.raise_arg("wrong number of arguments")); }
        let table = with_slot(&copy, vm, args[0], |slot| Ok(slot.table.clone()))?;
        with_slot(&copy, vm, this, |slot| {
            slot.table = table;
            slot.revision = slot.revision.wrapping_add(1);
            Ok(())
        })?;
        Ok(this)
    });

    Ok(())
}

fn with_slot<T>(
    tables: &Tables,
    vm: &mut Vm,
    value: Value,
    f: impl FnOnce(&mut Slot) -> Result<T, String>,
) -> Result<T, VmError> {
    let (_, id) = vm.data_of(value).ok_or_else(|| vm.raise_arg("invalid Table handle"))?;
    let mut slots = tables.lock().unwrap();
    let slot = slots.get_mut(&id).ok_or_else(|| vm.raise_arg("invalid Table handle"))?;
    f(slot).map_err(|e| vm.raise_arg(&e))
}

fn check_size(vm: &mut Vm, x: usize, y: usize, z: usize) -> Result<(), VmError> {
    if x.checked_mul(y).and_then(|n| n.checked_mul(z)).is_none_or(|n| n > 16_777_216) {
        return Err(vm.raise_arg("Table exceeds the 16 million element limit"));
    }
    Ok(())
}

fn table_dims(vm: &mut Vm, args: &[Value]) -> Result<(usize, usize, usize), VmError> {
    if args.is_empty() || args.len() > 3 { return Err(vm.raise_arg("wrong number of arguments")); }
    let x = index(vm, args[0])?;
    let y = if args.len() > 1 { index(vm, args[1])? } else { 1 };
    let z = if args.len() > 2 { index(vm, args[2])? } else { 1 };
    Ok((x, y, z))
}

fn coord_args(vm: &mut Vm, args: &[Value]) -> Result<(usize, usize, usize), VmError> {
    if args.len() != 3 { return Err(vm.raise_arg("wrong number of arguments")); }
    Ok((index(vm, args[0])?, index(vm, args[1])?, index(vm, args[2])?))
}

fn index(vm: &mut Vm, value: Value) -> Result<usize, VmError> {
    let n = vm.expect_int(value, "coordinate")?;
    usize::try_from(n).map_err(|_| vm.raise_arg("negative Table coordinate"))
}
