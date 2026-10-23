// SPDX-License-Identifier: GPL-2.0-or-later
//! Desktop GBA runner.
//!
//! The dashboard already supervises a child process and exchanges video,
//! input and lifecycle controls over [`crate::session`]. This module gives that
//! child a GBA implementation of the same contract: `rsxp run --game foo.gba`
//! routes here instead of the RGSS runtime, streams CPU frames to the
//! dashboard, and runs the core on a dedicated thread.
//!
//! `--headless` skips the session transport and optionally captures the final
//! frame as PPM. Audio is played in this process through CPAL; when a session
//! is active the dashboard does not need to own an audio device.

use crate::session::Runner;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rsxp::graphics::Bitmap;
use rsxp::input::Input;
use rsxp_core::{
    AudioSink, AudioStream, Button, Content, CoreControl, CoreEvent, CoreLoadInfo, EventSink,
    PcmBuffer, PixelFormat, Platform, VideoFrame, VideoSink,
};
use std::collections::VecDeque;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub fn run(
    rom: PathBuf,
    headless: bool,
    frames: Option<u64>,
    output: Option<PathBuf>,
) -> Result<u8, Box<dyn Error>> {
    let rom = rom.canonicalize()?;
    let root = rom.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
    let title = rom
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("GBA game")
        .to_owned();
    let save_dir = save_directory(&title);

    let audio = if headless { None } else { PcmAudio::new() };
    if let Some(audio) = &audio {
        // The core reads this at load time and configures its resampler to the
        // actual device rate.
        // SAFETY: no core/audio threads exist yet.
        unsafe { std::env::set_var("RSXP_GBA_AUDIO_RATE", audio.sample_rate.to_string()) };
    }

    let control = Arc::new(CoreControl::new());
    control.set_frame_limit(frames);
    let captured = Arc::new(Mutex::new(None));
    let runner = if headless { None } else { Runner::connect()? };
    let shared_runner = runner.map(|runner| Arc::new(Mutex::new(runner)));
    let video = match &shared_runner {
        Some(runner) => Video::Session(Arc::clone(runner)),
        None => Video::Capture(Arc::clone(&captured)),
    };
    let audio_sink: Box<dyn AudioSink> = match audio {
        Some(audio) => Box::new(audio),
        None => Box::new(SilentAudio),
    };
    let context = rsxp_core::CoreContext::new(
        Arc::new(FsPlatform {
            root,
            save_dir: save_dir.clone(),
            start: Instant::now(),
        }),
        Box::new(video),
        audio_sink,
        Box::new(LogEvents {
            save_dir,
            title: title.clone(),
        }),
        Arc::clone(&control),
    );
    let registration = rsxp_core_gba::registration();
    let handle = rsxp_frontend::spawn_core(registration, Content::file(&rom), context)
        .map_err(|error| format!("could not start GBA core: {error}"))?;

    if let Some(runner) = &shared_runner {
        let mut input = Input::default();
        while !handle.is_finished() {
            input.begin_frame();
            let (alive, paused) = {
                let mut runner = runner.lock().unwrap();
                (runner.poll(&mut input), runner.paused)
            };
            if !alive {
                control.stop();
                break;
            }
            control.set_paused(paused);
            control.set_fast_forward(rsxp::runtime_settings::fast_forward());
            update_buttons(&control, &input);
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    let result = handle
        .join()
        .map_err(|_| "GBA core thread panicked".to_string())?
        .map_err(|error| error.to_string())?;
    if let Some(path) = output
        && let Some(bitmap) = captured.lock().unwrap().take()
    {
        let mut writer = std::io::BufWriter::new(std::fs::File::create(&path)?);
        bitmap.write_ppm(&mut writer)?;
        use std::io::Write as _;
        writer.flush()?;
    }
    Ok(u8::try_from(result.code).unwrap_or(1))
}

fn update_buttons(control: &CoreControl, input: &Input) {
    let pressed = |key: i32| input.pressed(key);
    let set = |button, down| control.input().set_button(button, down);
    set(Button::Up, pressed(82) || pressed(604));
    set(Button::Down, pressed(81) || pressed(601));
    set(Button::Left, pressed(80) || pressed(602));
    set(Button::Right, pressed(79) || pressed(603));
    set(Button::South, pressed(27) || pressed(605) || pressed(607));
    set(Button::East, pressed(29) || pressed(606));
    set(Button::Start, pressed(40));
    set(Button::Select, pressed(42));
    set(Button::LeftShoulder, pressed(4) || pressed(611));
    set(Button::RightShoulder, pressed(22) || pressed(612));
}

struct FsPlatform {
    root: PathBuf,
    save_dir: PathBuf,
    start: Instant,
}

impl Platform for FsPlatform {
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

enum Video {
    Session(Arc<Mutex<Runner>>),
    Capture(Arc<Mutex<Option<Bitmap>>>),
}

impl VideoSink for Video {
    fn present(&mut self, frame: VideoFrame<'_>) {
        let Some(bitmap) = bitmap_from_frame(frame) else {
            return;
        };
        match self {
            Self::Session(runner) => {
                if let Ok(mut runner) = runner.lock() {
                    let _ = runner.frame(&bitmap);
                }
            }
            Self::Capture(captured) => *captured.lock().unwrap() = Some(bitmap),
        }
    }
}

fn bitmap_from_frame(frame: VideoFrame<'_>) -> Option<Bitmap> {
    if frame.width == 0 || frame.height == 0 {
        return None;
    }
    let bpp = frame.format.bytes_per_pixel();
    let width = frame.width as usize;
    let height = frame.height as usize;
    if frame.stride < width * bpp || frame.pixels.len() < frame.stride * height {
        return None;
    }
    let mut pixels = Vec::with_capacity(width * height);
    for row in 0..height {
        let start = row * frame.stride;
        let row = &frame.pixels[start..start + width * bpp];
        match frame.format {
            PixelFormat::Bgrx8 | PixelFormat::Bgra8 => {
                for pixel in row.chunks_exact(4) {
                    let alpha = if frame.format == PixelFormat::Bgrx8 {
                        255
                    } else {
                        pixel[3]
                    };
                    pixels.push(u32::from_be_bytes([alpha, pixel[2], pixel[1], pixel[0]]));
                }
            }
            PixelFormat::Rgba8 => {
                for pixel in row.chunks_exact(4) {
                    pixels.push(u32::from_be_bytes([pixel[3], pixel[0], pixel[1], pixel[2]]));
                }
            }
            PixelFormat::Rgb24 => {
                for pixel in row.chunks_exact(3) {
                    pixels.push(u32::from_be_bytes([255, pixel[0], pixel[1], pixel[2]]));
                }
            }
            PixelFormat::Rgb565 => {
                for pixel in row.chunks_exact(2) {
                    let value = u16::from_le_bytes([pixel[0], pixel[1]]);
                    pixels.push(u32::from_be_bytes([
                        255,
                        (((value >> 11) & 0x1f) as u8) << 3,
                        (((value >> 5) & 0x3f) as u8) << 2,
                        ((value & 0x1f) as u8) << 3,
                    ]));
                }
            }
            PixelFormat::Indexed8 => {
                for &value in row {
                    pixels.push(u32::from_be_bytes([255, value, value, value]));
                }
            }
        }
    }
    Bitmap::from_argb_owned(frame.width, frame.height, pixels).ok()
}

struct SilentAudio;

impl AudioSink for SilentAudio {}

struct PcmAudio {
    buffer: Arc<Mutex<VecDeque<i16>>>,
    stream: cpal::Stream,
    sample_rate: u32,
    paused: bool,
}

impl PcmAudio {
    fn new() -> Option<Self> {
        let host = cpal::default_host();
        let device = host.default_output_device()?;
        let supported = device.default_output_config().ok()?;
        if supported.sample_format() != cpal::SampleFormat::F32 {
            eprintln!("GBA audio disabled: output device is not f32");
            return None;
        }
        let config: cpal::StreamConfig = supported.config();
        let sample_rate = config.sample_rate;
        let channels = config.channels as usize;
        let buffer = Arc::new(Mutex::new(VecDeque::<i16>::new()));
        let audio_buffer = Arc::clone(&buffer);
        let stream = device
            .build_output_stream(
                config,
                move |output: &mut [f32], _| {
                    let mut queue = audio_buffer.lock().unwrap();
                    for frame in output.chunks_mut(channels) {
                        let left = queue.pop_front().unwrap_or(0) as f32 / 32768.0;
                        let right = queue.pop_front().unwrap_or(0) as f32 / 32768.0;
                        if channels == 1 {
                            frame[0] = (left + right) * 0.5;
                        } else {
                            frame[0] = left;
                            frame[1] = right;
                            for sample in &mut frame[2..] {
                                *sample = 0.0;
                            }
                        }
                    }
                },
                |error| eprintln!("GBA audio stream error: {error}"),
                None,
            )
            .ok()?;
        stream.play().ok()?;
        Some(Self {
            buffer,
            stream,
            sample_rate,
            paused: false,
        })
    }
}

impl AudioSink for PcmAudio {
    fn play_stream(&mut self, _stream: AudioStream<'_>) {}
    fn queue_pcm(&mut self, pcm: PcmBuffer<'_>) {
        let mut queue = self.buffer.lock().unwrap();
        const MAX_SECONDS: usize = 2;
        let max = self.sample_rate as usize * 2 * MAX_SECONDS;
        if queue.len() + pcm.samples.len() > max {
            let excess = (queue.len() + pcm.samples.len()).saturating_sub(max);
            let drop = excess.min(queue.len());
            queue.drain(..drop);
        }
        queue.extend(pcm.samples.iter().copied());
    }
    fn pause(&mut self, paused: bool) {
        if self.paused == paused {
            return;
        }
        self.paused = paused;
        if paused {
            let _ = self.stream.pause();
        } else {
            let _ = self.stream.play();
            self.buffer.lock().unwrap().clear();
        }
    }
}

struct LogEvents {
    save_dir: PathBuf,
    title: String,
}

impl EventSink for LogEvents {
    fn emit(&mut self, event: CoreEvent) {
        match event {
            CoreEvent::Loaded(CoreLoadInfo {
                width,
                height,
                sample_rate,
                audio_channels,
            }) => eprintln!("GBA: {width}x{height}, {sample_rate} Hz, {audio_channels} channels"),
            CoreEvent::Window(command) => eprintln!("GBA window: {command:?}"),
            CoreEvent::Log(_, message) => eprintln!("{message}"),
            CoreEvent::SaveState { slot, data } => {
                let path = self
                    .save_dir
                    .join(format!("{}.ss{}", safe_name(&self.title), slot));
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Err(error) = std::fs::write(&path, data) {
                    eprintln!("GBA: could not write {}: {error}", path.display());
                }
            }
            CoreEvent::SaveRam { .. } => {}
            CoreEvent::Screenshot { .. } => {}
            CoreEvent::Status(message) => eprintln!("GBA: {message}"),
            CoreEvent::Error(message) => eprintln!("GBA: {message}"),
            CoreEvent::Exited(status) => eprintln!("GBA exited: {}", status.code),
        }
    }
}

fn save_directory(title: &str) -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from(".local/share"));
    let path = base.join("rsxp").join(safe_name(title));
    let _ = std::fs::create_dir_all(&path);
    path
}

fn safe_name(label: &str) -> String {
    let mut name: String = label
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || matches!(character, '_' | '.' | ' ' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect();
    if name.is_empty() {
        name.push_str("gba");
    }
    name
}
