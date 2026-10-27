// SPDX-License-Identifier: GPL-2.0-or-later
//! Change-driven resource synchronization for SabiRuby.
//!
//! Port of the Magnus `retained` binding. The Ruby adapter
//! (`bindings/retained.rb`) is evaluated by `script_loader.rb`; this module
//! installs the globals and scalar setters it calls. Only IDs cross frames:
//! a setter marks a resource dirty (or volatile), `drain` returns the ids to
//! resolve, and unchanged sprites never build a snapshot or cross the bridge.
use sabiruby::{object::Aspec, Value, Vm, VmError};
use std::collections::{BTreeSet, HashMap};
use std::sync::{LazyLock, Mutex};

const SOURCE: &str = include_str!("../bindings/retained.rb");

#[derive(Default)]
struct Changes {
    dirty: BTreeSet<u64>,
    volatile: BTreeSet<u64>,
    dependents: HashMap<u64, BTreeSet<u64>>,
    dependencies: HashMap<u64, BTreeSet<u64>>,
    next_token: u64,
}
static CHANGES: LazyLock<Mutex<Changes>> = LazyLock::new(Mutex::default);

fn with_changes<T>(f: impl FnOnce(&mut Changes) -> T) -> T {
    f(&mut CHANGES.lock().unwrap_or_else(|e| e.into_inner()))
}

fn token(vm: &Vm, object: Value) -> Option<u64> {
    let id = vm.ivar_get(object.obj()?, "@__id");
    if let Value::Int(n) = id {
        return Some(n as u64);
    }
    match vm.ivar_get(object.obj()?, "@__fanplay_dependency_token") {
        Value::Int(n) => Some(n as u64),
        _ => None,
    }
}

fn touch(vm: &Vm, object: Value) -> Result<(), VmError> {
    let Some(id) = token(vm, object) else { return Ok(()) };
    with_changes(|changes| {
        if id < (1u64 << 63) {
            changes.dirty.insert(id);
        }
        if changes.dependents.contains_key(&id) {
            let mut work = vec![id];
            let mut visited = BTreeSet::new();
            while let Some(source) = work.pop() {
                if !visited.insert(source) { continue; }
                if let Some(ids) = changes.dependents.get(&source).cloned() {
                    for dependent in ids {
                        changes.dirty.insert(dependent);
                        if changes.dependents.contains_key(&dependent) { work.push(dependent); }
                    }
                }
            }
        }
    });
    Ok(())
}

fn watch(vm: &mut Vm, object: Value, dependent: u64) -> Result<(), VmError> {
    let id = match token(vm, object) {
        Some(id) => id,
        None if { let m = vm.intern("frozen?"); vm.funcall(object, m, &[], Value::Nil)?.truthy() } => return Ok(()),
        None => {
            let id = with_changes(|changes| {
                changes.next_token += 1;
                (1u64 << 63) | changes.next_token
            });
            let obj = object.obj().ok_or_else(|| vm.raise_type("dependency is not an object"))?;
            vm.ivar_set(obj, "@__fanplay_dependency_token", Value::Int(id as i64));
            id
        }
    };
    with_changes(|changes| {
        changes.dependents.entry(id).or_default().insert(dependent);
        changes.dependencies.entry(dependent).or_default().insert(id);
    });
    Ok(())
}

fn unwatch(id: u64) {
    with_changes(|changes| {
        if let Some(sources) = changes.dependencies.remove(&id) {
            for source in sources {
                if let Some(ids) = changes.dependents.get_mut(&source) {
                    ids.remove(&id);
                    if ids.is_empty() { changes.dependents.remove(&source); }
                }
            }
        }
    });
}

fn forget(id: u64) {
    super::world::forget(id);
    unwatch(id);
    with_changes(|changes| {
        changes.dirty.remove(&id);
        changes.volatile.remove(&id);
    });
}

fn volatile(id: u64) {
    with_changes(|changes| { changes.volatile.insert(id); });
}

fn drain(vm: &mut Vm) -> Value {
    let ids = with_changes(|changes| {
        let mut ids = std::mem::take(&mut changes.dirty);
        ids.extend(changes.volatile.iter().copied());
        ids
    });
    vm.ary_new(ids.into_iter().map(|id| Value::Int(id as i64)).collect())
}

fn numeric(v: Value) -> Option<f64> {
    match v { Value::Int(n) => Some(n as f64), Value::Float(f) => Some(f), _ => None }
}

fn scalar(vm: &mut Vm, object: Value, value: Value, field: &str) -> Result<Value, VmError> {
    let obj = object.obj().ok_or_else(|| vm.raise_type("scalar receiver is not an object"))?;
    let previous = vm.ivar_get(obj, field);
    vm.ivar_set(obj, field, value);
    let same = match (numeric(previous), numeric(value)) {
        (Some(a), Some(b)) => a == b && (a as i64) as f64 == a && (b as i64) as f64 == b || a == b,
        _ => matches!((previous, value), (Value::Nil, Value::Nil))
            || matches!((previous, value), (Value::True, Value::True))
            || matches!((previous, value), (Value::False, Value::False)),
    };
    if !value.is_nil()
        && numeric(value).is_none()
        && !matches!(value, Value::True | Value::False)
        && let Some(id) = token(vm, object)
    {
        volatile(id);
    }
    if !same { touch(vm, object)?; }
    Ok(value)
}

fn define_scalar(vm: &mut Vm, class: sabiruby::value::ObjId, name: &'static str) {
    let setter = format!("{name}=");
    let field = format!("@{name}");
    vm.define_closure_args(class, &setter, Aspec::req(1), move |vm, this, args, _blk| {
        if matches!(name, "x" | "y" | "z") { super::world::detach(vm, this, false)?; }
        scalar(vm, this, args[0], &field)
    });
}

fn install_scalars(vm: &mut Vm, class: Value, kind: i64) -> Result<Value, VmError> {
    let class = class.obj().ok_or_else(|| vm.raise_arg("class or module required"))?;
    for name in ["z", "ox", "oy", "visible"] { define_scalar(vm, class, name); }
    if kind != 1 {
        for name in ["zoom_x", "zoom_y", "blend_type"] { define_scalar(vm, class, name); }
    }
    if kind == 0 {
        for name in [
            "x", "y", "mirror", "angle", "invert", "bush_depth", "bush_opacity",
            "pattern_opacity", "pattern_tile", "pattern_scroll_x", "pattern_scroll_y",
            "pattern_zoom_x", "pattern_zoom_y", "pattern_blend_type",
        ] { define_scalar(vm, class, name); }
        super::world::install_getters(vm, class)?;
    }
    Ok(Value::Nil)
}

pub(super) fn install(vm: &mut Vm) {
    if std::env::var("FANPLAY_NATIVE_STATE").as_deref() == Ok("0") { return; }
    vm.define_closure(vm.core.object, "__fanplay_install_scalar_setters", |vm, _self, args, _blk| {
        if args.len() != 2 { return Err(vm.raise_arg("wrong number of arguments")); }
        let kind = vm.expect_int(args[1], "kind")?;
        install_scalars(vm, args[0], kind)
    });
    let source = format!("begin\n{SOURCE}\nrescue Exception => e\n  STDERR.puts(e.full_message)\n  raise\nend\n");
    vm.define_closure(vm.core.object, "__fanplay_retained_source", move |vm, _self, _args, _blk| Ok(vm.str_new(source.as_bytes())));
    vm.define_closure(vm.core.object, "__fanplay_touch", |vm, _self, args, _blk| {
        if args.len() != 1 { return Err(vm.raise_arg("wrong number of arguments")); }
        touch(vm, args[0])?; Ok(Value::Nil)
    });
    vm.define_closure(vm.core.object, "__fanplay_watch", |vm, _self, args, _blk| {
        if args.len() != 2 { return Err(vm.raise_arg("wrong number of arguments")); }
        let id = vm.expect_int(args[1], "id")? as u64;
        watch(vm, args[0], id)?; Ok(Value::Nil)
    });
    vm.define_closure(vm.core.object, "__fanplay_unwatch", |vm, _self, args, _blk| {
        if args.len() != 1 { return Err(vm.raise_arg("wrong number of arguments")); }
        unwatch(vm.expect_int(args[0], "id")? as u64); Ok(Value::Nil)
    });
    vm.define_closure(vm.core.object, "__fanplay_forget", |vm, _self, args, _blk| {
        if args.len() != 1 { return Err(vm.raise_arg("wrong number of arguments")); }
        forget(vm.expect_int(args[0], "id")? as u64); Ok(Value::Nil)
    });
    vm.define_closure(vm.core.object, "__fanplay_volatile", |vm, _self, args, _blk| {
        if args.len() != 1 { return Err(vm.raise_arg("wrong number of arguments")); }
        volatile(vm.expect_int(args[0], "id")? as u64); Ok(Value::Nil)
    });
    vm.define_closure(vm.core.object, "__fanplay_dirty_resources", |vm, _self, _args, _blk| Ok(drain(vm)));
    // World rendering is not ported yet; these answer "not world-bound" so the
    // retained adapter keeps the ordinary screen-coordinate path.
    for name in ["__fanplay_world_camera", "__fanplay_scroll_rgss_tiles"] {
        vm.define_closure(vm.core.object, name, |_vm, _self, _args, _blk| Ok(Value::False));
    }
    for name in ["__fanplay_world_release", "__fanplay_world_metadata_changed", "__fanplay_world_unbind"] {
        vm.define_closure(vm.core.object, name, |_vm, _self, _args, _blk| Ok(Value::Nil));
    }
}
