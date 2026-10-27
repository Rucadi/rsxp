// SPDX-License-Identifier: GPL-2.0-or-later
//! Apply engine-owned RGSS1 tile plans in one call. No Ruby values are retained.
//!
//! Port of the Magnus `bindings/tilemap.rs`. `__fanplay_apply_rgss_tiles(sprites,
//! plan, color, tone, viewport, clip)` is the batch entry point used by the legacy
//! `Tilemap` adapter. It preflights that every sprite is retained and not frozen,
//! converts `color`/`tone` once, then walks `plan`, a parallel array of `[bitmap,
//! src_x, src_y, src_size, x, y, z, priority]` rows. For each row it writes the
//! tile's `@bitmap`, `@src_rect`, `@x`/`@y`/`@z`, `@visible`,
//! `@__fanplay_rgss_priority`, `@color` and `@tone` ivars, clears the
//! `@__sync_state`/`@__native_state` snapshot caches, calls
//! `FANPLAY::Retained.rewire` when the bitmap identity changed, and builds a
//! `NativeTileUpdate`. Sprites beyond the plan are hidden. One
//! `engine::with_runtime_mut` call applies the updates, after every Ruby read,
//! allocation and dependency callback has finished. Returns `false` (game
//! fallback) on any unsupported sprite.
use fanplay_rgss::{
    engine::{self, NativeTileUpdate},
    graphics::{Color, Rect, Tone},
};
use sabiruby::{object::Aspec, value::ObjId, Vm, Value, VmError};

pub(super) fn install(vm: &mut Vm) {
    vm.define_closure_args(vm.core.object, "__fanplay_apply_rgss_tiles", Aspec::req(6), |vm, _self, args, _blk| apply(vm, args));
}

fn apply(vm: &mut Vm, args: &[Value]) -> Result<Value, VmError> {
    let sprites = array(vm, args[0], "sprite list")?;
    let plan = array(vm, args[1], "tile plan")?;
    let colors = array(vm, args[2], "color")?;
    let tones = array(vm, args[3], "tone")?;
    let clip = array(vm, args[5], "clip")?;

    // Preflight: any sprite the native path cannot own makes the whole call a
    // request for the ordinary Ruby fallback.
    let frozen = vm.intern("frozen?");
    for sprite in &sprites {
        let tile = object(vm, *sprite, "tile")?;
        if !vm.ivar_get(tile, "@__fanplay_retained").truthy() { return Ok(Value::False); }
        if vm.funcall(*sprite, frozen, &[], Value::Nil)?.truthy() { return Ok(Value::False); }
    }

    if colors.len() < 4 { return Err(vm.raise_arg("bad tile Color")); }
    if tones.len() < 4 { return Err(vm.raise_arg("bad tile Tone")); }
    if clip.len() < 4 { return Err(vm.raise_arg("bad tile clip Rect")); }
    let native_color = Color::rgba(
        channel(vm, colors[0])?,
        channel(vm, colors[1])?,
        channel(vm, colors[2])?,
        channel(vm, colors[3])?,
    );
    let native_tone = Tone {
        red: int(vm, tones[0])? as i32,
        green: int(vm, tones[1])? as i32,
        blue: int(vm, tones[2])? as i32,
        gray: int(vm, tones[3])? as i32,
    };
    let viewport = if args[4].truthy() { object_id(vm, args[4], "viewport")? } else { 0 };
    let clip = Rect::new(
        coord(vm, clip[0], "clip x")?,
        coord(vm, clip[1], "clip y")?,
        size(vm, clip[2], "clip width")?,
        size(vm, clip[3], "clip height")?,
    );

    let fanplay_sym = vm.intern("FANPLAY");
    let fanplay = vm.const_get(vm.core.object, fanplay_sym)
        .ok_or_else(|| vm.name_error(fanplay_sym, "uninitialized constant FANPLAY"))?;
    let retained_sym = vm.intern("Retained");
    let fanplay = object(vm, fanplay, "FANPLAY")?;
    let retained = vm.const_get(fanplay, retained_sym)
        .ok_or_else(|| vm.name_error(retained_sym, "uninitialized constant FANPLAY::Retained"))?;
    let rewire = vm.intern("rewire");

    let mut updates = Vec::with_capacity(plan.len());
    for (index, row) in plan.iter().enumerate() {
        let row = array(vm, *row, "tile row")?;
        if row.len() < 8 { return Err(vm.raise_arg("bad tile row")); }
        let tile = *sprites.get(index).ok_or_else(|| vm.raise_arg("tile plan is longer than the sprite list"))?;
        let tile_obj = object(vm, tile, "tile")?;
        let bitmap = row[0];
        let bitmap_id = object_id(vm, bitmap, "tile Bitmap")?;
        let previous = vm.ivar_get(tile_obj, "@bitmap");
        let changed = match previous.obj() {
            Some(old) => match vm.ivar_get(old, "@__id") { Value::Int(n) => n as u64 != bitmap_id, _ => true },
            None => true,
        };
        let sx = coord(vm, row[1], "src_x")?;
        let sy = coord(vm, row[2], "src_y")?;
        let tile_size = size(vm, row[3], "tile size")?;
        let x = vm.expect_int(row[4], "tile x")?;
        let y = vm.expect_int(row[5], "tile y")?;
        let z = coord(vm, row[6], "tile z")?;
        let priority = row[7];
        vm.ivar_set(tile_obj, "@bitmap", bitmap);
        let area_value = vm.ivar_get(tile_obj, "@src_rect");
        let area = object(vm, area_value, "src_rect")?;
        vm.ivar_set(area, "@x", Value::Int(sx as i64));
        vm.ivar_set(area, "@y", Value::Int(sy as i64));
        vm.ivar_set(area, "@width", Value::Int(tile_size as i64));
        vm.ivar_set(area, "@height", Value::Int(tile_size as i64));
        vm.ivar_set(tile_obj, "@x", Value::Int(x));
        vm.ivar_set(tile_obj, "@y", Value::Int(y));
        vm.ivar_set(tile_obj, "@z", Value::Int(z as i64));
        vm.ivar_set(tile_obj, "@visible", Value::True);
        vm.ivar_set(tile_obj, "@__fanplay_rgss_priority", priority);
        let color_value = vm.ivar_get(tile_obj, "@color");
        let color = object(vm, color_value, "tile color")?;
        let tone_value = vm.ivar_get(tile_obj, "@tone");
        let tone = object(vm, tone_value, "tile tone")?;
        for (slot, field) in ["@red", "@green", "@blue", "@alpha"].into_iter().enumerate() {
            vm.ivar_set(color, field, colors[slot]);
        }
        for (slot, field) in ["@red", "@green", "@blue", "@gray"].into_iter().enumerate() {
            vm.ivar_set(tone, field, tones[slot]);
        }
        // Geometry and appearance are now submitted natively. A later
        // notification must compare against a fresh complete snapshot.
        vm.ivar_set(tile_obj, "@__sync_state", Value::Nil);
        vm.ivar_set(tile_obj, "@__native_state", Value::Nil);
        if changed { vm.funcall(retained, rewire, &[tile], Value::Nil)?; }
        updates.push(NativeTileUpdate {
            id: object_id(vm, tile, "tile")?,
            bitmap: bitmap_id,
            src: Rect::new(sx, sy, tile_size, tile_size),
            x: x as f64,
            y: y as f64,
            z,
            visible: true,
            viewport,
            clip,
            color: native_color,
            tone: native_tone,
        });
    }

    let mut hidden = Vec::with_capacity(sprites.len().saturating_sub(plan.len()));
    for tile in sprites.iter().skip(plan.len()) {
        let tile_obj = object(vm, *tile, "tile")?;
        vm.ivar_set(tile_obj, "@visible", Value::False);
        vm.ivar_set(tile_obj, "@__sync_state", Value::Nil);
        vm.ivar_set(tile_obj, "@__native_state", Value::Nil);
        hidden.push(object_id(vm, *tile, "tile")?);
    }
    // All Ruby reads, allocations and dependency callbacks finish before
    // borrowing the runtime; finalizers may safely reenter the engine.
    let _ = engine::with_runtime_mut(|runtime| runtime.update_native_tiles(&updates, &hidden));
    Ok(Value::True)
}

fn array(vm: &mut Vm, value: Value, what: &str) -> Result<Vec<Value>, VmError> {
    vm.ary_vals(value).ok_or_else(|| vm.raise_type(&format!("{what} is not an Array")))
}

fn object(vm: &mut Vm, value: Value, what: &str) -> Result<ObjId, VmError> {
    value.obj().ok_or_else(|| vm.raise_type(&format!("{what} is not an object")))
}

fn object_id(vm: &mut Vm, value: Value, what: &str) -> Result<u64, VmError> {
    let obj = object(vm, value, what)?;
    match vm.ivar_get(obj, "@__id") {
        Value::Int(n) => Ok(n as u64),
        _ => Err(vm.raise_type(&format!("{what} has no @__id"))),
    }
}

fn coord(vm: &mut Vm, value: Value, what: &str) -> Result<i32, VmError> {
    let n = vm.expect_int(value, what)?;
    i32::try_from(n).map_err(|_| vm.raise_arg(&format!("{what} out of range")))
}

fn size(vm: &mut Vm, value: Value, what: &str) -> Result<u32, VmError> {
    let n = vm.expect_int(value, what)?;
    u32::try_from(n).map_err(|_| vm.raise_arg(&format!("negative {what}")))
}

// `tilemap.rb` passes `@color.to_a`, whose entries are already Integers, but
// plugins may hand the engine Floats or String numbers; `Color#red=` and
// `Tone#red=` coerce the same way through `Float(...)`/`Integer(...)`.
fn int(vm: &mut Vm, value: Value) -> Result<i64, VmError> {
    let string = vm.str_bytes(value).is_some();
    if string {
        let sym = vm.intern("Integer");
        let converted = vm.funcall(Value::Obj(vm.core.kernel), sym, &[value], Value::Nil)?;
        return vm.expect_int(converted, "tile integer");
    }
    vm.expect_int(value, "tile integer")
}

fn float(vm: &mut Vm, value: Value) -> Result<f64, VmError> {
    let string = vm.str_bytes(value).is_some();
    if string {
        let sym = vm.intern("Float");
        let converted = vm.funcall(Value::Obj(vm.core.kernel), sym, &[value], Value::Nil)?;
        return match converted { Value::Float(f) => Ok(f), Value::Int(i) => Ok(i as f64), _ => Err(vm.raise_type("no implicit conversion into Float")) };
    }
    match value {
        Value::Float(f) => Ok(f),
        Value::Int(i) => Ok(i as f64),
        _ => {
            let sym = vm.intern("to_f");
            match vm.funcall(value, sym, &[], Value::Nil)? {
                Value::Float(f) => Ok(f),
                Value::Int(i) => Ok(i as f64),
                _ => Err(vm.raise_type("no implicit conversion into Float")),
            }
        }
    }
}

fn channel(vm: &mut Vm, value: Value) -> Result<u8, VmError> {
    let value = float(vm, value)?;
    if !value.is_finite() { return Err(vm.raise(vm.core.float_domain_error, "non-finite channel")); }
    Ok(value.clamp(0.0, 255.0) as u8)
}
