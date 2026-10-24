// SPDX-License-Identifier: GPL-3.0-or-later
//! Nintendo DS adapter using Dust's interpreter and GPU renderers on desktop.
mod renderer;
mod rtc;
mod save;
mod state;
use dust_core::{
    audio::OutputSample,
    cpu::{arm7, arm9, interpreter::Interpreter},
    emu::{self, input::Keys, Emu, RunOutput},
    flash::Flash,
    spi::firmware,
    utils::{zeroed_box, BoxedByteSlice, Bytes},
    Model, SaveContents,
};
use rsxp_core::{
    AudioInfo, Button, CommandOutcome, Content, ContentKind, ContentProbe, Core, CoreCapabilities,
    CoreCommand, CoreContext, CoreError, CoreEvent, CoreInfo, CoreLoadInfo, CoreRegistration,
    CoreResult, ExitStatus, InputInfo, PcmBuffer, PixelFormat, PlatformError, PresentationSupport,
    ScreenInfo, ScreenRotation, VideoFrame,
};
use std::{cell::RefCell, num::NonZeroU32, path::Path, rc::Rc, time::Duration};

pub const CORE_ID: &str = "nds";
const STATE_REVISION: u32 = 1;
const SCREENS: &[ScreenInfo] = &[
    ScreenInfo {
        id: "top",
        width: 256,
        height: 192,
        x: 0,
        y: 0,
        rotation: ScreenRotation::None,
        touch: false,
    },
    ScreenInfo {
        id: "touch",
        width: 256,
        height: 192,
        x: 0,
        y: 192,
        rotation: ScreenRotation::None,
        touch: true,
    },
];
const BUTTONS: &[Button] = &[
    Button::Up,
    Button::Down,
    Button::Left,
    Button::Right,
    Button::South,
    Button::East,
    Button::North,
    Button::West,
    Button::Start,
    Button::Select,
    Button::LeftShoulder,
    Button::RightShoulder,
    Button::Extra0,
];

pub const INFO: CoreInfo = CoreInfo {
    id: CORE_ID,
    name: "Nintendo DS (Dust)",
    version: env!("CARGO_PKG_VERSION"),
    state_revision: STATE_REVISION,
    extensions: &["nds"],
    capabilities: CoreCapabilities::SAVE_STATES
        .union(CoreCapabilities::SAVE_RAM)
        .union(CoreCapabilities::RESET)
        .union(CoreCapabilities::MULTI_SCREEN)
        .union(CoreCapabilities::SCREENSHOTS),
    presentation: PresentationSupport::Cpu,
    pixel_formats: &[PixelFormat::Rgba8],
    audio: AudioInfo::new(48_000, 2, true),
    input: InputInfo {
        buttons: BUTTONS,
        pointer: true,
        pointer_button: true,
    },
    screens: SCREENS,
    options: &[],
};
pub fn registration() -> CoreRegistration {
    CoreRegistration {
        info: INFO,
        create,
        detect,
    }
}
pub fn create() -> Box<dyn Core> {
    Box::new(NdsAdapter::default())
}
pub fn detect(probe: &ContentProbe<'_>) -> u32 {
    if !probe.is_dir && probe.has_extension("nds") {
        80
    } else {
        0
    }
}

struct Audio {
    samples: Rc<RefCell<Vec<i16>>>,
}
impl dust_core::audio::Backend for Audio {
    fn handle_sample_chunk(&mut self, samples: &mut Vec<[OutputSample; 2]>) {
        let mut out = self.samples.borrow_mut();
        for sample in samples.drain(..).flatten() {
            out.push((sample * 32768.0).clamp(-32768.0, 32767.0) as i16);
        }
    }
}
#[derive(Default)]
struct NdsAdapter {
    context: Option<CoreContext>,
    emu: Option<Emu<Interpreter>>,
    gpu_output: Option<renderer::GpuOutput>,
    rom: Vec<u8>,
    bios7: Option<Box<Bytes<{ arm7::BIOS_SIZE }>>>,
    bios9: Option<Box<Bytes<{ arm9::BIOS_SIZE }>>>,
    firmware: Vec<u8>,
    samples: Rc<RefCell<Vec<i16>>>,
    video: Vec<u8>,
    save_name: String,
    sample_rate: u32,
}
fn read(content: &Content, context: &CoreContext) -> CoreResult<Vec<u8>> {
    if content.kind == ContentKind::Directory {
        return Err(CoreError::content("DS content must be an .nds ROM file"));
    }
    std::fs::read(&content.path)
        .or_else(|_| {
            context
                .platform
                .read_content(&content.path.to_string_lossy())
        })
        .or_else(|_| {
            content
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| PlatformError::not_found(content.path.display().to_string()))
                .and_then(|n| context.platform.read_content(n))
        })
        .map_err(|error| {
            CoreError::content(format!(
                "could not read {}: {error}",
                content.path.display()
            ))
        })
}
fn system_file(
    content: &Content,
    context: &CoreContext,
    env: &str,
    name: &str,
) -> CoreResult<Option<Vec<u8>>> {
    if let Some(path) = std::env::var_os(env) {
        return std::fs::read(&path)
            .map(Some)
            .map_err(|error| CoreError::content(format!("{env}: {error}")));
    }
    if let Some(bytes) =
        std::fs::read(content.path.parent().unwrap_or(Path::new(".")).join(name)).ok()
    {
        return Ok(Some(bytes));
    }
    context
        .platform
        .read_system_file(name)
        .map_err(|error| CoreError::content(format!("could not read {name}: {error}")))
}
fn bios<const N: usize>(bytes: Option<Vec<u8>>, name: &str) -> CoreResult<Option<Box<Bytes<N>>>> {
    bytes
        .map(|bytes| {
            if bytes.len() != N {
                return Err(CoreError::content(format!(
                    "{name} must be exactly {N} bytes"
                )));
            }
            let mut result = zeroed_box::<Bytes<N>>();
            result.copy_from_slice(&bytes);
            Ok(result)
        })
        .transpose()
}
fn validate_rom(rom: &[u8]) -> CoreResult<()> {
    if !(0x200..=0x2000_0000).contains(&rom.len()) {
        return Err(CoreError::content(
            "DS ROM must be between 512 bytes and 512 MiB",
        ));
    }
    for base in [0x20, 0x30] {
        let offset = u32::from_le_bytes(rom[base..base + 4].try_into().unwrap()) as usize;
        let size = u32::from_le_bytes(rom[base + 12..base + 16].try_into().unwrap()) as usize;
        if size == 0
            || size > 0x40_0000
            || offset.checked_add(size).is_none_or(|end| end > rom.len())
        {
            return Err(CoreError::content(
                "invalid or truncated DS ARM executable section",
            ));
        }
    }
    Ok(())
}
impl NdsAdapter {
    fn build(
        &self,
        saved: Option<Vec<u8>>,
    ) -> CoreResult<(Emu<Interpreter>, Option<renderer::GpuOutput>)> {
        let mut rom = BoxedByteSlice::new_zeroed(self.rom.len().next_power_of_two());
        rom[..self.rom.len()].copy_from_slice(&self.rom);
        let mut fw = BoxedByteSlice::new_zeroed(self.firmware.len());
        fw.copy_from_slice(&self.firmware);
        let firmware = Flash::new(
            SaveContents::Existing(fw),
            firmware::id_for_model(Model::Lite),
        )
        .map_err(|_| CoreError::content("invalid DS firmware size"))?;
        let renderers = renderer::new();
        let mut builder = emu::Builder::new(
            firmware,
            Some(Box::new(rom)),
            save::device(&self.rom, saved)?,
            Box::new(Audio {
                samples: self.samples.clone(),
            }),
            None,
            Box::<rtc::Clock>::default(),
            renderers.rx,
            renderers.tx,
            None,
        );
        builder.model = Model::Lite;
        builder.arm7_bios = self.bios7.clone();
        builder.arm9_bios = self.bios9.clone();
        builder.audio_custom_sample_rate = NonZeroU32::new(self.sample_rate);
        builder.audio_sample_chunk_size = 512;
        builder
            .build(Interpreter)
            .map(|emu| (emu, renderers.gpu_output))
            .map_err(|error| {
                CoreError::content(match error {
                    emu::BuildError::RomNeedsDecryptionButNoBiosProvided => {
                        "encrypted DS ROM needs your BIOS dump (RSXP_NDS_BIOS7)"
                    }
                    emu::BuildError::MissingSysFiles => "missing DS BIOS/firmware",
                    emu::BuildError::MissingRom => "missing DS ROM",
                    emu::BuildError::RomCreation(_) => "invalid DS cartridge",
                })
            })
    }
    fn flush_save(&mut self, emit: bool) -> CoreResult<()> {
        let emu = self.emu.as_mut().unwrap();
        let context = self.context.as_mut().unwrap();
        if emu.ds_slot.spi.contents().is_empty() {
            return Ok(());
        }
        if emu.ds_slot.spi.contents_dirty() || emit {
            context
                .platform
                .write_save(&self.save_name, emu.ds_slot.spi.contents())
                .map_err(|error| CoreError::runtime(error.to_string()))?;
            emu.ds_slot.spi.mark_contents_flushed();
        }
        if emit {
            context.events.emit(CoreEvent::SaveRam {
                data: emu.ds_slot.spi.contents().to_vec(),
            });
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
    fn flush_audio(&mut self) {
        self.samples.borrow_mut().clear();
        if let Some(context) = self.context.as_mut() {
            context.audio.reset_pcm();
        }
    }
    fn command(&mut self, command: CoreCommand) -> CoreResult<CommandOutcome> {
        Ok(match command {
            CoreCommand::Reset => {
                self.flush_save(false)?;
                let saved = self.emu.as_ref().unwrap().ds_slot.spi.contents().to_vec();
                let (emu, gpu_output) = self.build(Some(saved))?;
                self.emu = Some(emu);
                self.gpu_output = gpu_output;
                self.flush_audio();
                CommandOutcome::Succeeded
            }
            CoreCommand::SaveRam => {
                self.flush_save(true)?;
                CommandOutcome::Succeeded
            }
            CoreCommand::Screenshot => {
                self.context
                    .as_mut()
                    .unwrap()
                    .events
                    .emit(CoreEvent::Screenshot {
                        width: 256,
                        height: 384,
                        format: PixelFormat::Rgba8,
                        pixels: self.video.clone(),
                    });
                CommandOutcome::Succeeded
            }
            CoreCommand::SaveState { slot } => {
                let data = self.save_state()?;
                self.context
                    .as_mut()
                    .unwrap()
                    .events
                    .emit(CoreEvent::SaveState { slot, data });
                CommandOutcome::Succeeded
            }
            CoreCommand::LoadState { data, .. } => {
                self.load_state(&data)?;
                self.flush_audio();
                CommandOutcome::Succeeded
            }
            CoreCommand::SetOption { .. } | CoreCommand::Extension { .. } => {
                CommandOutcome::Unsupported
            }
        })
    }
}
impl Core for NdsAdapter {
    fn info(&self) -> &CoreInfo {
        &INFO
    }
    fn load(&mut self, content: Content, context: CoreContext) -> CoreResult<()> {
        self.rom = read(&content, &context)?;
        validate_rom(&self.rom)?;
        self.bios7 = bios(
            system_file(&content, &context, "RSXP_NDS_BIOS7", "bios7.bin")?,
            "bios7.bin",
        )?;
        self.bios9 = bios(
            system_file(&content, &context, "RSXP_NDS_BIOS9", "bios9.bin")?,
            "bios9.bin",
        )?;
        self.firmware = system_file(&content, &context, "RSXP_NDS_FIRMWARE", "firmware.bin")?
            .unwrap_or_else(|| firmware::default(Model::Lite).to_vec());
        if !firmware::is_valid_size(self.firmware.len()) {
            return Err(CoreError::content(
                "DS firmware must be 128, 256, or 512 KiB",
            ));
        }
        let settings =
            (u16::from_le_bytes(self.firmware[0x20..0x22].try_into().unwrap()) as usize) << 3;
        if settings + 0x200 > self.firmware.len() {
            return Err(CoreError::content(
                "DS firmware has invalid user-settings offsets",
            ));
        }
        self.sample_rate = if context.session.audio.is_active() {
            context.session.audio.sample_rate
        } else if let Some(output) = context.audio.output_config() {
            output.sample_rate
        } else {
            std::env::var("RSXP_NDS_AUDIO_RATE")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|v| (8000..=192000).contains(v))
                .unwrap_or(48000)
        }
        .clamp(8000, 192000);
        self.samples = Rc::new(RefCell::new(Vec::with_capacity(
            self.sample_rate as usize / 20 + 2048,
        )));
        self.video = vec![0; 256 * 384 * 4];
        let stem = content
            .path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy();
        self.save_name = format!(
            "{}.sav",
            stem.chars()
                .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                    c
                } else {
                    '_'
                })
                .collect::<String>()
        );
        let save_dir = context.platform.save_dir();
        let save_path = save_dir.join(&self.save_name);
        let saved = match context.platform.read_save(&self.save_name) {
            Ok(Some(bytes)) => Some(bytes),
            // An empty save directory means the platform owns save storage
            // (for example a browser memory store). Never probe `std::fs`,
            // which is unsupported there.
            Ok(None) if save_dir.as_os_str().is_empty() => None,
            Ok(None) => match std::fs::read(&save_path) {
                Ok(bytes) => Some(bytes),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => {
                    return Err(CoreError::content(format!(
                        "could not read {}: {e}",
                        save_path.display()
                    )))
                }
            },
            Err(e) => {
                return Err(CoreError::content(format!(
                    "could not read save {}: {e}",
                    self.save_name
                )))
            }
        };
        let (emu, gpu_output) = self.build(saved)?;
        self.emu = Some(emu);
        self.gpu_output = gpu_output;
        self.context = Some(context);
        self.context
            .as_mut()
            .unwrap()
            .events
            .emit(CoreEvent::Loaded(CoreLoadInfo {
                width: 256,
                height: 384,
                sample_rate: self.sample_rate,
                audio_channels: 2,
            }));
        Ok(())
    }
    fn run(&mut self) -> CoreResult<ExitStatus> {
        let control = self
            .context
            .as_ref()
            .ok_or_else(|| CoreError::runtime("DS core was not loaded"))?
            .control
            .clone();
        let mut commands = Vec::with_capacity(16);
        let mut frames = 0u64;
        let mut deadline = 0.0;
        let period = 560190.0 / 33513982.0;
        let mut audio_paused = false;
        while !control.should_stop() && control.frame_limit().is_none_or(|limit| frames < limit) {
            control.take_requests(&mut commands);
            for request in commands.drain(..) {
                let id = request.id;
                let generation = request.generation;
                let outcome = match self.command(request.command) {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        self.context
                            .as_mut()
                            .unwrap()
                            .events
                            .emit(CoreEvent::Error(error.to_string()));
                        CommandOutcome::Failed(error.to_string())
                    }
                };
                self.report(generation, id, outcome);
            }
            if control.paused() {
                if !audio_paused {
                    if let Some(context) = self.context.as_mut() {
                        context.audio.pause(true);
                    }
                    audio_paused = true;
                }
                self.context
                    .as_ref()
                    .unwrap()
                    .platform
                    .sleep(Duration::from_millis(2));
                deadline = 0.0;
                continue;
            }
            if audio_paused {
                if let Some(context) = self.context.as_mut() {
                    context.audio.pause(false);
                }
                audio_paused = false;
            }
            let emu = self.emu.as_mut().unwrap();
            let input = control.input().snapshot();
            let mut keys = Keys::empty();
            for (button, key) in [
                (Button::South, Keys::A),
                (Button::East, Keys::B),
                (Button::North, Keys::X),
                (Button::West, Keys::Y),
                (Button::Start, Keys::START),
                (Button::Select, Keys::SELECT),
                (Button::Up, Keys::UP),
                (Button::Down, Keys::DOWN),
                (Button::Left, Keys::LEFT),
                (Button::Right, Keys::RIGHT),
                (Button::LeftShoulder, Keys::L),
                (Button::RightShoulder, Keys::R),
            ] {
                if input.buttons.contains(button) {
                    keys |= key;
                }
            }
            emu.release_keys(Keys::all() & !keys);
            emu.press_keys(keys);
            if input.buttons.contains(Button::Extra0)
                && input.pointer_inside
                && (0..256).contains(&input.pointer_x)
                && (192..384).contains(&input.pointer_y)
            {
                emu.set_touch_pos([
                    (input.pointer_x * 16) as u16,
                    ((input.pointer_y - 192) * 16) as u16,
                ]);
            } else {
                emu.end_touch();
            }
            if emu.run() == RunOutput::Shutdown {
                break;
            }
            if let Some(gpu_output) = &mut self.gpu_output {
                let pixels = gpu_output
                    .next_rgba()
                    .ok_or_else(|| CoreError::runtime("Dust wgpu frame readback failed"))?;
                if pixels.len() != self.video.len() {
                    return Err(CoreError::runtime(
                        "Dust wgpu returned an invalid frame size",
                    ));
                }
                self.video.copy_from_slice(&pixels);
            } else {
                for (pixel, out) in emu
                    .gpu
                    .renderer_2d()
                    .framebuffer()
                    .iter()
                    .flatten()
                    .zip(self.video.chunks_exact_mut(4))
                {
                    out.copy_from_slice(&pixel.to_le_bytes());
                }
            }
            let context = self.context.as_mut().unwrap();
            context
                .video
                .present(VideoFrame::rgba8(256, 384, 1024, &self.video));
            {
                let mut samples = self.samples.borrow_mut();
                if !samples.is_empty() {
                    context.audio.queue_pcm(PcmBuffer {
                        sample_rate: self.sample_rate,
                        channels: 2,
                        samples: &samples,
                    });
                    samples.clear();
                }
            }
            frames += 1;
            if std::env::var_os("RSXP_NDS_TRACE").is_some() && frames % 60 == 0 {
                eprintln!("frame {frames}: arm9={:?} arm7={:?}", self.emu.as_ref().unwrap().arm9.regs(), self.emu.as_ref().unwrap().arm7.regs());
            }
            if control.frame_limit().is_some_and(|limit| frames >= limit) {
                break;
            }
            if frames % 300 == 0 {
                self.flush_save(false)?;
            }
            if control.fast_forward() {
                deadline = 0.0;
                continue;
            }
            let platform = &self.context.as_ref().unwrap().platform;
            let now = platform.now();
            if now <= 0.0 {
                continue;
            }
            if deadline == 0.0 || deadline < now - period {
                deadline = now;
            }
            deadline += period;
            if deadline > now {
                platform.sleep(Duration::from_secs_f64(deadline - now));
            }
        }
        self.flush_save(true)?;
        Ok(ExitStatus::success())
    }
}

#[cfg(test)]
mod tests;
