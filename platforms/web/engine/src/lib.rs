// SPDX-License-Identifier: GPL-2.0-or-later
//! Browser (WebAssembly) frontend for the rsxp engine.
//!
//! The engine core (rendering, archives, input, native dispatcher) is shared
//! with the desktop build. This crate supplies a `Host` that reads game files
//! from a virtual filesystem populated by JavaScript, decodes images with the
//! pure-Rust `image` crate, and rasterizes text with `fontdue`. Ruby runs
//! separately under ruby.wasm in the page and calls [`web_native_call`].
//!
//! This module exposes runtime and input operations.
//! `host` implements platform services and `files` owns virtual file access;
//! file exports are re-exported here to preserve the frontend API.
//!
//! Lifecycle: after files/archives are added, `web_init` builds the runtime;
//! `web_native_call` dispatches Ruby native commands while `web_key`/`web_text`/
//! `web_mouse`/`web_scroll` queue input. eframe owns the canvas, presentation
//! and UI input;
//! this module supplies game frames and browser-specific file services.
mod audio;
mod cartridge;
mod files;
mod host;

// Keep the Rust and JavaScript API names stable while isolating platform services.
pub use cartridge::{
    web_core_fast_forward, web_core_init, web_core_key, web_core_pause, web_core_pointer,
    web_core_pointer_button, web_core_present, web_core_reset, web_core_size, web_core_step,
    web_core_stop, web_core_take_messages,
};
pub use files::{web_add_archive, web_add_file, web_file_count, web_read_file, web_set_reader};
use host::WebHost;
use js_sys::Uint8Array;
use rsxp::engine::{self, Runtime, RuntimeOptions, ScriptedKey};
use rsxp::graphics::Bitmap;
use rsxp::host::InputEvent;
use std::cell::RefCell;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsValue;

thread_local! {
    static WEB_RUNNER: RefCell<Option<eframe::WebRunner>> = const { RefCell::new(None) };
    static PERFORMANCE: RefCell<Performance> = const { RefCell::new(Performance::new()) };
    // Retained storage keeps frame conversion allocation-free after the first
    // frame at each screen size (headless checks and canvas fallback paths).
    static FRAMEBUFFER: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Milliseconds since page load, or zero when the browser clock is unavailable.
pub(crate) fn now_seconds() -> f64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map(|performance| performance.now() / 1000.0)
        .unwrap_or(0.0)
}

struct Performance {
    last_frames: u64,
    window_start: f64,
    window_frames: u64,
    fps: f64,
}

impl Performance {
    const fn new() -> Self {
        Self {
            last_frames: 0,
            window_start: 0.0,
            window_frames: 0,
            fps: 0.0,
        }
    }

    /// Samples frames over a half-second window. Calling this more often than
    /// frames arrive is harmless; duplicate presents do not count.
    fn sample(&mut self, frames: u64, now: f64) -> f64 {
        if frames < self.last_frames || self.window_start == 0.0 {
            self.last_frames = frames;
            self.window_start = now;
            self.window_frames = 0;
        } else if frames > self.last_frames {
            self.window_frames += frames - self.last_frames;
            self.last_frames = frames;
        }
        let elapsed = now - self.window_start;
        if elapsed >= 0.5 {
            self.fps = self.window_frames as f64 / elapsed;
            self.window_start = now;
            self.window_frames = 0;
        }
        self.fps
    }
}

/// Uploads one completed frame and updates the workspace metrics. Shared by
/// the RGSS runtime and cartridge cores so both report the same FPS counter.
pub(crate) fn present_bitmap(bitmap: &Bitmap, frames: u64) {
    let now = now_seconds();
    let fps = PERFORMANCE.with(|performance| performance.borrow_mut().sample(frames, now));
    let _ = with_eframe_app(|app| {
        app.set_logical_size((bitmap.width(), bitmap.height()));
        app.update_bitmap(bitmap);
        app.set_performance(fps, frames);
        app.request_repaint();
    });
}

/// Starts the shared eframe application on the browser canvas.
#[wasm_bindgen]
pub async fn web_init_eframe(canvas: web_sys::HtmlCanvasElement) -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    let runner = eframe::WebRunner::new();
    let creator: eframe::AppCreator<'static> = Box::new(|_| {
        let mut app = rsxp_ui::SharedApp::default();
        let mut panel = rsxp::ui::session::SessionPanel::default();
        panel.capabilities = rsxp::ui::session::SessionCapabilities::BROWSER;
        app.workspace.session = Some(panel);
        Ok(Box::new(app))
    });
    runner
        .start(canvas, eframe::WebOptions::default(), creator)
        .await?;
    WEB_RUNNER.with(|slot| *slot.borrow_mut() = Some(runner));
    Ok(())
}

fn with_eframe_app<R>(f: impl FnOnce(&mut rsxp_ui::SharedApp) -> R) -> Option<R> {
    WEB_RUNNER.with(|slot| {
        let slot = slot.borrow();
        let runner = slot.as_ref()?;
        let mut app = runner.app_mut::<rsxp_ui::SharedApp>()?;
        Some(f(&mut app))
    })
}

#[wasm_bindgen]
pub fn web_eframe_present() {
    let _ = engine::with_runtime(|runtime| {
        present_bitmap(runtime.target(), runtime.frame_count());
    });
}

#[wasm_bindgen]
pub fn web_eframe_launcher(title: &str, status: &str, busy: bool) {
    let _ = with_eframe_app(|app| {
        let panel = app.workspace.session.get_or_insert_with(Default::default);
        panel.capabilities = rsxp::ui::session::SessionCapabilities::BROWSER;
        panel.status = status.into();
        panel.busy = busy;
        panel.recent = if title.is_empty() {
            Vec::new()
        } else {
            vec![title.into()]
        };
        app.request_repaint();
    });
}

#[wasm_bindgen]
pub fn web_eframe_launcher_actions() -> String {
    with_eframe_app(|app| {
        let actions = std::mem::take(&mut app.workspace.session_actions);
        serde_json::json!({"open": actions.open, "browse": actions.browse,
            "browse_game_file": actions.browse_game_file,
            "zip": actions.import_zip, "download": actions.download, "cancel": actions.cancel})
        .to_string()
    })
    .unwrap_or_else(|| "{}".into())
}

#[wasm_bindgen]
pub fn web_eframe_finish_launcher() {
    let _ = with_eframe_app(|app| {
        app.workspace.session = None;
        app.workspace.session_actions = Default::default();
        app.request_repaint();
    });
}

#[wasm_bindgen]
pub fn web_eframe_loading(done: u32, total: u32) {
    let _ = with_eframe_app(|app| {
        app.workspace.set_loading(done, total);
        app.request_repaint();
    });
}

#[wasm_bindgen]
pub fn web_eframe_game_stopped(message: &str) {
    let _ = with_eframe_app(|app| {
        app.workspace.game_stopped(message);
        app.request_repaint();
    });
}

#[wasm_bindgen]
pub fn web_eframe_log(text: &str) {
    let _ = with_eframe_app(|app| {
        if app.workspace.logs.len() >= 300 {
            app.workspace.logs.remove(0);
        }
        app.workspace.logs.push(text.to_owned());
        app.request_repaint();
    });
}

#[wasm_bindgen]
pub fn web_eframe_game_rect() -> Vec<f32> {
    with_eframe_app(|app| {
        let r = app.game_rect();
        vec![r.min.x, r.min.y, r.width(), r.height()]
    })
    .unwrap_or_default()
}

/// True while the shared workspace (search box, patch editor, API tool) owns
/// the keyboard, so the page does not also feed those keys to the game.
#[wasm_bindgen]
pub fn web_eframe_wants_keyboard() -> bool {
    with_eframe_app(|app| app.wants_keyboard()).unwrap_or(false)
}

#[wasm_bindgen]
pub fn web_eframe_take_reset() -> bool {
    let requested = rsxp::runtime_settings::reset_requested();
    if requested {
        rsxp::runtime_settings::clear_reset();
    }
    requested
}

#[wasm_bindgen]
pub fn web_eframe_copy_runtime_output() {
    let _ = with_eframe_app(|app| {
        web_sys::window().and_then(|window| {
            let text = app.workspace.logs.join("\n");
            window
                .get("__rsxp_copyText")?
                .dyn_into::<js_sys::Function>()
                .ok()
                .and_then(|function| {
                    function
                        .call1(&JsValue::NULL, &JsValue::from_str(&text))
                        .ok()
                })
        })
    });
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
    let result = engine::native_call_bytes(request);
    match result {
        Ok(Some(bytes)) => Ok(Some(bytes.into_boxed_slice())),
        Ok(None) => Ok(None),
        Err(message) => Err(JsValue::from_str(&message)),
    }
}

/// A synchronous zero-copy view of the current framebuffer's RGBA bytes.
///
/// Kept for headless verification and hosts that read the CPU frame directly.
/// The page's normal path uploads it through the shared eframe texture.
#[wasm_bindgen]
pub fn web_framebuffer() -> Uint8Array {
    engine::with_runtime(|runtime| {
        FRAMEBUFFER.with(|framebuffer| {
            let mut framebuffer = framebuffer.borrow_mut();
            runtime.target().write_rgba(&mut framebuffer);
            // SAFETY: the returned view is consumed synchronously by the
            // caller. Refreshing the view after wasm memory growth is the
            // caller's responsibility, as in the pre-eframe canvas path.
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

/// Import a game-folder patch using the same catalog as the desktop loader.
#[wasm_bindgen]
pub fn web_register_user_patch(name: &str, source: &str) -> bool {
    rsxp::runtime_patch::register_user_patch(name, source.into())
}

#[wasm_bindgen]
pub fn web_controls() -> String {
    rsxp::controls::encode()
}
#[wasm_bindgen]
pub fn web_controls_load(value: &str) {
    rsxp::controls::decode(value);
}
#[wasm_bindgen]
pub fn web_controllers_connected(count: usize) {
    let mut mapping = rsxp::controls::get();
    mapping.connected = count;
    rsxp::controls::set(mapping);
}
