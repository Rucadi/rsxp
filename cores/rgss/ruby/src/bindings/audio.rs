// SPDX-License-Identifier: GPL-2.0-or-later
//! Magnus implementation of the RGSS Audio module for the desktop VM.
use fanplay_rgss::engine;
use magnus::{function, prelude::*, Error, ExceptionClass, RString, Ruby, Value};

pub(super) fn bind(ruby: &Ruby) -> Result<(), Error> {
    let audio = ruby.define_module("Audio")?;
    audio.define_singleton_method("bgm_play", function!(bgm_play, -1))?;
    audio.define_singleton_method("bgm_stop", function!(bgm_stop, -1))?;
    audio.define_singleton_method("bgm_fade", function!(bgm_fade, -1))?;
    audio.define_singleton_method("bgm_pos", function!(bgm_pos, -1))?;
    audio.define_singleton_method("bgm_volume", function!(bgm_volume, 0))?;
    audio.define_singleton_method("bgm_set_volume", function!(bgm_set_volume, 1))?;
    audio.define_singleton_method("bgs_play", function!(bgs_play, -1))?;
    audio.define_singleton_method("bgs_stop", function!(bgs_stop, -1))?;
    audio.define_singleton_method("bgs_fade", function!(bgs_fade, -1))?;
    audio.define_singleton_method("bgs_pos", function!(bgs_pos, -1))?;
    audio.define_singleton_method("me_play", function!(me_play, -1))?;
    audio.define_singleton_method("me_stop", function!(me_stop, -1))?;
    audio.define_singleton_method("me_fade", function!(me_fade, -1))?;
    audio.define_singleton_method("se_play", function!(se_play, -1))?;
    audio.define_singleton_method("se_stop", function!(se_stop, -1))?;
    audio.define_singleton_method("setup_midi", function!(setup_midi, 0))?;
    audio.define_singleton_method("__reset__", function!(__reset__, 0))?;
    Ok(())
}

fn play(ruby: &Ruby, args: &[Value], channel: &str, position: bool) -> Result<(), Error> {
    let max = if position {
        5
    } else if matches!(channel, "me" | "se") {
        3
    } else {
        5
    };
    arity(ruby, args, 1, max, "play")?;
    let file: RString = args[0].funcall("to_s", ())?;
    // The shared Ruby shim repairs malformed/non-UTF-8 path strings before
    // they cross the byte bridge. Preserve that behavior for desktop MRI.
    let path = String::from_utf8_lossy(unsafe { file.as_slice() });
    let volume = if args.len() > 1 {
        volume(ruby, args[1])?
    } else {
        100
    };
    let pitch = if args.len() > 2 {
        float(ruby, args[2])?
    } else {
        100.0
    };
    let offset = if position && args.len() > 3 {
        float(ruby, args[3])?
    } else {
        0.0
    };
    let header = format!("audio_play\t{channel}\t{volume}\t{pitch}\t{offset}\n");
    let mut request = header.into_bytes();
    request.extend_from_slice(path.as_bytes());
    dispatch(ruby, &request)?;
    Ok(())
}

fn stop(ruby: &Ruby, channel: &str) -> Result<(), Error> {
    dispatch(ruby, format!("audio_stop\t{channel}\n").as_bytes())?;
    Ok(())
}

fn fade(ruby: &Ruby, args: &[Value], channel: &str, max_args: usize) -> Result<(), Error> {
    arity(ruby, args, 1, max_args, "fade")?;
    let milliseconds = integer(ruby, args[0])?;
    dispatch(
        ruby,
        format!("audio_fade\t{channel}\t{milliseconds}\n").as_bytes(),
    )?;
    Ok(())
}

fn position(ruby: &Ruby, channel: &str) -> Result<f64, Error> {
    let result = dispatch(ruby, format!("audio_pos\t{channel}\n").as_bytes())?;
    let result = std::str::from_utf8(&result)
        .map_err(|error| Error::new(ruby.exception_runtime_error(), error.to_string()))?;
    result
        .parse::<f64>()
        .map_err(|error| Error::new(ruby.exception_runtime_error(), error.to_string()))
}

fn volume_value(ruby: &Ruby) -> Result<i64, Error> {
    let result = dispatch(ruby, b"audio_volume\tbgm\n")?;
    let result = std::str::from_utf8(&result)
        .map_err(|error| Error::new(ruby.exception_runtime_error(), error.to_string()))?;
    result
        .parse::<i64>()
        .map_err(|error| Error::new(ruby.exception_runtime_error(), error.to_string()))
}

fn set_volume(ruby: &Ruby, value: Value) -> Result<(), Error> {
    let value = volume(ruby, value)?;
    dispatch(ruby, format!("audio_set_volume\tbgm\t{value}\n").as_bytes())?;
    Ok(())
}

fn dispatch(ruby: &Ruby, request: &[u8]) -> Result<Vec<u8>, Error> {
    match engine::native_call_bytes(request) {
        Ok(Some(result)) => Ok(result),
        Ok(None) => {
            let stop: ExceptionClass = ruby.eval("FANPLAYStop")?;
            Err(Error::new(stop, ""))
        }
        Err(error) => Err(Error::new(ruby.exception_runtime_error(), error)),
    }
}

fn integer(ruby: &Ruby, value: Value) -> Result<i64, Error> {
    ruby.module_kernel().funcall("Integer", (value,))
}

fn float(ruby: &Ruby, value: Value) -> Result<f64, Error> {
    ruby.module_kernel().funcall("Float", (value,))
}

fn volume(ruby: &Ruby, value: Value) -> Result<i64, Error> {
    Ok(integer(ruby, value)?.clamp(0, 100))
}

fn arity(ruby: &Ruby, args: &[Value], min: usize, max: usize, method: &str) -> Result<(), Error> {
    if (min..=max).contains(&args.len()) {
        return Ok(());
    }
    Err(Error::new(
        ruby.exception_arg_error(),
        format!(
            "wrong number of arguments for Audio.{method} (given {}, expected {min}..{max})",
            args.len()
        ),
    ))
}

fn bgm_play(ruby: &Ruby, args: &[Value]) -> Result<(), Error> {
    play(ruby, args, "bgm", true)
}
fn bgm_stop(ruby: &Ruby, args: &[Value]) -> Result<(), Error> {
    arity(ruby, args, 0, 1, "bgm_stop")?;
    stop(ruby, "bgm")
}
fn bgm_fade(ruby: &Ruby, args: &[Value]) -> Result<(), Error> {
    fade(ruby, args, "bgm", 2)
}
fn bgm_pos(ruby: &Ruby, args: &[Value]) -> Result<f64, Error> {
    arity(ruby, args, 0, 1, "bgm_pos")?;
    position(ruby, "bgm")
}
fn bgm_volume(ruby: &Ruby) -> Result<i64, Error> {
    volume_value(ruby)
}
fn bgm_set_volume(ruby: &Ruby, value: Value) -> Result<(), Error> {
    set_volume(ruby, value)
}
fn bgs_play(ruby: &Ruby, args: &[Value]) -> Result<(), Error> {
    play(ruby, args, "bgs", false)
}
fn bgs_stop(ruby: &Ruby, args: &[Value]) -> Result<(), Error> {
    arity(ruby, args, 0, 1, "bgs_stop")?;
    stop(ruby, "bgs")
}
fn bgs_fade(ruby: &Ruby, args: &[Value]) -> Result<(), Error> {
    fade(ruby, args, "bgs", 2)
}
fn bgs_pos(ruby: &Ruby, args: &[Value]) -> Result<f64, Error> {
    arity(ruby, args, 0, 1, "bgs_pos")?;
    position(ruby, "bgs")
}
fn me_play(ruby: &Ruby, args: &[Value]) -> Result<(), Error> {
    play(ruby, args, "me", false)
}
fn me_stop(ruby: &Ruby, _args: &[Value]) -> Result<(), Error> {
    stop(ruby, "me")
}
fn me_fade(ruby: &Ruby, args: &[Value]) -> Result<(), Error> {
    fade(ruby, args, "me", usize::MAX)
}
fn se_play(ruby: &Ruby, args: &[Value]) -> Result<(), Error> {
    play(ruby, args, "se", false)
}
fn se_stop(ruby: &Ruby, _args: &[Value]) -> Result<(), Error> {
    stop(ruby, "se")
}
fn setup_midi(ruby: &Ruby) -> Value {
    ruby.qnil().as_value()
}
fn __reset__(ruby: &Ruby) -> Result<(), Error> {
    for channel in ["bgm", "bgs", "me", "se"] {
        stop(ruby, channel)?;
    }
    Ok(())
}
