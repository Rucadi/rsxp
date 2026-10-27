// SPDX-License-Identifier: GPL-2.0-or-later
//! Game Boy and Game Boy Color core adapter.
//!
//! The emulator is the portable library from mvdnes/rboy. This crate owns
//! the FANPLAY scheduling, presentation, input, audio and storage boundary.

use ciborium::{de::from_reader, ser::into_writer};
use fanplay_core::{
    content_hash, AudioInfo, Button, ButtonSet, CommandOutcome, CommandRequest, Content,
    ContentKind, ContentProbe, Core, CoreCapabilities, CoreCommand, CoreContext, CoreError,
    CoreEvent, CoreInfo, CoreLoadInfo, CoreRegistration, CoreResult, ExitStatus, InputInfo,
    PcmBuffer, PixelFormat, PlatformError, PresentationSupport, ScreenInfo, ScreenRotation,
    StateEnvelope, VideoFrame,
};
use rboy::{device::Device, AudioPlayer, KeypadKey, SCREEN_H, SCREEN_W};
use std::io::Cursor;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const CORE_ID: &str = "gbc";
const STATE_REVISION: u32 = 2;
const FRAME_RATE: f64 = 59.7275;
const SOURCE_AUDIO_RATE: u32 = 44_100;
const MAX_FRAME_TICKS: u32 = 456 * 154 * 2;
const WIDTH: u32 = SCREEN_W as u32;
const HEIGHT: u32 = SCREEN_H as u32;

const SCREENS: &[ScreenInfo] = &[ScreenInfo {
    id: "main",
    width: WIDTH,
    height: HEIGHT,
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
];

const INPUT_MAP: &[(Button, KeypadKey)] = &[
    (Button::Right, KeypadKey::Right),
    (Button::Left, KeypadKey::Left),
    (Button::Up, KeypadKey::Up),
    (Button::Down, KeypadKey::Down),
    (Button::South, KeypadKey::A),
    (Button::East, KeypadKey::B),
    (Button::Select, KeypadKey::Select),
    (Button::Start, KeypadKey::Start),
];

pub const INFO: CoreInfo = CoreInfo {
    id: CORE_ID,
    name: "Game Boy / Game Boy Color",
    version: env!("CARGO_PKG_VERSION"),
    state_revision: STATE_REVISION,
    extensions: &["gb", "gbc"],
    capabilities: CoreCapabilities::SAVE_STATES
        .union(CoreCapabilities::SAVE_RAM)
        .union(CoreCapabilities::SCREENSHOTS)
        .union(CoreCapabilities::RESET),
    presentation: PresentationSupport::Cpu,
    pixel_formats: &[PixelFormat::Rgb24],
    audio: AudioInfo::new(SOURCE_AUDIO_RATE, 2, true),
    input: InputInfo {
        buttons: BUTTONS,
        pointer: false,
        pointer_button: false,
    },
    screens: SCREENS,
    options: &[],
};

pub fn register(registry: &mut fanplay_core::CoreRegistry) {
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
    Box::new(GbcAdapter::default())
}

/// Claims both conventional Game Boy cartridge extensions.
pub fn detect(probe: &ContentProbe<'_>) -> u32 {
    if !probe.is_dir && (probe.has_extension("gb") || probe.has_extension("gbc")) {
        80
    } else {
        0
    }
}

struct RboyAudio {
    samples: Arc<Mutex<Vec<i16>>>,
    sample_rate: u32,
}

impl AudioPlayer for RboyAudio {
    fn play(&mut self, left_channel: &[f32], right_channel: &[f32]) {
        let mut samples = self.samples.lock().expect("rboy audio buffer poisoned");
        for (&left, &right) in left_channel.iter().zip(right_channel) {
            samples.push((left.clamp(-1.0, 1.0) * 32_767.0) as i16);
            samples.push((right.clamp(-1.0, 1.0) * 32_767.0) as i16);
        }
    }

    fn samples_rate(&self) -> u32 {
        self.sample_rate
    }

    fn underflowed(&self) -> bool {
        false
    }
}

#[derive(Default)]
struct GbcAdapter {
    context: Option<CoreContext>,
    control: Option<Arc<fanplay_core::CoreControl>>,
    device: Option<Device>,
    rom: Vec<u8>,
    video: Vec<u8>,
    audio_samples: Arc<Mutex<Vec<i16>>>,
    audio_output: Vec<i16>,
    audio_enabled: bool,
    sample_rate: u32,
    save_name: String,
    buttons: ButtonSet,
    frames: u64,
    commands: Vec<CommandRequest>,
}

impl GbcAdapter {
    fn read_rom(content: &Content, context: &CoreContext) -> CoreResult<Vec<u8>> {
        if content.kind == ContentKind::Directory {
            return Err(CoreError::content(
                "Game Boy content must be a single .gb or .gbc ROM file",
            ));
        }
        std::fs::read(&content.path)
            .or_else(|native_error| {
                let path = content.path.to_string_lossy();
                let by_name = content.path.file_name().and_then(|name| name.to_str());
                context
                    .platform
                    .read_content(&path)
                    .or_else(|_| match by_name {
                        Some(name) => context.platform.read_content(name),
                        None => Err(PlatformError::not_found(path.as_ref())),
                    })
                    .map_err(|error| std::io::Error::other(format!("{native_error} ({error})")))
            })
            .map_err(|error| {
                CoreError::content(format!(
                    "could not read {}: {error}",
                    content.path.display()
                ))
            })
    }

    fn save_name(content: &Content) -> String {
        let stem = content
            .path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|stem| !stem.is_empty())
            .unwrap_or("game");
        let safe: String = stem
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                    character
                } else {
                    '_'
                }
            })
            .collect();
        format!("{safe}.sav")
    }

    fn new_device(&self) -> CoreResult<Device> {
        let is_cgb = self.rom.get(0x143).is_some_and(|flags| flags & 0x80 != 0);
        let mut device = if is_cgb {
            Device::new_cgb_from_buffer(self.rom.clone(), true, None)
        } else {
            Device::new_from_buffer(self.rom.clone(), true, None)
        }
        .map_err(|error| CoreError::content(error.to_owned()))?;

        if self.audio_enabled {
            device.enable_audio(
                Box::new(RboyAudio {
                    samples: Arc::clone(&self.audio_samples),
                    sample_rate: self.sample_rate,
                }),
                true,
            );
        }
        Ok(device)
    }

    fn load_initial_save(&mut self, context: &CoreContext) -> CoreResult<()> {
        let Some(data) = context
            .platform
            .read_save(&self.save_name)
            .map_err(|error| {
                CoreError::content(format!(
                    "could not read battery save {}: {error}",
                    self.save_name
                ))
            })?
        else {
            return Ok(());
        };
        if let Err(error) = self.load_ram_compat(&data) {
            self.emit_error(format!(
                "ignoring incompatible battery save {}: {error}",
                self.save_name
            ));
        }
        Ok(())
    }

    fn load_ram_compat(&mut self, data: &[u8]) -> Result<(), &'static str> {
        let Some(device) = self.device.as_mut() else {
            return Err("Game Boy core is not initialized");
        };
        let error = match device.loadram(data) {
            Ok(()) => return Ok(()),
            Err(error) => error,
        };
        if error != "Loaded ram is too small" {
            return Err(error);
        }

        // Older FANPLAY saves stored MBC3 RAM without rboy's eight-byte RTC
        // prefix. Retrying with a zero RTC preserves those saves while still
        // rejecting truncated or otherwise incompatible data.
        let mut legacy_mbc3 = Vec::with_capacity(data.len() + 8);
        legacy_mbc3.extend_from_slice(&[0; 8]);
        legacy_mbc3.extend_from_slice(data);
        device.loadram(&legacy_mbc3)
    }

    fn new_backend(&mut self) -> CoreResult<()> {
        self.device = Some(self.new_device()?);
        self.audio_samples
            .lock()
            .expect("rboy audio buffer poisoned")
            .clear();
        self.audio_output.clear();
        self.buttons = ButtonSet::empty();
        self.frames = 0;
        Ok(())
    }

    fn reset(&mut self) -> CoreResult<()> {
        self.flush_save(true)?;
        self.new_backend()?;
        let save_data = {
            let context = self
                .context
                .as_ref()
                .ok_or_else(|| CoreError::runtime("Game Boy core has no context"))?;
            context
                .platform
                .read_save(&self.save_name)
                .map_err(|error| {
                    CoreError::content(format!(
                        "could not read battery save {}: {error}",
                        self.save_name
                    ))
                })?
        };
        if let Some(data) = save_data {
            if let Err(error) = self.load_ram_compat(&data) {
                self.emit_error(format!(
                    "ignoring incompatible battery save {}: {error}",
                    self.save_name
                ));
            }
        }
        Ok(())
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

    fn emit_error(&mut self, message: impl Into<String>) {
        if let Some(context) = self.context.as_mut() {
            context.events.emit(CoreEvent::Error(message.into()));
        }
    }

    fn flush_audio(&mut self) {
        self.audio_samples
            .lock()
            .expect("rboy audio buffer poisoned")
            .clear();
        self.audio_output.clear();
        if let Some(context) = self.context.as_mut() {
            context.audio.reset_pcm();
        }
    }

    fn state_identity(&self) -> u64 {
        content_hash(&self.rom)
    }

    fn save_state(&mut self, slot: &str) -> CommandOutcome {
        let Some(device) = self.device.as_ref() else {
            return CommandOutcome::Failed("Game Boy core is not initialized".into());
        };
        let mut payload = Vec::new();
        if let Err(error) = into_writer(device, &mut payload) {
            return CommandOutcome::Failed(error.to_string());
        }
        if payload.is_empty() {
            return CommandOutcome::Failed("Game Boy save state is empty".into());
        }
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
            Err(error) => CommandOutcome::Failed(error.to_string()),
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
        let mut device: Device = match from_reader(Cursor::new(&envelope.payload)) {
            Ok(device) => device,
            Err(error) => {
                self.emit_error(format!("could not load state {slot}: {error}"));
                return CommandOutcome::Failed(error.to_string());
            }
        };
        if self.audio_enabled {
            device.enable_audio(
                Box::new(RboyAudio {
                    samples: Arc::clone(&self.audio_samples),
                    sample_rate: self.sample_rate,
                }),
                true,
            );
        }
        self.device = Some(device);
        self.flush_audio();
        self.buttons = ButtonSet::empty();
        CommandOutcome::Succeeded
    }

    fn emit_save_ram(&mut self) {
        let Some(device) = self.device.as_mut() else {
            return;
        };
        if !device.ram_is_battery_backed() {
            return;
        }
        let data = device.dumpram();
        if let Some(context) = self.context.as_mut() {
            context.events.emit(CoreEvent::SaveRam { data });
        }
    }

    fn flush_save(&mut self, emit: bool) -> CoreResult<()> {
        let changed = self
            .device
            .as_mut()
            .map(|device| device.check_and_reset_ram_updated())
            .unwrap_or(false);
        if emit || changed {
            self.emit_save_ram();
        }
        Ok(())
    }

    fn set_buttons(&mut self, buttons: ButtonSet) {
        let Some(device) = self.device.as_mut() else {
            return;
        };
        for &(button, key) in INPUT_MAP {
            let was_pressed = self.buttons.contains(button);
            let is_pressed = buttons.contains(button);
            match (was_pressed, is_pressed) {
                (false, true) => device.keydown(key),
                (true, false) => device.keyup(key),
                _ => {}
            }
        }
        self.buttons = buttons;
    }

    fn present_frame(&mut self) {
        let buttons = self
            .control
            .as_ref()
            .map(|control| control.input().snapshot().buttons)
            .unwrap_or_default();
        self.set_buttons(buttons);
        {
            let Some(device) = self.device.as_mut() else {
                return;
            };
            let mut ticks = 0;
            while ticks < MAX_FRAME_TICKS && !device.check_and_reset_gpu_updated() {
                ticks = ticks.saturating_add(device.do_cycle());
            }
            self.video.clear();
            self.video.extend_from_slice(device.get_gpu_data());
        }
        if let Some(context) = self.context.as_mut() {
            context.video.present(VideoFrame::new(
                WIDTH,
                HEIGHT,
                (WIDTH * 3) as usize,
                PixelFormat::Rgb24,
                &self.video,
            ));
        }
        self.frames += 1;
    }

    fn drain_audio(&mut self) {
        if !self.audio_enabled {
            return;
        }
        self.audio_output.clear();
        let mut samples = self
            .audio_samples
            .lock()
            .expect("rboy audio buffer poisoned");
        self.audio_output.extend(samples.drain(..));
        drop(samples);
        if self.audio_output.is_empty() {
            return;
        }
        if let Some(context) = self.context.as_mut() {
            context.audio.queue_pcm(PcmBuffer {
                sample_rate: self.sample_rate,
                channels: 2,
                samples: &self.audio_output,
            });
        }
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
                CoreCommand::SaveRam => match self.flush_save(true) {
                    Ok(()) => CommandOutcome::Succeeded,
                    Err(error) => CommandOutcome::Failed(error.to_string()),
                },
                CoreCommand::Screenshot => {
                    if let Some(context) = self.context.as_mut() {
                        let mut pixels = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
                        for rgb in self.video.chunks_exact(3) {
                            pixels.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
                        }
                        context.events.emit(CoreEvent::Screenshot {
                            width: WIDTH,
                            height: HEIGHT,
                            format: PixelFormat::Rgba8,
                            pixels,
                        });
                    }
                    CommandOutcome::Succeeded
                }
                CoreCommand::SetOption { .. } | CoreCommand::Extension { .. } => {
                    CommandOutcome::Unsupported
                }
            };
            self.report(generation, id, outcome);
        }
    }
}

impl Core for GbcAdapter {
    fn info(&self) -> &CoreInfo {
        &INFO
    }

    fn load(&mut self, content: Content, context: CoreContext) -> CoreResult<()> {
        self.rom = Self::read_rom(&content, &context)?;
        if self.rom.len() < 0x150 {
            return Err(CoreError::content(
                "Game Boy ROM is smaller than its header",
            ));
        }
        self.save_name = Self::save_name(&content);
        self.sample_rate = if context.session.audio.is_active() {
            context.session.audio.sample_rate
        } else if let Some(output) = context.audio.output_config() {
            output.sample_rate
        } else {
            SOURCE_AUDIO_RATE
        }
        .clamp(8_000, 192_000);
        self.audio_enabled =
            context.session.audio.is_active() || context.audio.output_config().is_some();
        self.control = Some(Arc::clone(&context.control));
        self.context = Some(context);
        self.new_backend()?;
        let context = self.context.take().expect("context stored above");
        self.load_initial_save(&context)?;
        self.context = Some(context);
        if let Some(context) = self.context.as_mut() {
            context.events.emit(CoreEvent::Loaded(CoreLoadInfo {
                width: WIDTH,
                height: HEIGHT,
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
            .ok_or_else(|| CoreError::runtime("Game Boy core was not loaded"))?;
        if self.device.is_none() {
            return Err(CoreError::runtime("Game Boy core was not initialized"));
        }
        let mut commands = Vec::with_capacity(16);
        let mut next_deadline = 0.0;
        let mut audio_paused = false;
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
            if run_frames.is_multiple_of(60) {
                let _ = self.flush_save(false);
            }
            if let Some(limit) = control.frame_limit()
                && run_frames >= limit
            {
                break;
            }
            if control.fast_forward() {
                next_deadline = 0.0;
                continue;
            }
            let period = control
                .target_fps()
                .map_or(1.0 / FRAME_RATE, |fps| 1.0 / f64::from(fps));
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
        self.flush_save(true)?;
        Ok(ExitStatus::success())
    }

    fn run_slice(&mut self) -> CoreResult<fanplay_core::RunSliceStatus> {
        let control = self
            .control
            .clone()
            .ok_or_else(|| CoreError::runtime("Game Boy core was not loaded"))?;
        if self.device.is_none() {
            return Err(CoreError::runtime("Game Boy core was not initialized"));
        }
        if control.should_stop() {
            self.flush_save(true)?;
            return Ok(fanplay_core::RunSliceStatus::Exited(ExitStatus::success()));
        }

        // Move the reusable command buffer out briefly to avoid borrowing the
        // adapter and one of its fields mutably at the same time.
        let mut commands = std::mem::take(&mut self.commands);
        self.apply_commands(&mut commands);
        self.commands = commands;
        if control.should_stop() {
            self.flush_save(true)?;
            return Ok(fanplay_core::RunSliceStatus::Exited(ExitStatus::success()));
        }
        if control.paused() {
            if let Some(context) = self.context.as_mut() {
                context.audio.pause(true);
            }
            return Ok(fanplay_core::RunSliceStatus::Continue);
        }
        if let Some(context) = self.context.as_mut() {
            context.audio.pause(false);
        }

        self.present_frame();
        self.drain_audio();
        if self.frames.is_multiple_of(60) {
            let _ = self.flush_save(false);
        }
        if control
            .frame_limit()
            .is_some_and(|limit| self.frames >= limit)
        {
            self.flush_save(true)?;
            return Ok(fanplay_core::RunSliceStatus::Exited(ExitStatus::success()));
        }
        Ok(fanplay_core::RunSliceStatus::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn routes_both_game_boy_extensions() {
        let gb = ContentProbe::new(Path::new("pokemon.gb"), false, &[]);
        let gbc = ContentProbe::new(Path::new("pokemon.GBC"), false, &[]);
        assert_eq!(detect(&gb), 80);
        assert_eq!(detect(&gbc), 80);
        assert_eq!(
            detect(&ContentProbe::new(Path::new("pokemon.gba"), false, &[])),
            0
        );
    }

    #[test]
    fn cartridge_header_selects_the_rboy_mode() {
        let mut classic = vec![0u8; 0x150];
        classic[0x147] = 0x00;
        assert!(Device::new_from_buffer(classic, true, None).is_ok());

        let mut color = vec![0u8; 0x150];
        color[0x143] = 0x80;
        color[0x147] = 0x00;
        assert!(Device::new_cgb_from_buffer(color, true, None).is_ok());
    }

    #[test]
    fn accepts_legacy_mbc3_ram_without_rtc_prefix() {
        let mut rom = vec![0u8; 0x8000];
        rom[0x147] = 0x13;
        rom[0x149] = 0x03;

        let mut adapter = GbcAdapter::default();
        adapter.device = Some(Device::new_from_buffer(rom, true, None).unwrap());
        assert!(adapter.load_ram_compat(&vec![0; 0x8000]).is_ok());
    }
}
