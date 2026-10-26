// SPDX-License-Identifier: GPL-2.0-or-later
//! GBA and DS cartridge sessions for the browser.
//!
//! The RGSS path in `app.js` runs Ruby in a Fiber and calls the engine per
//! frame. Cartridge cores own a native `Core::run` loop instead, so the page
//! drives them one frame at a time by setting a one-frame limit before each
//! call. Everything else is shared with the desktop host: `fanplay-core` routing,
//! `Platform`/`VideoSink`/`AudioSink`/`EventSink`, `CoreControl` input and the
//! same egui workspace used for RGSS.
//!
//! ROMs arrive from the file picker or a dropped file. They are copied into
//! the virtual filesystem supplied by `files.rs`; the core's normal
//! `read_content` fallback then sees them without any native file access.

use crate::files::read_via_host;
use fanplay_rgss::graphics::Bitmap;
use fanplay_core::{
    AudioConfig, AudioSink, Button, Content, Core, CoreCommand, CoreContext, CoreControl,
    CoreEvent, EventSink, PcmBuffer, Platform, PlatformError, PlatformResult, VideoFrame,
    VideoSink,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use wasm_bindgen::prelude::*;

#[derive(Clone, Default)]
struct SaveStore(Arc<Mutex<HashMap<String, Vec<u8>>>>);

impl SaveStore {
    fn read(&self, name: &str) -> Option<Vec<u8>> {
        self.0.lock().unwrap().get(name).cloned()
    }

    fn write(&self, name: &str, bytes: &[u8]) {
        self.0
            .lock()
            .unwrap()
            .insert(name.to_owned(), bytes.to_vec());
    }
}

/// Browser cartridge platform. Content and firmware come from the virtual
/// filesystem; battery saves stay in memory for this session.
struct CartridgePlatform {
    rom_name: String,
    saves: SaveStore,
}

impl Platform for CartridgePlatform {
    fn read_content(&self, path: &str) -> PlatformResult<Vec<u8>> {
        read_via_host(path)
            .or_else(|| read_via_host(&self.rom_name))
            .ok_or_else(|| PlatformError::not_found(path))
    }

    fn read_system_file(&self, path: &str) -> PlatformResult<Option<Vec<u8>>> {
        Ok(read_via_host(path))
    }

    fn read_save(&self, name: &str) -> PlatformResult<Option<Vec<u8>>> {
        Ok(self.saves.read(name))
    }

    fn write_save(&self, name: &str, bytes: &[u8]) -> PlatformResult<()> {
        self.saves.write(name, bytes);
        Ok(())
    }

    fn save_dir(&self) -> PathBuf {
        // GBA's upstream backend writes its battery file through `std::fs`.
        // There is no writable native filesystem in the browser, so use the
        // backend's in-memory backup path instead of a bogus path.
        PathBuf::new()
    }

    fn now(&self) -> f64 {
        crate::now_seconds()
    }

    fn sleep(&self, _duration: Duration) {
        // The page calls the core once per animation frame, which provides the
        // pacing that a native host would get from a blocking sleep.
    }

    fn log(&self, level: fanplay_core::LogLevel, message: &str) {
        let _ = level;
        web_sys::console::log_1(&message.into());
    }
}

struct CartridgeVideo {
    frame: Arc<Mutex<Option<Bitmap>>>,
    frames: Arc<AtomicU64>,
    rgba: Vec<u8>,
}

impl VideoSink for CartridgeVideo {
    fn present(&mut self, frame: VideoFrame<'_>) {
        if fanplay_frontend::video::convert_rgba(&frame, &mut self.rgba).is_ok()
            && let Ok(bitmap) = Bitmap::from_rgba(frame.width, frame.height, &self.rgba)
        {
            *self.frame.lock().unwrap() = Some(bitmap);
            self.frames.fetch_add(1, Ordering::Relaxed);
        }
    }
}

struct CartridgeEvents {
    messages: Arc<Mutex<Vec<String>>>,
}

impl EventSink for CartridgeEvents {
    fn emit(&mut self, event: CoreEvent) {
        let message = match event {
            CoreEvent::Log(_, message) | CoreEvent::Status(message) | CoreEvent::Error(message) => {
                Some(message)
            }
            CoreEvent::Loaded(info) => Some(format!(
                "Loaded {}x{} core frame at {} Hz",
                info.width, info.height, info.sample_rate
            )),
            CoreEvent::Window(_)
            | CoreEvent::SaveState { .. }
            | CoreEvent::SaveRam { .. }
            | CoreEvent::Screenshot { .. }
            | CoreEvent::CommandResult { .. }
            | CoreEvent::Exited(_) => None,
        };
        if let Some(message) = message {
            let mut messages = self.messages.lock().unwrap();
            if messages.len() >= 200 {
                messages.remove(0);
            }
            messages.push(message);
        }
    }
}

struct PcmState {
    context: web_sys::AudioContext,
    next_time: f64,
}

thread_local! {
    // The AudioContext is a JavaScript value and therefore not `Send`; the
    // core contract stores sinks behind `Send`, so keep the device on this
    // (single-threaded) wasm thread and expose only a zero-sized sink.
    static PCM_AUDIO: RefCell<Option<PcmState>> = const { RefCell::new(None) };
}

struct CartridgeAudio;

fn with_pcm_state(mut f: impl FnMut(&mut PcmState)) {
    PCM_AUDIO.with(|slot| {
        if slot.borrow().is_none()
            && let Ok(context) = web_sys::AudioContext::new()
        {
            *slot.borrow_mut() = Some(PcmState {
                context,
                next_time: 0.0,
            });
        }
        if let Some(state) = slot.borrow_mut().as_mut() {
            f(state);
        }
    });
}

impl AudioSink for CartridgeAudio {
    fn output_config(&self) -> Option<AudioConfig> {
        Some(AudioConfig::new(48_000, 2))
    }

    fn queue_pcm(&mut self, pcm: PcmBuffer<'_>) {
        let frames = pcm.frames();
        let channels = usize::from(pcm.channels.max(1));
        if frames == 0 || pcm.sample_rate == 0 || pcm.samples.len() < frames * channels {
            return;
        }
        with_pcm_state(|state| {
            // Browsers start the context suspended until a user gesture. The
            // page's first key press or click also resumes it; retrying here
            // keeps audio alive if the core starts before that gesture.
            if state.context.state() == web_sys::AudioContextState::Suspended {
                let _ = state.context.resume();
            }
            let Ok(buffer) = state.context.create_buffer(
                pcm.channels as u32,
                frames as u32,
                pcm.sample_rate as f32,
            ) else {
                return;
            };
            let mut channel = vec![0.0f32; frames];
            for index in 0..channels {
                for (frame, sample) in channel.iter_mut().enumerate() {
                    *sample = f32::from(pcm.samples[frame * channels + index]) / 32768.0;
                }
                let _ = buffer.copy_to_channel(&channel, index as i32);
            }
            let Ok(source) = state.context.create_buffer_source() else {
                return;
            };
            source.set_buffer(Some(&buffer));
            if source
                .connect_with_audio_node(&state.context.destination())
                .is_err()
            {
                return;
            }
            let now = state.context.current_time();
            let duration = frames as f64 / f64::from(pcm.sample_rate);
            if state.next_time < now {
                state.next_time = now + 0.02;
            }
            if source.start_with_when(state.next_time).is_ok() {
                state.next_time += duration;
            }
        });
    }

    fn pause(&mut self, paused: bool) {
        with_pcm_state(|state| {
            if paused {
                let _ = state.context.suspend();
            } else {
                let _ = state.context.resume();
                state.next_time = 0.0;
            }
        });
    }

    fn reset_pcm(&mut self) {
        with_pcm_state(|state| state.next_time = 0.0);
    }
}

struct CartridgeSession {
    core: Box<dyn Core>,
    control: Arc<CoreControl>,
    frame: Arc<Mutex<Option<Bitmap>>>,
    frames: Arc<AtomicU64>,
    messages: Arc<Mutex<Vec<String>>>,
}

thread_local! {
    static SESSION: RefCell<Option<CartridgeSession>> = const { RefCell::new(None) };
}

/// Loads a GBA or DS ROM. Returns the core id (`gba`/`nds`).
#[wasm_bindgen]
pub fn web_core_init(name: &str, bytes: &[u8]) -> Result<String, JsValue> {
    console_error_panic_hook::set_once();
    let extension = Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| JsValue::from_str("The ROM must be a .gba or .nds file"))?;
    if !matches!(extension.as_str(), "gba" | "nds") {
        return Err(JsValue::from_str(&format!(
            "Unsupported cartridge file '{name}'; expected a .gba or .nds ROM"
        )));
    }
    let path = Path::new(name);
    let probe = fanplay_core::ContentProbe::new(path, false, bytes);
    let registration = fanplay_cores::catalog()
        .route_probe(&probe)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;

    // The core reads through `Platform::read_content`, which falls back to the
    // virtual filesystem. Store the ROM under both the chosen name and a short
    // alias so system files beside the ROM can also be resolved.
    crate::files::web_add_file(name, bytes);
    let alias = format!("rom.{extension}");
    crate::files::web_add_file(&alias, bytes);

    let frame = Arc::new(Mutex::new(None));
    let frames = Arc::new(AtomicU64::new(0));
    let messages = Arc::new(Mutex::new(Vec::new()));
    let saves = SaveStore::default();
    let control = Arc::new(CoreControl::new());
    let platform = Arc::new(CartridgePlatform {
        rom_name: name.to_owned(),
        saves,
    });
    let mut core = (registration.create)();
    let content = Content::file(PathBuf::from(name));
    let context = CoreContext::new(
        platform,
        Box::new(CartridgeVideo {
            frame: Arc::clone(&frame),
            frames: Arc::clone(&frames),
            rgba: Vec::new(),
        }),
        Box::new(CartridgeAudio),
        Box::new(CartridgeEvents {
            messages: Arc::clone(&messages),
        }),
        Arc::clone(&control),
    );

    core.load(content, context)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;

    let id = registration.info.id.to_owned();
    SESSION.with(|slot| {
        *slot.borrow_mut() = Some(CartridgeSession {
            core,
            control,
            frame,
            frames,
            messages,
        });
    });
    Ok(id)
}

/// Runs exactly one emulated frame. The core keeps its machine state between
/// calls; errors are returned to the page so they can be shown in the log.
#[wasm_bindgen]
pub fn web_core_step() -> Result<(), JsValue> {
    SESSION.with(|slot| {
        let mut slot = slot.borrow_mut();
        let session = slot
            .as_mut()
            .ok_or_else(|| JsValue::from_str("No cartridge session is loaded"))?;
        session.control.set_frame_limit(Some(1));
        session
            .core
            .run()
            .map(|_| ())
            .map_err(|error| JsValue::from_str(&error.to_string()))
    })
}

/// Draws the latest cartridge frame in the shared workspace.
#[wasm_bindgen]
pub fn web_core_present() {
    let latest = SESSION.with(|slot| {
        let slot = slot.borrow();
        let session = slot.as_ref()?;
        let frame = session.frame.lock().unwrap().clone();
        Some((frame, session.frames.load(Ordering::Relaxed)))
    });
    if let Some((Some(bitmap), frames)) = latest {
        crate::present_bitmap(&bitmap, frames);
    }
}

/// Frame dimensions `[width, height]`, or an empty vector before the first
/// frame. The page uses this for mouse/touch mapping.
#[wasm_bindgen]
pub fn web_core_size() -> Vec<u32> {
    SESSION.with(|slot| {
        let slot = slot.borrow();
        let Some(session) = slot.as_ref() else {
            return Vec::new();
        };
        let frame = session.frame.lock().unwrap();
        frame
            .as_ref()
            .map(|bitmap| vec![bitmap.width(), bitmap.height()])
            .unwrap_or_default()
    })
}

fn core_button(code: i32) -> Option<Button> {
    Some(match code {
        82 | 604 => Button::Up,
        81 | 601 => Button::Down,
        80 | 602 => Button::Left,
        79 | 603 => Button::Right,
        27 | 605 => Button::South,
        29 | 606 => Button::East,
        22 | 608 => Button::North,
        4 | 607 => Button::West,
        40 => Button::Start,
        42 => Button::Select,
        20 | 611 => Button::LeftShoulder,
        26 | 612 => Button::RightShoulder,
        _ => return None,
    })
}

#[wasm_bindgen]
pub fn web_core_key(code: i32, down: bool) {
    SESSION.with(|slot| {
        if let Some(session) = slot.borrow().as_ref()
            && let Some(button) = core_button(code)
        {
            session.control.input().set_button(button, down);
        }
    });
}

#[wasm_bindgen]
pub fn web_core_pointer(x: i32, y: i32, inside: bool) {
    SESSION.with(|slot| {
        if let Some(session) = slot.borrow().as_ref() {
            session.control.input().set_pointer(x, y, inside);
        }
    });
}

/// Touch/stylus contact used by the DS lower screen.
#[wasm_bindgen]
pub fn web_core_pointer_button(down: bool) {
    SESSION.with(|slot| {
        if let Some(session) = slot.borrow().as_ref() {
            session.control.input().set_button(Button::Extra0, down);
        }
    });
}

#[wasm_bindgen]
pub fn web_core_fast_forward(enabled: bool) {
    SESSION.with(|slot| {
        if let Some(session) = slot.borrow().as_ref() {
            session.control.set_fast_forward(enabled);
        }
    });
}

#[wasm_bindgen]
pub fn web_core_pause(paused: bool) {
    SESSION.with(|slot| {
        if let Some(session) = slot.borrow().as_ref() {
            session.control.set_paused(paused);
        }
    });
}

#[wasm_bindgen]
pub fn web_core_reset() -> Result<(), JsValue> {
    SESSION.with(|slot| {
        let slot = slot.borrow();
        let session = slot
            .as_ref()
            .ok_or_else(|| JsValue::from_str("No cartridge session is loaded"))?;
        session
            .control
            .send_request(CoreCommand::Reset)
            .map(|_| ())
            .map_err(|error| JsValue::from_str(&error.to_string()))
    })
}

#[wasm_bindgen]
pub fn web_core_stop() {
    SESSION.with(|slot| {
        if let Some(session) = slot.borrow_mut().take() {
            session.control.stop();
        }
    });
}

/// Drains queued core log/error lines as a JSON array of strings.
#[wasm_bindgen]
pub fn web_core_take_messages() -> String {
    let messages = SESSION.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|session| std::mem::take(&mut *session.messages.lock().unwrap()))
            .unwrap_or_default()
    });
    serde_json::to_string(&messages).unwrap_or_else(|_| "[]".into())
}
