// SPDX-License-Identifier: GPL-2.0-or-later
//! Native `__fanplay_sprite_sync` for SabiRuby.
//!
//! The Ruby shim still owns the getters and the change snapshot; this entry
//! converts the 39 values the snapshot submitted into a typed `SpriteState`
//! without building the tab-joined command string. `sprite.rb` detects the
//! private global and uses this path when it is installed.
use fanplay_rgss::{
    engine::{self, SpriteState},
    graphics::{Color, Rect, Tone},
};
use sabiruby::{Vm, Value, VmError};

pub(super) fn install(vm: &mut Vm) {
    if std::env::var("FANPLAY_SPRITE_SYNC").as_deref() == Ok("ruby") { return; }
    vm.define_closure(vm.core.object, "__fanplay_sprite_sync", |vm, _self, args, _blk| sync(vm, args));
    // `sprite.rb` gates the native path on `private_method_defined?`.
    let name = vm.intern("__fanplay_sprite_sync");
    let _ = vm.set_visibility(vm.core.object, name, sabiruby::object::Vis::Private);
}

fn sync(vm: &mut Vm, args: &[Value]) -> Result<Value, VmError> {
    if args.len() != 39 { return Err(vm.raise_arg("invalid sprite snapshot")); }
    let id = vm.expect_int(args[1], "id")? as u64;
    let blend = int_or(vm, args[2], 0)?;
    if !matches!(blend, 0..=2) { return Err(vm.raise(vm.core.not_implemented_error, "unsupported sprite blend type")); }
    let zoom_x = float_or(vm, args[3], 1.0)?;
    let zoom_y = float_or(vm, args[4], 1.0)?;
    if !zoom_x.is_finite() || !zoom_y.is_finite() { return Err(vm.raise(vm.core.float_domain_error, "non-finite sprite zoom")); }
    let bitmap = args[5];
    let viewport = args[6];
    let pattern = args[7];
    let mut visible = bool_or(vm, args[8], false)? && bitmap.truthy() && !disposed(vm, bitmap)?;
    let clip = if viewport.truthy() {
        visible &= bool_get(vm, viewport, "visible", false)? && !disposed(vm, viewport)?;
        let rect = get(vm, viewport, "rect")?;
        rectangle(vm, rect)?
    } else {
        let graphics_sym = vm.intern("Graphics");
        let graphics = vm.const_get(vm.core.object, graphics_sym)
            .ok_or_else(|| vm.name_error(graphics_sym, "uninitialized constant Graphics"))?;
        Rect::new(0, 0, int_get(vm, graphics, "width")?.max(0) as u32, int_get(vm, graphics, "height")?.max(0) as u32)
    };
    if pattern.truthy() && disposed(vm, pattern)? {
        return Err(vm.raise(vm.core.runtime_error, "disposed pattern Bitmap"));
    }
    let state = SpriteState {
        clip,
        zoom_x,
        zoom_y,
        visible,
        bitmap: if bitmap.truthy() { int_get(vm, bitmap, "__id")? as u64 } else { 0 },
        src: Rect::new(
            int_or(vm, args[9], 0)? as i32,
            int_or(vm, args[10], 0)? as i32,
            int_or(vm, args[11], 0)?.max(0) as u32,
            int_or(vm, args[12], 0)?.max(0) as u32,
        ),
        x: int_or(vm, args[13], 0)? as f64,
        y: int_or(vm, args[14], 0)? as f64,
        ox: int_or(vm, args[15], 0)? as f64,
        oy: int_or(vm, args[16], 0)? as f64,
        angle: float_or(vm, args[17], 0.0)?,
        mirror: bool_or(vm, args[18], false)?,
        z: int_or(vm, args[19], 0)? as i32,
        opacity: channel_or(vm, args[20], 255)?,
        blend_type: blend as i32,
        color: Color::rgba(
            channel_or(vm, args[29], 0)?,
            channel_or(vm, args[30], 0)?,
            channel_or(vm, args[31], 0)?,
            channel_or(vm, args[32], 255)?,
        ),
        tone: Tone {
            red: int_or(vm, args[33], 0)? as i32,
            green: int_or(vm, args[34], 0)? as i32,
            blue: int_or(vm, args[35], 0)? as i32,
            gray: int_or(vm, args[36], 0)? as i32,
        },
        invert: bool_or(vm, args[21], false)?,
        bush_depth: int_or(vm, args[37], 0)?.max(0).min(i32::MAX as i64) as i32,
        bush_opacity: channel_or(vm, args[38], 128)?,
        pattern: if pattern.truthy() { int_get(vm, pattern, "__id")? as u64 } else { 0 },
        pattern_blend: int_or(vm, args[22], 0)? as i32,
        pattern_tile: bool_or(vm, args[23], false)?,
        pattern_opacity: channel_or(vm, args[24], 255)?,
        pattern_scroll_x: int_or(vm, args[25], 0)? as f64,
        pattern_scroll_y: int_or(vm, args[26], 0)? as f64,
        pattern_zoom_x: float_or(vm, args[27], 1.0)?,
        pattern_zoom_y: float_or(vm, args[28], 1.0)?,
        viewport: if viewport.truthy() { int_get(vm, viewport, "__id")? as u64 } else { 0 },
    };
    let world_bound = super::world::bound(id);
    engine::with_runtime_mut(|runtime| {
        if !world_bound { runtime.clear_world_tile(id); }
        runtime.sync_sprite(id, state)
    }).unwrap_or_else(|| Err("runtime is closed".into())).map_err(|e| vm.raise(vm.core.runtime_error, &e))?;
    Ok(Value::Nil)
}

fn get(vm: &mut Vm, obj: Value, name: &str) -> Result<Value, VmError> {
    let sym = vm.intern(name);
    vm.funcall(obj, sym, &[], Value::Nil)
}
fn disposed(vm: &mut Vm, obj: Value) -> Result<bool, VmError> {
    Ok(get(vm, obj, "disposed?")?.truthy())
}
fn int_get(vm: &mut Vm, obj: Value, name: &str) -> Result<i64, VmError> {
    let value = get(vm, obj, name)?;
    vm.expect_int(value, name)
}
fn bool_get(vm: &mut Vm, obj: Value, name: &str, default: bool) -> Result<bool, VmError> {
    let value = get(vm, obj, name)?;
    bool_or(vm, value, default)
}
fn int_or(vm: &mut Vm, value: Value, default: i64) -> Result<i64, VmError> {
    if value.is_nil() { Ok(default) } else { vm.expect_int(value, "sprite property") }
}
fn float_or(vm: &mut Vm, value: Value, default: f64) -> Result<f64, VmError> {
    if value.is_nil() { return Ok(default); }
    match value {
        Value::Float(f) => Ok(f),
        Value::Int(i) => Ok(i as f64),
        _ => {
            let sym = vm.intern("to_f");
            let v = vm.funcall(value, sym, &[], Value::Nil)?;
            match v { Value::Float(f) => Ok(f), Value::Int(i) => Ok(i as f64), _ => Err(vm.raise_type("no implicit conversion into Float")) }
        }
    }
}
fn bool_or(_vm: &mut Vm, value: Value, default: bool) -> Result<bool, VmError> {
    if value.is_nil() { Ok(default) } else { Ok(value.truthy()) }
}
fn channel_or(vm: &mut Vm, value: Value, default: u8) -> Result<u8, VmError> {
    if value.is_nil() { return Ok(default); }
    let f = float_or(vm, value, default as f64)?;
    if !f.is_finite() { return Err(vm.raise(vm.core.float_domain_error, "non-finite channel")); }
    Ok(f.clamp(0.0, 255.0) as u8)
}
fn rectangle(vm: &mut Vm, rect: Value) -> Result<Rect, VmError> {
    let arr = get(vm, rect, "to_a")?;
    let vals = vm.ary_vals(arr).ok_or_else(|| vm.raise_type("Rect#to_a must be an Array"))?;
    if vals.len() < 4 { return Err(vm.raise_arg("bad Rect")); }
    Ok(Rect::new(
        vm.expect_int(vals[0], "x")? as i32,
        vm.expect_int(vals[1], "y")? as i32,
        vm.expect_int(vals[2], "width")?.max(0) as u32,
        vm.expect_int(vals[3], "height")?.max(0) as u32,
    ))
}
