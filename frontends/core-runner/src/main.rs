// SPDX-License-Identifier: GPL-2.0-or-later
//! Minimal headless frontend for the generic core contract.
//!
//! This is intentionally not the desktop UI. It exists so new cores can be
//! exercised end-to-end (content routing, dedicated core thread, frame/audio
//! sinks, save-state and battery-save events) before the full frontend grows
//! core selection.
//!
//! ```sh
//! cargo run -p rsxp-core-runner -- --content game.gba --frames 120 --output frame.ppm
//! cargo run -p rsxp-core-runner --features rgss -- \
//!   --content game/Game.ini --frames 60
//! ```

use rsxp_core::{
    AudioChannel, AudioSink, AudioStream, Content, CoreCommand, CoreControl, CoreEvent,
    CoreLoadInfo, EventSink, LogLevel, PcmBuffer, PixelFormat, Platform, VideoFrame, VideoSink,
};
use rsxp_frontend::spawn_core;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn main() {
    if let Err(error) = run() {
        eprintln!("rsxp-core-runner: {error}");
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
    let catalog = rsxp_cores::catalog();
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
    let save_dir = options.save_dir.clone().unwrap_or_else(|| {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
            .map(|base| base.join("rsxp/saves"))
            .unwrap_or_else(|| root.join(".rsxp-saves"))
    });

    let control = Arc::new(CoreControl::new());
    control.set_frame_limit(Some(options.frames));
    control.set_fast_forward(options.fast_forward);
    if let Some(slot) = &options.load_state {
        let path = state_path(&save_dir, &content, slot);
        let data = std::fs::read(&path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        control.send(CoreCommand::LoadState {
            slot: slot.clone(),
            data,
        });
    }

    let frames = Arc::new(Mutex::new(None));
    let context = rsxp_core::CoreContext::new(
        Arc::new(NativePlatform {
            root,
            save_dir: save_dir.clone(),
            start: Instant::now(),
        }),
        Box::new(RecordingVideo(Arc::clone(&frames))),
        Box::new(SilentAudio::default()),
        Box::new(NativeEvents {
            save_dir: save_dir.clone(),
            label: content.label.clone(),
        }),
        Arc::clone(&control),
    );
    eprintln!(
        "rsxp-core-runner: core={} content={}",
        registration.info.id,
        content.path.display()
    );
    let handle = spawn_core(registration, content.clone(), context)
        .map_err(|error| format!("could not start core thread: {error}"))?;
    if let Some(slot) = &options.save_state {
        // Give the core a moment to finish loading, then request a save at the
        // next frame boundary.
        control.send(CoreCommand::SaveState { slot: slot.clone() });
    }
    let status = handle
        .join()
        .map_err(|_| "core thread panicked".to_string())?
        .map_err(|error| error.to_string())?;
    eprintln!("rsxp-core-runner: core exited with code {}", status.code);
    if let Some(output) = options.output
        && let Some(frame) = frames.lock().unwrap().take()
    {
        write_ppm(&output, &frame)?;
        eprintln!("rsxp-core-runner: wrote {}", output.display());
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
                        "rsxp-core-runner --content PATH [--core ID] [--frames N] \
                         [--fast-forward] [--save-state SLOT] [--load-state SLOT] \
                         [--save-dir DIR] [--output frame.ppm]"
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
    start: Instant,
}

impl Platform for NativePlatform {
    fn read_file(&self, path: &str) -> Option<Vec<u8>> {
        std::fs::read(self.root.join(path)).ok()
    }
    fn write_file(&self, path: &str, bytes: &[u8]) -> Result<(), String> {
        let path = self.save_dir.join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        std::fs::write(path, bytes).map_err(|error| error.to_string())
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
}

struct RecordingVideo(Arc<Mutex<Option<Frame>>>);

impl VideoSink for RecordingVideo {
    fn present(&mut self, frame: VideoFrame<'_>) {
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
    save_dir: PathBuf,
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
                let path = self
                    .save_dir
                    .join(format!("{}.ss{}", safe_name(&self.label), slot));
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                match std::fs::write(&path, data) {
                    Ok(()) => eprintln!("save state: {}", path.display()),
                    Err(error) => eprintln!("could not write {}: {error}", path.display()),
                }
            }
            CoreEvent::SaveRam { data } => {
                let path = self
                    .save_dir
                    .join(format!("{}.sav", safe_name(&self.label)));
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Err(error) = std::fs::write(&path, data) {
                    eprintln!("could not write {}: {error}", path.display());
                }
            }
            CoreEvent::Screenshot { .. } => {}
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
                PixelFormat::Indexed8 => out.extend_from_slice(&[pixel[0], pixel[0], pixel[0]]),
            }
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(path, out).map_err(|error| error.to_string())
}
