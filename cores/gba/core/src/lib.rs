// SPDX-License-Identifier: GPL-2.0-or-later
//! Game Boy Advance core adapter.
//!
//! Backend: [`rustboyadvance_core::GameBoyAdvance`] from
//! [RustBoyAdvance-NG](https://github.com/michelhe/rustboyadvance-ng). The
//! upstream crate is MIT licensed and exposes the emulator as a library; no
//! SDL/frontend dependencies are linked.
//!
//! The backend needs a 16 KiB GBA BIOS. This adapter embeds the open-source
//! Normmatt/VBA-M replacement BIOS from ReGBA, so no external file is required.
//! `FANPLAY_GBA_BIOS` or `gba_bios.bin`/`bios.bin` beside the ROM or in the save
//! directory can override it with the user's own dump.
//!
//! The steady-state loop performs no allocation: the backend renders into its
//! own `u32` framebuffer, which is submitted zero-copy as `Bgrx8` on
//! little-endian hosts, and audio is drained from a preallocated ring buffer.

use fanplay_core::{
    content_hash, AudioInfo, Button, CommandOutcome, CommandRequest, Content, ContentKind,
    ContentProbe, Core, CoreCapabilities, CoreCommand, CoreContext, CoreError, CoreEvent, CoreInfo,
    CoreLoadInfo, CoreRegistration, CoreRegistry, CoreResult, ExitStatus, InputInfo, PcmBuffer,
    PixelFormat, PlatformError, PresentationSupport, ScreenInfo, ScreenRotation, StateEnvelope,
    VideoFrame,
};
use rustboyadvance_core::keypad::KEYINPUT_ALL_RELEASED;
use rustboyadvance_core::prelude::{
    GameBoyAdvance, GamepakBuilder, SimpleAudioInterface, DISPLAY_HEIGHT, DISPLAY_WIDTH,
};
use rustboyadvance_utils::audio::SampleConsumer;
use rustboyadvance_utils::Consumer;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub const CORE_ID: &str = "gba";
const DEFAULT_SAMPLE_RATE: u32 = 48_000;
const BIOS_BYTES: usize = 0x4000;
/// Open-source GBA BIOS replacement (Normmatt/VBA-M, GPL-2.0-or-later).
const EMBEDDED_BIOS: &[u8; BIOS_BYTES] = include_bytes!("../assets/gba_bios.bin");
const FRAME_RATE: f64 = 59.7275;
const AUDIO_SCRATCH: usize = 16_384;
const STATE_REVISION: u32 = 1;
const SCREENS: &[ScreenInfo] = &[ScreenInfo {
    id: "main",
    width: DISPLAY_WIDTH as u32,
    height: DISPLAY_HEIGHT as u32,
    x: 0,
    y: 0,
    rotation: ScreenRotation::None,
    touch: false,
}];
const BUTTONS: &[Button] = &[
    Button::Up,
    Button::Down,
    Button::Left,
    Button::Right,
    Button::South,
    Button::East,
    Button::Start,
    Button::Select,
    Button::LeftShoulder,
    Button::RightShoulder,
];

pub const INFO: CoreInfo = CoreInfo {
    id: CORE_ID,
    name: "Game Boy Advance",
    version: env!("CARGO_PKG_VERSION"),
    state_revision: STATE_REVISION,
    extensions: &["gba"],
    capabilities: CoreCapabilities::SAVE_STATES
        .union(CoreCapabilities::SAVE_RAM)
        .union(CoreCapabilities::SCREENSHOTS)
        .union(CoreCapabilities::RESET),
    presentation: PresentationSupport::Cpu,
    pixel_formats: &[PixelFormat::Bgrx8, PixelFormat::Rgba8],
    audio: AudioInfo::new(DEFAULT_SAMPLE_RATE, 2, true),
    input: InputInfo {
        buttons: BUTTONS,
        pointer: false,
        pointer_button: false,
    },
    screens: SCREENS,
    options: &[],
};

pub fn register(registry: &mut CoreRegistry) {
    registry.register(registration());
}

pub fn registration() -> CoreRegistration {
    CoreRegistration {
        info: INFO,
        create,
        detect,
    }
}

pub fn create() -> Box<dyn Core> {
    Box::new(GbaAdapter::default())
}

/// Claims `.gba` content. The backend validates the ROM header after loading.
pub fn detect(probe: &ContentProbe<'_>) -> u32 {
    if probe.is_dir || !probe.has_extension("gba") {
        return 0;
    }
    80
}

#[derive(Default)]
struct GbaAdapter {
    context: Option<CoreContext>,
    control: Option<Arc<fanplay_core::CoreControl>>,
    core: Option<GameBoyAdvance>,
    audio_consumer: Option<SampleConsumer>,
    audio_scratch: Vec<i16>,
    bios: Vec<u8>,
    rom: Vec<u8>,
    save_path: Option<PathBuf>,
    sample_rate: u32,
    frames: u64,
    /// Used only when the host is big-endian; the common path is zero-copy.
    #[cfg(target_endian = "big")]
    video: Vec<u8>,
}

impl GbaAdapter {
    fn read_rom(content: &Content, context: &CoreContext) -> CoreResult<Vec<u8>> {
        if content.kind == ContentKind::Directory {
            return Err(CoreError::content(
                "GBA content must be a single .gba ROM file",
            ));
        }
        match std::fs::read(&content.path) {
            Ok(rom) => Ok(rom),
            Err(native_error) => {
                let path = content.path.to_string_lossy();
                let by_name = content.path.file_name().and_then(|name| name.to_str());
                context
                    .platform
                    .read_content(&path)
                    .or_else(|_| match by_name {
                        Some(name) => context.platform.read_content(name),
                        None => Err(PlatformError::not_found(path.as_ref())),
                    })
                    .map_err(|error| {
                        CoreError::content(format!(
                            "could not read {}: {native_error} ({error})",
                            content.path.display()
                        ))
                    })
            }
        }
    }

    fn read_bios(content: &Content, context: &CoreContext) -> CoreResult<Vec<u8>> {
        if let Some(path) = std::env::var_os("FANPLAY_GBA_BIOS").map(PathBuf::from) {
            return read_bios_file(&path).ok_or_else(|| {
                CoreError::content(format!(
                    "{}: BIOS must be at least {BIOS_BYTES} bytes",
                    path.display()
                ))
            });
        }
        let root = content.path.parent().unwrap_or_else(|| Path::new("."));
        for name in ["gba_bios.bin", "bios.bin"] {
            if let Some(bytes) = read_bios_file(&root.join(name)) {
                return Ok(bytes);
            }
            match context.platform.read_system_file(name) {
                Ok(Some(bytes)) if bytes.len() >= BIOS_BYTES => {
                    return Ok(bytes[..BIOS_BYTES].to_vec());
                }
                Ok(_) => {}
                Err(error) => {
                    return Err(CoreError::content(format!(
                        "could not read {name}: {error}"
                    )));
                }
            }
        }
        let save_dir = context.platform.save_dir();
        if !save_dir.as_os_str().is_empty() {
            for name in ["gba_bios.bin", "bios.bin"] {
                if let Some(bytes) = read_bios_file(&save_dir.join(name)) {
                    return Ok(bytes);
                }
            }
        }
        Ok(EMBEDDED_BIOS.to_vec())
    }

    fn save_path(content: &Content, context: &CoreContext) -> Option<PathBuf> {
        let dir = context.platform.save_dir();
        if dir.as_os_str().is_empty() {
            return None;
        }
        let stem = content
            .path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|stem| !stem.is_empty())
            .unwrap_or("game");
        let mut name: String = stem
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                    character
                } else {
                    '_'
                }
            })
            .collect();
        name.push_str(".sav");
        std::fs::create_dir_all(&dir).ok()?;
        Some(dir.join(name))
    }

    fn new_backend(&mut self) -> CoreResult<()> {
        let mut builder = GamepakBuilder::new().take_buffer(self.rom.clone().into_boxed_slice());
        if uses_rtc(&self.rom) {
            builder = builder.with_rtc();
        }
        builder = match &self.save_path {
            Some(path) => {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                builder.save_path(path)
            }
            None => builder.without_backup_to_file(),
        };
        let cartridge = builder
            .build()
            .map_err(|error| CoreError::content(error.to_string()))?;
        let (audio, consumer) = SimpleAudioInterface::create_channel(
            i32::try_from(self.sample_rate.max(1)).unwrap_or(DEFAULT_SAMPLE_RATE as i32),
            Some(16_384),
        );
        let mut core = GameBoyAdvance::new(self.bios.clone().into_boxed_slice(), cartridge, audio);
        // Start directly in the cartridge with the backend's post-BIOS state.
        // Keep the BIOS mapped for SWI services, but do not execute its boot
        // sequence or run game frames before the host starts the core.
        core.skip_bios();
        // skip_bios changes PC/registers but does not refill the CPU pipeline.
        core.cpu.reload_pipeline32();
        self.core = Some(core);
        self.audio_consumer = Some(consumer);
        self.frames = 0;
        Ok(())
    }

    fn reset(&mut self) -> CoreResult<()> {
        // Recreate the machine from the ROM. Battery saves are file-backed by
        // the backend and reopen automatically.
        self.new_backend()
    }

    fn apply_commands(&mut self, commands: &mut Vec<CommandRequest>) {
        let Some(control) = self.control.clone() else {
            return;
        };
        control.take_requests(commands);
        for request in commands.drain(..) {
            let id = request.id;
            let generation = request.generation;
            let outcome = match request.command {
                CoreCommand::Reset => match self.reset() {
                    Ok(()) => {
                        self.flush_audio();
                        CommandOutcome::Succeeded
                    }
                    Err(error) => {
                        self.emit_error(error.to_string());
                        CommandOutcome::Failed(error.to_string())
                    }
                },
                CoreCommand::SaveState { slot } => self.save_state(&slot),
                CoreCommand::LoadState { slot, data } => self.load_state(&slot, &data),
                CoreCommand::SaveRam => {
                    self.emit_save_ram();
                    CommandOutcome::Succeeded
                }
                CoreCommand::Screenshot => {
                    self.emit_screenshot();
                    CommandOutcome::Succeeded
                }
                CoreCommand::SetOption { .. } | CoreCommand::Extension { .. } => {
                    CommandOutcome::Unsupported
                }
            };
            self.report(generation, id, outcome);
        }
    }

    fn report(&mut self, generation: u64, id: u64, outcome: CommandOutcome) {
        if let Some(context) = self.context.as_mut() {
            context.events.emit(CoreEvent::CommandResult {
                id,
                generation,
                outcome,
                response: None,
            });
        }
    }

    fn flush_audio(&mut self) {
        if let Some(context) = self.context.as_mut() {
            context.audio.reset_pcm();
        }
    }

    fn state_identity(&self) -> u64 {
        content_hash(&self.rom)
    }

    fn save_state(&mut self, slot: &str) -> CommandOutcome {
        let Some(core) = self.core.as_ref() else {
            return CommandOutcome::Failed("GBA core is not initialized".into());
        };
        let payload = match core.save_state() {
            Ok(data) if !data.is_empty() => data,
            Ok(_) => {
                self.emit_error("GBA save state is empty".into());
                return CommandOutcome::Failed("GBA save state is empty".into());
            }
            Err(error) => {
                self.emit_error(format!("could not save state: {error}"));
                return CommandOutcome::Failed(error.to_string());
            }
        };
        let envelope = StateEnvelope::new(CORE_ID, STATE_REVISION, self.state_identity(), payload);
        match envelope.encode() {
            Ok(data) => {
                if let Some(context) = self.context.as_mut() {
                    context.events.emit(CoreEvent::SaveState {
                        slot: slot.to_owned(),
                        data,
                    });
                }
                CommandOutcome::Succeeded
            }
            Err(error) => {
                self.emit_error(format!("could not encode state: {error}"));
                CommandOutcome::Failed(error.to_string())
            }
        }
    }

    fn load_state(&mut self, slot: &str, data: &[u8]) -> CommandOutcome {
        let envelope =
            match StateEnvelope::decode_for(data, CORE_ID, STATE_REVISION, self.state_identity()) {
                Ok(envelope) => envelope,
                Err(error) => {
                    self.emit_error(format!("could not load state {slot}: {error}"));
                    return CommandOutcome::Failed(error.to_string());
                }
            };
        let Some(core) = self.core.as_mut() else {
            return CommandOutcome::Failed("GBA core is not initialized".into());
        };
        match core.restore_state(&envelope.payload) {
            Ok(()) => {
                self.flush_audio();
                CommandOutcome::Succeeded
            }
            Err(error) => {
                self.emit_error(format!("could not load state {slot}: {error}"));
                CommandOutcome::Failed(error.to_string())
            }
        }
    }

    fn emit_error(&mut self, message: String) {
        if let Some(context) = self.context.as_mut() {
            context.events.emit(CoreEvent::Error(message));
        }
    }

    fn emit_save_ram(&mut self) {
        let Some(path) = &self.save_path else {
            return;
        };
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            return;
        };
        let Some(context) = self.context.as_mut() else {
            return;
        };
        let data = match context.platform.read_save(name) {
            Ok(Some(data)) => Some(data),
            Ok(None) => match std::fs::read(path) {
                Ok(data) => Some(data),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => {
                    context.events.emit(CoreEvent::Error(format!(
                        "could not read battery save {}: {error}",
                        path.display()
                    )));
                    return;
                }
            },
            Err(error) => {
                context.events.emit(CoreEvent::Error(format!(
                    "could not read battery save {name}: {error}"
                )));
                return;
            }
        };
        if let Some(data) = data {
            context.events.emit(CoreEvent::SaveRam { data });
        }
    }

    fn emit_screenshot(&mut self) {
        let Some(core) = self.core.as_ref() else {
            return;
        };
        let pixels = core.get_frame_buffer();
        let mut rgba = Vec::with_capacity(pixels.len() * 4);
        for pixel in pixels {
            // RustBoyAdvance stores RGB24 in the low 24 bits: 0x00RRGGBB.
            rgba.extend_from_slice(&[
                ((pixel >> 19) & 0xff) as u8,
                ((pixel >> 11) & 0xff) as u8,
                ((pixel >> 3) & 0xff) as u8,
                255,
            ]);
        }
        if let Some(context) = self.context.as_mut() {
            context.events.emit(CoreEvent::Screenshot {
                width: DISPLAY_WIDTH as u32,
                height: DISPLAY_HEIGHT as u32,
                format: PixelFormat::Rgba8,
                pixels: rgba,
            });
        }
    }

    fn set_buttons(&mut self, buttons: fanplay_core::ButtonSet) {
        let Some(core) = self.core.as_mut() else {
            return;
        };
        let mut keys = KEYINPUT_ALL_RELEASED;
        for (button, bit) in [
            (Button::South, 0),
            (Button::East, 1),
            (Button::Select, 2),
            (Button::Start, 3),
            (Button::Right, 4),
            (Button::Left, 5),
            (Button::Up, 6),
            (Button::Down, 7),
            (Button::RightShoulder, 8),
            (Button::LeftShoulder, 9),
        ] {
            if buttons.contains(button) {
                keys &= !(1 << bit);
            }
        }
        *core.get_key_state_mut() = keys;
    }

    fn present_frame(&mut self) {
        let buttons = self
            .control
            .as_ref()
            .map(|control| control.input().snapshot().buttons)
            .unwrap_or_default();
        self.set_buttons(buttons);
        let Some(core) = self.core.as_mut() else {
            return;
        };
        core.frame();
        let pixels = core.get_frame_buffer();
        debug_assert_eq!(pixels.len(), DISPLAY_WIDTH * DISPLAY_HEIGHT);
        #[cfg(target_endian = "little")]
        {
            // RustBoyAdvance writes 0x00RRGGBB. In little-endian memory that is
            // B, G, R, X; submit it zero-copy and let the frontend treat X as
            // opaque.
            let bytes: &[u8] = bytemuck::cast_slice(pixels);
            if let Some(context) = self.context.as_mut() {
                context.video.present(VideoFrame::bgrx8(
                    DISPLAY_WIDTH as u32,
                    DISPLAY_HEIGHT as u32,
                    DISPLAY_WIDTH * 4,
                    bytes,
                ));
            }
        }
        #[cfg(target_endian = "big")]
        {
            if self.video.len() != pixels.len() * 4 {
                self.video.resize(pixels.len() * 4, 0);
            }
            for (pixel, out) in pixels.iter().zip(self.video.chunks_exact_mut(4)) {
                out.copy_from_slice(&[
                    ((pixel >> 19) & 0xff) as u8,
                    ((pixel >> 11) & 0xff) as u8,
                    ((pixel >> 3) & 0xff) as u8,
                    255,
                ]);
            }
            if let Some(context) = self.context.as_mut() {
                context.video.present(VideoFrame::rgba8(
                    DISPLAY_WIDTH as u32,
                    DISPLAY_HEIGHT as u32,
                    DISPLAY_WIDTH * 4,
                    &self.video,
                ));
            }
        }
        self.frames += 1;
    }

    fn drain_audio(&mut self) {
        let Some(consumer) = self.audio_consumer.as_mut() else {
            return;
        };
        let count = consumer.pop_slice(&mut self.audio_scratch);
        if count == 0 {
            return;
        }
        if let Some(context) = self.context.as_mut() {
            context.audio.queue_pcm(PcmBuffer {
                sample_rate: self.sample_rate,
                channels: 2,
                samples: &self.audio_scratch[..count],
            });
        }
    }
}

impl Core for GbaAdapter {
    fn info(&self) -> &CoreInfo {
        &INFO
    }

    fn load(&mut self, content: Content, context: CoreContext) -> CoreResult<()> {
        let rom = Self::read_rom(&content, &context)?;
        let bios = Self::read_bios(&content, &context)?;
        self.rom = rom;
        self.bios = bios;
        self.save_path = Self::save_path(&content, &context);
        let requested = context.session.audio;
        self.sample_rate = if requested.is_active() {
            requested.sample_rate
        } else if let Some(output) = context.audio.output_config() {
            output.sample_rate
        } else {
            std::env::var("FANPLAY_GBA_AUDIO_RATE")
                .ok()
                .and_then(|value| value.parse().ok())
                .filter(|rate: &u32| (8_000..=192_000).contains(rate))
                .unwrap_or(DEFAULT_SAMPLE_RATE)
        };
        self.sample_rate = self.sample_rate.clamp(8_000, 192_000);
        self.control = Some(Arc::clone(&context.control));
        self.context = Some(context);
        self.audio_scratch = vec![0i16; AUDIO_SCRATCH];
        self.new_backend()?;
        self.flush_audio();
        if let Some(context) = self.context.as_mut() {
            context.events.emit(CoreEvent::Loaded(CoreLoadInfo {
                width: DISPLAY_WIDTH as u32,
                height: DISPLAY_HEIGHT as u32,
                sample_rate: self.sample_rate,
                audio_channels: 2,
            }));
        }
        Ok(())
    }

    fn run(&mut self) -> CoreResult<ExitStatus> {
        let control = self
            .control
            .clone()
            .ok_or_else(|| CoreError::runtime("GBA core was not loaded"))?;
        if self.core.is_none() {
            return Err(CoreError::runtime("GBA core was not initialized"));
        }
        let mut commands = Vec::with_capacity(16);
        let period = 1.0 / FRAME_RATE;
        let mut next_deadline = 0.0f64;
        let mut audio_paused = false;
        // The frontend may drive the core one frame at a time (the browser
        // cannot block its main thread across a whole session). Treat the
        // frame limit as a per-`run` budget so repeated calls each advance.
        let mut run_frames = 0u64;
        while !control.should_stop() {
            self.apply_commands(&mut commands);
            if control.should_stop() {
                break;
            }
            if control.paused() {
                if !audio_paused {
                    if let Some(context) = self.context.as_mut() {
                        context.audio.pause(true);
                    }
                    audio_paused = true;
                }
                if let Some(context) = self.context.as_ref() {
                    context.platform.sleep(Duration::from_millis(2));
                }
                continue;
            }
            if audio_paused {
                if let Some(context) = self.context.as_mut() {
                    context.audio.pause(false);
                }
                audio_paused = false;
            }
            self.present_frame();
            self.drain_audio();
            run_frames += 1;
            if let Some(limit) = control.frame_limit()
                && run_frames >= limit
            {
                break;
            }
            if control.fast_forward() {
                next_deadline = 0.0;
                continue;
            }
            let Some(context) = self.context.as_ref() else {
                continue;
            };
            let now = context.platform.now();
            if now <= 0.0 {
                continue;
            }
            if next_deadline <= 0.0 || next_deadline < now - period {
                next_deadline = now + period;
            } else {
                let wait = next_deadline - now;
                if wait > 0.0 {
                    context.platform.sleep(Duration::from_secs_f64(wait));
                }
                next_deadline += period;
            }
        }
        self.emit_save_ram();
        Ok(ExitStatus::success())
    }
}

fn read_bios_file(path: &Path) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path).ok()?;
    (bytes.len() >= BIOS_BYTES).then(|| bytes[..BIOS_BYTES].to_vec())
}

fn uses_rtc(rom: &[u8]) -> bool {
    // The fourth game-code byte is the region. Upstream only overrides BPEE,
    // leaving other Emerald releases (and Ruby/Sapphire) without a clock.
    matches!(rom.get(0xAC..0xAF), Some(b"BPE" | b"AXV" | b"AXP"))
        || rom
            .windows(b"SIIRTC_V".len())
            .any(|bytes| bytes == b"SIIRTC_V")
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanplay_core::audio::NullAudioSink;
    use fanplay_core::events::NullEventSink;
    use fanplay_core::{CoreControl, VideoSink};
    use std::sync::{Arc, Mutex};

    struct HashSink(Arc<Mutex<u64>>);
    impl VideoSink for HashSink {
        fn present(&mut self, frame: VideoFrame<'_>) {
            assert!(frame.validate().is_ok());
            let hash = frame.pixels.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
                (h ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3)
            });
            *self.0.lock().unwrap() = hash;
        }
    }

    fn replay_rom() -> Vec<u8> {
        let program: [u32; 10] = [
            0xE3A0_0003, // MOV r0, #3 (mode 3)
            0xE380_0B01, // ORR r0, r0, #0x400 (BG2 enable)
            0xE3A0_1301, // MOV r1, #0x04000000
            0xE581_0000, // STR r0, [r1] (DISPCNT)
            0xE3A0_2406, // MOV r2, #0x06000000
            0xE3A0_3402, // MOV r3, #0x02000000
            0xE593_4000, // loop: LDR r4, [r3]
            0xE284_4001, // ADD r4, r4, #1
            0xE583_4000, // STR r4, [r3]
            0xE1C2_40B0, // STRH r4, [r2] (pixel 0,0)
        ];
        let mut rom = vec![0u8; 0x200];
        for (index, instruction) in program.iter().enumerate() {
            rom[index * 4..index * 4 + 4].copy_from_slice(&instruction.to_le_bytes());
        }
        // Branch back to `loop`; the counter keeps advancing so frames differ.
        rom[10 * 4..10 * 4 + 4].copy_from_slice(&0xEAFF_FFFAu32.to_le_bytes());
        rom
    }

    fn replay_adapter() -> (GbaAdapter, Arc<Mutex<u64>>) {
        let hash = Arc::new(Mutex::new(0));
        let control = Arc::new(CoreControl::new());
        let context = CoreContext::new(
            Arc::new(GbaTestPlatform),
            Box::new(HashSink(Arc::clone(&hash))),
            Box::new(NullAudioSink),
            Box::new(NullEventSink),
            control.clone(),
        );
        let mut adapter = GbaAdapter {
            bios: EMBEDDED_BIOS.to_vec(),
            rom: replay_rom(),
            sample_rate: DEFAULT_SAMPLE_RATE,
            context: Some(context),
            control: Some(control),
            ..Default::default()
        };
        adapter.new_backend().unwrap();
        (adapter, hash)
    }

    struct GbaTestPlatform;
    impl fanplay_core::Platform for GbaTestPlatform {
        fn read_content(&self, path: &str) -> fanplay_core::PlatformResult<Vec<u8>> {
            Err(fanplay_core::PlatformError::not_found(path))
        }
        fn save_dir(&self) -> PathBuf {
            std::env::temp_dir()
        }
        fn now(&self) -> f64 {
            0.0
        }
    }

    fn advance(adapter: &mut GbaAdapter, hash: &Arc<Mutex<u64>>, frames: usize) -> Vec<u64> {
        let mut hashes = Vec::with_capacity(frames);
        for _ in 0..frames {
            adapter.present_frame();
            hashes.push(*hash.lock().unwrap());
        }
        hashes
    }

    #[test]
    fn state_restore_replays_the_same_frames_and_rejects_corruption() {
        let (mut adapter, hash) = replay_adapter();
        advance(&mut adapter, &hash, 3);
        let payload = adapter.core.as_ref().unwrap().save_state().unwrap();
        let envelope =
            StateEnvelope::new(CORE_ID, STATE_REVISION, adapter.state_identity(), payload)
                .encode()
                .unwrap();
        let expected = advance(&mut adapter, &hash, 3);

        let mut corrupt = envelope.clone();
        corrupt[40] ^= 1;
        assert!(matches!(
            adapter.load_state("1", &corrupt),
            CommandOutcome::Failed(_)
        ));
        // A rejected state must leave the running machine untouched.
        let after_failure = advance(&mut adapter, &hash, 1);
        let restored = adapter.load_state("1", &envelope);
        assert_eq!(restored, CommandOutcome::Succeeded);
        let replayed = advance(&mut adapter, &hash, 3);
        assert_eq!(replayed, expected);
        assert_ne!(after_failure[0], expected[0]);
    }

    #[test]
    fn pokemon_rtc_is_enabled_for_every_region_on_load_and_reset() {
        for family in [b"BPE", b"AXV", b"AXP"] {
            for region in b"EJDFIS" {
                let mut rom = vec![0; 0x200];
                rom[0xAC..0xAF].copy_from_slice(family);
                rom[0xAF] = *region;
                let mut adapter = GbaAdapter {
                    bios: EMBEDDED_BIOS.to_vec(),
                    rom,
                    sample_rate: DEFAULT_SAMPLE_RATE,
                    ..Default::default()
                };
                adapter.new_backend().unwrap();
                let core = adapter.core.as_ref().unwrap();
                assert!(core.cpu.bus.cartridge.get_gpio().is_some());
                adapter.reset().unwrap();
                let core = adapter.core.as_ref().unwrap();
                assert!(core.cpu.bus.cartridge.get_gpio().is_some());
            }
        }
    }

    #[test]
    fn rtc_signature_supports_other_games_without_enabling_unrelated_roms() {
        assert!(uses_rtc(b"homebrew SIIRTC_V001"));
        assert!(!uses_rtc(b""));
        assert!(!uses_rtc(&[0; 0x200]));
        let mut rom = vec![0; 0x200];
        rom[0xAC..0xB0].copy_from_slice(b"BPRE");
        assert!(!uses_rtc(&rom));
    }

    #[test]
    fn startup_and_reset_do_not_execute_hidden_frames() {
        let mut rom = vec![0; 0x200];
        rom[..4].copy_from_slice(&0xEAFF_FFFEu32.to_le_bytes());
        rom[0xB2] = 0x96;
        rom[0xBD] = rom[0xA0..=0xBC]
            .iter()
            .fold(0u8, |sum, byte| sum.wrapping_sub(*byte))
            .wrapping_sub(0x19);
        let mut adapter = GbaAdapter {
            bios: EMBEDDED_BIOS.to_vec(),
            rom,
            sample_rate: DEFAULT_SAMPLE_RATE,
            ..Default::default()
        };
        for _ in 0..2 {
            adapter.new_backend().unwrap();
            // ARM PC is two instructions ahead after filling the pipeline.
            assert_eq!(adapter.core.as_ref().unwrap().cpu.pc, 0x0800_0008);
            let mut samples = [0; 32];
            assert_eq!(
                adapter
                    .audio_consumer
                    .as_mut()
                    .unwrap()
                    .pop_slice(&mut samples),
                0
            );
        }
    }
}
