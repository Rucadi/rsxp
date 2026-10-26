// SPDX-License-Identifier: GPL-2.0-or-later
//! One native call for tile scrolling: keep Ruby-visible coordinates in sync,
//! update retained geometry directly, and avoid full property snapshots.
//!
//! Two global functions are installed. `__fanplay_scroll_tiles(tiles, ox, oy,
//! width, height, source_height, move_xy, move_z, bridge, enabled)` walks a
//! 3-level `tiles[i][j][k]` pool; it preflights that every sprite is retained
//! (`@__fanplay_retained`) and, when `move_z`, has an integer `@priority`. Tiles not
//! marked `@__fanplay_map_covered` are skipped. For each covered tile it detaches
//! the sprite from its world group, computes screen `x = i*width - ox`,
//! `y = j*height - oy` and the RGSS priority `z` (reflection `-100`, bridge or
//! zero priority `0`, else `(j + priority + 1) * source_height + 1`), writes the
//! `@x`/`@y`/`@z` ivars, and submits a `SpriteMotion` through a single runtime
//! borrow. `__fanplay_scroll_rgss_tiles(tiles, dx, dy)` is the RGSS1 variant whose
//! priority `z` also includes the pixel camera `y`, subtracting `dx`/`dy` with
//! saturation.
//!
//! `patch_snapshot` rewrites the Ruby `@__sync_state` array slots `15`/`16`
//! (x/y) and `23` (z) and clears `@__native_state`, so a later full sync compares
//! against the fresh position instead of the pre-scroll one. All Ruby work
//! completes before the engine borrow; overflow is reported as `RangeError` and
//! unsupported pools return `false` so the game keeps its Ruby fallback.
use fanplay_rgss::engine::{self, SpriteMotion};
use magnus::{
    function,
    prelude::*,
    value::{Fixnum, LazyId},
    Error, RArray, RObject, Ruby, TryConvert, Value,
};

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
// Bulk geometry changes must also advance Ruby's last-submitted snapshot.
// Otherwise returning to that old position can incorrectly skip the next sync.
fn patch_snapshot(
    tile: RObject,
    position: Option<(i64, i64)>,
    z: Option<i32>,
) -> Result<(), Error> {
    if let Some(state) = RArray::from_value(get!(tile, "@__sync_state", Value)) {
        if let Some((x, y)) = position {
            state.store(15, x)?;
            state.store(16, y)?;
        }
        if let Some(z) = z {
            state.store(23, z)?;
        }
    }
    set!(tile, "@__native_state", ());
    Ok(())
}
fn scroll(ruby: &Ruby, args: &[Value]) -> Result<bool, Error> {
    if args.len() != 10 {
        return Err(Error::new(
            ruby.exception_arg_error(),
            "invalid tile scroll arguments",
        ));
    }
    let tiles = RArray::try_convert(args[0])?;
    let ox = i64::try_convert(args[1])?;
    let oy = i64::try_convert(args[2])?;
    let width = i64::try_convert(args[3])?;
    let height = i64::try_convert(args[4])?;
    let source_height = i64::try_convert(args[5])?;
    let move_xy = args[6].to_bool();
    let move_z = args[7].to_bool();
    let bridge = args[8].to_bool();
    let enabled = args[9].to_bool();
    if !enabled {
        return Ok(false);
    }
    // Preflight the whole pool before changing anything. Overrides, mutable
    // inputs and compatibility resources keep the game's Ruby helper path.
    for i in 0..tiles.len() {
        let column: RArray = tiles.entry(i as isize)?;
        for j in 0..column.len() {
            let stack: RArray = column.entry(j as isize)?;
            for k in 0..stack.len() {
                let tile: RObject = stack.entry(k as isize)?;
                if !get!(tile, "@__fanplay_retained", Value).to_bool() {
                    return Ok(false);
                }
                if move_z && Fixnum::from_value(get!(tile, "@priority", Value)).is_none() {
                    return Ok(false);
                }
            }
        }
    }
    let mut motions = Vec::new();
    for i in 0..tiles.len() {
        let column: RArray = tiles.entry(i as isize)?;
        for j in 0..column.len() {
            let stack: RArray = column.entry(j as isize)?;
            let x = (i as i64)
                .checked_mul(width)
                .and_then(|x| x.checked_sub(ox));
            let y = (j as i64)
                .checked_mul(height)
                .and_then(|y| y.checked_sub(oy));
            let (Some(x), Some(y)) = (x, y) else {
                return Err(Error::new(
                    ruby.exception_range_error(),
                    "tile scroll overflow",
                ));
            };
            for k in 0..stack.len() {
                let tile: RObject = stack.entry(k as isize)?;
                if !get!(tile, "@__fanplay_map_covered", Value).to_bool() {
                    continue;
                }
                super::world::detach(tile, true)?;
                let id = get!(tile, "@__id", u64);
                let z = if move_z {
                    let priority = get!(tile, "@priority", i64);
                    let z = if get!(tile, "@shows_reflection", Value).to_bool() {
                        -100
                    } else if (get!(tile, "@bridge", Value).to_bool() && bridge) || priority == 0 {
                        0
                    } else {
                        (j as i64)
                            .checked_add(priority)
                            .and_then(|n| n.checked_add(1))
                            .and_then(|n| n.checked_mul(source_height))
                            .and_then(|n| n.checked_add(1))
                            .ok_or_else(|| {
                                Error::new(ruby.exception_range_error(), "tile priority overflow")
                            })?
                    };
                    let z = i32::try_from(z).map_err(|_| {
                        Error::new(ruby.exception_range_error(), "tile priority overflow")
                    })?;
                    set!(tile, "@z", z);
                    Some(z)
                } else {
                    None
                };
                if move_xy {
                    set!(tile, "@x", x);
                    set!(tile, "@y", y);
                }
                patch_snapshot(tile, move_xy.then_some((x, y)), z)?;
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
    engine::with_runtime_mut(|runtime| runtime.move_sprites(&motions));
    Ok(true)
}
// RGSS1's priority z includes the pixel camera y (unlike Essentials v21).
fn scroll_rgss(tiles: RArray, dx: i64, dy: i64) -> Result<bool, Error> {
    let mut motions = Vec::with_capacity(tiles.len());
    let mut indices = Vec::with_capacity(tiles.len());
    for index in 0..tiles.len() {
        let tile: RObject = tiles.entry(index as isize)?;
        if !get!(tile, "@__fanplay_retained", Value).to_bool() || tile.is_frozen() {
            return Ok(false);
        }
        if !get!(tile, "@visible", Value).to_bool() {
            continue;
        }
        indices.push(index);
        let Some(x) = Fixnum::from_value(get!(tile, "@x", Value)) else {
            return Ok(false);
        };
        let Some(y) = Fixnum::from_value(get!(tile, "@y", Value)) else {
            return Ok(false);
        };
        let Some(z) = Fixnum::from_value(get!(tile, "@z", Value)) else {
            return Ok(false);
        };
        let x = x.to_i64().saturating_sub(dx);
        let y = y.to_i64().saturating_sub(dy);
        let z = if get!(tile, "@__fanplay_rgss_priority", Value).to_bool() {
            z.to_i64().saturating_sub(dy)
        } else {
            z.to_i64()
        };
        let Ok(z) = i32::try_from(z) else {
            return Ok(false);
        };
        motions.push(SpriteMotion {
            id: get!(tile, "@__id", u64),
            position: Some((x as f64, y as f64)),
            z: Some(z),
        });
    }
    // Preflight completes before mutating any Ruby or retained engine state.
    for (&index, motion) in indices.iter().zip(&motions) {
        let tile: RObject = tiles.entry(index as isize)?;
        let (x, y) = motion.position.unwrap();
        set!(tile, "@x", x as i64);
        set!(tile, "@y", y as i64);
        set!(tile, "@z", motion.z.unwrap());
        patch_snapshot(tile, Some((x as i64, y as i64)), motion.z)?;
    }
    engine::with_runtime_mut(|runtime| runtime.move_sprites(&motions));
    Ok(true)
}
pub(super) fn bind(ruby: &Ruby) -> Result<(), Error> {
    ruby.define_global_function("__fanplay_scroll_rgss_tiles", function!(scroll_rgss, 3))?;
    ruby.define_global_function("__fanplay_scroll_tiles", function!(scroll, -1))?;
    Ok(())
}
