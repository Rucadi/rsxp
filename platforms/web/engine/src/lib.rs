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
use rsxp_extension_api::{OperationError, OperationRequest, OperationResponse};
use std::cell::RefCell;
use std::collections::HashMap;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsValue;

thread_local! {
    static WEB_RUNNER: RefCell<Option<eframe::WebRunner>> = const { RefCell::new(None) };
    static PERFORMANCE: RefCell<Performance> = const { RefCell::new(Performance::new()) };
    // Retained storage keeps frame conversion allocation-free after the first
    // frame at each screen size (headless checks and canvas fallback paths).
    static FRAMEBUFFER: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static EXTENSION_BRIDGE: RefCell<HashMap<u64, (u64, u64)>> = RefCell::new(HashMap::new());
    static PLUGIN_HTTP_BRIDGE: RefCell<HashMap<u64, Result<u64, (u16, String)>>> =
        RefCell::new(HashMap::new());
    static EXTENSIONS_ACTIVATED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
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
        panel.api_bridge = true;
        panel.api_status = "Browser bridge · no TCP listener".into();
        app.set_save_buttons(true);
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

fn pump_extensions() {
    let requests = with_eframe_app(|app| app.take_extension_requests()).unwrap_or_default();
    for request in requests {
        if let Some(plugin_id) = request.operation.strip_suffix(".__webview.open") {
            if let (Some(view_id), Some(title), Some(html)) = (
                request
                    .arguments
                    .get("view_id")
                    .and_then(serde_json::Value::as_str),
                request
                    .arguments
                    .get("title")
                    .and_then(serde_json::Value::as_str),
                request
                    .arguments
                    .get("html")
                    .and_then(serde_json::Value::as_str),
            ) {
                browser_webview_call(
                    "__rsxp_openPluginWebview",
                    serde_json::json!({
                        "plugin_id": plugin_id,
                        "view_id": view_id,
                        "title": title,
                        "html": html,
                    })
                    .to_string(),
                );
            }
            let _ = with_eframe_app(|app| {
                app.dispatch_extension_response(OperationResponse {
                    id: request.id,
                    generation: request.generation,
                    result: Ok(serde_json::json!({"opened": true})),
                })
            });
            continue;
        }
        if let Some(plugin_id) = request.operation.strip_suffix(".__webview.close") {
            if let Some(view_id) = request
                .arguments
                .get("view_id")
                .and_then(serde_json::Value::as_str)
            {
                browser_webview_call(
                    "__rsxp_closePluginWebview",
                    serde_json::json!({"plugin_id": plugin_id, "view_id": view_id}).to_string(),
                );
            }
            let _ = with_eframe_app(|app| {
                app.dispatch_extension_response(OperationResponse {
                    id: request.id,
                    generation: request.generation,
                    result: Ok(serde_json::json!({"closed": true})),
                })
            });
            continue;
        }
        let Some((operation, args)) = legacy_extension_call(&request) else {
            let _ = with_eframe_app(|app| {
                app.dispatch_extension_response(OperationResponse {
                    id: request.id,
                    generation: request.generation,
                    result: Err(OperationError::new(
                        "unsupported_operation",
                        request.operation,
                    )),
                })
            });
            continue;
        };
        match rsxp::api_bridge::send_from_ui(operation, args) {
            Ok(bridge_id) => EXTENSION_BRIDGE.with(|pending| {
                pending
                    .borrow_mut()
                    .insert(bridge_id, (request.id, request.generation));
            }),
            Err(error) => {
                let _ = with_eframe_app(|app| {
                    app.dispatch_extension_response(OperationResponse {
                        id: request.id,
                        generation: request.generation,
                        result: Err(OperationError::new("queue", error)),
                    })
                });
            }
        }
    }

    let pending: Vec<_> =
        EXTENSION_BRIDGE.with(|pending| pending.borrow().keys().copied().collect());
    for bridge_id in pending {
        let Some(reply) = rsxp::api_bridge::take_ui_response(bridge_id) else {
            continue;
        };
        let Some((id, generation)) =
            EXTENSION_BRIDGE.with(|pending| pending.borrow_mut().remove(&bridge_id))
        else {
            continue;
        };
        let result = match reply {
            Ok(body) => serde_json::from_str(&body)
                .map_err(|error| OperationError::new("invalid_response", error.to_string())),
            Err(error) => Err(OperationError::new("transport", error)),
        };
        let _ = with_eframe_app(|app| {
            app.dispatch_extension_response(OperationResponse {
                id,
                generation,
                result,
            })
        });
    }
}

fn browser_webview_call(function_name: &str, payload: String) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Ok(function) = js_sys::Reflect::get(&window, &JsValue::from_str(function_name))
        .and_then(|value| value.dyn_into::<js_sys::Function>())
    else {
        web_eframe_log(&format!(
            "browser plugin webview hook {function_name} is unavailable"
        ));
        return;
    };
    let _ = function.call1(&JsValue::NULL, &JsValue::from_str(&payload));
}

/// Queues an HTTP request made by a browser plugin webview. The parent page
/// owns the iframe, while this bridge reuses the same game-thread API queue as
/// the desktop HTTP server.
#[wasm_bindgen]
pub fn web_eframe_plugin_http(
    // Browser plugin request ids come from a JavaScript counter. Keep this
    // binding numeric; wasm-bindgen exposes u64 as BigInt, while the browser
    // host intentionally sends an ordinary safe integer.
    request_id: u32,
    method: &str,
    target: &str,
    _body: &str,
) -> Result<(), JsValue> {
    let route = rsxp::api_bridge::route_http(&method.to_ascii_uppercase(), target);
    let result = match route {
        Ok((operation, args)) => {
            rsxp::api_bridge::send_from_ui(&operation, args).map_err(|error| (500, error))
        }
        Err((status, error)) => Err((status, error)),
    };
    PLUGIN_HTTP_BRIDGE.with(|pending| {
        pending
            .borrow_mut()
            .insert(u64::from(request_id), result);
    });
    Ok(())
}

/// Resolves one game image through the shared Rust backend for a browser
/// plugin webview. The backend crops Pokémon sprite sheets to their first
/// square frame and returns a one-shot data URL; no plugin-side sprite cache
/// is involved.
#[wasm_bindgen]
pub fn web_eframe_plugin_asset(category: &str, name: &str) -> String {
    with_eframe_app(|app| app.plugin_asset_data_url(category, name))
        .flatten()
        .unwrap_or_default()
}

/// Returns completed browser webview HTTP requests as a small JSON array.
#[wasm_bindgen]
pub fn web_eframe_take_plugin_http_responses() -> String {
    let ids =
        PLUGIN_HTTP_BRIDGE.with(|pending| pending.borrow().keys().copied().collect::<Vec<_>>());
    let mut completed = Vec::new();
    for request_id in ids {
        let state = PLUGIN_HTTP_BRIDGE.with(|pending| pending.borrow().get(&request_id).cloned());
        let Some(state) = state else { continue };
        let response = match state {
            Err((status, error)) => Some((status, serde_json::json!({"error": error}).to_string())),
            Ok(bridge_id) => {
                rsxp::api_bridge::take_ui_response(bridge_id).map(|reply| match reply {
                    Ok(body) => (200, body),
                    Err(error) => (500, serde_json::json!({"error": error}).to_string()),
                })
            }
        };
        if let Some((status, body)) = response {
            completed.push(serde_json::json!({"id": request_id, "status": status, "body": body}));
            PLUGIN_HTTP_BRIDGE.with(|pending| {
                pending.borrow_mut().remove(&request_id);
            });
        }
    }
    serde_json::Value::Array(completed).to_string()
}

fn activate_extensions_once() {
    if EXTENSIONS_ACTIVATED.with(std::cell::Cell::get) {
        return;
    }
    if let Some(result) = with_eframe_app(|app| app.activate_extensions()) {
        if let Err(error) = result {
            web_eframe_log(&format!("Extension activation failed: {error}"));
        }
        EXTENSIONS_ACTIVATED.with(|active| active.set(true));
    }
}

fn legacy_extension_call(request: &OperationRequest) -> Option<(&str, Vec<String>)> {
    if request.operation.ends_with(".__ruby.inject") {
        let source = request.arguments.get("source")?.as_str()?;
        let name = request
            .arguments
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("plugin");
        let name = name
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '_' | '-') {
                    character
                } else {
                    '-'
                }
            })
            .collect::<String>();
        let patch_name = format!(
            "rhai-{}-{}",
            if name.is_empty() { "plugin" } else { &name },
            request.id
        );
        return Some((
            "ruby_inject",
            vec![patch_name, rsxp::api_bridge::hex_encode(source)],
        ));
    }
    if request.operation.ends_with(".__ruby.unload") {
        let handle = request.arguments.get("handle")?;
        let handle = handle
            .as_str()
            .map(str::to_owned)
            .or_else(|| {
                handle
                    .as_i64()
                    .filter(|value| *value > 0)
                    .map(|value| format!("rhai-{value}"))
            })?;
        return Some(("ruby_unload", vec![handle]));
    }
    let operation = request.operation.rsplit_once('.')?.1;
    let arguments = request.arguments.as_object()?.values().map(|value| {
        value
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| value.to_string())
    });
    Some((operation, arguments.collect()))
}

#[wasm_bindgen]
pub fn web_eframe_present() {
    let _ = engine::with_runtime(|runtime| {
        present_bitmap(runtime.target(), runtime.frame_count());
    });
    pump_extensions();
}

/// Activates extensions for an RGSS session. Cartridge sessions deliberately
/// do not call this until their cores expose compatible operations.
#[wasm_bindgen]
pub fn web_eframe_activate_extensions() {
    activate_extensions_once();
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
            "zip": actions.import_zip, "download": actions.download, "cancel": actions.cancel,
            "create_backup": actions.create_backup, "restore_latest": actions.restore_latest,
            "export_saves": actions.export_saves, "import_saves": actions.import_saves})
        .to_string()
    })
    .unwrap_or_else(|| "{}".into())
}

/// Returns save and API intents emitted by the shared workspace while a game
/// is running. The page handles export/import and local-storage snapshots; the
/// browser control bridge is always available, so start/stop are informational.
#[wasm_bindgen]
pub fn web_eframe_player_actions() -> String {
    with_eframe_app(|app| {
        let (export_saves, import_saves, create_backup, restore_latest) =
            app.take_save_actions();
        let (open_game_folder, open_save_folder, open_patches_folder) = app.take_folder_actions();
        let actions = std::mem::take(&mut app.workspace.session_actions);
        serde_json::json!({
            "export_saves": export_saves,
            "import_saves": import_saves,
            "create_backup": create_backup,
            "restore_latest": restore_latest,
            "open_game_folder": open_game_folder,
            "open_save_folder": open_save_folder,
            "open_patches_folder": open_patches_folder,
            "start_api": actions.start_api,
            "stop_api": actions.stop_api,
        })
        .to_string()
    })
    .unwrap_or_else(|| "{}".into())
}

/// The same typed operation catalog as the desktop server, exposed through an
/// in-page bridge because a browser cannot bind a TCP listener.
#[wasm_bindgen]
pub fn web_control_discovery() -> String {
    browser_control_catalog()
        .discovery(browser_control_status())
        .to_string()
}

#[wasm_bindgen]
pub fn web_control_openapi() -> String {
    browser_control_catalog()
        .openapi(browser_control_status())
        .to_string()
}

#[wasm_bindgen]
pub fn web_control_routes() -> String {
    rsxp::api_bridge::extension_routes_json()
}

/// Queues a game operation through the shared bridge. The page appends body
/// fields to the query string for the generic Rhai route matcher.
#[wasm_bindgen]
pub fn web_control_request(method: &str, target: &str) -> Result<u32, JsValue> {
    let method = method.to_ascii_uppercase();
    let (operation, arguments) =
        rsxp::api_bridge::route_http(&method, target).map_err(|(_, error)| JsValue::from_str(&error))?;
    rsxp::api_bridge::send_from_ui(&operation, arguments)
        .map(|id| id as u32)
        .map_err(|error| JsValue::from_str(&error))
}

#[wasm_bindgen]
pub fn web_control_take_response(id: u32) -> String {
    match rsxp::api_bridge::take_ui_response(u64::from(id)) {
        None => serde_json::json!({"ready": false}).to_string(),
        Some(Ok(body)) => {
            let value = serde_json::from_str::<serde_json::Value>(&body)
                .unwrap_or_else(|_| serde_json::json!(body));
            serde_json::json!({"ready": true, "status": 200, "body": value}).to_string()
        }
        Some(Err(error)) => {
            serde_json::json!({"ready": true, "status": 503, "body": {"error": error}}).to_string()
        }
    }
}

fn browser_control_status() -> serde_json::Value {
    let (active, paused, title, source) = with_eframe_app(|app| {
        let panel = app.workspace.session.as_ref();
        (
            app.workspace.session.as_ref().is_some_and(|panel| panel.active),
            panel.is_some_and(|panel| panel.paused),
            panel.map(|panel| panel.title.clone()).unwrap_or_default(),
            panel.map(|panel| panel.source.clone()).unwrap_or_default(),
        )
    })
    .unwrap_or_default();
    serde_json::json!({
        "server": "rsxp-control",
        "mode": "browser",
        "base_url": "browser://rsxp",
        "game_running": active,
        "game_available": active,
        "launcher_available": false,
        "paused": paused,
        "game": { "title": title, "source": source },
        "notes": "Browsers cannot bind a loopback TCP listener. Use window.RSXP.control.request() for the same catalog. Save snapshots and settings are stored in browser local storage."
    })
}

fn browser_control_catalog() -> rsxp_control::Catalog {
    let mut catalog = rsxp_control::Catalog::standard();
    // The browser cannot launch external files or open native folders, but the
    // launcher pause/live endpoints remain meaningful and the rest are reported
    // as unavailable by the request handler.
    catalog.remove_scope(rsxp_control::Scope::Saves);
    catalog.remove_scope(rsxp_control::Scope::Settings);
    // Keep the browser-specific save and settings operations documented so an
    // external caller still sees the same shape.
    catalog.extend(browser_save_endpoints());
    catalog.extend(browser_settings_endpoints());
    let routes = serde_json::from_str::<serde_json::Value>(
        &rsxp::api_bridge::extension_routes_json(),
    )
    .ok();
    if let Some(routes) = routes
        && let Some(routes) = routes.get("routes").and_then(serde_json::Value::as_array)
    {
        catalog.merge_dynamic(routes.iter().filter_map(|route| {
            let method = route.get("method")?.as_str()?;
            let path = route.get("path")?.as_str()?;
            let operation = route.get("operation")?.as_str()?;
            let mut endpoint = rsxp_control::Endpoint::new(
                operation,
                method,
                path,
                format!("Game operation {operation}"),
                "Route supplied by a loaded Rhai plugin and executed by the browser game bridge.",
                rsxp_control::Scope::Extensions,
            );
            for argument in route
                .get("arguments")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str)
            {
                let location = if path.contains(&format!("{{{argument}}}")) {
                    rsxp_control::ParameterLocation::Path
                } else {
                    rsxp_control::ParameterLocation::Query
                };
                endpoint = endpoint.parameter(rsxp_control::Parameter::new(
                    argument,
                    location,
                    rsxp_control::ParameterType::String,
                    location == rsxp_control::ParameterLocation::Path,
                    "Plugin route argument",
                ));
            }
            Some(endpoint)
        }));
    }
    catalog
}

fn browser_save_endpoints() -> Vec<rsxp_control::Endpoint> {
    use rsxp_control::{Endpoint, Parameter, ParameterType, Scope};
    vec![
        Endpoint::new(
            "saves.info",
            "GET",
            "/v1/saves",
            "Browser save bundle",
            "Returns the current game's save files and named snapshots from browser local storage.",
            Scope::Saves,
        ),
        Endpoint::new(
            "saves.backup",
            "POST",
            "/v1/saves/backup",
            "Create a browser save snapshot",
            "Snapshots the current save bundle in browser local storage under a timestamp name.",
            Scope::Saves,
        )
        .mutates()
        .parameter(Parameter::body("name", ParameterType::String, false, "Optional snapshot name")),
        Endpoint::new(
            "saves.restore",
            "POST",
            "/v1/saves/restore",
            "Restore a browser save snapshot",
            "Restores a named snapshot. The page should be reloaded afterwards so Ruby loads the files.",
            Scope::Saves,
        )
        .mutates()
        .parameter(Parameter::body("backup", ParameterType::String, true, "Snapshot name")),
        Endpoint::new(
            "saves.delete",
            "DELETE",
            "/v1/saves/backups/{name}",
            "Delete a browser save snapshot",
            "Deletes one named browser snapshot without touching live save files.",
            Scope::Saves,
        )
        .mutates()
        .parameter(Parameter::path("name", ParameterType::String, "Snapshot name")),
    ]
}

fn browser_settings_endpoints() -> Vec<rsxp_control::Endpoint> {
    use rsxp_control::{Endpoint, Parameter, ParameterType, Scope};
    vec![
        Endpoint::new(
            "settings.common.get",
            "GET",
            "/v1/settings/common",
            "Read browser common settings",
            "Returns settings shared by every browser game.",
            Scope::Settings,
        ),
        Endpoint::new(
            "settings.common.update",
            "PUT",
            "/v1/settings/common",
            "Replace browser common settings",
            "Stores common settings in browser local storage.",
            Scope::Settings,
        )
        .mutates()
        .request(serde_json::json!({"type": "object"})),
        Endpoint::new(
            "settings.game.get",
            "GET",
            "/v1/settings/games/{key}",
            "Read browser game settings",
            "Returns the per-game override and effective settings.",
            Scope::Settings,
        )
        .parameter(Parameter::path("key", ParameterType::String, "Game key")),
        Endpoint::new(
            "settings.game.update",
            "PUT",
            "/v1/settings/games/{key}",
            "Replace browser game settings",
            "Stores a per-game override in browser local storage.",
            Scope::Settings,
        )
        .mutates()
        .parameter(Parameter::path("key", ParameterType::String, "Game key"))
        .parameter(Parameter::body("settings", ParameterType::Object, false, "Settings or null")),
    ]
}

#[wasm_bindgen]
pub fn web_eframe_finish_launcher() {    let _ = with_eframe_app(|app| {
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
    EXTENSIONS_ACTIVATED.with(|active| active.set(false));
    EXTENSION_BRIDGE.with(|pending| pending.borrow_mut().clear());
    PLUGIN_HTTP_BRIDGE.with(|pending| pending.borrow_mut().clear());
    browser_webview_call("__rsxp_closeAllPluginWebviews", "{}".into());
    let _ = with_eframe_app(|app| {
        app.reset_extensions();
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

/// Applies common/per-game fast-forward before the browser session starts.
#[wasm_bindgen]
pub fn web_runtime_fast_forward(enabled: bool) {
    rsxp::runtime_settings::set_fast_forward(enabled);
}

#[wasm_bindgen]
pub fn web_runtime_settings() -> String {
    serde_json::json!({
        "fast_forward": rsxp::runtime_settings::fast_forward(),
        "fps": rsxp::runtime_settings::rate_override(),
    })
    .to_string()
}

#[wasm_bindgen]
pub fn web_runtime_set_rate(fps: u32) {
    if fps == 0 {
        rsxp::runtime_settings::clear_rate_override();
    } else {
        rsxp::runtime_settings::set_rate_override(fps);
    }
}

#[wasm_bindgen]
pub fn web_eframe_game_paused() -> bool {
    with_eframe_app(|app| {
        app.workspace
            .session
            .as_ref()
            .is_some_and(|panel| panel.paused)
    })
    .unwrap_or(false)
}

#[wasm_bindgen]
pub fn web_eframe_set_paused(paused: bool) {
    let _ = with_eframe_app(|app| {
        if let Some(panel) = app.workspace.session.as_mut() {
            panel.paused = paused;
        }
        app.request_repaint();
    });
}

#[wasm_bindgen]
pub fn web_eframe_copy_runtime_output() {    let _ = with_eframe_app(|app| {
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
