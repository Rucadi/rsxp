// SPDX-License-Identifier: GPL-2.0-or-later
//! Browser (WebAssembly) frontend for the rsxp engine.
//!
//! The engine core (rendering, archives, input, native dispatcher) is shared
//! with the desktop build. This crate supplies a `Host` that reads game files
//! from a virtual filesystem populated by JavaScript, decodes images with the
//! pure-Rust `image` crate, and rasterizes text with `fontdue`. Ruby runs
//! separately under ruby.wasm in the page and calls [`web_native_call`].
//!
//! This module exposes runtime, input, profiling and framebuffer operations.
//! `host` implements platform services and `files` owns virtual file access;
//! file exports are re-exported here to preserve the frontend API.
use js_sys::Uint8Array;
mod audio;
mod files;
mod host;
mod present;

// Keep the Rust and JavaScript API names stable while isolating platform services.
pub use files::{web_add_archive, web_add_file, web_file_count, web_read_file, web_set_reader};
use host::WebHost;
use rsxp::engine::{self, Runtime, RuntimeOptions, ScriptedKey};
use rsxp::host::InputEvent;
use std::cell::RefCell;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsValue;

thread_local! {
    // Retained storage keeps the frame conversion allocation-free after the
    // first frame at each screen size.
    static FRAMEBUFFER: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static PROFILE: RefCell<WebProfile> = RefCell::new(WebProfile::default());
}

#[derive(Default)]
struct WebProfile {
    enabled: bool,
    native_ms: f64,
    native_calls: u32,
    graphics_ms: f64,
    graphics_calls: u32,
}

fn profile_now() -> f64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map(|performance| performance.now())
        .unwrap_or_else(js_sys::Date::now)
}

/// Creates the engine runtime. Call after adding files and archives.
#[wasm_bindgen]
pub fn web_init(
    width: u32,
    height: u32,
    version: u8,
    rate: u32,
    frame_limit: Option<u32>,
) -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    let archives = files::take_archives();
    let runtime = Runtime::new(RuntimeOptions {
        width,
        height,
        version,
        headless: false,
        frame_limit: frame_limit.map(u64::from),
        rate,
        fast_forward: false,
        archives,
        scripted: Vec::<ScriptedKey>::new(),
        host: Box::new(WebHost::new()),
    })
    .map_err(|e| JsValue::from_str(&e))?;
    engine::set_runtime(runtime);
    Ok(())
}

/// Dispatches one native command. Returns `null` for a clean stop.
#[wasm_bindgen]
pub fn web_native_call(request: &[u8]) -> Result<Option<Box<[u8]>>, JsValue> {
    let operation = request
        .split(|byte| *byte == b'\t' || *byte == b'\n')
        .next()
        .unwrap_or_default();
    let started = PROFILE.with(|profile| profile.borrow().enabled.then(profile_now));
    let result = engine::native_call_bytes(request);
    if let Some(started) = started {
        let elapsed = profile_now() - started;
        PROFILE.with(|profile| {
            let mut profile = profile.borrow_mut();
            profile.native_ms += elapsed;
            profile.native_calls += 1;
            if operation == b"graphics_update" {
                profile.graphics_ms += elapsed;
                profile.graphics_calls += 1;
            }
        });
    }
    match result {
        Ok(Some(bytes)) => Ok(Some(bytes.into_boxed_slice())),
        Ok(None) => Ok(None),
        Err(message) => Err(JsValue::from_str(&message)),
    }
}

/// Enables or disables per-frame profiling for the browser FPS overlay.
#[wasm_bindgen]
pub fn web_profile_set_enabled(enabled: bool) {
    PROFILE.with(|profile| {
        let mut profile = profile.borrow_mut();
        profile.enabled = enabled;
        profile.native_ms = 0.0;
        profile.native_calls = 0;
        profile.graphics_ms = 0.0;
        profile.graphics_calls = 0;
    });
}

/// Returns `[native_ms, native_calls, graphics_update_ms, graphics_update_calls]`
/// since the previous take, then resets the counters.
#[wasm_bindgen]
pub fn web_profile_take() -> Vec<f64> {
    PROFILE.with(|profile| {
        let mut profile = profile.borrow_mut();
        let result = vec![
            profile.native_ms,
            f64::from(profile.native_calls),
            profile.graphics_ms,
            f64::from(profile.graphics_calls),
        ];
        profile.native_ms = 0.0;
        profile.native_calls = 0;
        profile.graphics_ms = 0.0;
        profile.graphics_calls = 0;
        result
    })
}

/// A synchronous zero-copy view of the current framebuffer's RGBA bytes.
#[wasm_bindgen]
pub fn web_framebuffer() -> Uint8Array {
    engine::with_runtime(|runtime| {
        FRAMEBUFFER.with(|framebuffer| {
            let mut framebuffer = framebuffer.borrow_mut();
            runtime.target().write_rgba(&mut framebuffer);
            // SAFETY: the returned view is consumed synchronously by the page's
            // ImageData/putImageData call. The page refreshes the view and its
            // ImageData if the wasm memory buffer or byte offset changes.
            unsafe { Uint8Array::view(&framebuffer) }
        })
    })
    .unwrap_or_else(|| Uint8Array::new(&JsValue::NULL))
}

#[wasm_bindgen]
pub fn web_width() -> u32 {
    engine::with_runtime(|runtime| runtime.target().width()).unwrap_or(0)
}

#[wasm_bindgen]
pub fn web_height() -> u32 {
    engine::with_runtime(|runtime| runtime.target().height()).unwrap_or(0)
}

#[wasm_bindgen]
pub fn web_key(scancode: i32, down: bool) {
    engine::with_runtime_mut(|runtime| {
        runtime.queue_input(if down {
            InputEvent::KeyDown(scancode)
        } else {
            InputEvent::KeyUp(scancode)
        });
    });
}

#[wasm_bindgen]
pub fn web_text(text: &str) {
    engine::with_runtime_mut(|runtime| runtime.queue_input(InputEvent::Text(text.to_string())));
}

#[wasm_bindgen]
pub fn web_mouse(x: i32, y: i32, inside: bool) {
    engine::with_runtime_mut(|runtime| runtime.queue_input(InputEvent::Mouse { x, y, inside }));
}

#[wasm_bindgen]
pub fn web_scroll(delta: i32) {
    engine::with_runtime_mut(|runtime| runtime.queue_input(InputEvent::Scroll(delta)));
}

// ---------------------------------------------------------------------------
// wgpu presentation and the egui workspace.
// ---------------------------------------------------------------------------

/// Sets up the wgpu surface and the egui workspace for `canvas`. Returns the
/// adapter name. On the web wgpu uses WebGPU where available and WebGL2
/// otherwise, so this works even with the GPU disabled (software WebGL).
#[wasm_bindgen]
pub async fn web_init_gpu(
    canvas: web_sys::HtmlCanvasElement,
    width: u32,
    height: u32,
    pixels_per_point: f32,
) -> Result<String, JsValue> {
    let mut presenter = present::Presenter::new(canvas, (width, height))
        .await
        .map_err(|error| JsValue::from_str(&error))?;
    presenter.set_pixels_per_point(pixels_per_point);
    let name = presenter.adapter_name().to_string();
    present::install(presenter);
    Ok(name)
}

/// Whether the egui workspace presenter is active.
#[wasm_bindgen]
pub fn web_gpu_active() -> bool {
    present::is_active()
}

/// Uploads the current frame and draws the game plus the egui workspace.
#[wasm_bindgen]
pub fn web_present_gpu() -> Result<(), JsValue> {
    let result = engine::with_runtime(|runtime| {
        present::with_presenter(|presenter| presenter.present(runtime.target()))
    });
    match result {
        Some(Some(Ok(()))) => Ok(()),
        Some(Some(Err(error))) => Err(JsValue::from_str(&error)),
        _ => Ok(()),
    }
}

#[wasm_bindgen]
pub fn web_ui_resize(width: u32, height: u32) {
    present::with_presenter(|presenter| presenter.set_size((width, height)));
}

#[wasm_bindgen]
pub fn web_ui_pixels_per_point(value: f32) {
    present::with_presenter(|presenter| presenter.set_pixels_per_point(value));
}

#[wasm_bindgen]
pub fn web_ui_pointer(x: f32, y: f32) {
    present::with_presenter(|presenter| presenter.queue_pointer(x, y));
}

#[wasm_bindgen]
pub fn web_ui_button(button: u8, pressed: bool, x: f32, y: f32) {
    let button = match button {
        0 => egui::PointerButton::Primary,
        1 => egui::PointerButton::Middle,
        2 => egui::PointerButton::Secondary,
        _ => return,
    };
    present::with_presenter(|presenter| presenter.queue_button(button, pressed, x, y));
}

#[wasm_bindgen]
pub fn web_ui_scroll(delta: f32) {
    present::with_presenter(|presenter| presenter.queue_scroll(delta));
}

#[wasm_bindgen]
pub fn web_ui_key(code: &str, pressed: bool) {
    if let Some(key) = present::egui_key_from_code(code) {
        present::with_presenter(|presenter| presenter.queue_key(key, pressed));
    }
}

/// Shows (or, with `total == 0`, clears) the preload progress screen.
#[wasm_bindgen]
pub fn web_ui_loading(done: u32, total: u32) {
    present::with_presenter(|presenter| presenter.set_loading(done, total));
}

/// The game view rect in physical canvas pixels (x, y, w, h), so the page can
/// forward pointer input to the game only when it is over the game view.
#[wasm_bindgen]
pub fn web_ui_game_rect() -> Vec<u32> {
    present::with_presenter(|presenter| {
        let (x, y, width, height) = presenter.game_rect();
        vec![x, y, width, height]
    })
    .unwrap_or_default()
}

/// Uses the same nonblocking command queue as the desktop workspace.
#[wasm_bindgen]
pub fn web_game_request(operation: &str, args: Vec<String>) -> Result<u64, JsValue> {
    rsxp::api_bridge::send_from_ui(operation, args).map_err(|error| JsValue::from_str(&error))
}

/// Takes a completed JSON reply; absent while the game has not processed it.
#[wasm_bindgen]
pub fn web_game_response(id: u64) -> Result<Option<String>, JsValue> {
    rsxp::api_bridge::take_ui_response(id)
        .transpose()
        .map_err(|error| JsValue::from_str(&error))
}

/// Keeps crash diagnostics visible without changing the game rendering path.
#[wasm_bindgen]
pub fn web_ui_game_stopped(message: &str) {
    present::with_presenter(|presenter| presenter.game_stopped(message));
}
#[wasm_bindgen]
pub fn web_ui_log(text: &str) {
    present::with_presenter(|presenter| presenter.log(text));
}
/// A stopped game cannot service its Input.update reset hook; let the page restart it.
#[wasm_bindgen]
pub fn web_ui_take_reset() -> bool {
    let requested = rsxp::runtime_settings::reset_requested();
    if requested { rsxp::runtime_settings::clear_reset(); }
    requested
}

/// Import a game-folder patch using the same catalog as the desktop loader.
#[wasm_bindgen]
pub fn web_register_user_patch(name: &str, source: &str) -> bool {
    rsxp::runtime_patch::register_user_patch(name, source.into())
}

#[wasm_bindgen]
pub fn web_controls() -> String { rsxp::controls::encode() }
#[wasm_bindgen]
pub fn web_controls_load(value: &str) { rsxp::controls::decode(value); }
#[wasm_bindgen]
pub fn web_controllers_connected(count: usize) {
    let mut mapping = rsxp::controls::get();
    mapping.connected = count;
    rsxp::controls::set(mapping);
}
