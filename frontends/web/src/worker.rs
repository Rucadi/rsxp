// SPDX-License-Identifier: GPL-2.0-or-later
//! RGSS runs in an isolated browser worker; the window only exchanges messages.
use super::*;
use fanplay_core::{CommandOutcome, CoreError, CoreResult};
use fanplay_extension_api::OperationRequest;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[wasm_bindgen(inline_js = r#"
export function fanplay_clock() { return performance.now() / 1000; }
export function fanplay_worker_create(module, callback, entry, label, paths, buffers, save) {
  const bindings = [...document.querySelectorAll('link[rel="modulepreload"]')]
    .find(link => /\/fanplay-web-[^/]+\.js$/.test(new URL(link.href).pathname));
  const runtime = [...document.scripts].find(script => /ruby_runtime[^/]*\.js$/.test(script.src));
  const loader = [...document.scripts].find(script => script.src.includes('/@ruby/wasm-wasi@'));
  if (!bindings || !runtime || !loader) throw new Error('RGSS worker runtime assets are missing');
  const worker = new Worker(new URL('rgss_worker.js', document.baseURI), { type: 'module' });
  worker.onmessage = event => callback(event.data);
  worker.onerror = event => { event.preventDefault(); callback({type:'error', message:event.message}); };
  const transfers = buffers.map(bytes => bytes.buffer);
  // wasm-bindgen exposes `save` as a Uint8Array view over WebAssembly.Memory.
  // That backing buffer is not detachable, so putting `save.buffer` in the
  // transfer list throws during RGSS restart/import. The save is small and is
  // copied once into an independent transferable buffer; game assets remain
  // zero-copy transferred from the JS-owned Uint8Arrays above.
  const saveBytes = save && save.byteLength ? new Uint8Array(save) : new Uint8Array();
  if (saveBytes.byteLength) transfers.push(saveBytes.buffer);
  worker.postMessage({type:'load', module, entry, label, paths, buffers, save: saveBytes,
    nativeDiagnostics: new URL(globalThis.location.href).searchParams.get('fanplay_native_diagnostics') === '1',
    webviewDiagnostics: new URL(globalThis.location.href).searchParams.get('fanplay_webview_diagnostics') === '1',
    bindings: bindings.href, rubyRuntime: runtime.src, rubyLoader: loader.src,
    rubyWasm: new URL('ruby.wasm', document.baseURI).href}, transfers);
  return worker;
}
export function fanplay_worker_post(worker, json) { worker.postMessage(JSON.parse(json)); }
export function fanplay_worker_ack(worker) { worker.postMessage({type: 'frame_ack'}); }
export function fanplay_worker_terminate(worker) { worker.onmessage = null; worker.onerror = null; worker.terminate(); }
export function fanplay_worker_emit(json, bytes, field) {
  const message = JSON.parse(json);
  if (field) message[field] = new Uint8Array(bytes);
  globalThis.__fanplay_worker_output(message);
}
export function fanplay_worker_emit_frame(width, height, frames, format, bytes) {
  globalThis.__fanplay_worker_output({
    type: 'frame', width, height, frames, format, pixels: new Uint8Array(bytes)
  });
}
"#)]
extern "C" {
    fn fanplay_clock() -> f64;
    #[wasm_bindgen(catch)]
    fn fanplay_worker_create(
        module: JsValue,
        callback: &js_sys::Function,
        entry: &str,
        label: &str,
        paths: &js_sys::Array,
        buffers: &js_sys::Array,
        save: &[u8],
    ) -> Result<JsValue, JsValue>;
    fn fanplay_worker_post(worker: &JsValue, json: &str);
    fn fanplay_worker_ack(worker: &JsValue);
    fn fanplay_worker_terminate(worker: &JsValue);
    fn fanplay_worker_emit(json: &str, bytes: &[u8], field: &str);
    fn fanplay_worker_emit_frame(width: u32, height: u32, frames: u64, format: &str, bytes: &[u8]);
}
pub(super) fn now() -> f64 {
    fanplay_clock()
}
fn emit(value: Value) {
    fanplay_worker_emit(&value.to_string(), &[], "");
}
fn emit_bytes(value: Value, bytes: &[u8], field: &str) {
    fanplay_worker_emit(&value.to_string(), bytes, field);
}
fn field(value: &JsValue, name: &str) -> JsValue {
    js_sys::Reflect::get(value, &JsValue::from_str(name)).unwrap_or(JsValue::UNDEFINED)
}
fn number(value: &JsValue, name: &str) -> f64 {
    field(value, name).as_f64().unwrap_or(0.0)
}
fn string(value: &JsValue, name: &str) -> String {
    field(value, name).as_string().unwrap_or_default()
}
fn flag(value: &JsValue, name: &str) -> bool {
    field(value, name).as_bool().unwrap_or(false)
}

#[derive(Default)]
struct Inbox {
    frame: Option<JsValue>,
    messages: Vec<JsValue>,
}

#[derive(Clone)]
enum RemoteRequest {
    ExportSave,
    Plugin(OperationRequest),
}

#[derive(Clone)]
pub(super) struct RemoteHandle {
    requests: Arc<Mutex<Vec<RemoteRequest>>>,
    exported_save: Arc<Mutex<Option<Result<Vec<u8>, String>>>>,
}

impl RemoteHandle {
    pub fn request_save_export(&self) {
        self.requests
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(RemoteRequest::ExportSave);
    }

    pub fn request_plugin(&self, request: OperationRequest) {
        self.requests
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(RemoteRequest::Plugin(request));
    }

    pub fn take_exported_save(&self) -> Option<Result<Vec<u8>, String>> {
        self.exported_save
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
    }
}

pub(super) struct RemoteCore {
    info: CoreInfo,
    platform: Arc<BrowserPlatform>,
    frame: Arc<Mutex<PresentedFrame>>,
    context: Option<CoreContext>,
    worker: Option<JsValue>,
    inbox: Rc<RefCell<Inbox>>,
    callback: Option<Closure<dyn FnMut(JsValue)>>,
    initial_save: Option<Vec<u8>>,
    requests: Arc<Mutex<Vec<RemoteRequest>>>,
    exported_save: Arc<Mutex<Option<Result<Vec<u8>, String>>>>,
    ready: bool,
    stopped: bool,
    paused: bool,
}
impl RemoteCore {
    pub fn new(
        info: CoreInfo,
        platform: Arc<BrowserPlatform>,
        frame: Arc<Mutex<PresentedFrame>>,
        initial_save: Option<Vec<u8>>,
    ) -> Self {
        Self {
            info,
            platform,
            frame,
            context: None,
            worker: None,
            inbox: Rc::default(),
            callback: None,
            initial_save,
            requests: Arc::new(Mutex::new(Vec::new())),
            exported_save: Arc::new(Mutex::new(None)),
            ready: false,
            stopped: false,
            paused: false,
        }
    }
    pub fn handle(&self) -> RemoteHandle {
        RemoteHandle {
            requests: Arc::clone(&self.requests),
            exported_save: Arc::clone(&self.exported_save),
        }
    }
    fn post(&self, value: Value) {
        if let Some(worker) = &self.worker {
            fanplay_worker_post(worker, &value.to_string());
        }
    }

    fn ack_frame(&self) {
        if let Some(worker) = &self.worker {
            fanplay_worker_ack(worker);
        }
    }

    fn audio(&mut self, data: &JsValue) {
        let Some(context) = self.context.as_mut() else {
            return;
        };
        let channel = AudioChannel(number(data, "channel") as u8);
        match string(data, "op").as_str() {
            "play" => {
                let bytes = js_sys::Uint8Array::new(&field(data, "data")).to_vec();
                let path = string(data, "path");
                context.audio.play_stream(AudioStream {
                    channel,
                    path: &path,
                    data: &bytes,
                    volume: number(data, "volume") as i32,
                    pitch: number(data, "pitch"),
                    position: number(data, "position"),
                    looping: flag(data, "looping"),
                });
            }
            "stop" => context.audio.stop(channel),
            "fade" => context
                .audio
                .fade(channel, number(data, "milliseconds") as i32),
            "volume" => context
                .audio
                .set_volume(channel, number(data, "volume") as i32),
            _ => {}
        }
    }
}
impl Core for RemoteCore {
    fn info(&self) -> &CoreInfo {
        &self.info
    }
    fn load(&mut self, content: Content, context: CoreContext) -> CoreResult<()> {
        let inbox = Rc::clone(&self.inbox);
        let callback = Closure::wrap(Box::new(move |message: JsValue| {
            let mut inbox = inbox.borrow_mut();
            if string(&message, "type") == "frame" {
                inbox.frame = Some(message);
            } else {
                inbox.messages.push(message);
            }
        }) as Box<dyn FnMut(JsValue)>);
        let paths = js_sys::Array::new();
        let buffers = js_sys::Array::new();
        for (path, bytes) in self.platform.files.iter() {
            paths.push(&JsValue::from_str(path));
            buffers.push(&js_sys::Uint8Array::from(bytes.as_slice()));
        }
        let worker = fanplay_worker_create(
            wasm_bindgen::module(),
            callback.as_ref().unchecked_ref(),
            &self.platform.content_name,
            &content.label,
            &paths,
            &buffers,
            self.initial_save.as_deref().unwrap_or_default(),
        )
        .map_err(|e| CoreError::runtime(format!("could not start RGSS worker: {e:?}")))?;
        self.context = Some(context);
        self.callback = Some(callback);
        self.initial_save = None;
        self.worker = Some(worker);
        Ok(())
    }
    fn run(&mut self) -> CoreResult<ExitStatus> {
        Err(CoreError::unsupported("browser worker uses run_slice"))
    }
    fn run_slice(&mut self) -> CoreResult<RunSliceStatus> {
        let (frame, messages) = {
            let mut inbox = self.inbox.borrow_mut();
            (inbox.frame.take(), std::mem::take(&mut inbox.messages))
        };
        for message in messages {
            match string(&message, "type").as_str() {
                "ready" => self.ready = true,
                "error" => {
                    self.stopped = true;
                    return Err(CoreError::runtime(string(&message, "message")));
                }
                "stopped" => self.stopped = true,
                "audio" => self.audio(&message),
                "text_input" => {
                    self.frame
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .text_input = flag(&message, "enabled");
                }
                "native_bindings" => {
                    if let Some(context) = &mut self.context {
                        context
                            .events
                            .emit(CoreEvent::Status(string(&message, "status")));
                    }
                }
                "save_export" => {
                    let result = if flag(&message, "missing") {
                        Err("there is no RGSS save in the game's VFS; save in-game first".into())
                    } else {
                        Ok(js_sys::Uint8Array::new(&field(&message, "data")).to_vec())
                    };
                    *self
                        .exported_save
                        .lock()
                        .unwrap_or_else(|error| error.into_inner()) = Some(result);
                }
                "plugin_response" => {
                    if let Some(context) = &mut self.context {
                        context.events.emit(CoreEvent::CommandResult {
                            id: 0,
                            generation: 0,
                            outcome: CommandOutcome::Succeeded,
                            response: Some(string(&message, "response").into_bytes()),
                        });
                    }
                }
                "loaded" => {
                    if let Some(context) = &mut self.context {
                        context
                            .events
                            .emit(CoreEvent::Loaded(fanplay_core::CoreLoadInfo {
                                width: number(&message, "width") as u32,
                                height: number(&message, "height") as u32,
                                sample_rate: 48_000,
                                audio_channels: 2,
                            }));
                    }
                }
                _ => {}
            }
        }
        if let Some(message) = frame {
            let mut frame = self.frame.lock().unwrap_or_else(|e| e.into_inner());
            let pixels = js_sys::Uint8Array::new(&field(&message, "pixels"));
            frame.pixels.resize(pixels.length() as usize, 0);
            pixels.copy_to(&mut frame.pixels);
            frame.format = match string(&message, "format").as_str() {
                "bgra8" => PixelFormat::Bgra8,
                _ => PixelFormat::Rgba8,
            };
            frame.width = number(&message, "width") as u32;
            frame.height = number(&message, "height") as u32;
            frame.game_frames = number(&message, "frames") as u64;
            frame.revision = frame.revision.wrapping_add(1);
            drop(frame);
            self.ack_frame();
        }
        let requests = if self.ready {
            std::mem::take(
                &mut *self
                    .requests
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()),
            )
        } else {
            Vec::new()
        };
        for request in requests {
            match request {
                RemoteRequest::ExportSave => self.post(serde_json::json!({"type":"save_export"})),
                RemoteRequest::Plugin(request) => self.post(serde_json::json!({
                    "type": "plugin_request",
                    "request_id": request.id,
                    "generation": request.generation,
                    "operation": request.operation,
                    "arguments": request.arguments,
                })),
            }
        }
        let Some(context) = &mut self.context else {
            return Err(CoreError::runtime("worker not loaded"));
        };
        if context.control.should_stop() {
            self.post(serde_json::json!({"type":"stop"}));
            self.stopped = true;
        } else if self.ready {
            let mut events = Vec::new();
            context.control.input().take_events(&mut events);
            let paused = context.control.paused();
            if self.paused != paused {
                context.audio.pause(paused);
                self.paused = paused;
            }
            // Fixed-size arrays stay on the WASM stack. The previous Vec
            // construction allocated two tiny buffers for every worker tick.
            let positions = [
                context.audio.position(AudioChannel(0)),
                context.audio.position(AudioChannel(1)),
                context.audio.position(AudioChannel(2)),
                context.audio.position(AudioChannel(3)),
            ];
            let volumes = [
                context.audio.volume(AudioChannel(0)),
                context.audio.volume(AudioChannel(1)),
                context.audio.volume(AudioChannel(2)),
                context.audio.volume(AudioChannel(3)),
            ];
            let message = serde_json::json!({"type":"control", "paused":paused,
                "fast":context.control.fast_forward(), "rate":context.control.target_fps(),
                "events":events.into_iter().map(input_json).collect::<Vec<_>>(),
                "positions":positions, "volumes":volumes});
            self.post(message);
        }
        Ok(if self.stopped {
            RunSliceStatus::Exited(ExitStatus::success())
        } else {
            RunSliceStatus::Continue
        })
    }
}
impl Drop for RemoteCore {
    fn drop(&mut self) {
        if let Some(worker) = &self.worker {
            fanplay_worker_terminate(worker);
        }
        if let Some(context) = &mut self.context {
            for i in 0..4 {
                context.audio.stop(AudioChannel(i));
            }
        }
    }
}

fn input_json(event: InputEvent) -> Value {
    match event {
        InputEvent::Key { code, down, repeat } => {
            serde_json::json!({"kind":"key","code":code,"down":down,"repeat":repeat})
        }
        InputEvent::Text(text) => serde_json::json!({"kind":"text","text":text}),
        InputEvent::PointerMove { x, y, inside } => {
            serde_json::json!({"kind":"pointer","x":x,"y":y,"inside":inside})
        }
        InputEvent::PointerButton { button, down } => {
            serde_json::json!({"kind":"button","button":button,"down":down})
        }
        InputEvent::Wheel { x, y } => serde_json::json!({"kind":"wheel","x":x,"y":y}),
        InputEvent::Focus(focus) => serde_json::json!({"kind":"focus","focus":focus}),
    }
}

fn parse_input_js(event: &JsValue) -> Option<InputEvent> {
    let integer = |name: &str| field(event, name).as_f64().map(|value| value as i32);
    let boolean = |name: &str| field(event, name).as_bool().unwrap_or(false);
    Some(match string(event, "kind").as_str() {
        "key" => InputEvent::Key {
            code: integer("code")? as u32,
            down: boolean("down"),
            repeat: boolean("repeat"),
        },
        "text" => InputEvent::Text(field(event, "text").as_string()?),
        "pointer" => InputEvent::PointerMove {
            x: integer("x")?,
            y: integer("y")?,
            inside: boolean("inside"),
        },
        "button" => InputEvent::PointerButton {
            button: integer("button")? as u8,
            down: boolean("down"),
        },
        "wheel" => InputEvent::Wheel {
            x: integer("x")?,
            y: integer("y")?,
        },
        "focus" => InputEvent::Focus(boolean("focus")),
        _ => return None,
    })
}

thread_local! {
    static SESSION: RefCell<Option<WebSession>> = const { RefCell::new(None) };
    static POSITIONS: Cell<[f64;4]> = const { Cell::new([0.0;4]) };
    static VOLUMES: Cell<[i32;4]> = const { Cell::new([100;4]) };
    static PENDING_PLUGIN_REQUESTS: RefCell<Vec<PendingPluginRequest>> = const { RefCell::new(Vec::new()) };
}

struct PendingPluginRequest {
    bridge_id: u64,
    request_id: u64,
    generation: u64,
}

/// Adapts the generic plugin envelope to the positional argument protocol
/// consumed by FANPLAY::ExtensionBridge in Ruby. This mirrors the desktop
/// adapter, but intentionally bypasses the window-side route registry because
/// this function runs in a separate WASM instance inside the RGSS worker.
fn legacy_plugin_call(request: &OperationRequest) -> Option<(&str, Vec<String>)> {
    if request.operation.ends_with(".__ruby.inject") {
        let source = request.arguments.get("source")?.as_str()?;
        let name = request
            .arguments
            .get("name")
            .and_then(Value::as_str)
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
            vec![patch_name, fanplay_rgss::api_bridge::hex_encode(source)],
        ));
    }
    if request.operation.ends_with(".__ruby.unload") {
        let handle = request.arguments.get("handle")?;
        let handle = handle.as_str().map(str::to_owned).or_else(|| {
            handle
                .as_i64()
                .filter(|value| *value > 0)
                .map(|value| format!("rhai-{value}"))
        })?;
        return Some(("ruby_unload", vec![handle]));
    }
    let operation = request.operation.rsplit_once('.')?.1;
    let arguments = match &request.arguments {
        Value::Object(values) => values.values().collect::<Vec<_>>(),
        // Webview HTTP routes arrive as an ordered positional list. Dashboard
        // requests use an object, preserving the existing generic API shape.
        Value::Array(values) => values.iter().collect::<Vec<_>>(),
        _ => return None,
    }
    .into_iter()
    .map(|value| {
        value
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| value.to_string())
    });
    Some((operation, arguments.collect()))
}

fn plugin_error(request_id: u64, generation: u64, code: &str, message: impl Into<String>) -> Value {
    serde_json::json!({
        "type": "plugin_response",
        "response": serde_json::json!({
            "request_id": request_id,
            "generation": generation,
            "error": {"code": code, "message": message.into()}
        }).to_string()
    })
}

fn poll_plugin_responses() {
    PENDING_PLUGIN_REQUESTS.with(|pending| {
        let mut pending = pending.borrow_mut();
        let mut waiting = Vec::with_capacity(pending.len());
        for request in pending.drain(..) {
            let Some(result) = fanplay_rgss::api_bridge::take_ui_response(request.bridge_id) else {
                waiting.push(request);
                continue;
            };
            let response = match result {
                Ok(payload) => match serde_json::from_str::<Value>(&payload) {
                    Ok(value) => {
                        if let Some(error) = value.get("error") {
                            let message = error
                                .as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| error.to_string());
                            serde_json::json!({
                                "request_id": request.request_id,
                                "generation": request.generation,
                                "error": {
                                    "code": "game",
                                    "message": message
                                }
                            })
                        } else {
                            serde_json::json!({
                                "request_id": request.request_id,
                                "generation": request.generation,
                                "result": value
                            })
                        }
                    }
                    Err(error) => serde_json::json!({
                        "request_id": request.request_id,
                        "generation": request.generation,
                        "error": {"code": "invalid_response", "message": error.to_string()}
                    }),
                },
                Err(error) => serde_json::json!({
                    "request_id": request.request_id,
                    "generation": request.generation,
                    "error": {"code": "game", "message": error}
                }),
            };
            emit(serde_json::json!({
                "type": "plugin_response",
                "response": response.to_string()
            }));
        }
        pending.extend(waiting);
    });
}
struct WorkerAudio;
impl AudioSink for WorkerAudio {
    fn play_stream(&mut self, stream: AudioStream<'_>) {
        emit_bytes(
            serde_json::json!({"type":"audio","op":"play","channel":stream.channel.0,
            "path":stream.path,"volume":stream.volume,"pitch":stream.pitch,"position":stream.position,
            "looping":stream.looping}),
            stream.data,
            "data",
        );
    }
    fn stop(&mut self, channel: AudioChannel) {
        emit(serde_json::json!({"type":"audio","op":"stop","channel":channel.0}));
    }
    fn fade(&mut self, channel: AudioChannel, milliseconds: i32) {
        emit(
            serde_json::json!({"type":"audio","op":"fade","channel":channel.0,"milliseconds":milliseconds}),
        );
    }
    fn position(&self, channel: AudioChannel) -> f64 {
        POSITIONS.with(|p| p.get().get(channel.0 as usize).copied().unwrap_or(0.0))
    }
    fn set_volume(&mut self, channel: AudioChannel, volume: i32) {
        VOLUMES.with(|v| {
            let mut values = v.get();
            if let Some(value) = values.get_mut(channel.0 as usize) {
                *value = volume;
            }
            v.set(values);
        });
        emit(serde_json::json!({"type":"audio","op":"volume","channel":channel.0,"volume":volume}));
    }
    fn volume(&self, channel: AudioChannel) -> i32 {
        VOLUMES.with(|v| v.get().get(channel.0 as usize).copied().unwrap_or(100))
    }
}

#[wasm_bindgen]
pub fn fanplay_worker_load(
    entry: String,
    label: String,
    paths: js_sys::Array,
    buffers: js_sys::Array,
    save: js_sys::Uint8Array,
) -> Result<(), JsValue> {
    js_sys::Reflect::set(
        &js_sys::global(),
        &JsValue::from_str("__fanplay_initial_save"),
        &save,
    )
    .map_err(|error| JsValue::from_str(&format!("could not install initial save: {error:?}")))?;
    let mut files = HashMap::new();
    for i in 0..paths.length() {
        let path = paths
            .get(i)
            .as_string()
            .ok_or_else(|| JsValue::from_str("invalid worker asset path"))?;
        files.insert(path, js_sys::Uint8Array::new(&buffers.get(i)).to_vec());
    }
    let session = WebSession::load(
        BrowserGame {
            label,
            entry,
            files: Arc::new(files),
        },
        Box::new(WorkerAudio),
        None,
        None,
    )
    .map_err(|e| JsValue::from_str(&e))?;
    SESSION.with(|slot| *slot.borrow_mut() = Some(session));
    Ok(())
}
#[wasm_bindgen]
pub fn fanplay_worker_control(data: JsValue) -> Result<(), JsValue> {
    SESSION.with(|slot| {
        if let Some(session) = slot.borrow().as_ref() {
            session.control.set_paused(flag(&data, "paused"));
            session.control.set_fast_forward(flag(&data, "fast"));
            session
                .control
                .set_target_fps(field(&data, "rate").as_f64().map(|n| n as u32));
            if let Ok(events) = field(&data, "events").dyn_into::<js_sys::Array>() {
                for index in 0..events.length() {
                    if let Some(event) = parse_input_js(&events.get(index)) {
                        session.control.input().push_event(event);
                    }
                }
            }
        }
    });
    if let Ok(positions) = field(&data, "positions").dyn_into::<js_sys::Array>() {
        POSITIONS.with(|p| {
            let mut values = p.get();
            for (index, value) in values.iter_mut().enumerate() {
                *value = positions.get(index as u32).as_f64().unwrap_or(0.0);
            }
            p.set(values);
        });
    }
    if let Ok(volumes) = field(&data, "volumes").dyn_into::<js_sys::Array>() {
        VOLUMES.with(|v| {
            let mut values = v.get();
            for (index, value) in values.iter_mut().enumerate() {
                *value = volumes
                    .get(index as u32)
                    .as_f64()
                    .map(|volume| volume as i32)
                    .unwrap_or(100);
            }
            v.set(values);
        });
    }
    Ok(())
}

#[wasm_bindgen]
pub fn fanplay_worker_plugin_request(
    request_id: u64,
    generation: u64,
    operation: String,
    arguments: String,
) -> Result<(), JsValue> {
    let request = OperationRequest {
        id: request_id,
        generation,
        operation,
        arguments: serde_json::from_str(&arguments)
            .map_err(|error| JsValue::from_str(&format!("invalid plugin arguments: {error}")))?,
    };
    let Some((method, args)) = legacy_plugin_call(&request) else {
        emit(plugin_error(
            request.id,
            request.generation,
            "unsupported_operation",
            request.operation,
        ));
        return Ok(());
    };
    match fanplay_rgss::api_bridge::send_from_ui_worker(method, args) {
        Ok(bridge_id) => PENDING_PLUGIN_REQUESTS.with(|pending| {
            pending.borrow_mut().push(PendingPluginRequest {
                bridge_id,
                request_id: request.id,
                generation: request.generation,
            });
        }),
        Err(error) => emit(plugin_error(request.id, request.generation, "queue", error)),
    }
    Ok(())
}

#[wasm_bindgen]
pub fn fanplay_worker_step() -> Result<JsValue, JsValue> {
    SESSION.with(|slot| {
        let mut slot = slot.borrow_mut();
        let session = slot
            .as_mut()
            .ok_or_else(|| JsValue::from_str("RGSS worker is not loaded"))?;
        let status = session
            .core
            .run_slice()
            .map_err(|e| JsValue::from_str(&e.to_string()))?;
        poll_plugin_responses();
        let alive = matches!(status, RunSliceStatus::Continue);
        let events = std::mem::take(&mut *session.events.lock().unwrap_or_else(|e| e.into_inner()));
        for event in events {
            match event {
                CoreEvent::Loaded(info) => emit(
                    serde_json::json!({"type":"loaded","width":info.width,"height":info.height}),
                ),
                CoreEvent::Error(message) => return Err(JsValue::from_str(&message)),
                CoreEvent::Window(fanplay_core::WindowCommand::TextInput(enabled)) => {
                    emit(serde_json::json!({"type":"text_input","enabled":enabled}))
                }
                CoreEvent::Status(status) => {
                    emit(serde_json::json!({"type":"native_bindings","status":status}))
                }
                _ => {}
            }
        }
        let frame = session.frame.lock().unwrap_or_else(|e| e.into_inner());
        let presented = frame.revision != session.uploaded_revision;
        if presented {
            session.uploaded_revision = frame.revision;
            let frames = fanplay_rgss::engine::with_runtime(|r| r.frame_count()).unwrap_or(0);
            let format = match frame.format {
                PixelFormat::Bgra8 => "bgra8",
                _ => "rgba8",
            };
            fanplay_worker_emit_frame(frame.width, frame.height, frames, format, &frame.pixels);
        }
        drop(frame);
        // Return the small scheduler result as a JS object directly. The
        // previous JSON stringify/parse pair ran once per worker step and
        // allocated even though the consumer immediately read three fields.
        let result = js_sys::Object::new();
        js_sys::Reflect::set(
            &result,
            &JsValue::from_str("alive"),
            &JsValue::from_bool(alive),
        )
        .map_err(|error| JsValue::from_str(&format!("could not set worker status: {error:?}")))?;
        js_sys::Reflect::set(
            &result,
            &JsValue::from_str("rate"),
            &JsValue::from_f64(session.effective_rate() as f64),
        )
        .map_err(|error| JsValue::from_str(&format!("could not set worker rate: {error:?}")))?;
        js_sys::Reflect::set(
            &result,
            &JsValue::from_str("presented"),
            &JsValue::from_bool(presented),
        )
        .map_err(|error| {
            JsValue::from_str(&format!("could not set worker frame status: {error:?}"))
        })?;
        Ok(result.into())
    })
}
