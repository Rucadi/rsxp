// SPDX-License-Identifier: GPL-2.0-or-later
//! Minimal headless frontend for the generic core contract.
//!
//! This is intentionally not the desktop UI. It exists so new cores can be
//! exercised end-to-end (content routing, dedicated core thread, frame/audio
//! sinks, save-state and battery-save events) before the full frontend grows
//! core selection.
//!
//! User data is content-scoped under `<save-dir>/<core>/<content-hash>/`.
//! Save states use the versioned `fanplay_core::StateEnvelope`; a raw state from an
//! older build is rejected rather than restored without validation. Writes are
//! queued on a bounded worker and flushed before the runner reports success.
//!
//! ```sh
//! cargo run -p fanplay-core-runner -- --content game.gba --frames 120 --output frame.ppm
//! cargo run -p fanplay-core-runner --features rgss -- \
//!   --content game/Game.ini --frames 60
//! ```

use fanplay_core::{
    AudioChannel, AudioSink, AudioStream, Content, CoreCommand, CoreControl, CoreEvent,
    CoreLoadInfo, EventSink, LogLevel, PcmBuffer, PixelFormat, Platform, PlatformError,
    PlatformResult, SessionConfig, VideoFrame, VideoSink,
};
use fanplay_frontend::{spawn_core, ContentId, SaveStore, StorageWorker};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    if let Err(error) = run() {
        eprintln!("fanplay-core-runner: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let options = Options::parse(std::env::args().skip(1))?;
    let path = options
        .content
        .canonicalize()
        .map_err(|error| format!("{}: {error}", options.content.display()))?;
    let content = if path.is_dir() {
        Content::directory(path)
    } else {
        Content::file(path)
    };
    let catalog = fanplay_cores::catalog();
    let registration = match &options.core {
        Some(id) => *catalog
            .get(id)
            .ok_or_else(|| format!("core '{id}' is not enabled in this build"))?,
        None => catalog.route(&content).map_err(|error| error.to_string())?,
    };
    let root = content
        .path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let base = options.save_dir.clone().unwrap_or_else(|| {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
            .map(|base| {
                let current = base.join("fanplay/saves");
                let legacy = base.join("rsxp/saves");
                if !current.is_dir() && legacy.is_dir() { legacy } else { current }
            })
            .unwrap_or_else(|| {
                let current = root.join(".fanplay-saves");
                let legacy = root.join(".rsxp-saves");
                if !current.is_dir() && legacy.is_dir() { legacy } else { current }
            })
    });
    let content_id = ContentId::of(&content)
        .map_err(|error| format!("could not identify {}: {error}", content.path.display()))?;
    let store = SaveStore::new(&base, registration.info.id, content_id)
        .map_err(|error| format!("could not open save store {}: {error}", base.display()))?;
    // Preserve title-based saves from older runner builds without deleting them.
    let legacy_stem = safe_name(&content.label);
    if let Ok(copied) = store.migrate_named_from(&base, &legacy_stem)
        && copied > 0
    {
        eprintln!("fanplay-core-runner: migrated {copied} legacy save file(s)");
    }
    let worker = StorageWorker::spawn(Arc::clone(store.storage()), 16);
    let save_dir = store.native_save_dir().unwrap_or_else(|| base.clone());

    let control = Arc::new(CoreControl::new());
    control.set_frame_limit(Some(options.frames));
    control.set_fast_forward(options.fast_forward);
    if let Some(slot) = &options.load_state {
        let data = match store.read_state(slot, registration.info.state_revision) {
            Ok(Some(state)) => state.encode().map_err(|error| error.to_string())?,
            Ok(None) => {
                let legacy = state_path(&base, &content, slot);
                match std::fs::read(&legacy) {
                    Ok(data) if data.starts_with(fanplay_core::state::STATE_MAGIC) => data,
                    Ok(_) => {
                        return Err(format!(
                            "{} is a legacy raw state; re-create it with this build so it can be validated",
                            legacy.display()
                        ))
                    }
                    Err(error) => {
                        return Err(format!("could not read {}: {error}", legacy.display()))
                    }
                }
            }
            Err(error) => return Err(error.to_string()),
        };
        control
            .send_request(CoreCommand::LoadState {
                slot: slot.clone(),
                data,
            })
            .map_err(|error| error.to_string())?;
    }

    let frames = Arc::new(Mutex::new(None));
    let session = SessionConfig::default();
    let context = fanplay_core::CoreContext::new(
        Arc::new(NativePlatform {
            root,
            save_dir,
            submitter: worker.submitter(),
            start: Instant::now(),
        }),
        Box::new(RecordingVideo(Arc::clone(&frames))),
        Box::new(SilentAudio::default()),
        Box::new(NativeEvents {
            submitter: worker.submitter(),
            label: content.label.clone(),
        }),
        Arc::clone(&control),
    )
    .with_session(session);
    eprintln!(
        "fanplay-core-runner: core={} content={}",
        registration.info.id,
        content.path.display()
    );
    let handle = spawn_core(registration, content.clone(), context)
        .map_err(|error| format!("could not start core thread: {error}"))?;
    if let Some(slot) = &options.save_state {
        // Give the core a moment to finish loading, then request a save at the
        // next frame boundary.
        control
            .send_request(CoreCommand::SaveState { slot: slot.clone() })
            .map_err(|error| error.to_string())?;
    }
    let status = handle
        .join()
        .map_err(|_| "core thread panicked".to_string())?
        .map_err(|error| error.to_string())?;
    worker.flush().map_err(|error| error.to_string())?;
    let failures = worker.status().failures();
    if failures > 0 {
        let detail = worker
            .status()
            .last_error()
            .unwrap_or_else(|| "unknown storage failure".into());
        return Err(format!("{failures} save write(s) failed: {detail}"));
    }
    worker.shutdown();
    eprintln!("fanplay-core-runner: core exited with code {}", status.code);
    if let Some(output) = options.output
        && let Some(frame) = frames.lock().unwrap().take()
    {
        write_ppm(&output, &frame)?;
        eprintln!("fanplay-core-runner: wrote {}", output.display());
    }
    Ok(())
}

struct Options {
    content: PathBuf,
    core: Option<String>,
    frames: u64,
    fast_forward: bool,
    save_state: Option<String>,
    load_state: Option<String>,
    save_dir: Option<PathBuf>,
    output: Option<PathBuf>,
}

impl Options {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let args: Vec<String> = args.into_iter().collect();
        let mut options = Self {
            content: PathBuf::new(),
            core: None,
            frames: 60,
            fast_forward: false,
            save_state: None,
            load_state: None,
            save_dir: None,
            output: None,
        };
        let mut index = 0;
        while index < args.len() {
            let argument = &args[index];
            let value = |index: usize| {
                args.get(index + 1)
                    .cloned()
                    .ok_or_else(|| format!("{argument} requires a value"))
            };
            match argument.as_str() {
                "--content" | "--rom" | "--game" => {
                    options.content = PathBuf::from(value(index)?);
                    index += 1;
                }
                "--core" => {
                    options.core = Some(value(index)?);
                    index += 1;
                }
                "--frames" => {
                    options.frames = value(index)?
                        .parse()
                        .map_err(|_| "--frames requires a positive integer".to_string())?;
                    if options.frames == 0 {
                        return Err("--frames must be positive".into());
                    }
                    index += 1;
                }
                "--fast-forward" => options.fast_forward = true,
                "--save-state" => {
                    options.save_state = Some(value(index)?);
                    index += 1;
                }
                "--load-state" => {
                    options.load_state = Some(value(index)?);
                    index += 1;
                }
                "--save-dir" => {
                    options.save_dir = Some(PathBuf::from(value(index)?));
                    index += 1;
                }
                "--output" => {
                    options.output = Some(PathBuf::from(value(index)?));
                    index += 1;
                }
                "--help" | "-h" => {
                    println!(
                        "fanplay-core-runner --content PATH [--core ID] [--frames N] \
                         [--fast-forward] [--save-state SLOT] [--load-state SLOT] \
                         [--save-dir DIR] [--output frame.ppm]\n\
                         Saves are stored under DIR/<core>/<content-hash>/."
                    );
                    std::process::exit(0);
                }
                other => return Err(format!("unknown argument: {other}")),
            }
            index += 1;
        }
        if options.content.as_os_str().is_empty() {
            return Err("--content PATH is required".into());
        }
        Ok(options)
    }
}

struct NativePlatform {
    root: PathBuf,
    save_dir: PathBuf,
    submitter: fanplay_frontend::StorageSubmitter,
    start: Instant,
}

impl Platform for NativePlatform {
    fn read_content(&self, path: &str) -> PlatformResult<Vec<u8>> {
        std::fs::read(self.root.join(path)).map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => PlatformError::not_found(path),
            std::io::ErrorKind::PermissionDenied => PlatformError::permission_denied(path),
            _ => PlatformError::io(format!("{path}: {error}")),
        })
    }
    fn read_system_file(&self, path: &str) -> PlatformResult<Option<Vec<u8>>> {
        match std::fs::read(self.root.join(path)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
    fn read_save(&self, name: &str) -> PlatformResult<Option<Vec<u8>>> {
        match std::fs::read(self.save_dir.join(name)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
    fn write_save(&self, name: &str, bytes: &[u8]) -> PlatformResult<()> {
        self.submitter
            .submit(SaveStore::save_key(name), bytes.to_vec())
            .map_err(|error| PlatformError::io(error.to_string()))
    }
    fn save_dir(&self) -> PathBuf {
        self.save_dir.clone()
    }
    fn now(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }
    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

struct Frame {
    width: u32,
    height: u32,
    stride: usize,
    format: PixelFormat,
    pixels: Vec<u8>,
    palette: Option<Vec<u8>>,
}

struct RecordingVideo(Arc<Mutex<Option<Frame>>>);

impl VideoSink for RecordingVideo {
    fn present(&mut self, frame: VideoFrame<'_>) {
        if frame.validate().is_err() {
            return;
        }
        let mut slot = self.0.lock().unwrap();
        let reuse = slot
            .as_ref()
            .is_some_and(|previous| previous.pixels.capacity() >= frame.pixels.len());
        let pixels = if reuse {
            let mut previous = slot.take().unwrap();
            previous.pixels.clear();
            previous.pixels.extend_from_slice(frame.pixels);
            previous.pixels
        } else {
            frame.pixels.to_vec()
        };
        *slot = Some(Frame {
            width: frame.width,
            height: frame.height,
            stride: frame.stride,
            format: frame.format,
            pixels,
            palette: frame.palette.map(<[u8]>::to_vec),
        });
    }
}

#[derive(Default)]
struct SilentAudio {
    samples: u64,
}

impl AudioSink for SilentAudio {
    fn play_stream(&mut self, _stream: AudioStream<'_>) {}
    fn stop(&mut self, _channel: AudioChannel) {}
    fn queue_pcm(&mut self, pcm: PcmBuffer<'_>) {
        self.samples += pcm.samples.len() as u64;
    }
}

struct NativeEvents {
    submitter: fanplay_frontend::StorageSubmitter,
    label: String,
}

impl EventSink for NativeEvents {
    fn emit(&mut self, event: CoreEvent) {
        match event {
            CoreEvent::Loaded(CoreLoadInfo {
                width,
                height,
                sample_rate,
                audio_channels,
            }) => {
                eprintln!("loaded: {width}x{height}, {sample_rate} Hz, {audio_channels} channels")
            }
            CoreEvent::Window(command) => eprintln!("window: {command:?}"),
            CoreEvent::Log(level, message) => log(level, &message),
            CoreEvent::SaveState { slot, data } => {
                let bytes = data.len();
                if let Err(error) = self.submitter.submit(SaveStore::state_key(&slot), data) {
                    eprintln!("could not queue save state {slot}: {error}");
                } else {
                    eprintln!("save state queued: {slot}");
                    if let Ok(saved_at) = SystemTime::now().duration_since(UNIX_EPOCH)
                        && let Err(error) = self.submitter.submit(
                            SaveStore::state_metadata_key(&slot),
                            SaveStore::state_metadata(saved_at.as_secs(), bytes),
                        )
                    {
                        eprintln!("could not queue save metadata {slot}: {error}");
                    }
                }
            }
            CoreEvent::SaveRam { data } => {
                let name = format!("{}.sav", safe_name(&self.label));
                if let Err(error) = self.submitter.submit(SaveStore::save_key(&name), data) {
                    eprintln!("could not queue battery save: {error}");
                }
            }
            CoreEvent::Screenshot { .. } => {}
            CoreEvent::CommandResult { id, outcome, .. } => {
                eprintln!("command {id}: {outcome:?}");
            }
            CoreEvent::Status(message) => eprintln!("status: {message}"),
            CoreEvent::Error(message) => eprintln!("error: {message}"),
            CoreEvent::Exited(status) => eprintln!("exited: {}", status.code),
        }
    }
}

fn log(level: LogLevel, message: &str) {
    match level {
        LogLevel::Error => eprintln!("error: {message}"),
        LogLevel::Warn => eprintln!("warning: {message}"),
        LogLevel::Info => eprintln!("{message}"),
        LogLevel::Debug => {}
    }
}

fn state_path(save_dir: &Path, content: &Content, slot: &str) -> PathBuf {
    save_dir.join(format!("{}.ss{}", safe_name(&content.label), slot))
}

fn safe_name(label: &str) -> String {
    let mut name: String = label
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect();
    if name.is_empty() {
        name.push_str("game");
    }
    name
}

fn write_ppm(path: &Path, frame: &Frame) -> Result<(), String> {
    let mut out = Vec::with_capacity(frame.width as usize * frame.height as usize * 3 + 32);
    out.extend_from_slice(format!("P6\n{} {}\n255\n", frame.width, frame.height).as_bytes());
    let bpp = frame.format.bytes_per_pixel();
    for row in 0..frame.height as usize {
        let start = row * frame.stride;
        let end = start + frame.width as usize * bpp;
        for pixel in frame.pixels[start..end].chunks_exact(bpp) {
            match frame.format {
                PixelFormat::Rgba8 => out.extend_from_slice(&pixel[..3]),
                PixelFormat::Bgra8 | PixelFormat::Bgrx8 => {
                    out.extend_from_slice(&[pixel[2], pixel[1], pixel[0]])
                }
                PixelFormat::Rgb24 => out.extend_from_slice(&pixel[..3]),
                PixelFormat::Rgb565 => {
                    let value = u16::from_le_bytes([pixel[0], pixel[1]]);
                    out.extend_from_slice(&[
                        ((value >> 11) as u8) << 3,
                        (((value >> 5) & 0x3f) as u8) << 2,
                        ((value & 0x1f) as u8) << 3,
                    ]);
                }
                PixelFormat::Indexed8 => {
                    let palette = frame.palette.as_deref().unwrap_or(&[]);
                    let offset = usize::from(pixel[0]) * 4;
                    if offset + 3 < palette.len() {
                        out.extend_from_slice(&palette[offset..offset + 3]);
                    } else {
                        out.extend_from_slice(&[pixel[0], pixel[0], pixel[0]]);
                    }
                }
            }
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(path, out).map_err(|error| error.to_string())
}
