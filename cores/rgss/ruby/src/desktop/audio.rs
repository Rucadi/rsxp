// SPDX-License-Identifier: GPL-2.0-or-later
//! Native RGSS `Audio` module for SabiRuby.
//!
//! The Ruby shim in `rgss/audio.rb` keeps the public surface; these methods
//! dispatch through the engine's byte command channel without building the
//! Ruby-side `FANPLAY.call` argument string per play/fade.
use sabiruby::{object::Aspec, Vm, Value, VmError};

pub(super) fn install(vm: &mut Vm) -> Result<(), String> {
    let audio = vm.define_module("Audio");
    let sc = vm.singleton_class(Value::Obj(audio)).map_err(|e| vm.describe_error(&e))?;
    fn def(vm: &mut Vm, sc: sabiruby::value::ObjId, name: &str, f: fn(&mut Vm, &[Value]) -> Result<Value, VmError>) {
        vm.define_closure_args(sc, name, Aspec::any(), move |vm, _self, args, _blk| f(vm, args));
    }
    def(vm, sc, "bgm_play", |vm, a| play(vm, a, "bgm", true));
    def(vm, sc, "bgm_stop", |vm, a| { arity(vm, a, 0, 1, "bgm_stop")?; stop(vm, "bgm") });
    def(vm, sc, "bgm_fade", |vm, a| fade(vm, a, "bgm", 2));
    def(vm, sc, "bgm_pos", |vm, a| { arity(vm, a, 0, 1, "bgm_pos")?; position(vm, "bgm") });
    def(vm, sc, "bgm_volume", |vm, _a| volume_value(vm));
    def(vm, sc, "bgm_set_volume", |vm, a| { if a.len()!=1 { return Err(vm.raise_arg("wrong number of arguments")); } set_volume(vm, a[0]) });
    def(vm, sc, "bgs_play", |vm, a| play(vm, a, "bgs", false));
    def(vm, sc, "bgs_stop", |vm, a| { arity(vm, a, 0, 1, "bgs_stop")?; stop(vm, "bgs") });
    def(vm, sc, "bgs_fade", |vm, a| fade(vm, a, "bgs", 2));
    def(vm, sc, "bgs_pos", |vm, a| { arity(vm, a, 0, 1, "bgs_pos")?; position(vm, "bgs") });
    def(vm, sc, "me_play", |vm, a| play(vm, a, "me", false));
    def(vm, sc, "me_stop", |vm, _a| stop(vm, "me"));
    def(vm, sc, "me_fade", |vm, a| fade(vm, a, "me", usize::MAX));
    def(vm, sc, "se_play", |vm, a| play(vm, a, "se", false));
    def(vm, sc, "se_stop", |vm, _a| stop(vm, "se"));
    def(vm, sc, "setup_midi", |_vm, _a| Ok(Value::Nil));
    def(vm, sc, "__reset__", |vm, _a| { for c in ["bgm","bgs","me","se"] { stop(vm,c)?; } Ok(Value::Nil) });
    Ok(())
}

fn play(vm: &mut Vm, args: &[Value], channel: &str, position: bool) -> Result<Value, VmError> {
    let max = if position { 5 } else if matches!(channel, "me" | "se") { 3 } else { 5 };
    arity(vm, args, 1, max, "play")?;
    let to_s = vm.intern("to_s");
    let text = vm.funcall(args[0], to_s, &[], Value::Nil)?;
    let path = vm.expect_str(text, "path")?;
    let volume = if args.len() > 1 { volume(vm, args[1])? } else { 100 };
    let pitch = if args.len() > 2 { float(vm, args[2])? } else { 100.0 };
    let offset = if position && args.len() > 3 { float(vm, args[3])? } else { 0.0 };
    let mut request = format!("audio_play\t{channel}\t{volume}\t{pitch}\t{offset}\n").into_bytes();
    request.extend_from_slice(&path);
    dispatch(vm, &request)?;
    Ok(Value::Nil)
}

fn stop(vm: &mut Vm, channel: &str) -> Result<Value, VmError> {
    dispatch(vm, format!("audio_stop\t{channel}\n").as_bytes())?;
    Ok(Value::Nil)
}

fn fade(vm: &mut Vm, args: &[Value], channel: &str, max: usize) -> Result<Value, VmError> {
    arity(vm, args, 1, max, "fade")?;
    let ms = integer(vm, args[0])?;
    dispatch(vm, format!("audio_fade\t{channel}\t{ms}\n").as_bytes())?;
    Ok(Value::Nil)
}

fn position(vm: &mut Vm, channel: &str) -> Result<Value, VmError> {
    let result = dispatch(vm, format!("audio_pos\t{channel}\n").as_bytes())?;
    let text = std::str::from_utf8(&result).map_err(|e| vm.raise(vm.core.runtime_error, &e.to_string()))?;
    let value = text.parse::<f64>().map_err(|e| vm.raise(vm.core.runtime_error, &e.to_string()))?;
    Ok(Value::Float(value))
}

fn volume_value(vm: &mut Vm) -> Result<Value, VmError> {
    let result = dispatch(vm, b"audio_volume\tbgm\n")?;
    let text = std::str::from_utf8(&result).map_err(|e| vm.raise(vm.core.runtime_error, &e.to_string()))?;
    let value = text.parse::<i64>().map_err(|e| vm.raise(vm.core.runtime_error, &e.to_string()))?;
    Ok(Value::Int(value))
}

fn set_volume(vm: &mut Vm, value: Value) -> Result<Value, VmError> {
    let value = volume(vm, value)?;
    dispatch(vm, format!("audio_set_volume\tbgm\t{value}\n").as_bytes())?;
    Ok(Value::Nil)
}

fn dispatch(vm: &mut Vm, request: &[u8]) -> Result<Vec<u8>, VmError> {
    match fanplay_rgss::engine::native_call_bytes(request) {
        Ok(Some(result)) => Ok(result),
        Ok(None) => {
            let sym = vm.intern("FANPLAYStop");
            let class = vm.const_get(vm.core.object, sym).and_then(|v| v.obj()).unwrap_or(vm.core.runtime_error);
            Err(vm.raise(class, ""))
        }
        Err(error) => Err(vm.raise(vm.core.runtime_error, &error)),
    }
}

fn integer(vm: &mut Vm, value: Value) -> Result<i64, VmError> { vm.expect_int(value, "integer") }

fn float(vm: &mut Vm, value: Value) -> Result<f64, VmError> {
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

fn volume(vm: &mut Vm, value: Value) -> Result<i64, VmError> { Ok(integer(vm, value)?.clamp(0, 100)) }

fn arity(vm: &mut Vm, args: &[Value], min: usize, max: usize, method: &str) -> Result<(), VmError> {
    if (min..=max).contains(&args.len()) { return Ok(()); }
    Err(vm.raise_arg(&format!("wrong number of arguments for Audio.{method} (given {}, expected {min}..{max})", args.len())))
}
