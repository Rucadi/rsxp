// SPDX-License-Identifier: GPL-2.0-or-later
//! Typed desktop sprite synchronization. Ruby getters remain authoritative,
//! including overrides; only conversion and snapshot submission move to Rust.
use magnus::{
    function, prelude::*, value::LazyId, Error, ExceptionClass, RArray, RString, Ruby, TryConvert,
    Value,
};
use rsxp::{
    engine::{self, SpriteState},
    graphics::{Color, Rect, Tone},
};

macro_rules! get {
    ($object:expr, $name:literal) => {{
        static ID: LazyId = LazyId::new($name);
        $object.funcall(*ID, ())?
    }};
}
pub(super) fn int(ruby: &Ruby, value: Value) -> Result<i64, Error> {
    if RString::from_value(value).is_some() {
        ruby.module_kernel().funcall("Integer", (value,))
    } else {
        i64::try_convert(value)
    }
}
fn float(ruby: &Ruby, value: Value) -> Result<f64, Error> {
    if RString::from_value(value).is_some() {
        ruby.module_kernel().funcall("Float", (value,))
    } else {
        f64::try_convert(value)
    }
}
pub(super) fn channel(ruby: &Ruby, value: Value) -> Result<u8, Error> {
    let value = float(ruby, value)?;
    if !value.is_finite() {
        return Err(Error::new(
            ruby.exception_float_domain_error(),
            "non-finite channel",
        ));
    }
    Ok(value.clamp(0.0, 255.0) as u8)
}
fn rectangle(value: Value) -> Result<Rect, Error> {
    let values = RArray::try_convert(value)?;
    Ok(Rect::new(
        values.entry(0)?,
        values.entry(1)?,
        values.entry(2)?,
        values.entry(3)?,
    ))
}
// Read the Ruby argument stack directly; no intermediate snapshot Array.
fn argument<T: TryConvert>(args: &[Value], index: usize) -> Result<T, Error> {
    T::try_convert(args[index])
}
// Games occasionally leave a sprite property unset (nil). RGSS reads such
// properties as their defaults, so a single stray nil must not abort the frame.
fn int_or(ruby: &Ruby, value: Value, default: i64) -> Result<i64, Error> {
    if value.is_nil() {
        Ok(default)
    } else {
        int(ruby, value)
    }
}
fn float_or(ruby: &Ruby, value: Value, default: f64) -> Result<f64, Error> {
    if value.is_nil() {
        Ok(default)
    } else {
        float(ruby, value)
    }
}
fn channel_or(ruby: &Ruby, value: Value, default: u8) -> Result<u8, Error> {
    if value.is_nil() {
        Ok(default)
    } else {
        channel(ruby, value)
    }
}
fn bool_or(value: Value, default: bool) -> Result<bool, Error> {
    if value.is_nil() {
        Ok(default)
    } else {
        bool::try_convert(value)
    }
}
fn sync(ruby: &Ruby, args: &[Value]) -> Result<(), Error> {
    // self, id, blend, zoom_x, zoom_y, bitmap, viewport, pattern, visible,
    // src x/y/w/h, x, y, ox, oy, angle, mirror, z, opacity, invert,
    // pattern blend/tile/opacity/scroll x/y/zoom x/y, color rgba, tone rgbg,
    // bush depth/opacity.
    if args.len() != 39 {
        return Err(Error::new(
            ruby.exception_arg_error(),
            "invalid sprite snapshot",
        ));
    }
    crate::runtime::direct_call("sprite_sync_native", || {
        let id: u64 = argument(args, 1)?;
        let blend: Value = argument(args, 2)?;
        let blend = if blend.is_nil() {
            ruby.integer_from_i64(0).as_value()
        } else {
            blend
        };
        if !blend.equal(ruby.integer_from_i64(0))?
            && !blend.equal(ruby.integer_from_i64(1))?
            && !blend.equal(ruby.integer_from_i64(2))?
        {
            return Err(Error::new(
                ruby.exception_not_imp_error(),
                "unsupported sprite blend type",
            ));
        }
        let zoom_x = float_or(ruby, argument(args, 3)?, 1.0)?;
        let zoom_y = float_or(ruby, argument(args, 4)?, 1.0)?;
        if !zoom_x.is_finite() || !zoom_y.is_finite() {
            return Err(Error::new(
                ruby.exception_float_domain_error(),
                "non-finite sprite zoom",
            ));
        }
        let bitmap: Value = argument(args, 5)?;
        let viewport: Value = argument(args, 6)?;
        let pattern: Value = argument(args, 7)?;
        let visible: bool = bool_or(argument(args, 8)?, false)?;
        let bitmap_disposed: bool = if bitmap.to_bool() {
            get!(bitmap, "disposed?")
        } else {
            false
        };
        let mut visible = visible && bitmap.to_bool() && !bitmap_disposed;
        let clip = if viewport.to_bool() {
            let viewport_visible: bool = get!(viewport, "visible");
            let disposed: bool = get!(viewport, "disposed?");
            visible &= viewport_visible && !disposed;
            {
                let rect: Value = get!(viewport, "rect");
                rectangle(get!(rect, "to_a"))?
            }
        } else {
            let graphics: Value = ruby.class_object().const_get("Graphics")?;
            Rect::new(0, 0, get!(graphics, "width"), get!(graphics, "height"))
        };
        if pattern.to_bool() {
            let disposed: bool = get!(pattern, "disposed?");
            if disposed {
                let class: ExceptionClass = ruby.class_object().const_get("RGSSError")?;
                return Err(Error::new(class, "disposed pattern Bitmap"));
            }
        }
        let state = SpriteState {
            clip,
            zoom_x,
            zoom_y,
            visible,
            bitmap: if bitmap.to_bool() {
                get!(bitmap, "__id")
            } else {
                0
            },
            src: Rect::new(
                int_or(ruby, argument(args, 9)?, 0)? as i32,
                int_or(ruby, argument(args, 10)?, 0)? as i32,
                int_or(ruby, argument(args, 11)?, 0)?.max(0) as u32,
                int_or(ruby, argument(args, 12)?, 0)?.max(0) as u32,
            ),
            x: int_or(ruby, argument(args, 13)?, 0)? as f64,
            y: int_or(ruby, argument(args, 14)?, 0)? as f64,
            ox: int_or(ruby, argument(args, 15)?, 0)? as f64,
            oy: int_or(ruby, argument(args, 16)?, 0)? as f64,
            angle: float_or(ruby, argument(args, 17)?, 0.0)?,
            mirror: bool_or(argument(args, 18)?, false)?,
            z: i32::try_convert(
                ruby.integer_from_i64(int_or(ruby, argument(args, 19)?, 0)?)
                    .as_value(),
            )?,
            opacity: channel_or(ruby, argument(args, 20)?, 255)?,
            blend_type: i32::try_convert(blend)?,
            color: Color::rgba(
                channel_or(ruby, argument(args, 29)?, 0)?,
                channel_or(ruby, argument(args, 30)?, 0)?,
                channel_or(ruby, argument(args, 31)?, 0)?,
                channel_or(ruby, argument(args, 32)?, 255)?,
            ),
            tone: Tone {
                red: int_or(ruby, argument(args, 33)?, 0)? as i32,
                green: int_or(ruby, argument(args, 34)?, 0)? as i32,
                blue: int_or(ruby, argument(args, 35)?, 0)? as i32,
                gray: int_or(ruby, argument(args, 36)?, 0)? as i32,
            },
            invert: bool_or(argument(args, 21)?, false)?,
            bush_depth: int_or(ruby, argument(args, 37)?, 0)?
                .max(0)
                .min(i64::from(i32::MAX)) as i32,
            bush_opacity: channel_or(ruby, argument(args, 38)?, 128)?,
            pattern: if pattern.to_bool() {
                get!(pattern, "__id")
            } else {
                0
            },
            pattern_blend: i32::try_convert(
                ruby.integer_from_i64(int_or(ruby, argument(args, 22)?, 0)?)
                    .as_value(),
            )?,
            pattern_tile: bool_or(argument(args, 23)?, false)?,
            pattern_opacity: channel_or(ruby, argument(args, 24)?, 255)?,
            pattern_scroll_x: int_or(ruby, argument(args, 25)?, 0)? as f64,
            pattern_scroll_y: int_or(ruby, argument(args, 26)?, 0)? as f64,
            pattern_zoom_x: float_or(ruby, argument(args, 27)?, 1.0)?,
            pattern_zoom_y: float_or(ruby, argument(args, 28)?, 1.0)?,
            viewport: if viewport.to_bool() {
                get!(viewport, "__id")
            } else {
                0
            },
        };
        // No Ruby calls while the runtime is borrowed: conversions above may
        // trigger GC and resource finalizers that reenter the command bridge.
        let world_bound = super::world::bound(id);
        engine::with_runtime_mut(|runtime| {
            if !world_bound {
                runtime.clear_world_tile(id);
            }
            runtime.sync_sprite(id, state)
        })
        .unwrap_or_else(|| Err("runtime is closed".into()))
        .map_err(|message| Error::new(ruby.exception_runtime_error(), message))
    })
}
pub(super) fn bind(ruby: &Ruby) -> Result<(), Error> {
    if std::env::var("RSXP_SPRITE_SYNC").as_deref() != Ok("ruby") {
        ruby.define_global_function("__rsxp_sprite_sync", function!(sync, -1))?;
    }
    Ok(())
}
