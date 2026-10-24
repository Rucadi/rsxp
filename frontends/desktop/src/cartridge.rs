// SPDX-License-Identifier: GPL-2.0-or-later
//! Desktop host for cartridge cores.
//!
//! Content is routed by [`rsxp_frontend::CoreCatalog`], so this module has no
//! knowledge of GBA or DS. It supplies the generic media bridge:
//!
//! - a reusable ARGB frame buffer converted once per frame and sent to the
//!   dashboard transport (or retained for a headless PPM capture),
//! - a bounded SPSC PCM ring shared with CPAL, with no mutex or allocation in
//!   the producer callback after warm-up,
//! - a bounded save worker keyed by core id and content identity,
//! - a dedicated session thread that owns the transport while the core thread
//!   owns emulation.
//!
//! Core-specific behavior (screen layout, touch mapping, buttons) comes from
//! [`CoreInfo`](rsxp_core::CoreInfo), not from an extension branch here.

use crate::session::{Runner, TransportInput};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rsxp_core::{
    AudioConfig, AudioSink, Button, Content, CoreControl, CoreEvent, CoreLoadInfo,
    CoreRegistration, EventSink, FrameError, PcmBuffer, PcmRing, PcmRingConsumer, PcmRingProducer,
    PcmStatus, Platform, PlatformError, PlatformResult, SessionConfig, VideoFrame, VideoSink,
};
use rsxp_frontend::{video::convert_argb, ContentId, SaveStore, StorageSubmitter, StorageWorker};
use std::collections::VecDeque;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const AUDIO_SECONDS: u64 = 2;
const FRAME_QUEUE: usize = 2;
const SAVE_QUEUE: usize = 32;

/// Runs one cartridge session.
pub fn run(
    content: Content,
    registration: CoreRegistration,
    headless: bool,
    frames: Option<u64>,
    output: Option<PathBuf>,
) -> Result<u8, Box<dyn Error>> {
    let content = Content {
        path: content
            .path
            .canonicalize()
            .map_err(|error| format!("{}: {error}", content.path.display()))?,
        ..content
    };
    let root = content
        .path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let title = content.label.clone();
    let content_id = ContentId::of(&content)?;
    let base = data_base().join("rsxp/saves");
    let store = SaveStore::new(&base, registration.info.id, content_id)?;
    if let Ok(copied) = store.migrate_from(&legacy_data_dir(&title))
        && copied > 0
    {
        eprintln!("Cartridge: migrated {copied} legacy save file(s)");
    }
    let worker = StorageWorker::spawn(Arc::clone(store.storage()), SAVE_QUEUE);

    let audio = if headless { None } else { PcmAudio::new() };
    let audio_config = audio
        .as_ref()
        .and_then(|audio| audio.output_config())
        .unwrap_or_else(|| registration.info.audio.preferred);
    let session = if headless {
        SessionConfig::default()
    } else {
        SessionConfig::default().with_audio(audio_config)
    };

    let control = Arc::new(CoreControl::new());
    control.set_frame_limit(frames);

    let runner = if headless { None } else { Runner::connect()? };
    if !headless && runner.is_none() && frames.is_none() && output.is_none() {
        return Err(
            "cartridge display requires a dashboard session; use --headless or --frames".into(),
        );
    }

    let mut session_thread = None;
    let capture = Arc::new(Mutex::new(Frame::default()));
    let video: Box<dyn VideoSink> = if headless {
        Box::new(CaptureVideo::new(Arc::clone(&capture)))
    } else if let Some(runner) = runner {
        let (frames_out, frames_in) = std::sync::mpsc::sync_channel(FRAME_QUEUE);
        let (recycle_out, recycle_in) = std::sync::mpsc::sync_channel(FRAME_QUEUE + 1);
        let host_input = HostInput::new(Arc::clone(&control), registration.info.input.buttons);
        let thread_control = Arc::clone(&control);
        let join = std::thread::Builder::new()
            .name(format!("rsxp-{}-session", registration.info.id))
            .spawn(move || {
                session_loop(runner, frames_in, recycle_out, host_input, thread_control)
            })?;
        session_thread = Some(join);
        Box::new(TransportVideo::new(frames_out, recycle_in))
    } else {
        Box::new(CaptureVideo::new(Arc::clone(&capture)))
    };

    let audio_sink: Box<dyn AudioSink> = match audio {
        Some(audio) => Box::new(audio),
        None => Box::new(SilentAudio),
    };
    let save_dir = store.native_save_dir().unwrap_or_else(|| base.clone());
    let context = rsxp_core::CoreContext::new(
        Arc::new(CartridgePlatform {
            root,
            save_dir,
            submitter: worker.submitter(),
            start: Instant::now(),
        }),
        video,
        audio_sink,
        Box::new(CartridgeEvents {
            submitter: worker.submitter(),
            label: title.clone(),
        }),
        Arc::clone(&control),
    )
    .with_session(session);

    let handle = rsxp_frontend::spawn_core(registration, content, context)
        .map_err(|error| format!("could not start cartridge core: {error}"))?;
    let result = handle
        .join()
        .map_err(|_| "cartridge core thread panicked".to_string())?
        .map_err(|error| error.to_string());
    if let Some(join) = session_thread {
        let _ = join.join();
    }
    worker.flush()?;
    let failures = worker.status().failures();
    if failures > 0 {
        let detail = worker
            .status()
            .last_error()
            .unwrap_or_else(|| "unknown storage failure".into());
        return Err(format!("{failures} save write(s) failed: {detail}").into());
    }
    worker.shutdown();
    if let Some(path) = output {
        // Headless capture is the only caller that needs a final bitmap.
        // Session runs stream every frame, so a PPM request without a capture
        // sink is simply ignored.
        if let Ok(frame) = capture.lock()
            && frame.width > 0
            && frame.height > 0
        {
            write_ppm(&path, &frame)?;
        }
    }
    result
        .map(|status| u8::try_from(status.code).unwrap_or(1))
        .map_err(Into::into)
}

#[derive(Clone, Default)]
struct Frame {
    width: u32,
    height: u32,
    pixels: Vec<u32>,
}

impl Frame {
    fn prepare(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        self.pixels
            .resize((width as usize).saturating_mul(height as usize), 0);
    }
}

/// Converts a borrowed contract frame into packed, native-endian ARGB.
fn convert(frame: &VideoFrame<'_>, out: &mut Frame) -> Result<(), FrameError> {
    out.prepare(frame.width, frame.height);
    convert_argb(frame, &mut out.pixels)
}

struct TransportVideo {
    out: SyncSender<Frame>,
    recycle: Receiver<Frame>,
    free: VecDeque<Frame>,
}

impl TransportVideo {
    fn new(out: SyncSender<Frame>, recycle: Receiver<Frame>) -> Self {
        Self {
            out,
            recycle,
            free: VecDeque::with_capacity(FRAME_QUEUE + 1),
        }
    }
}

impl VideoSink for TransportVideo {
    fn present(&mut self, frame: VideoFrame<'_>) {
        while let Ok(recycled) = self.recycle.try_recv() {
            self.free.push_back(recycled);
        }
        let mut converted = self.free.pop_front().unwrap_or_default();
        if convert(&frame, &mut converted).is_err() {
            self.free.push_front(converted);
            return;
        }
        match self.out.try_send(converted) {
            Ok(()) => {}
            Err(TrySendError::Full(converted)) | Err(TrySendError::Disconnected(converted)) => {
                // Bounded backpressure: drop this frame rather than stall the
                // core thread. The next frame reuses the buffer.
                self.free.push_front(converted);
            }
        }
    }
}

struct CaptureVideo {
    shared: Arc<Mutex<Frame>>,
    frame: Frame,
}

impl CaptureVideo {
    fn new(shared: Arc<Mutex<Frame>>) -> Self {
        Self {
            shared,
            frame: Frame::default(),
        }
    }
}

impl VideoSink for CaptureVideo {
    fn present(&mut self, frame: VideoFrame<'_>) {
        if convert(&frame, &mut self.frame).is_ok()
            && let Ok(mut capture) = self.shared.lock()
        {
            std::mem::swap(&mut *capture, &mut self.frame);
        }
    }
}

fn session_loop(
    mut runner: Runner,
    frames: Receiver<Frame>,
    recycle: SyncSender<Frame>,
    mut input: HostInput,
    control: Arc<CoreControl>,
) {
    loop {
        match frames.recv_timeout(Duration::from_millis(2)) {
            Ok(frame) => {
                if runner
                    .frame_argb(frame.width, frame.height, &frame.pixels)
                    .is_err()
                {
                    control.stop();
                    break;
                }
                let _ = recycle.try_send(frame);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if !runner.poll(&mut input) {
            control.stop();
            break;
        }
        control.set_paused(runner.paused);
        input.publish();
    }
}

struct HostInput {
    control: Arc<CoreControl>,
    supported: Vec<Button>,
    keys: Vec<bool>,
    pointer: (i32, i32, bool),
}

impl HostInput {
    fn new(control: Arc<CoreControl>, supported: &'static [Button]) -> Self {
        Self {
            control,
            supported: supported.to_vec(),
            keys: vec![false; 520],
            pointer: (0, 0, false),
        }
    }

    fn pressed(&self, code: i32) -> bool {
        usize::try_from(code + 8)
            .ok()
            .and_then(|index| self.keys.get(index))
            .copied()
            .unwrap_or(false)
    }

    fn set(&self, button: Button, pressed: bool) {
        if self.supported.contains(&button) {
            self.control.input().set_button(button, pressed);
        }
    }

    fn publish(&self) {
        self.set(Button::Up, self.pressed(82) || self.pressed(604));
        self.set(Button::Down, self.pressed(81) || self.pressed(601));
        self.set(Button::Left, self.pressed(80) || self.pressed(602));
        self.set(Button::Right, self.pressed(79) || self.pressed(603));
        self.set(Button::South, self.pressed(27) || self.pressed(605));
        self.set(Button::East, self.pressed(29) || self.pressed(606));
        self.set(Button::North, self.pressed(22) || self.pressed(608));
        self.set(Button::West, self.pressed(4) || self.pressed(607));
        self.set(Button::Extra0, self.pressed(1));
        self.set(Button::Start, self.pressed(40));
        self.set(Button::Select, self.pressed(42));
        self.set(Button::LeftShoulder, self.pressed(20) || self.pressed(611));
        self.set(Button::RightShoulder, self.pressed(26) || self.pressed(612));
        self.control
            .input()
            .set_pointer(self.pointer.0, self.pointer.1, self.pointer.2);
    }
}

impl TransportInput for HostInput {
    fn key_down(&mut self, code: i32) {
        if let Ok(index) = usize::try_from(code + 8)
            && let Some(slot) = self.keys.get_mut(index)
        {
            *slot = true;
        }
    }
    fn key_up(&mut self, code: i32) {
        if let Ok(index) = usize::try_from(code + 8)
            && let Some(slot) = self.keys.get_mut(index)
        {
            *slot = false;
        }
    }
    fn set_pointer(&mut self, x: i32, y: i32, inside: bool) {
        self.pointer = (x, y, inside);
    }
    fn add_scroll(&mut self, _delta: i32) {}
    fn push_text(&mut self, _text: &str) {}
    fn fast_forward(&mut self, enabled: bool) {
        self.control.set_fast_forward(enabled);
    }
}

struct CartridgePlatform {
    root: PathBuf,
    save_dir: PathBuf,
    submitter: StorageSubmitter,
    start: Instant,
}

impl Platform for CartridgePlatform {
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

struct SilentAudio;
impl AudioSink for SilentAudio {}

struct PcmAudio {
    producer: PcmRingProducer,
    consumer: Arc<PcmRingConsumer>,
    stream: cpal::Stream,
    config: AudioConfig,
    paused: bool,
    channel_mismatch: bool,
}

impl PcmAudio {
    fn new() -> Option<Self> {
        let host = cpal::default_host();
        let device = host.default_output_device()?;
        let supported = device.default_output_config().ok()?;
        if supported.sample_format() != cpal::SampleFormat::F32 {
            eprintln!("Cartridge audio disabled: output device is not f32");
            return None;
        }
        let stream_config: cpal::StreamConfig = supported.config();
        let config = AudioConfig::new(stream_config.sample_rate, stream_config.channels);
        let samples =
            (u64::from(config.sample_rate) * u64::from(config.channels) * AUDIO_SECONDS) as usize;
        let (producer, consumer) = PcmRing::new(samples.max(2048));
        let consumer = Arc::new(consumer);
        let callback_consumer = Arc::clone(&consumer);
        let device_channels = config.channels as usize;
        let stream = device
            .build_output_stream(
                stream_config,
                move |output: &mut [f32], _| {
                    for frame in output.chunks_mut(device_channels) {
                        let mut stereo = [0i16; 2];
                        callback_consumer.pop_fill(&mut stereo);
                        let left = f32::from(stereo[0]) / 32768.0;
                        let right = f32::from(stereo[1]) / 32768.0;
                        if device_channels == 1 {
                            frame[0] = (left + right) * 0.5;
                        } else {
                            frame[0] = left;
                            if device_channels > 1 {
                                frame[1] = right;
                            }
                            for sample in &mut frame[2..] {
                                *sample = 0.0;
                            }
                        }
                    }
                },
                |error| eprintln!("Cartridge audio stream error: {error}"),
                None,
            )
            .ok()?;
        stream.play().ok()?;
        Some(Self {
            producer,
            consumer,
            stream,
            config,
            paused: false,
            channel_mismatch: false,
        })
    }
}

impl AudioSink for PcmAudio {
    fn queue_pcm(&mut self, pcm: PcmBuffer<'_>) {
        if pcm.sample_rate != self.config.sample_rate || pcm.channels != 2 {
            if !self.channel_mismatch {
                self.channel_mismatch = true;
                eprintln!(
                    "Cartridge audio format mismatch: core {} Hz/{} ch, device {} Hz/{} ch",
                    pcm.sample_rate, pcm.channels, self.config.sample_rate, self.config.channels
                );
            }
            return;
        }
        self.producer.push(pcm.samples);
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
            self.consumer.reset();
        }
    }

    fn output_config(&self) -> Option<AudioConfig> {
        Some(self.config)
    }

    fn reset_pcm(&mut self) {
        self.consumer.reset();
    }

    fn pcm_status(&self) -> PcmStatus {
        self.consumer.status(self.config.channels)
    }
}

struct CartridgeEvents {
    submitter: StorageSubmitter,
    label: String,
}

impl EventSink for CartridgeEvents {
    fn emit(&mut self, event: CoreEvent) {
        match event {
            CoreEvent::Loaded(CoreLoadInfo {
                width,
                height,
                sample_rate,
                audio_channels,
            }) => eprintln!(
                "Cartridge: {width}x{height}, {sample_rate} Hz, {audio_channels} channels"
            ),
            CoreEvent::Window(command) => eprintln!("Cartridge window: {command:?}"),
            CoreEvent::Log(_, message) => eprintln!("{message}"),
            CoreEvent::SaveState { slot, data } => {
                if let Err(error) = self.submitter.submit(SaveStore::state_key(&slot), data) {
                    eprintln!("Cartridge: could not queue save state {slot}: {error}");
                }
            }
            CoreEvent::SaveRam { data } => {
                let name = format!("{}.sav", safe_name(&self.label));
                if let Err(error) = self.submitter.submit(SaveStore::save_key(&name), data) {
                    eprintln!("Cartridge: could not queue battery save: {error}");
                }
            }
            CoreEvent::Screenshot { .. } => {}
            CoreEvent::CommandResult { id, outcome, .. } => {
                if !outcome.succeeded() {
                    eprintln!("Cartridge: command {id}: {outcome:?}");
                }
            }
            CoreEvent::Status(message) => eprintln!("Cartridge: {message}"),
            CoreEvent::Error(message) => eprintln!("Cartridge: {message}"),
            CoreEvent::Exited(status) => eprintln!("Cartridge exited: {}", status.code),
        }
    }
}

fn data_base() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from(".local/share"))
}

fn legacy_data_dir(title: &str) -> PathBuf {
    data_base().join("rsxp").join(safe_name(title))
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
        name.push_str("game");
    }
    name
}

fn write_ppm(path: &Path, frame: &Frame) -> Result<(), Box<dyn Error>> {
    use std::io::Write as _;
    let mut out = Vec::with_capacity(frame.width as usize * frame.height as usize * 3 + 32);
    out.extend_from_slice(format!("P6\n{} {}\n255\n", frame.width, frame.height).as_bytes());
    for pixel in &frame.pixels {
        out.extend_from_slice(&[(pixel >> 16) as u8, (pixel >> 8) as u8, *pixel as u8]);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut writer = std::io::BufWriter::new(std::fs::File::create(path)?);
    writer.write_all(&out)?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsxp_core::PixelFormat;

    fn frame(format: PixelFormat, pixels: &[u8]) -> VideoFrame<'_> {
        VideoFrame::new(2, 1, pixels.len(), format, pixels)
    }

    #[test]
    fn converts_common_formats_to_packed_argb() {
        let mut out = Frame::default();
        convert(
            &frame(PixelFormat::Bgra8, &[0, 0, 255, 255, 0, 255, 0, 255]),
            &mut out,
        )
        .unwrap();
        assert_eq!(out.pixels, [0xffff_0000, 0xff00_ff00]);
        convert(
            &frame(PixelFormat::Rgba8, &[255, 0, 0, 255, 0, 255, 0, 255]),
            &mut out,
        )
        .unwrap();
        assert_eq!(out.pixels, [0xffff_0000, 0xff00_ff00]);
        let palette = [255, 0, 0, 255, 0, 255, 0, 255];
        let indexed = VideoFrame::indexed8(2, 1, 2, &[0, 1], &palette);
        convert(&indexed, &mut out).unwrap();
        assert_eq!(out.pixels, [0xffff_0000, 0xff00_ff00]);
    }

    #[test]
    fn capture_does_not_retain_an_invalid_frame() {
        let shared = Arc::new(Mutex::new(Frame::default()));
        let mut capture = CaptureVideo::new(Arc::clone(&shared));
        capture.present(VideoFrame {
            width: 2,
            height: 1,
            stride: 1,
            format: PixelFormat::Rgb24,
            pixels: &[1, 2, 3],
            palette: None,
        });
        assert_eq!(shared.lock().unwrap().width, 0);
        capture.present(frame(PixelFormat::Bgrx8, &[0, 0, 255, 0, 0, 255, 0, 0]));
        assert_eq!(shared.lock().unwrap().width, 2);
    }

    #[test]
    fn host_input_maps_keys_to_supported_buttons() {
        let control = Arc::new(CoreControl::new());
        let mut input = HostInput::new(
            Arc::clone(&control),
            &[Button::South, Button::Start, Button::Extra0],
        );
        input.key_down(27);
        input.key_down(1);
        input.set_pointer(10, 20, true);
        input.publish();
        assert!(control.input().buttons().contains(Button::South));
        assert!(control.input().buttons().contains(Button::Extra0));
        assert!(!control.input().buttons().contains(Button::East));
        let snapshot = control.input().snapshot();
        assert_eq!((snapshot.pointer_x, snapshot.pointer_y), (10, 20));
        input.key_up(27);
        input.publish();
        assert!(!control.input().buttons().contains(Button::South));
    }
}
