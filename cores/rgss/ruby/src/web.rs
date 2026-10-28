// SPDX-License-Identifier: GPL-2.0-or-later
//! Browser Ruby boundary backed by the official ruby.wasm CRuby build.
//!
//! The VM itself lives in JavaScript because ruby.wasm exposes its browser
//! loader there. Rust still owns the RGSS engine, archive lookup, rendering,
//! input and audio; this module only schedules Ruby and transports bytes.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use fanplay_rgss::engine;
use js_sys::{global, Function, Reflect};
use wasm_bindgen::{closure::Closure, JsCast, JsValue};

type NativeCallback = Closure<dyn FnMut(String) -> Result<String, JsValue>>;

pub struct RubyCore {
    native_callback: NativeCallback,
    started: bool,
}

impl RubyCore {
    pub fn prepare() -> Result<Self, String> {
        let callback = Closure::wrap(Box::new(|encoded: String| {
            let request = BASE64.decode(encoded).map_err(|error| {
                JsValue::from_str(&format!("invalid Ruby bridge request: {error}"))
            })?;
            match engine::native_call_bytes(&request) {
                Ok(Some(response)) => Ok(BASE64.encode(response)),
                Ok(None) => Ok("__FANPLAY_STOP__".to_owned()),
                Err(error) => Err(JsValue::from_str(&error)),
            }
        })
            as Box<dyn FnMut(String) -> Result<String, JsValue>>);
        Reflect::set(
            &global(),
            &JsValue::from_str("__fanplay_native_b64"),
            callback.as_ref(),
        )
        .map_err(|error| format!("could not install Ruby bridge: {error:?}"))?;
        Ok(Self {
            native_callback: callback,
            started: false,
        })
    }

    pub fn start_with_config(
        &mut self,
        version: u8,
        scripts: &str,
        title: &str,
    ) -> Result<(), String> {
        if self.started {
            return Ok(());
        }
        call(
            "fanplay_ruby_start",
            &[
                JsValue::from_str(fanplay_rgss::ruby::RGSS_WEB_SOURCE),
                JsValue::from_f64(f64::from(version)),
                JsValue::from_str(scripts),
                JsValue::from_str(title),
            ],
        )?;
        self.started = true;
        Ok(())
    }

    pub fn step(&mut self) -> Result<bool, String> {
        if !self.started {
            return Err("Ruby core has not been started".into());
        }
        let value = call("fanplay_ruby_step", &[])?;
        value
            .as_bool()
            .ok_or_else(|| "ruby.wasm returned a non-boolean step status".to_owned())
    }

    /// Native-core compatibility entry point. Browser sessions normally use
    /// `start_with_config` and `step` so eframe remains responsive.
    pub fn start(&mut self) -> Result<i64, String> {
        self.start_with_config(3, "Data/Scripts.rvdata2", "FANPLAY")?;
        while self.step()? {}
        Ok(0)
    }

    pub fn startup_status(&self) -> Result<i64, String> {
        if self.started {
            Ok(0)
        } else {
            Err("Ruby core has not been started".into())
        }
    }

    pub fn inject_eval(&self, source: &str, _name: &str) -> Result<(), String> {
        call("fanplay_ruby_eval", &[JsValue::from_str(source)]).map(|_| ())
    }

    pub fn extract_i64(&self, expression: &str) -> Result<i64, String> {
        let value = call("fanplay_ruby_eval", &[JsValue::from_str(expression)])?;
        value
            .as_f64()
            .map(|number| number as i64)
            .or_else(|| value.as_string()?.parse().ok())
            .ok_or_else(|| "Ruby expression did not return an integer".to_owned())
    }

    pub fn extract_string(&self, expression: &str) -> Result<String, String> {
        let value = call("fanplay_ruby_eval", &[JsValue::from_str(expression)])?;
        value
            .as_string()
            .or_else(|| value.as_f64().map(|number| number.to_string()))
            .ok_or_else(|| "Ruby expression did not return a string".to_owned())
    }
}

impl Drop for RubyCore {
    fn drop(&mut self) {
        let _ = Reflect::set(
            &global(),
            &JsValue::from_str("__fanplay_native_b64"),
            &JsValue::UNDEFINED,
        );
        let _ = &self.native_callback;
    }
}

fn call(name: &str, args: &[JsValue]) -> Result<JsValue, String> {
    let value = Reflect::get(&global(), &JsValue::from_str(name))
        .map_err(|error| format!("could not find {name}: {error:?}"))?;
    let function = value
        .dyn_into::<Function>()
        .map_err(|_| format!("{name} is not a JavaScript function"))?;
    let arguments = js_sys::Array::new();
    for argument in args {
        arguments.push(argument);
    }
    function
        .apply(&JsValue::UNDEFINED, &arguments)
        .map_err(|error| format!("{name} failed: {error:?}"))
}
