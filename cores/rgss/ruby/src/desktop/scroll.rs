// SPDX-License-Identifier: GPL-2.0-or-later
//! One native call for tile scrolling: keep Ruby-visible coordinates in sync,
//! update retained geometry directly, and avoid full property snapshots.
//!
//! Port of the Magnus `bindings/scroll.rs`. Two global functions are installed.
//! `__fanplay_scroll_tiles(tiles, ox, oy, width, height, source_height, move_xy,
//! move_z, bridge, enabled)` walks a 3-level `tiles[i][j][k]` pool; it preflights
//! that every sprite is retained (`@__fanplay_retained`) and, when `move_z`, has
//! an integer `@priority`. Tiles not marked `@__fanplay_map_covered` are skipped.
//! For each covered tile it detaches the sprite from its world group, computes
//! screen `x = i*width - ox`, `y = j*height - oy` and the RGSS priority `z`
//! (reflection `-100`, bridge or zero priority `0`, else
//! `(j + priority + 1) * source_height + 1`), writes the `@x`/`@y`/`@z` ivars,
//! and submits a `SpriteMotion` through a single runtime borrow.
//! `__fanplay_scroll_rgss_tiles(tiles, dx, dy)` is the RGSS1 variant whose
//! priority `z` also includes the pixel camera `y`, subtracting `dx`/`dy` with
//! saturation.
//!
//! `patch_snapshot` rewrites the Ruby `@__sync_state` array slots `15`/`16`
//! (x/y) and `23` (z) and clears `@__native_state`, so a later full sync
//! compares against the fresh position instead of the pre-scroll one. All Ruby
//! work completes before the engine borrow; overflow is reported as
//! `RangeError` and unsupported pools return `false` so the game keeps its Ruby
//! fallback.
use fanplay_rgss::engine::{self, SpriteMotion};
use sabiruby::{object::Aspec, Value, Vm, VmError};

/// Reads an instance variable, nil where the receiver cannot hold one.
fn get(vm: &Vm, object: Value, name: &str) -> Value {
    match object.obj() {
        Some(id) => vm.ivar_get(id, name),
        None => Value::Nil,
    }
}

fn get_int(vm: &mut Vm, object: Value, name: &str) -> Result<i64, VmError> {
    let value = get(vm, object, name);
    vm.expect_int(value, name)
}

fn set(vm: &mut Vm, object: Value, name: &str, value: Value) -> Result<(), VmError> {
    let obj = object
        .obj()
        .ok_or_else(|| vm.raise_type("tile is not an object"))?;
    vm.ivar_set(obj, name, value);
    Ok(())
}

/// `@__id` as the engine handle. The old binding read it through a `u64`
/// conversion; a negative or non-Integer ivar is a type error.
fn sprite_id(vm: &mut Vm, object: Value) -> Result<u64, VmError> {
    let value = get(vm, object, "@__id");
    let n = vm.expect_int(value, "@__id")?;
    u64::try_from(n).map_err(|_| vm.raise_type("@__id is not a sprite handle"))
}

fn frozen(vm: &mut Vm, object: Value) -> Result<bool, VmError> {
    let name = vm.intern("frozen?");
    Ok(vm.funcall(object, name, &[], Value::Nil)?.truthy())
}

fn ary(vm: &mut Vm, value: Value, what: &str) -> Result<Vec<Value>, VmError> {
    match vm.ary_vals(value) {
        Some(values) => Ok(values),
        None => Err(vm.raise_type(&format!("{what} is not an Array"))),
    }
}

// Bulk geometry changes must also advance Ruby's last-submitted snapshot.
// Otherwise returning to that old position can incorrectly skip the next sync.
fn patch_snapshot(
    vm: &mut Vm,
    tile: Value,
    position: Option<(i64, i64)>,
    z: Option<i32>,
) -> Result<(), VmError> {
    let state = get(vm, tile, "@__sync_state");
    if vm.obj_is_kind_of(state, vm.core.array) {
        // No public Array store on the host API; `[]=` writes the same slots
        // (an out-of-range index pads instead of raising, see the port notes).
        let store = vm.intern("[]=");
        if let Some((x, y)) = position {
            vm.funcall(state, store, &[Value::Int(15), Value::Int(x)], Value::Nil)?;
            vm.funcall(state, store, &[Value::Int(16), Value::Int(y)], Value::Nil)?;
        }
        if let Some(z) = z {
            vm.funcall(state, store, &[Value::Int(23), Value::Int(z as i64)], Value::Nil)?;
        }
    }
    set(vm, tile, "@__native_state", Value::Nil)
}

fn scroll(vm: &mut Vm, args: &[Value]) -> Result<Value, VmError> {
    if args.len() != 10 {
        return Err(vm.raise_arg("invalid tile scroll arguments"));
    }
    let tiles = ary(vm, args[0], "tile pool")?;
    let ox = vm.expect_int(args[1], "ox")?;
    let oy = vm.expect_int(args[2], "oy")?;
    let width = vm.expect_int(args[3], "width")?;
    let height = vm.expect_int(args[4], "height")?;
    let source_height = vm.expect_int(args[5], "source_height")?;
    let move_xy = args[6].truthy();
    let move_z = args[7].truthy();
    let bridge = args[8].truthy();
    let enabled = args[9].truthy();
    if !enabled {
        return Ok(Value::False);
    }
    // Preflight the whole pool before changing anything. Overrides, mutable
    // inputs and compatibility resources keep the game's Ruby helper path.
    for column in &tiles {
        let column = ary(vm, *column, "tile column")?;
        for stack in &column {
            let stack = ary(vm, *stack, "tile stack")?;
            for &tile in &stack {
                if !get(vm, tile, "@__fanplay_retained").truthy() {
                    return Ok(Value::False);
                }
                if move_z && !matches!(get(vm, tile, "@priority"), Value::Int(_)) {
                    return Ok(Value::False);
                }
            }
        }
    }
    let mut motions = Vec::new();
    for (i, column) in tiles.iter().enumerate() {
        let column = ary(vm, *column, "tile column")?;
        for (j, stack) in column.iter().enumerate() {
            let stack = ary(vm, *stack, "tile stack")?;
            let x = (i as i64)
                .checked_mul(width)
                .and_then(|x| x.checked_sub(ox));
            let y = (j as i64)
                .checked_mul(height)
                .and_then(|y| y.checked_sub(oy));
            let (Some(x), Some(y)) = (x, y) else {
                return Err(vm.raise(vm.core.range_error, "tile scroll overflow"));
            };
            for &tile in &stack {
                if !get(vm, tile, "@__fanplay_map_covered").truthy() {
                    continue;
                }
                super::world::detach(vm, tile, true)?;
                let id = sprite_id(vm, tile)?;
                let z = if move_z {
                    let priority = get_int(vm, tile, "@priority")?;
                    let z = if get(vm, tile, "@shows_reflection").truthy() {
                        -100
                    } else if (get(vm, tile, "@bridge").truthy() && bridge) || priority == 0 {
                        0
                    } else {
                        (j as i64)
                            .checked_add(priority)
                            .and_then(|n| n.checked_add(1))
                            .and_then(|n| n.checked_mul(source_height))
                            .and_then(|n| n.checked_add(1))
                            .ok_or_else(|| {
                                vm.raise(vm.core.range_error, "tile priority overflow")
                            })?
                    };
                    let z = i32::try_from(z).map_err(|_| {
                        vm.raise(vm.core.range_error, "tile priority overflow")
                    })?;
                    set(vm, tile, "@z", Value::Int(z as i64))?;
                    Some(z)
                } else {
                    None
                };
                if move_xy {
                    set(vm, tile, "@x", Value::Int(x))?;
                    set(vm, tile, "@y", Value::Int(y))?;
                }
                patch_snapshot(vm, tile, move_xy.then_some((x, y)), z)?;
                motions.push(SpriteMotion {
                    id,
                    position: move_xy.then_some((x as f64, y as f64)),
                    z,
                });
            }
        }
    }
    // All Ruby work finishes before taking an engine borrow. Dirty snapshots
    // (newly exposed or animated tiles) later resolve against these new ivars.
    let _ = engine::with_runtime_mut(|runtime| runtime.move_sprites(&motions));
    Ok(Value::True)
}

// RGSS1's priority z includes the pixel camera y (unlike Essentials v21).
fn scroll_rgss(vm: &mut Vm, args: &[Value]) -> Result<Value, VmError> {
    let tiles = ary(vm, args[0], "tile list")?;
    let dx = vm.expect_int(args[1], "dx")?;
    let dy = vm.expect_int(args[2], "dy")?;
    let mut motions = Vec::with_capacity(tiles.len());
    let mut indices = Vec::with_capacity(tiles.len());
    for (index, &tile) in tiles.iter().enumerate() {
        if !get(vm, tile, "@__fanplay_retained").truthy() || frozen(vm, tile)? {
            return Ok(Value::False);
        }
        if !get(vm, tile, "@visible").truthy() {
            continue;
        }
        indices.push(index);
        let (Value::Int(x), Value::Int(y), Value::Int(z)) =
            (get(vm, tile, "@x"), get(vm, tile, "@y"), get(vm, tile, "@z"))
        else {
            return Ok(Value::False);
        };
        let x = x.saturating_sub(dx);
        let y = y.saturating_sub(dy);
        let z = if get(vm, tile, "@__fanplay_rgss_priority").truthy() {
            z.saturating_sub(dy)
        } else {
            z
        };
        let Ok(z) = i32::try_from(z) else {
            return Ok(Value::False);
        };
        motions.push(SpriteMotion {
            id: sprite_id(vm, tile)?,
            position: Some((x as f64, y as f64)),
            z: Some(z),
        });
    }
    // Preflight completes before mutating any Ruby or retained engine state.
    for (&index, motion) in indices.iter().zip(&motions) {
        let tile = tiles[index];
        let (x, y) = motion.position.unwrap();
        set(vm, tile, "@x", Value::Int(x as i64))?;
        set(vm, tile, "@y", Value::Int(y as i64))?;
        set(vm, tile, "@z", Value::Int(motion.z.unwrap() as i64))?;
        patch_snapshot(vm, tile, Some((x as i64, y as i64)), motion.z)?;
    }
    let _ = engine::with_runtime_mut(|runtime| runtime.move_sprites(&motions));
    Ok(Value::True)
}

pub(super) fn install(vm: &mut Vm) {
    // The Magnus binding took `-1` arguments and checked the count itself.
    vm.define_closure(vm.core.object, "__fanplay_scroll_tiles", |vm, _self, args, _blk| {
        scroll(vm, args)
    });
    vm.define_closure_args(
        vm.core.object,
        "__fanplay_scroll_rgss_tiles",
        Aspec::req(3),
        |vm, _self, args, _blk| scroll_rgss(vm, args),
    );
}
