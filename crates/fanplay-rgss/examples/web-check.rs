// SPDX-License-Identifier: GPL-2.0-or-later
//! Headless web engine used by the Ruby WASM regression/benchmark tools.
#![cfg(target_arch = "wasm32")]
use fanplay_rgss::{
    engine::{self, Runtime, RuntimeOptions},
    graphics::{Bitmap, Color},
    host::Host,
};
use wasm_bindgen::prelude::*;

struct CheckHost;
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = globalThis)]
    fn __fanplay_check_read(path: &str) -> JsValue;
}
impl Host for CheckHost {
    fn read_file(&self, path: &str) -> Option<Vec<u8>> {
        let bytes = __fanplay_check_read(path);
        if bytes.is_null() || bytes.is_undefined() {
            return None;
        }
        Some(js_sys::Uint8Array::new(&bytes).to_vec())
    }
    fn image(&self, bytes: &[u8]) -> Result<Bitmap, String> {
        fanplay_rgss::portable_media::image(bytes)
    }
    fn text(
        &self,
        font: Option<&[u8]>,
        size: i32,
        bold: bool,
        italic: bool,
        text: &str,
        color: Color,
        render: bool,
    ) -> Result<(u32, u32, Option<Bitmap>), String> {
        fanplay_rgss::portable_media::text(font, size, bold, italic, text, color, render)
    }
    fn now(&self) -> f64 {
        js_sys::Date::now() / 1000.0
    }
}

#[wasm_bindgen]
pub fn init() -> Result<(), JsValue> {
    engine::take_runtime();
    let runtime = Runtime::new(RuntimeOptions {
        width: 160,
        height: 120,
        version: 1,
        rate: 40,
        headless: true,
        frame_limit: None,
        fast_forward: true,
        archives: vec![],
        scripted: vec![],
        host: Box::new(CheckHost),
    })
    .map_err(|error| JsValue::from_str(&error))?;
    engine::set_runtime(runtime);
    Ok(())
}

#[wasm_bindgen]
pub fn native_call(request: &[u8]) -> Result<Option<Vec<u8>>, JsValue> {
    engine::native_call_bytes(request).map_err(|error| JsValue::from_str(&error))
}

#[wasm_bindgen]
pub fn framebuffer() -> Vec<u8> {
    engine::with_runtime(|runtime| runtime.target().to_rgba()).unwrap_or_default()
}
