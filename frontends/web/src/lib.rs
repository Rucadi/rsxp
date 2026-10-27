#![cfg(target_arch = "wasm32")]

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use eframe::{App, Frame, WebOptions, WebRunner};
use egui::{Context, Ui};
use fanplay_core::audio::{
    AudioChannel, AudioConfig, AudioSink, AudioStream, PcmBuffer, PcmRing, PcmRingConsumer,
    PcmRingProducer,
};
use fanplay_core::{
    Button, ButtonSet, Content, ContentProbe, Core, CoreCapabilities, CoreCommand, CoreContext,
    CoreControl, CoreEvent, CoreInfo, EventSink, ExitStatus, Platform, PlatformError,
    PlatformResult, RunSliceStatus, SessionConfig, VideoFrame, VideoSink,
};
use fanplay_extension_api::{OperationError, OperationResponse};
use fanplay_frontend::{video::convert_rgba, ContentId};
use fanplay_ui::{
    session::{SessionCapabilities, SessionPanel},
    ui::backend::Backend,
    SharedApp, StateSlot, WorkspaceInfo,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use wasm_bindgen::{closure::Closure, prelude::*, JsCast};
use wasm_bindgen_futures::JsFuture;
use zip::ZipArchive;

fn normalize_browser_path(path: &str) -> String {
    path.replace('\\', "/")
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>()
        .join("/")
        .to_ascii_lowercase()
}

#[derive(Clone)]
struct BrowserGame {
    label: String,
    entry: String,
    files: BTreeMap<String, Vec<u8>>,
}

impl BrowserGame {
    fn from_single(name: String, bytes: Vec<u8>) -> Result<Self, String> {
        if name.to_ascii_lowercase().ends_with(".zip") {
            return Self::from_zip(name, &bytes);
        }
        let entry = normalize_browser_path(&name);
        if entry.is_empty() {
            return Err("selected game file has no usable name".into());
        }
        let mut files = BTreeMap::new();
        files.insert(entry.clone(), bytes);
        let label = Path::new(&name)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(&name)
            .to_owned();
        if matches!(
            Path::new(&entry).extension().and_then(|ext| ext.to_str()),
            Some("rgssad" | "rgss2a" | "rgss3a" | "rxdata" | "rvdata" | "rvdata2")
        ) {
            Self::from_files(label, files)
        } else {
            Ok(Self {
                label,
                entry,
                files,
            })
        }
    }

    fn from_zip(name: String, bytes: &[u8]) -> Result<Self, String> {
        let mut archive = ZipArchive::new(Cursor::new(bytes))
            .map_err(|error| format!("could not open {name} as ZIP: {error}"))?;
        let mut files = BTreeMap::new();
        for index in 0..archive.len() {
            let mut file = archive
                .by_index(index)
                .map_err(|error| format!("could not read ZIP entry {index}: {error}"))?;
            if file.is_dir() {
                continue;
            }
            let path = normalize_browser_path(file.name());
            if path.is_empty() || path.split('/').any(|part| part == "..") {
                continue;
            }
            let mut data = Vec::new();
            file.read_to_end(&mut data)
                .map_err(|error| format!("could not extract ZIP entry {path}: {error}"))?;
            files.insert(path, data);
        }
        Self::from_files(
            Path::new(&name)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("game")
                .to_owned(),
            files,
        )
    }

    fn from_files(label: String, mut files: BTreeMap<String, Vec<u8>>) -> Result<Self, String> {
        let root = files
            .keys()
            .next()
            .and_then(|path| path.split('/').next())
            .map(str::to_owned);
        let strip_root = root.as_deref().is_some_and(|root| {
            files.keys().all(|path| {
                path.starts_with(root) && path.as_bytes().get(root.len()) == Some(&b'/')
            })
        });
        if strip_root {
            let root = root.unwrap_or_default();
            files = files
                .into_iter()
                .filter_map(|(path, bytes)| {
                    let path = path.strip_prefix(&root)?.strip_prefix('/')?;
                    Some((path.to_owned(), bytes))
                })
                .collect();
        }
        let entry = ["game.rgss3a", "game.rgss2a", "game.rgssad"]
            .iter()
            .find_map(|candidate| files.keys().find(|path| path.ends_with(candidate)))
            .or_else(|| {
                files.keys().find(|path| {
                    matches!(
                        Path::new(path).extension().and_then(|ext| ext.to_str()),
                        Some("rgssad" | "rgss2a" | "rgss3a")
                    )
                })
            })
            .or_else(|| {
                [
                    "data/scripts.rvdata2",
                    "data/scripts.rvdata",
                    "data/scripts.rxdata",
                ]
                .iter()
                .find_map(|candidate| files.keys().find(|path| path == candidate))
            })
            .cloned()
            .ok_or_else(|| {
                "folder or ZIP does not contain a Game.rgss* archive or Data/Scripts.*".to_owned()
            })?;
        Ok(Self {
            label,
            entry,
            files,
        })
    }

    fn from_dropped_files(label: String, entries: Vec<(String, Vec<u8>)>) -> Result<Self, String> {
        let files = entries
            .into_iter()
            .map(|(path, bytes)| (normalize_browser_path(&path), bytes))
            .collect();
        Self::from_files(label, files)
    }
}

#[derive(Default)]
struct BrowserPlatform {
    files: Mutex<BTreeMap<String, Vec<u8>>>,
    saves: Mutex<BTreeMap<String, Vec<u8>>>,
    content_id: String,
    content_name: String,
}

impl BrowserPlatform {
    const SYSTEM_STORAGE_VERSION: &str = "v2";

    fn with_files(
        files: BTreeMap<String, Vec<u8>>,
        content_id: String,
        content_name: String,
    ) -> Self {
        Self {
            files: Mutex::new(files),
            saves: Mutex::new(BTreeMap::new()),
            content_id,
            content_name,
        }
    }

    fn storage() -> Option<web_sys::Storage> {
        web_sys::window()?.local_storage().ok().flatten()
    }

    fn save_key(&self, name: &str) -> String {
        format!("fanplay.save.{}.{}", self.content_id, name)
    }

    fn backup_key(&self, name: &str) -> String {
        format!("fanplay.backup.{}.{}", self.content_id, name)
    }

    fn system_key(name: &str) -> String {
        format!("fanplay.system.{}.{}", Self::SYSTEM_STORAGE_VERSION, name)
    }

    fn write_system(name: &str, bytes: &[u8]) -> PlatformResult<()> {
        let Some(storage) = Self::storage() else {
            return Err(PlatformError::unsupported("browser storage is unavailable"));
        };
        let name = Path::new(name)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(name)
            .to_ascii_lowercase();
        storage
            .set_item(&Self::system_key(&name), &BASE64.encode(bytes))
            .map_err(|error| {
                PlatformError::io(format!("could not persist browser system file: {error:?}"))
            })
    }

    fn create_backup(&self, name: &str) -> PlatformResult<()> {
        let Some(bytes) = self.read_save(name)? else {
            return Ok(());
        };
        let Some(storage) = Self::storage() else {
            return Err(PlatformError::unsupported("browser storage is unavailable"));
        };
        storage
            .set_item(&self.backup_key(name), &BASE64.encode(bytes))
            .map_err(|error| {
                PlatformError::io(format!("could not create browser backup: {error:?}"))
            })
    }

    fn restore_backup(&self, name: &str) -> PlatformResult<bool> {
        let Some(storage) = Self::storage() else {
            return Err(PlatformError::unsupported("browser storage is unavailable"));
        };
        let Some(encoded) = storage.get_item(&self.backup_key(name)).map_err(|error| {
            PlatformError::io(format!("could not read browser backup: {error:?}"))
        })?
        else {
            return Ok(false);
        };
        let bytes = BASE64
            .decode(encoded)
            .map_err(|error| PlatformError::io(format!("browser backup is corrupt: {error}")))?;
        self.write_save(name, &bytes)?;
        Ok(true)
    }
}

impl Platform for BrowserPlatform {
    fn read_content(&self, path: &str) -> PlatformResult<Vec<u8>> {
        let requested = normalize_browser_path(path);
        let content_name = normalize_browser_path(&self.content_name);
        let content_extension = Path::new(&content_name)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default();
        let default_script = match content_extension {
            "rxdata" => "data/scripts.rxdata",
            "rvdata" => "data/scripts.rvdata",
            "rvdata2" => "data/scripts.rvdata2",
            _ => "",
        };
        let files = self.files.lock().unwrap_or_else(|error| error.into_inner());
        let lookup = if requested.is_empty() {
            Some(content_name.as_str())
        } else if requested == content_name {
            Some(content_name.as_str())
        } else if !default_script.is_empty() && requested == default_script {
            Some(content_name.as_str())
        } else {
            Some(requested.as_str())
        };
        files
            .get(lookup.unwrap_or_default())
            .cloned()
            .ok_or_else(|| PlatformError::not_found(path))
    }

    fn read_system_file(&self, path: &str) -> PlatformResult<Option<Vec<u8>>> {
        let Some(name) = Path::new(path).file_name().and_then(|name| name.to_str()) else {
            return Ok(None);
        };
        if !matches!(
            name.to_ascii_lowercase().as_str(),
            "bios7.bin" | "bios9.bin" | "firmware.bin" | "gba_bios.bin" | "bios.bin"
        ) {
            return Ok(None);
        }
        let Some(storage) = Self::storage() else {
            return Ok(None);
        };
        let Some(encoded) = storage.get_item(&Self::system_key(name)).map_err(|error| {
            PlatformError::io(format!("could not read browser system file: {error:?}"))
        })?
        else {
            return Ok(None);
        };
        BASE64
            .decode(encoded)
            .map(Some)
            .map_err(|error| PlatformError::io(format!("browser system file is corrupt: {error}")))
    }

    fn read_save(&self, name: &str) -> PlatformResult<Option<Vec<u8>>> {
        if let Some(bytes) = self
            .saves
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(name)
            .cloned()
        {
            return Ok(Some(bytes));
        }
        let Some(storage) = Self::storage() else {
            return Ok(None);
        };
        let Some(encoded) = storage.get_item(&self.save_key(name)).map_err(|error| {
            PlatformError::io(format!("could not read browser save: {error:?}"))
        })?
        else {
            return Ok(None);
        };
        let bytes = BASE64
            .decode(encoded)
            .map_err(|error| PlatformError::io(format!("browser save is corrupt: {error}")))?;
        self.saves
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(name.to_owned(), bytes.clone());
        Ok(Some(bytes))
    }

    fn write_save(&self, name: &str, bytes: &[u8]) -> PlatformResult<()> {
        self.saves
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(name.to_owned(), bytes.to_vec());
        if let Some(storage) = Self::storage() {
            storage
                .set_item(&self.save_key(name), &BASE64.encode(bytes))
                .map_err(|error| {
                    PlatformError::io(format!("could not persist browser save: {error:?}"))
                })?;
        }
        Ok(())
    }

    fn save_dir(&self) -> PathBuf {
        PathBuf::new()
    }

    fn now(&self) -> f64 {
        web_sys::window()
            .and_then(|window| window.performance())
            .map_or(0.0, |performance| performance.now() / 1000.0)
    }

    fn log(&self, level: fanplay_core::LogLevel, message: &str) {
        let value = JsValue::from_str(message);
        match level {
            fanplay_core::LogLevel::Error => web_sys::console::error_1(&value),
            fanplay_core::LogLevel::Warn => web_sys::console::warn_1(&value),
            fanplay_core::LogLevel::Info | fanplay_core::LogLevel::Debug => {
                web_sys::console::log_1(&value)
            }
        }
    }
}

#[derive(Default)]
struct PresentedFrame {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    revision: u64,
}

struct BrowserVideo {
    frame: Arc<Mutex<PresentedFrame>>,
}

impl VideoSink for BrowserVideo {
    fn present(&mut self, frame: VideoFrame<'_>) {
        let mut presented = self.frame.lock().unwrap_or_else(|error| error.into_inner());
        if convert_rgba(&frame, &mut presented.rgba).is_err() {
            return;
        }
        presented.width = frame.width;
        presented.height = frame.height;
        presented.revision = presented.revision.wrapping_add(1);
    }
}

struct BrowserEvents {
    events: Arc<Mutex<Vec<CoreEvent>>>,
}

impl EventSink for BrowserEvents {
    fn emit(&mut self, event: CoreEvent) {
        self.events
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(event);
    }
}

#[derive(Clone)]
struct BrowserAudioSink {
    producer: PcmRingProducer,
    consumer: Arc<Mutex<PcmRingConsumer>>,
    config: AudioConfig,
    paused: Arc<AtomicBool>,
}

impl AudioSink for BrowserAudioSink {
    fn play_stream(&mut self, _stream: AudioStream<'_>) {}

    fn stop(&mut self, _channel: AudioChannel) {}

    fn fade(&mut self, _channel: AudioChannel, _milliseconds: i32) {}

    fn set_volume(&mut self, _channel: AudioChannel, _volume: i32) {}

    fn queue_pcm(&mut self, pcm: PcmBuffer<'_>) {
        if self.paused.load(Ordering::Acquire) || pcm.samples.is_empty() {
            return;
        }
        if pcm.channels == self.config.channels {
            self.producer.push(pcm.samples);
        } else if pcm.channels == 1 && self.config.channels == 2 {
            let mut stereo = Vec::with_capacity(pcm.samples.len() * 2);
            for sample in pcm.samples {
                stereo.extend_from_slice(&[*sample, *sample]);
            }
            self.producer.push(&stereo);
        }
    }

    fn pause(&mut self, paused: bool) {
        self.paused.store(paused, Ordering::Release);
    }

    fn output_config(&self) -> Option<AudioConfig> {
        Some(self.config)
    }

    fn reset_pcm(&mut self) {
        if let Ok(consumer) = self.consumer.lock() {
            consumer.reset();
        }
    }

    fn pcm_status(&self) -> fanplay_core::PcmStatus {
        self.consumer.lock().map_or_else(
            |_| fanplay_core::PcmStatus::default(),
            |consumer| consumer.status(2),
        )
    }
}

#[derive(Default)]
struct BrowserRuntimeState {
    control: Option<Arc<CoreControl>>,
    core_id: Option<&'static str>,
}

#[derive(Clone, Default)]
struct BrowserBackend {
    state: Arc<Mutex<BrowserRuntimeState>>,
}

impl BrowserBackend {
    fn attach(&self, control: Arc<CoreControl>, core_id: &'static str) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.control = Some(control);
        state.core_id = Some(core_id);
    }

    fn control(&self) -> Option<Arc<CoreControl>> {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .control
            .clone()
    }

    fn frame_budget(&self, base_fps: u32) -> usize {
        if !self.supports_speed_control() {
            return 1;
        }
        if self.fast_forward() {
            return 8;
        }
        let base_fps = base_fps.max(1);
        let rate = self.current_rate();
        if rate == 0 {
            1
        } else {
            rate.saturating_add(base_fps - 1)
                .checked_div(base_fps)
                .unwrap_or(1)
                .clamp(1, 16) as usize
        }
    }
}

impl Backend for BrowserBackend {
    fn fast_forward(&self) -> bool {
        self.control().is_some_and(|control| control.fast_forward())
    }

    fn set_fast_forward(&self, enabled: bool) {
        if let Some(control) = self.control() {
            control.set_fast_forward(enabled);
        }
    }

    fn supports_speed_control(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .core_id
            .is_none_or(|core_id| core_id != "gbc")
    }

    fn current_rate(&self) -> u32 {
        self.control()
            .and_then(|control| control.target_fps())
            .unwrap_or_default()
    }

    fn set_rate_override(&self, rate: u32) {
        if let Some(control) = self.control() {
            control.set_target_fps((rate > 0).then_some(rate));
        }
    }

    fn request_reset(&self) {
        if let Some(control) = self.control() {
            let _ = control.send(CoreCommand::Reset);
        }
    }

    fn patch_enabled(&self, id: &str) -> bool {
        fanplay_rgss::runtime_patch::is_enabled(id)
    }

    fn set_patch_enabled(&self, id: &str, enabled: bool) -> bool {
        fanplay_rgss::runtime_patch::set_enabled(id, enabled)
    }

    fn patch_value(&self, id: &str) -> Option<f64> {
        fanplay_rgss::runtime_patch::value(id)
    }

    fn set_patch_value(&self, id: &str, value: f64) -> bool {
        fanplay_rgss::runtime_patch::set_value(id, value)
    }

    fn asset_image(&self, _category: &str, _name: &str) -> Option<Arc<image::RgbaImage>> {
        None
    }

    fn handle_plugin_operation(
        &self,
        _operation: &str,
        _arguments: &Value,
    ) -> Option<Result<Value, String>> {
        None
    }
}

struct BrowserAudioDriver {
    context: web_sys::AudioContext,
    _node: web_sys::AudioNode,
    _callback: Closure<dyn FnMut(web_sys::AudioProcessingEvent)>,
}

struct BrowserAudio {
    sink: BrowserAudioSink,
    driver: Option<BrowserAudioDriver>,
}

impl BrowserAudio {
    fn new() -> Self {
        let (producer, consumer) = PcmRing::new(48_000 * 2 * 2);
        let consumer = Arc::new(Mutex::new(consumer));
        let paused = Arc::new(AtomicBool::new(false));
        let (config, driver) = match web_sys::AudioContext::new() {
            Ok(context) => {
                let sample_rate = context.sample_rate().round().max(1.0) as u32;
                let processor = context
                    .create_script_processor_with_buffer_size_and_number_of_input_channels_and_number_of_output_channels(
                        2048,
                        0,
                        2,
                    );
                match processor {
                    Ok(processor) => {
                        let callback_consumer = Arc::clone(&consumer);
                        let callback_paused = Arc::clone(&paused);
                        let callback =
                            Closure::wrap(Box::new(move |event: web_sys::AudioProcessingEvent| {
                                let Ok(output) = event.output_buffer() else {
                                    return;
                                };
                                let length = output.length() as usize;
                                let mut samples = vec![0_i16; length * 2];
                                if !callback_paused.load(Ordering::Acquire)
                                    && let Ok(consumer) = callback_consumer.lock()
                                {
                                    consumer.pop_fill(&mut samples);
                                }
                                let mut left = Vec::with_capacity(length);
                                let mut right = Vec::with_capacity(length);
                                for pair in samples.chunks_exact(2) {
                                    left.push(f32::from(pair[0]) / 32_768.0);
                                    right.push(f32::from(pair[1]) / 32_768.0);
                                }
                                let _ = output.copy_to_channel(&left, 0);
                                let _ = output.copy_to_channel(&right, 1);
                            })
                                as Box<dyn FnMut(web_sys::AudioProcessingEvent)>);
                        processor.set_onaudioprocess(Some(callback.as_ref().unchecked_ref()));
                        let node = processor.unchecked_into::<web_sys::AudioNode>();
                        let destination =
                            context.destination().unchecked_into::<web_sys::AudioNode>();
                        let connected = node.connect_with_audio_node(&destination).is_ok();
                        if connected {
                            (
                                AudioConfig::new(sample_rate, 2),
                                Some(BrowserAudioDriver {
                                    context,
                                    _node: node,
                                    _callback: callback,
                                }),
                            )
                        } else {
                            (AudioConfig::new(sample_rate, 2), None)
                        }
                    }
                    Err(_) => (AudioConfig::new(sample_rate, 2), None),
                }
            }
            Err(_) => (AudioConfig::new(48_000, 2), None),
        };
        Self {
            sink: BrowserAudioSink {
                producer,
                consumer,
                config,
                paused,
            },
            driver,
        }
    }

    fn sink(&self) -> BrowserAudioSink {
        self.sink.clone()
    }

    fn resume(&self) {
        if let Some(driver) = &self.driver {
            let _ = driver.context.resume();
        }
    }
}

struct WebSession {
    core: Box<dyn Core>,
    control: Arc<CoreControl>,
    platform: Arc<BrowserPlatform>,
    frame: Arc<Mutex<PresentedFrame>>,
    events: Arc<Mutex<Vec<CoreEvent>>>,
    plugin_responses: Vec<OperationResponse>,
    info: CoreInfo,
    save_name: String,
    state_slots: BTreeMap<String, Vec<u8>>,
    frames: u64,
    uploaded_revision: u64,
    running: bool,
}

impl WebSession {
    fn load(game: BrowserGame, audio: Box<dyn AudioSink>) -> Result<Self, String> {
        let BrowserGame {
            label: name,
            entry,
            files,
        } = game;
        let bytes = files
            .get(&entry)
            .cloned()
            .ok_or_else(|| format!("selected RGSS entry {entry} is missing"))?;
        let path = PathBuf::from(&entry);
        let header_len = bytes.len().min(64);
        let probe = ContentProbe::new(&path, false, &bytes[..header_len]);
        let catalog = fanplay_cores::catalog();
        let registration = catalog
            .route_probe(&probe)
            .map_err(|error| error.to_string())?;
        let info = registration.info;
        let content_id = ContentId::of_bytes(&bytes).hex();
        let platform = Arc::new(BrowserPlatform::with_files(
            files,
            content_id.clone(),
            entry.clone(),
        ));
        let state_slots = Self::load_state_slots(&content_id);
        let frame = Arc::new(Mutex::new(PresentedFrame::default()));
        let events = Arc::new(Mutex::new(Vec::new()));
        let control = Arc::new(CoreControl::new());
        let context = CoreContext::new(
            Arc::clone(&platform) as Arc<dyn Platform>,
            Box::new(BrowserVideo {
                frame: Arc::clone(&frame),
            }),
            audio,
            Box::new(BrowserEvents {
                events: Arc::clone(&events),
            }),
            Arc::clone(&control),
        )
        .with_session(SessionConfig::headless());
        let mut core = (registration.create)();
        core.load(Content::file(path).with_label(name.clone()), context)
            .map_err(|error| error.to_string())?;

        let save_name = format!(
            "{}.sav",
            Path::new(&name)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("game")
        );
        Ok(Self {
            core,
            control,
            platform,
            frame,
            events,
            plugin_responses: Vec::new(),
            info,
            save_name,
            state_slots,
            frames: 0,
            uploaded_revision: 0,
            running: true,
        })
    }

    fn tick(&mut self) -> Result<(), String> {
        if !self.running {
            return Ok(());
        }
        let paused = self.control.paused();
        match self.core.run_slice().map_err(|error| error.to_string())? {
            RunSliceStatus::Continue => {}
            RunSliceStatus::Exited(_status) => self.running = false,
        }
        if !paused {
            self.frames = self.frames.saturating_add(1);
        }
        self.process_events()?;
        Ok(())
    }

    fn process_events(&mut self) -> Result<(), String> {
        let events = std::mem::take(
            &mut *self
                .events
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        );
        for event in events {
            match event {
                CoreEvent::SaveRam { data } => self
                    .platform
                    .write_save(&self.save_name, &data)
                    .map_err(|error| error.to_string())?,
                CoreEvent::SaveState { slot, data } => {
                    self.persist_state(&slot, &data)?;
                    self.state_slots.insert(slot, data);
                }
                CoreEvent::Error(message) => return Err(message),
                CoreEvent::Exited(ExitStatus { .. }) => self.running = false,
                CoreEvent::Loaded(_)
                | CoreEvent::Window(_)
                | CoreEvent::Log(_, _)
                | CoreEvent::Screenshot { .. }
                | CoreEvent::Status(_) => {}
                CoreEvent::CommandResult {
                    response: Some(response),
                    ..
                } => {
                    if let Some(response) = decode_plugin_response(&response) {
                        self.plugin_responses.push(response);
                    }
                }
                CoreEvent::CommandResult { response: None, .. } => {}
            }
        }
        Ok(())
    }

    fn upload_frame(&mut self, shared: &mut SharedApp, context: &Context) {
        let frame = self.frame.lock().unwrap_or_else(|error| error.into_inner());
        if frame.revision == 0 || frame.revision == self.uploaded_revision {
            return;
        }
        shared.set_rgba_frame(context, frame.width, frame.height, &frame.rgba);
        self.uploaded_revision = frame.revision;
    }

    fn stop(&self) {
        self.control.stop();
    }

    fn load_state_slots(content_id: &str) -> BTreeMap<String, Vec<u8>> {
        let mut slots = BTreeMap::new();
        let Some(storage) = BrowserPlatform::storage() else {
            return slots;
        };
        let prefix = format!("fanplay.state.{content_id}.");
        let length = storage.length().unwrap_or(0);
        for index in 0..length {
            let Some(key) = storage.key(index).ok().flatten() else {
                continue;
            };
            let Some(slot) = key.strip_prefix(&prefix) else {
                continue;
            };
            let Ok(Some(encoded)) = storage.get_item(&key) else {
                continue;
            };
            if let Ok(data) = BASE64.decode(encoded) {
                slots.insert(slot.to_owned(), data);
            }
        }
        slots
    }

    fn persist_state(&self, slot: &str, data: &[u8]) -> Result<(), String> {
        let Some(storage) = BrowserPlatform::storage() else {
            return Err("browser storage is unavailable".into());
        };
        storage
            .set_item(
                &format!("fanplay.state.{}.{}", self.platform.content_id, slot),
                &BASE64.encode(data),
            )
            .map_err(|error| format!("could not persist save state: {error:?}"))
    }

    fn state_slots(&self) -> Vec<StateSlot> {
        (1..=4)
            .map(|slot| {
                let slot = slot.to_string();
                let data = self.state_slots.get(&slot);
                StateSlot {
                    slot,
                    saved_at: data.map(|_| "saved this session".into()),
                    bytes: data.map_or(0, Vec::len),
                }
            })
            .collect()
    }

    fn save_state(&self, slot: String) {
        let _ = self.control.send(CoreCommand::SaveState { slot });
    }

    fn load_state(&self, slot: String) {
        let Some(data) = self.state_slots.get(&slot).cloned() else {
            return;
        };
        let _ = self.control.send(CoreCommand::LoadState { slot, data });
    }

    fn take_plugin_responses(&mut self) -> Vec<OperationResponse> {
        std::mem::take(&mut self.plugin_responses)
    }
}

enum PendingFile {
    Game(Result<BrowserGame, String>),
    Save(String, Result<Vec<u8>, String>),
    Plugin(String, Result<Vec<u8>, String>),
    System(String, Result<Vec<u8>, String>),
}

#[derive(Clone, Copy)]
enum FileKind {
    Game,
    Save,
    Plugin,
    System,
}

type PendingFiles = Arc<Mutex<Vec<PendingFile>>>;
type UploadStatus = Arc<Mutex<Option<String>>>;
type FileChange = Closure<dyn FnMut(web_sys::Event)>;
type KeyboardGuard = Closure<dyn FnMut(web_sys::KeyboardEvent)>;

struct WebApp {
    shared: SharedApp,
    backend: BrowserBackend,
    session: Option<WebSession>,
    audio: BrowserAudio,
    library: BTreeMap<String, BrowserGame>,
    pending_files: PendingFiles,
    upload_status: UploadStatus,
    file_input: Option<(web_sys::HtmlInputElement, FileChange)>,
    held_buttons: ButtonSet,
    _keyboard_guard: KeyboardGuard,
}

impl WebApp {
    fn new(keyboard_guard: KeyboardGuard) -> Self {
        let backend = BrowserBackend::default();
        let mut shared = SharedApp::default();
        shared.set_backend(Box::new(backend.clone()));
        shared.set_info(WorkspaceInfo {
            core_id: "gbc",
            logical_size: (160, 144),
            fps: 0.0,
            base_fps: 60,
            frames: 0,
            supports_save_states: true,
            state_slots: Vec::<StateSlot>::new(),
            global_fullscreen: false,
            show_fullscreen: true,
            show_save_buttons: true,
        });
        let mut session = SessionPanel::default();
        session.capabilities = SessionCapabilities::WEB;
        session.status = "Drop a GB, GBC, GBA, NDS ROM, or RGSS archive to begin".into();
        shared.workspace.session = Some(session);
        Self {
            shared,
            backend,
            session: None,
            audio: BrowserAudio::new(),
            library: BTreeMap::new(),
            pending_files: Arc::new(Mutex::new(Vec::new())),
            upload_status: Arc::new(Mutex::new(None)),
            file_input: None,
            held_buttons: ButtonSet::empty(),
            _keyboard_guard: keyboard_guard,
        }
    }

    fn collect_input(&mut self, context: &Context) {
        let Some(session) = &self.session else {
            return;
        };
        let control = Arc::clone(&session.control);
        if context.egui_wants_keyboard_input() {
            if self.held_buttons.any(u32::MAX) {
                control
                    .input()
                    .set_button_bits(self.held_buttons.bits(), false);
                self.held_buttons = ButtonSet::empty();
            }
            return;
        }
        let events = context.input(|input| input.events.clone());
        let touch_rect = self.shared.touch_rect();
        for event in events {
            match event {
                egui::Event::Key {
                    key,
                    physical_key,
                    pressed,
                    repeat: false,
                    ..
                } => {
                    let Some(button) = keyboard_button(physical_key.unwrap_or(key)) else {
                        continue;
                    };
                    control.input().set_button(button, pressed);
                    self.held_buttons.set(button, pressed);
                }
                egui::Event::PointerMoved(position) => {
                    update_touch_input(&control, touch_rect, position, false);
                }
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    ..
                } => {
                    update_touch_input(&control, touch_rect, pos, pressed);
                }
                _ => {}
            }
        }
    }

    fn collect_dropped_files(&self, context: &Context) {
        let upload_status = Arc::clone(&self.upload_status);
        let files = context.input(|input| input.raw.dropped_files.clone());
        for file in files {
            let name = file.path().to_string_lossy().into_owned();
            let kind = if is_system_file_name(&name) {
                FileKind::System
            } else {
                FileKind::Game
            };
            let pending = Arc::clone(&self.pending_files);
            let upload_status = Arc::clone(&upload_status);
            let repaint = context.clone();
            wasm_bindgen_futures::spawn_local(async move {
                *upload_status
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) = Some(format!("Reading {name}…"));
                repaint.request_repaint();
                let pending_file = match kind {
                    FileKind::Game => {
                        let result = file
                            .bytes_async()
                            .await
                            .and_then(|bytes| BrowserGame::from_single(name.clone(), bytes));
                        repaint.request_repaint();
                        PendingFile::Game(result)
                    }
                    FileKind::System => PendingFile::System(name, file.bytes_async().await),
                    FileKind::Save | FileKind::Plugin => unreachable!(),
                };
                pending
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push(pending_file);
            });
        }
    }

    fn start_pending_files(&mut self) {
        if let Some(status) = self
            .upload_status
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            if let Some(panel) = self.shared.workspace.session.as_mut() {
                panel.status = status;
                panel.busy = true;
            }
        }
        let files = std::mem::take(
            &mut *self
                .pending_files
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        );
        if !files.is_empty() {
            if let Some(panel) = self.shared.workspace.session.as_mut() {
                panel.busy = false;
            }
        }
        for file in files {
            match file {
                PendingFile::Game(Ok(game)) => self.start_game(game),
                PendingFile::Game(Err(error))
                | PendingFile::Save(_, Err(error))
                | PendingFile::Plugin(_, Err(error))
                | PendingFile::System(_, Err(error)) => self.report_error(error),
                PendingFile::Save(name, Ok(bytes)) => {
                    if let Some(session) = &self.session {
                        if let Err(error) = session.platform.write_save(&session.save_name, &bytes)
                        {
                            self.report_error(error.to_string());
                        } else if let Some(panel) = self.shared.workspace.session.as_mut() {
                            panel.status = format!("Imported {name}");
                        }
                    }
                }
                PendingFile::Plugin(name, Ok(bytes)) => {
                    let id = Path::new(&name)
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .filter(|stem| !stem.is_empty())
                        .unwrap_or("browser-plugin")
                        .to_owned();
                    match String::from_utf8(bytes) {
                        Ok(source) => {
                            if let Err(error) = self.shared.paste_plugin(id, source) {
                                self.report_error(error);
                            } else if let Some(panel) = self.shared.workspace.session.as_mut() {
                                panel.status = format!("Loaded plugin {name}");
                            }
                        }
                        Err(error) => {
                            self.report_error(format!("plugin {name} is not UTF-8: {error}"))
                        }
                    }
                }
                PendingFile::System(name, Ok(bytes)) => {
                    if let Err(error) = BrowserPlatform::write_system(&name, &bytes) {
                        self.report_error(error.to_string());
                    } else if let Some(panel) = self.shared.workspace.session.as_mut() {
                        panel.status = format!("Installed browser system file {name}");
                    }
                }
            }
        }
    }

    fn start_game(&mut self, game: BrowserGame) {
        let name = game.label.clone();
        if let Some(session) = &self.session {
            session.stop();
        }
        self.audio.resume();
        self.library.insert(name.clone(), game.clone());
        match WebSession::load(game, Box::new(self.audio.sink())) {
            Ok(session) => {
                let info = session.info;
                self.backend.attach(Arc::clone(&session.control), info.id);
                let logical_size = if info.id == "nds" {
                    (256, 384)
                } else if info.id == "rgss" {
                    (544, 416)
                } else {
                    info.screens
                        .first()
                        .map_or((160, 144), |screen| (screen.width, screen.height))
                };
                let state_slots = session.state_slots();
                self.shared.set_info(WorkspaceInfo {
                    core_id: info.id,
                    logical_size,
                    fps: 0.0,
                    base_fps: 60,
                    frames: 0,
                    supports_save_states: info.capabilities.contains(CoreCapabilities::SAVE_STATES),
                    state_slots,
                    global_fullscreen: false,
                    show_fullscreen: true,
                    show_save_buttons: true,
                });
                if let Some(panel) = self.shared.workspace.session.as_mut() {
                    panel.active = true;
                    panel.busy = false;
                    panel.error = None;
                    panel.title = name.clone();
                    panel.source = info.id.into();
                    panel.status = "Running".into();
                    panel.recent.retain(|recent| recent != &name);
                    panel.recent.insert(0, name);
                    panel.recent.truncate(24);
                }
                self.shared.reset_plugins();
                if let Err(error) = self.shared.activate_plugins_for_core(info.id) {
                    self.report_error(error);
                }
                self.session = Some(session);
            }
            Err(error) => self.report_error(error),
        }
    }

    fn report_error(&mut self, error: String) {
        if let Some(panel) = self.shared.workspace.session.as_mut() {
            panel.active = false;
            panel.busy = false;
            panel.error = Some(error);
            panel.status = "Could not load game".into();
        }
    }

    fn open_file_picker(&mut self, context: &Context) -> Result<(), String> {
        self.open_picker(
            ".gb,.gbc,.gba,.nds,.bin,.zip,.rgssad,.rgss2a,.rgss3a,.rxdata,.rvdata,.rvdata2",
            FileKind::Game,
            false,
            context,
        )
    }

    fn open_folder_picker(&mut self, context: &Context) -> Result<(), String> {
        self.open_picker("", FileKind::Game, true, context)
    }

    fn open_save_picker(&mut self, context: &Context) -> Result<(), String> {
        self.open_picker(".sav,.srm,.dat", FileKind::Save, false, context)
    }

    fn open_plugin_picker(&mut self, context: &Context) -> Result<(), String> {
        self.open_picker(".rhai", FileKind::Plugin, false, context)
    }

    fn open_picker(
        &mut self,
        accept: &str,
        kind: FileKind,
        directory: bool,
        context: &Context,
    ) -> Result<(), String> {
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| "browser document is unavailable".to_owned())?;
        let input = document
            .create_element("input")
            .map_err(|error| format!("could not create file input: {error:?}"))?
            .dyn_into::<web_sys::HtmlInputElement>()
            .map_err(|_| "created file input has the wrong type".to_owned())?;
        input.set_type("file");
        if !accept.is_empty() {
            input.set_accept(accept);
        }
        if directory {
            input.set_multiple(true);
            input
                .set_attribute("webkitdirectory", "")
                .map_err(|error| format!("could not enable folder selection: {error:?}"))?;
        }
        input
            .style()
            .set_property("display", "none")
            .map_err(|error| format!("could not hide file input: {error:?}"))?;
        let input_for_callback = input.clone();
        let pending = Arc::clone(&self.pending_files);
        let upload_status = Arc::clone(&self.upload_status);
        let repaint = context.clone();
        *upload_status
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(if directory {
            "Preparing folder upload…".to_owned()
        } else {
            "Preparing file upload…".to_owned()
        });
        context.request_repaint();
        let callback = Closure::wrap(Box::new(move |_event: web_sys::Event| {
            let Some(file_list) = input_for_callback.files() else {
                return;
            };
            let pending = Arc::clone(&pending);
            let upload_status = Arc::clone(&upload_status);
            let repaint = repaint.clone();
            let selected = (0..file_list.length())
                .filter_map(|index| file_list.get(index))
                .collect::<Vec<_>>();
            wasm_bindgen_futures::spawn_local(async move {
                let pending_file = if directory {
                    let total = selected.len();
                    let label = selected
                        .first()
                        .map(relative_browser_file_path)
                        .and_then(|path| path.split('/').next().map(str::to_owned))
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| "game".to_owned())
                        .to_owned();
                    let mut entries = Vec::with_capacity(selected.len());
                    let mut error = None;
                    for (index, file) in selected.into_iter().enumerate() {
                        let path = relative_browser_file_path(&file);
                        match file.array_buffer().await {
                            Ok(buffer) => {
                                entries.push((path, js_sys::Uint8Array::new(&buffer).to_vec()));
                                let percent = if total == 0 {
                                    100
                                } else {
                                    ((index + 1) * 100) / total
                                };
                                *upload_status
                                    .lock()
                                    .unwrap_or_else(|error| error.into_inner()) = Some(format!(
                                    "Reading RGSS folder: {}/{} files ({}%)",
                                    index + 1,
                                    total,
                                    percent
                                ));
                                repaint.request_repaint();
                            }
                            Err(value) => {
                                error = Some(format!("could not read folder entry: {value:?}"));
                                break;
                            }
                        }
                    }
                    *upload_status
                        .lock()
                        .unwrap_or_else(|error| error.into_inner()) =
                        Some("Indexing RGSS folder…".to_owned());
                    repaint.request_repaint();
                    PendingFile::Game(
                        error.map_or_else(|| BrowserGame::from_dropped_files(label, entries), Err),
                    )
                } else {
                    let Some(file) = selected.into_iter().next() else {
                        return;
                    };
                    let name = file.name();
                    let size = file.size() as u64;
                    *upload_status
                        .lock()
                        .unwrap_or_else(|error| error.into_inner()) =
                        Some(format!("Reading {name} ({})…", format_bytes(size)));
                    repaint.request_repaint();
                    let result = file
                        .array_buffer()
                        .await
                        .map(|buffer| js_sys::Uint8Array::new(&buffer).to_vec())
                        .map_err(|error| format!("could not read {name}: {error:?}"));
                    let kind = if matches!(kind, FileKind::Game) && is_system_file_name(&name) {
                        FileKind::System
                    } else {
                        kind
                    };
                    match kind {
                        FileKind::Game => PendingFile::Game(result.and_then(|bytes| {
                            if name.to_ascii_lowercase().ends_with(".zip") {
                                *upload_status
                                    .lock()
                                    .unwrap_or_else(|error| error.into_inner()) =
                                    Some(format!("Extracting {name}…"));
                                repaint.request_repaint();
                            }
                            BrowserGame::from_single(name, bytes)
                        })),
                        FileKind::Save => PendingFile::Save(name, result),
                        FileKind::Plugin => PendingFile::Plugin(name, result),
                        FileKind::System => PendingFile::System(name, result),
                    }
                };
                pending
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push(pending_file);
            });
        }) as Box<dyn FnMut(web_sys::Event)>);
        input.set_onchange(Some(callback.as_ref().unchecked_ref()));
        document
            .body()
            .ok_or_else(|| "browser document has no body".to_owned())?
            .append_child(&input)
            .map_err(|error| format!("could not attach file input: {error:?}"))?;
        input.click();
        self.file_input = Some((input, callback));
        Ok(())
    }

    fn handle_session_actions(&mut self, context: &Context) {
        let actions = std::mem::take(&mut self.shared.workspace.session_actions);
        if self.shared.take_plugin_import_request() {
            if let Err(error) = self.open_plugin_picker(context) {
                self.report_error(error);
            }
        }
        if let Some(name) = actions.open {
            if let Some(game) = self.library.get(&name).cloned() {
                self.start_game(game);
            } else {
                self.report_error(format!("game {name:?} is no longer available in this tab"));
            }
        }
        if actions.browse {
            if let Err(error) = self.open_folder_picker(context) {
                self.report_error(error);
            }
        }
        if actions.browse_game_file {
            if let Err(error) = self.open_file_picker(context) {
                self.report_error(error);
            }
        }
        if actions.stop {
            if let Some(session) = &self.session {
                session.stop();
            }
        }
        if actions.restart {
            if let Some(session) = &self.session {
                session.control.set_paused(false);
                let _ = session.control.send(CoreCommand::Reset);
                if let Some(panel) = self.shared.workspace.session.as_mut() {
                    panel.active = true;
                    panel.paused = false;
                    panel.status = "Running".into();
                }
            }
        }
        let (output_export, output_import, output_backup, output_restore) =
            self.shared.take_save_actions();
        if actions.export_saves || output_export {
            if let Some(session) = &self.session {
                match session.platform.read_save(&session.save_name) {
                    Ok(Some(bytes)) => {
                        if let Err(error) = download_bytes(&session.save_name, &bytes) {
                            self.report_error(error);
                        }
                    }
                    Ok(None) => self.report_error("there is no battery save to export".into()),
                    Err(error) => self.report_error(error.to_string()),
                }
            }
        }
        if actions.import_saves || output_import {
            if let Err(error) = self.open_save_picker(context) {
                self.report_error(error);
            }
        }
        if actions.create_backup || output_backup {
            if let Some(session) = &self.session {
                match session.platform.create_backup(&session.save_name) {
                    Ok(()) => {
                        if let Some(panel) = self.shared.workspace.session.as_mut() {
                            panel.status = "Browser save backup created".into();
                        }
                    }
                    Err(error) => self.report_error(error.to_string()),
                }
            }
        }
        if actions.restore_latest || output_restore {
            if let Some(session) = &self.session {
                if session.running {
                    self.report_error(
                        "stop the game before restoring a browser save backup".into(),
                    );
                    return;
                }
                match session.platform.restore_backup(&session.save_name) {
                    Ok(true) => {
                        if let Some(panel) = self.shared.workspace.session.as_mut() {
                            panel.status = "Browser save backup restored".into();
                        }
                    }
                    Ok(false) => self.report_error("no browser save backup exists".into()),
                    Err(error) => self.report_error(error.to_string()),
                }
            }
        }
        let (save_slot, load_slot) = self.shared.take_state_actions();
        if let Some(session) = &self.session {
            if let Some(slot) = save_slot {
                session.save_state(slot);
            }
            if let Some(slot) = load_slot {
                session.load_state(slot);
            }
        }
    }

    fn poll_plugins(&mut self) {
        for request in self.shared.take_plugin_requests() {
            if let Some((core, operation)) =
                request.operation.split_once(".__").and_then(|(_, suffix)| {
                    let (core, operation) = suffix.split_once('.')?;
                    matches!(core, "gba" | "nds").then_some((core, operation))
                })
            {
                let Some(session) = &self.session else {
                    let _ = self.shared.dispatch_plugin_response(OperationResponse {
                        id: request.id,
                        generation: request.generation,
                        result: Err(OperationError::new(
                            "no_session",
                            "no active cartridge session",
                        )),
                    });
                    continue;
                };
                let payload = serde_json::json!({
                    "request_id": request.id,
                    "generation": request.generation,
                    "operation": operation,
                    "arguments": request.arguments,
                });
                let command = CoreCommand::Extension {
                    id: format!("{core}.rhai"),
                    payload: match serde_json::to_vec(&payload) {
                        Ok(payload) => payload,
                        Err(error) => {
                            let _ = self.shared.dispatch_plugin_response(OperationResponse {
                                id: request.id,
                                generation: request.generation,
                                result: Err(OperationError::new("encode", error.to_string())),
                            });
                            continue;
                        }
                    },
                };
                if let Err(error) = session.control.send_request(command) {
                    let _ = self.shared.dispatch_plugin_response(OperationResponse {
                        id: request.id,
                        generation: request.generation,
                        result: Err(OperationError::new("queue", error.to_string())),
                    });
                }
                continue;
            }

            let _ = self.shared.dispatch_plugin_response(OperationResponse {
                id: request.id,
                generation: request.generation,
                result: Err(OperationError::new(
                    "unsupported_operation",
                    request.operation,
                )),
            });
        }

        if let Some(session) = &mut self.session {
            for response in session.take_plugin_responses() {
                let _ = self.shared.dispatch_plugin_response(response);
            }
        }
    }
}

impl App for WebApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut Frame) {
        let context = ui.ctx().clone();
        self.collect_dropped_files(&context);
        self.start_pending_files();
        self.collect_input(&context);

        let paused = self
            .shared
            .workspace
            .session
            .as_ref()
            .is_some_and(|panel| panel.paused);
        let mut session_error = None;
        if let Some(session) = &mut self.session {
            session.control.set_paused(paused);
            if session.running {
                let budget = self.backend.frame_budget(60);
                for _ in 0..budget {
                    if let Err(error) = session.tick() {
                        session_error = Some(error);
                        break;
                    }
                    if !session.running {
                        break;
                    }
                }
                session.upload_frame(&mut self.shared, &context);
                self.shared
                    .set_performance(59.7 * budget as f64, session.frames);
                self.shared.set_state_slots(session.state_slots());
            } else if let Some(panel) = self.shared.workspace.session.as_mut() {
                panel.active = false;
            }
        }
        if let Some(error) = session_error {
            self.report_error(error);
        }

        self.poll_plugins();
        self.shared.draw(ui);
        self.handle_session_actions(&context);
        if self.session.as_ref().is_some_and(|session| session.running) {
            context.request_repaint();
        }
    }
}

fn keyboard_button(key: egui::Key) -> Option<Button> {
    use egui::Key;
    Some(match key {
        Key::ArrowUp => Button::Up,
        Key::ArrowDown => Button::Down,
        Key::ArrowLeft => Button::Left,
        Key::ArrowRight => Button::Right,
        Key::Z => Button::South,
        Key::X => Button::East,
        Key::Q => Button::LeftShoulder,
        Key::W => Button::RightShoulder,
        Key::Enter => Button::Start,
        Key::Backspace => Button::Select,
        _ => return None,
    })
}

fn is_system_file_name(name: &str) -> bool {
    matches!(
        Path::new(name)
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.to_ascii_lowercase()),
        Some(name)
            if matches!(
                name.as_str(),
                "bios7.bin" | "bios9.bin" | "firmware.bin" | "gba_bios.bin" | "bios.bin"
            )
    )
}

fn relative_browser_file_path(file: &web_sys::File) -> String {
    let path = js_sys::Reflect::get(file.as_ref(), &JsValue::from_str("webkitRelativePath"))
        .ok()
        .and_then(|value| value.as_string())
        .unwrap_or_default();
    if path.is_empty() {
        file.name()
    } else {
        path
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

fn install_browser_key_guard() -> Result<KeyboardGuard, JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("browser window is missing"))?;
    let callback = Closure::wrap(Box::new(move |event: web_sys::KeyboardEvent| {
        let key = event.key();
        let function_key = key
            .strip_prefix('F')
            .and_then(|number| number.parse::<u8>().ok())
            .is_some_and(|number| (1..=12).contains(&number));
        let text_target = event
            .target()
            .and_then(|target| target.dyn_into::<web_sys::HtmlElement>().ok())
            .is_some_and(|element| matches!(element.tag_name().as_str(), "INPUT" | "TEXTAREA"));
        let game_key = matches!(
            key.as_str(),
            "ArrowUp"
                | "ArrowDown"
                | "ArrowLeft"
                | "ArrowRight"
                | "Enter"
                | "Backspace"
                | "z"
                | "x"
                | "q"
                | "w"
        );
        if function_key || (game_key && !text_target) {
            event.prevent_default();
        }
    }) as Box<dyn FnMut(web_sys::KeyboardEvent)>);
    window.add_event_listener_with_callback("keydown", callback.as_ref().unchecked_ref())?;
    Ok(callback)
}

fn update_touch_input(
    control: &CoreControl,
    rect: egui::Rect,
    position: egui::Pos2,
    pressed: bool,
) {
    if rect != egui::Rect::ZERO && rect.contains(position) {
        let x = ((position.x - rect.min.x) / rect.width() * 256.0)
            .floor()
            .clamp(0.0, 255.0) as i32;
        let y = 192
            + ((position.y - rect.min.y) / rect.height() * 192.0)
                .floor()
                .clamp(0.0, 191.0) as i32;
        control.input().set_pointer(x, y, true);
        control.input().set_button(Button::Extra0, pressed);
    } else if !pressed {
        control.input().set_pointer(0, 0, false);
        control.input().set_button(Button::Extra0, false);
    }
}

fn download_bytes(name: &str, bytes: &[u8]) -> Result<(), String> {
    let document = web_sys::window()
        .and_then(|window| window.document())
        .ok_or_else(|| "browser document is unavailable".to_owned())?;
    let parts = js_sys::Array::new();
    parts.push(&js_sys::Uint8Array::from(bytes).into());
    let blob = web_sys::Blob::new_with_u8_array_sequence(&parts.into())
        .map_err(|error| format!("could not create download: {error:?}"))?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)
        .map_err(|error| format!("could not create download URL: {error:?}"))?;
    let anchor = document
        .create_element("a")
        .map_err(|error| format!("could not create download link: {error:?}"))?
        .dyn_into::<web_sys::HtmlAnchorElement>()
        .map_err(|_| "download link has the wrong type".to_owned())?;
    anchor.set_href(&url);
    anchor.set_download(name);
    web_sys::HtmlElement::click(&anchor);
    let _ = web_sys::Url::revoke_object_url(&url);
    Ok(())
}

fn decode_plugin_response(bytes: &[u8]) -> Option<OperationResponse> {
    let value = serde_json::from_slice::<Value>(bytes).ok()?;
    let id = value.get("request_id")?.as_u64()?;
    let generation = value.get("generation")?.as_u64()?;
    let result = if let Some(result) = value.get("result") {
        Ok(result.clone())
    } else {
        let error = value.get("error")?;
        Err(OperationError::new(
            error.get("code").and_then(Value::as_str).unwrap_or("core"),
            error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("core operation failed"),
        ))
    };
    Some(OperationResponse {
        id,
        generation,
        result,
    })
}

#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    let document = web_sys::window()
        .and_then(|window| window.document())
        .ok_or_else(|| JsValue::from_str("browser document is unavailable"))?;
    let canvas = document
        .get_element_by_id("fanplay-canvas")
        .ok_or_else(|| JsValue::from_str("fanplay canvas is missing"))?
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .map_err(|_| JsValue::from_str("fanplay canvas has the wrong type"))?;
    let keyboard_guard = install_browser_key_guard()?;
    let runner = WebRunner::new();
    wasm_bindgen_futures::spawn_local(async move {
        let ready =
            js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("__fanplayRubyReady"));
        match ready {
            Ok(value) if value.is_instance_of::<js_sys::Promise>() => {
                if let Err(error) = JsFuture::from(js_sys::Promise::from(value)).await {
                    web_sys::console::error_1(&error);
                    return;
                }
            }
            Ok(_) => {}
            Err(error) => {
                web_sys::console::error_1(&error);
                return;
            }
        }
        let options = WebOptions {
            max_fps: Some(60),
            ..Default::default()
        };
        if let Err(error) = runner
            .start(
                canvas,
                options,
                Box::new(move |_creation_context| Ok(Box::new(WebApp::new(keyboard_guard)))),
            )
            .await
        {
            web_sys::console::error_1(&error);
        }
    });
    Ok(())
}
