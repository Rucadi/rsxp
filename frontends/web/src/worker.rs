// SPDX-License-Identifier: GPL-2.0-or-later
//! RGSS runs in an isolated browser worker; the window only exchanges messages.
use super::*;
use fanplay_core::{CoreError, CoreResult};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[wasm_bindgen(inline_js = r#"
export function fanplay_clock() { return performance.now() / 1000; }
export function fanplay_worker_create(module, callback, entry, label, paths, buffers) {
  const bindings = [...document.querySelectorAll('link[rel="modulepreload"]')]
    .find(link => /\/fanplay-web-[^/]+\.js$/.test(new URL(link.href).pathname));
  const runtime = [...document.scripts].find(script => /ruby_runtime[^/]*\.js$/.test(script.src));
  const loader = [...document.scripts].find(script => script.src.includes('/@ruby/wasm-wasi@'));
  if (!bindings || !runtime || !loader) throw new Error('RGSS worker runtime assets are missing');
  const worker = new Worker(new URL('rgss_worker.js', document.baseURI), { type: 'module' });
  worker.onmessage = event => callback(event.data);
  worker.onerror = event => { event.preventDefault(); callback({type:'error', message:event.message}); };
  worker.postMessage({type:'load', module, entry, label, paths, buffers,
    bindings: bindings.href, rubyRuntime: runtime.src, rubyLoader: loader.src,
    rubyWasm: new URL('ruby.wasm', document.baseURI).href}, buffers.map(bytes => bytes.buffer));
  return worker;
}
export function fanplay_worker_post(worker, json) { worker.postMessage(JSON.parse(json)); }
export function fanplay_worker_terminate(worker) { worker.onmessage = null; worker.onerror = null; worker.terminate(); }
export function fanplay_worker_emit(json, bytes, field) {
  const message = JSON.parse(json);
  if (field) message[field] = new Uint8Array(bytes);
  globalThis.__fanplay_worker_output(message);
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
    ) -> Result<JsValue, JsValue>;
    fn fanplay_worker_post(worker: &JsValue, json: &str);
    fn fanplay_worker_terminate(worker: &JsValue);
    fn fanplay_worker_emit(json: &str, bytes: &[u8], field: &str);
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

pub(super) struct RemoteCore {
    info: CoreInfo,
    platform: Arc<BrowserPlatform>,
    frame: Arc<Mutex<PresentedFrame>>,
    context: Option<CoreContext>,
    worker: Option<JsValue>,
    inbox: Rc<RefCell<Inbox>>,
    callback: Option<Closure<dyn FnMut(JsValue)>>,
    ready: bool,
    stopped: bool,
    paused: bool,
}
impl RemoteCore {
    pub fn new(
        info: CoreInfo,
        platform: Arc<BrowserPlatform>,
        frame: Arc<Mutex<PresentedFrame>>,
    ) -> Self {
        Self {
            info,
            platform,
            frame,
            context: None,
            worker: None,
            inbox: Rc::default(),
            callback: None,
            ready: false,
            stopped: false,
            paused: false,
        }
    }
    fn post(&self, value: Value) {
        if let Some(worker) = &self.worker {
            fanplay_worker_post(worker, &value.to_string());
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
    fn load(&mut self, _: Content, context: CoreContext) -> CoreResult<()> {
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
            &self.platform.content_name,
            &paths,
            &buffers,
        )
        .map_err(|e| CoreError::runtime(format!("could not start RGSS worker: {e:?}")))?;
        self.context = Some(context);
        self.callback = Some(callback);
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
            frame.rgba.resize(pixels.length() as usize, 0);
            pixels.copy_to(&mut frame.rgba);
            frame.width = number(&message, "width") as u32;
            frame.height = number(&message, "height") as u32;
            frame.game_frames = number(&message, "frames") as u64;
            frame.revision = frame.revision.wrapping_add(1);
            drop(frame);
            self.post(serde_json::json!({"type":"frame_ack"}));
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
            let positions: Vec<_> = (0..4)
                .map(|i| context.audio.position(AudioChannel(i)))
                .collect();
            let volumes: Vec<_> = (0..4)
                .map(|i| context.audio.volume(AudioChannel(i)))
                .collect();
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
fn parse_input(event: &Value) -> Option<InputEvent> {
    let n = |name| event[name].as_i64().unwrap_or(0) as i32;
    let b = |name| event[name].as_bool().unwrap_or(false);
    Some(match event["kind"].as_str()? {
        "key" => InputEvent::Key {
            code: n("code") as u32,
            down: b("down"),
            repeat: b("repeat"),
        },
        "text" => InputEvent::Text(event["text"].as_str()?.to_owned()),
        "pointer" => InputEvent::PointerMove {
            x: n("x"),
            y: n("y"),
            inside: b("inside"),
        },
        "button" => InputEvent::PointerButton {
            button: n("button") as u8,
            down: b("down"),
        },
        "wheel" => InputEvent::Wheel {
            x: n("x"),
            y: n("y"),
        },
        "focus" => InputEvent::Focus(b("focus")),
        _ => return None,
    })
}

thread_local! {
    static SESSION: RefCell<Option<WebSession>> = const { RefCell::new(None) };
    static POSITIONS: Cell<[f64;4]> = const { Cell::new([0.0;4]) };
    static VOLUMES: Cell<[i32;4]> = const { Cell::new([100;4]) };
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
) -> Result<(), JsValue> {
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
    )
    .map_err(|e| JsValue::from_str(&e))?;
    SESSION.with(|slot| *slot.borrow_mut() = Some(session));
    Ok(())
}
#[wasm_bindgen]
pub fn fanplay_worker_control(json: &str) -> Result<(), JsValue> {
    let data: Value = serde_json::from_str(json).map_err(|e| JsValue::from_str(&e.to_string()))?;
    SESSION.with(|slot| {
        if let Some(session) = slot.borrow().as_ref() {
            session
                .control
                .set_paused(data["paused"].as_bool().unwrap_or(false));
            session
                .control
                .set_fast_forward(data["fast"].as_bool().unwrap_or(false));
            session
                .control
                .set_target_fps(data["rate"].as_u64().map(|n| n as u32));
            for event in data["events"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(parse_input)
            {
                session.control.input().push_event(event);
            }
        }
    });
    if let Some(positions) = data["positions"].as_array() {
        POSITIONS.with(|p| {
            let mut values = p.get();
            for (value, position) in values.iter_mut().zip(positions) {
                *value = position.as_f64().unwrap_or(0.0);
            }
            p.set(values);
        });
    }
    if let Some(volumes) = data["volumes"].as_array() {
        VOLUMES.with(|v| {
            let mut values = v.get();
            for (value, volume) in values.iter_mut().zip(volumes) {
                *value = volume.as_i64().unwrap_or(100) as i32;
            }
            v.set(values);
        });
    }
    Ok(())
}
#[wasm_bindgen]
pub fn fanplay_worker_step() -> Result<JsValue, JsValue> {
    SESSION.with(|slot| {
        let mut slot = slot.borrow_mut();
        let session = slot.as_mut().ok_or_else(|| JsValue::from_str("RGSS worker is not loaded"))?;
        let status = session.core.run_slice().map_err(|e| JsValue::from_str(&e.to_string()))?;
        let alive = matches!(status, RunSliceStatus::Continue);
        let events = std::mem::take(&mut *session.events.lock().unwrap_or_else(|e| e.into_inner()));
        for event in events {
            match event {
                CoreEvent::Loaded(info) => emit(serde_json::json!({"type":"loaded","width":info.width,"height":info.height})),
                CoreEvent::Error(message) => return Err(JsValue::from_str(&message)),
                _ => {}
            }
        }
        let frame = session.frame.lock().unwrap_or_else(|e| e.into_inner());
        let presented = frame.revision != session.uploaded_revision;
        if presented {
            session.uploaded_revision = frame.revision;
            let frames = fanplay_rgss::engine::with_runtime(|r| r.frame_count()).unwrap_or(0);
            emit_bytes(serde_json::json!({"type":"frame","width":frame.width,"height":frame.height,"frames":frames}), &frame.rgba, "pixels");
        }
        drop(frame);
        js_sys::JSON::parse(&serde_json::json!({"alive":alive,"rate":session.effective_rate(),"presented":presented}).to_string())
    })
}
