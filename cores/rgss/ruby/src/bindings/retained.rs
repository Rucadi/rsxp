// SPDX-License-Identifier: GPL-2.0-or-later
//! Change-driven synchronization. Only IDs cross frame boundaries; Ruby owns
//! object lifetimes and the engine owns the last resolved rendering state.
//!
//! This module installs the `__rsxp_*` global functions consumed by the bundled
//! `retained.rb` adapter (also exposed verbatim as `__rsxp_retained_source` via
//! `include_str!`) and the per-class scalar setters. `CHANGES` is a process-wide
//! mutex holding the dirty/volatile ID sets and the dependency graph; it never
//! holds Ruby values.
//!
//! ID model: a resource is keyed by its engine id `@__id`, while dependency-only
//! objects (nested `Rect`/`Color`/`Tone` values) get a synthetic token with the
//! high bit set, stored as `@__rsxp_dependency_token`. `touch` marks a resource
//! dirty and transitively dirties its dependents; `watch`/`unwatch` maintain the
//! edge sets; `volatile` forces an id to re-resolve every frame; `drain` returns
//! the union of dirty and volatile ids and clears the dirty set; `forget` drops a
//! resource from both the world registry and the graph.
//!
//! `install_scalars(class, kind)` is the only class-level entry point; `kind` is
//! `0` for `Sprite`, `1` for `Viewport`, `2` for `Plane` (the values chosen by
//! `retained.rb`). Every kind gets `z/ox/oy/visible`; non-viewport kinds also get
//! `zoom_x/zoom_y/blend_type`; `Sprite` additionally gets the full scalar set and
//! the world-aware `x/y/z` getters from `world`. Setters run through `scalar`,
//! which keeps the Ruby ivars authoritative, compares numerically to detect
//! no-op writes, flags non-scalar values volatile, and touches the object only
//! when the value actually changed. `x/y/z` setters first detach the sprite from
//! any world group.
use magnus::{
    function, method,
    prelude::*,
    value::{Fixnum, LazyId, Qfalse, Qtrue},
    Error, Float, Integer, RArray, RClass, RObject, Ruby, Value,
};
use std::{
    collections::{BTreeSet, HashMap},
    sync::{LazyLock, Mutex},
};

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
    f(&mut CHANGES.lock().unwrap_or_else(|error| error.into_inner()))
}

fn token(object: RObject) -> Result<Option<u64>, Error> {
    let id: Option<u64> = object.ivar_get("@__id")?;
    if id.is_some() {
        return Ok(id);
    }
    object.ivar_get("@__rsxp_dependency_token")
}
fn touch(object: RObject) -> Result<(), Error> {
    if let Some(id) = token(object)? {
        with_changes(|changes| {
            if id < (1u64 << 63) {
                changes.dirty.insert(id);
            }
            if changes.dependents.contains_key(&id) {
                let mut work = vec![id];
                let mut visited = BTreeSet::new();
                while let Some(source) = work.pop() {
                    if !visited.insert(source) {
                        continue;
                    }
                    if let Some(ids) = changes.dependents.get(&source).cloned() {
                        for dependent in ids {
                            changes.dirty.insert(dependent);
                            if changes.dependents.contains_key(&dependent) {
                                work.push(dependent);
                            }
                        }
                    }
                }
            }
        });
    }
    Ok(())
}
fn watch(object: RObject, dependent: u64) -> Result<(), Error> {
    let id = match token(object)? {
        Some(id) => id,
        // Frozen values cannot mutate; don't attach bookkeeping ivars to them.
        None if object.is_frozen() => return Ok(()),
        None => {
            let id = with_changes(|changes| {
                changes.next_token += 1;
                (1u64 << 63) | changes.next_token
            });
            object.ivar_set("@__rsxp_dependency_token", id)?;
            id
        }
    };
    with_changes(|changes| {
        changes.dependents.entry(id).or_default().insert(dependent);
        changes
            .dependencies
            .entry(dependent)
            .or_default()
            .insert(id);
    });
    Ok(())
}
fn unwatch(id: u64) {
    with_changes(|changes| {
        if let Some(sources) = changes.dependencies.remove(&id) {
            for source in sources {
                if let Some(ids) = changes.dependents.get_mut(&source) {
                    ids.remove(&id);
                    if ids.is_empty() {
                        changes.dependents.remove(&source);
                    }
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
    with_changes(|changes| {
        changes.volatile.insert(id);
    });
}
fn drain(ruby: &Ruby) -> RArray {
    let ids = with_changes(|changes| {
        let mut ids = std::mem::take(&mut changes.dirty);
        ids.extend(changes.volatile.iter().copied());
        ids
    });
    ruby.ary_from_iter(ids)
}
// Plain RGSS scalar accessors run directly in Rust. Keep their Ruby-visible
// ivars for compatibility, but avoid a Ruby wrapper/bind_call on hot setters.
fn scalar(object: RObject, value: Value, field: magnus::value::OpaqueId) -> Result<Value, Error> {
    let previous: Value = object.ivar_get(field)?;
    object.ivar_set(field, value)?;
    let same = if let (Some(a), Some(b)) = (Fixnum::from_value(previous), Fixnum::from_value(value))
    {
        a.to_i64() == b.to_i64()
    } else if let (Some(a), Some(b)) = (Fixnum::from_value(previous), Float::from_value(value)) {
        a.to_i64() as f64 == b.to_f64() && b.to_f64() as i64 == a.to_i64()
    } else if let (Some(a), Some(b)) = (Float::from_value(previous), Fixnum::from_value(value)) {
        a.to_f64() == b.to_i64() as f64 && a.to_f64() as i64 == b.to_i64()
    } else if let (Some(a), Some(b)) = (Float::from_value(previous), Float::from_value(value)) {
        a.to_f64() == b.to_f64()
    } else {
        (previous.is_nil() && value.is_nil())
            || (Qtrue::from_value(previous).is_some() && Qtrue::from_value(value).is_some())
            || (Qfalse::from_value(previous).is_some() && Qfalse::from_value(value).is_some())
    };
    if !value.is_nil()
        && Integer::from_value(value).is_none()
        && Float::from_value(value).is_none()
        && Qtrue::from_value(value).is_none()
        && Qfalse::from_value(value).is_none()
        && let Some(id) = token(object)?
    {
        volatile(id);
    }
    if !same {
        touch(object)?;
    }
    Ok(value)
}
macro_rules! accessors {
    ($($name:ident),* $(,)?) => { $(
        fn $name(object: RObject, value: Value) -> Result<Value, Error> {
            static FIELD: LazyId = LazyId::new(concat!("@", stringify!($name)));
            if matches!(stringify!($name),"x"|"y"|"z") {super::world::detach(object,false)?;}
            scalar(object, value, *FIELD)
        }
    )* };
}
accessors!(
    x,
    y,
    z,
    ox,
    oy,
    zoom_x,
    zoom_y,
    visible,
    mirror,
    blend_type,
    angle,
    invert,
    bush_depth,
    bush_opacity,
    pattern_opacity,
    pattern_tile,
    pattern_scroll_x,
    pattern_scroll_y,
    pattern_zoom_x,
    pattern_zoom_y,
    pattern_blend_type
);
fn install_scalars(class: RClass, kind: u8) -> Result<(), Error> {
    macro_rules! bind { ($($name:ident),* $(,)?) => { $(
        class.define_method(concat!(stringify!($name), "="), method!($name, 1))?;
    )* }; }
    bind!(z, ox, oy, visible);
    if kind != 1 {
        bind!(zoom_x, zoom_y, blend_type);
    }
    if kind == 0 {
        super::world::install_getters(class)?;
        bind!(
            x,
            y,
            mirror,
            angle,
            invert,
            bush_depth,
            bush_opacity,
            pattern_opacity,
            pattern_tile,
            pattern_scroll_x,
            pattern_scroll_y,
            pattern_zoom_x,
            pattern_zoom_y,
            pattern_blend_type
        );
    }
    Ok(())
}
fn source() -> &'static str {
    include_str!("retained.rb")
}

pub(super) fn bind(ruby: &Ruby) -> Result<(), Error> {
    with_changes(|changes| *changes = Changes::default());
    ruby.define_global_function(
        "__rsxp_install_scalar_setters",
        function!(install_scalars, 2),
    )?;
    ruby.define_global_function("__rsxp_retained_source", function!(source, 0))?;
    ruby.define_global_function("__rsxp_touch", function!(touch, 1))?;
    ruby.define_global_function("__rsxp_watch", function!(watch, 2))?;
    ruby.define_global_function("__rsxp_unwatch", function!(unwatch, 1))?;
    ruby.define_global_function("__rsxp_forget", function!(forget, 1))?;
    ruby.define_global_function("__rsxp_volatile", function!(volatile, 1))?;
    ruby.define_global_function("__rsxp_dirty_resources", function!(drain, 0))?;
    Ok(())
}
