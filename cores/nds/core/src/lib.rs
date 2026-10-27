// SPDX-License-Identifier: GPL-3.0-or-later
//! Nintendo DS adapter using Dust's interpreter and GPU renderers on desktop.
mod cheats;
mod pacing;
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
use fanplay_core::{
    AudioInfo, Button, CommandOutcome, CommandRequest, Content, ContentKind, ContentProbe, Core,
    CoreCapabilities, CoreCommand, CoreContext, CoreError, CoreEvent, CoreInfo, CoreLoadInfo,
    CoreRegistration, CoreResult, ExitStatus, InputInfo, PcmBuffer, PixelFormat, PlatformError,
    PresentationSupport, ScreenInfo, ScreenRotation, VideoFrame,
};
use serde_json::{json, Value};
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
        .union(CoreCapabilities::CHEATS)
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
    cheats: cheats::ActionReplay,
    frames: u64,
    commands: Vec<CommandRequest>,
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
                        "encrypted DS ROM needs your BIOS dump (FANPLAY_NDS_BIOS7)"
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
    fn report(
        &mut self,
        generation: u64,
        id: u64,
        outcome: CommandOutcome,
        response: Option<Vec<u8>>,
    ) {
        if let Some(context) = self.context.as_mut() {
            context.events.emit(CoreEvent::CommandResult {
                id,
                generation,
                outcome,
                response,
            });
        }
    }
    fn flush_audio(&mut self) {
        self.samples.borrow_mut().clear();
        if let Some(context) = self.context.as_mut() {
            context.audio.reset_pcm();
        }
    }
    fn command(&mut self, command: CoreCommand) -> CoreResult<(CommandOutcome, Option<Vec<u8>>)> {
        Ok(match command {
            CoreCommand::Reset => {
                self.flush_save(false)?;
                let saved = self.emu.as_ref().unwrap().ds_slot.spi.contents().to_vec();
                let (emu, gpu_output) = self.build(Some(saved))?;
                self.emu = Some(emu);
                self.gpu_output = gpu_output;
                self.frames = 0;
                self.flush_audio();
                (CommandOutcome::Succeeded, None)
            }
            CoreCommand::SaveRam => {
                self.flush_save(true)?;
                (CommandOutcome::Succeeded, None)
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
                (CommandOutcome::Succeeded, None)
            }
            CoreCommand::SaveState { slot } => {
                let data = self.save_state()?;
                self.context
                    .as_mut()
                    .unwrap()
                    .events
                    .emit(CoreEvent::SaveState { slot, data });
                (CommandOutcome::Succeeded, None)
            }
            CoreCommand::LoadState { data, .. } => {
                self.load_state(&data)?;
                self.flush_audio();
                (CommandOutcome::Succeeded, None)
            }
            CoreCommand::SetOption { .. } => (CommandOutcome::Unsupported, None),
            CoreCommand::Extension { id, payload } => self.extension_command(&id, &payload),
        })
    }

    fn extension_command(&mut self, id: &str, payload: &[u8]) -> (CommandOutcome, Option<Vec<u8>>) {
        if id != "nds.rhai" {
            return (CommandOutcome::Unsupported, None);
        }
        let request = serde_json::from_slice::<Value>(payload);
        let request_id = request
            .as_ref()
            .ok()
            .and_then(|request| request.get("request_id"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let generation = request
            .as_ref()
            .ok()
            .and_then(|request| request.get("generation"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let response = match request {
            Ok(request) => {
                let operation = request
                    .get("operation")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let arguments = request.get("arguments").unwrap_or(&Value::Null);
                match self.handle_extension_operation(operation, arguments) {
                    Ok(result) => {
                        json!({"request_id": request_id, "generation": generation, "result": result})
                    }
                    Err(error) => {
                        json!({"request_id": request_id, "generation": generation, "error": {"code": "nds", "message": error}})
                    }
                }
            }
            Err(error) => {
                json!({"request_id": request_id, "generation": generation, "error": {"code": "invalid_request", "message": error.to_string()}})
            }
        };
        let body = serde_json::to_vec(&response).unwrap_or_else(|_| b"{}".to_vec());
        (CommandOutcome::Succeeded, Some(body))
    }

    fn handle_extension_operation(
        &mut self,
        operation: &str,
        arguments: &Value,
    ) -> Result<Value, String> {
        match operation {
            "read" => {
                let (space, address, width) = memory_arguments(arguments)?;
                Ok(json!({
                    "space": space,
                    "address": address,
                    "width": width,
                    "value": self.read_space(space, address, width)?,
                }))
            }
            "read_bytes" => {
                let space = arguments
                    .get("space")
                    .and_then(Value::as_str)
                    .ok_or("read_bytes is missing space")?;
                let address = number_u32(arguments, "address")?;
                let length = arguments
                    .get("length")
                    .and_then(Value::as_u64)
                    .ok_or("read_bytes is missing length")?;
                let length =
                    usize::try_from(length).map_err(|_| "read_bytes length is too large")?;
                if length > 1_048_576 {
                    return Err("read_bytes length exceeds 1 MiB".into());
                }
                Ok(json!({
                    "space": space,
                    "address": address,
                    "bytes": self.read_bytes(space, address, length)?,
                }))
            }
            "write" => {
                let (space, address, width) = memory_arguments(arguments)?;
                let value = arguments
                    .get("value")
                    .and_then(Value::as_u64)
                    .ok_or("write is missing value")?;
                self.write_space(space, address, width, value)?;
                Ok(json!({"written": true, "space": space, "address": address, "width": width}))
            }
            "cheat_add" => {
                let code = arguments
                    .get("code")
                    .and_then(Value::as_str)
                    .ok_or("cheat_add is missing code")?;
                let lines = self.cheats.add(code)?;
                Ok(json!({"added": true, "lines": lines}))
            }
            "cheat_remove" => {
                let code = arguments
                    .get("code")
                    .and_then(Value::as_str)
                    .ok_or("cheat_remove is missing code")?;
                let removed = self.cheats.remove(code)?;
                Ok(json!({"removed": removed}))
            }
            "cheat_clear" => {
                self.cheats.clear();
                Ok(json!({"cleared": true}))
            }
            _ => Err(format!("unknown NDS extension operation {operation:?}")),
        }
    }

    fn read_space(&mut self, space: &str, address: u32, width: u32) -> Result<u32, String> {
        match space {
            "ram" | "memory" => self
                .emu
                .as_mut()
                .map(|emu| cheats::read_memory(emu, address, width))
                .ok_or_else(|| "NDS emulator is not running".into()),
            "cartridge" | "rom" => read_blob(&self.rom, address, width),
            "save" | "save_ram" => self
                .emu
                .as_ref()
                .map(|emu| read_blob(emu.ds_slot.spi.contents(), address, width))
                .unwrap_or_else(|| Err("NDS emulator is not running".into())),
            _ => Err(format!("unknown NDS memory space {space:?}")),
        }
    }

    fn read_bytes(&mut self, space: &str, address: u32, length: usize) -> Result<Vec<u8>, String> {
        match space {
            "cartridge" | "rom" => read_blob_bytes(&self.rom, address, length),
            "save" | "save_ram" => self
                .emu
                .as_ref()
                .map(|emu| read_blob_bytes(emu.ds_slot.spi.contents(), address, length))
                .unwrap_or_else(|| Err("NDS emulator is not running".into())),
            "ram" | "memory" => self
                .emu
                .as_mut()
                .map(|emu| cheats::read_memory_bytes(emu, address, length))
                .ok_or_else(|| "NDS emulator is not running".into()),
            _ => Err(format!("unknown NDS memory space {space:?}")),
        }
    }

    fn write_space(
        &mut self,
        space: &str,
        address: u32,
        width: u32,
        value: u64,
    ) -> Result<(), String> {
        if width != 1 && width != 2 && width != 4 {
            return Err("memory width must be 1, 2, or 4".into());
        }
        let max = (1_u64 << (width * 8)) - 1;
        if value > max {
            return Err(format!("value does not fit in {width}-byte write"));
        }
        match space {
            "ram" | "memory" => self
                .emu
                .as_mut()
                .map(|emu| cheats::write_memory(emu, address, width, value as u32))
                .ok_or_else(|| "NDS emulator is not running".into()),
            "save" | "save_ram" => {
                let emu = self
                    .emu
                    .as_mut()
                    .ok_or_else(|| "NDS emulator is not running".to_owned())?;
                write_blob(emu.ds_slot.spi.contents_mut(), address, width, value as u32)?;
                emu.ds_slot.spi.mark_contents_dirty();
                Ok(())
            }
            "cartridge" | "rom" => Err("NDS cartridge ROM is read-only".into()),
            _ => Err(format!("unknown NDS memory space {space:?}")),
        }
    }
}

fn number_u32(arguments: &Value, key: &str) -> Result<u32, String> {
    arguments
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| format!("{key} is missing or outside the u32 range"))
}

fn memory_arguments(arguments: &Value) -> Result<(&str, u32, u32), String> {
    let space = arguments
        .get("space")
        .and_then(Value::as_str)
        .ok_or("memory operation is missing space")?;
    let address = number_u32(arguments, "address")?;
    let width = arguments
        .get("width")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or("memory operation is missing width")?;
    if !matches!(width, 1 | 2 | 4) {
        return Err("memory width must be 1, 2, or 4".into());
    }
    Ok((space, address, width))
}

fn read_blob(bytes: &[u8], address: u32, width: u32) -> Result<u32, String> {
    let data = read_blob_bytes(bytes, address, width as usize)?;
    Ok(match width {
        1 => data[0] as u32,
        2 => u16::from_le_bytes([data[0], data[1]]) as u32,
        4 => u32::from_le_bytes([data[0], data[1], data[2], data[3]]),
        _ => unreachable!(),
    })
}

fn read_blob_bytes(bytes: &[u8], address: u32, length: usize) -> Result<Vec<u8>, String> {
    let start = usize::try_from(address).map_err(|_| "address is outside the host range")?;
    let end = start.checked_add(length).ok_or("memory range overflows")?;
    bytes
        .get(start..end)
        .map(ToOwned::to_owned)
        .ok_or_else(|| "memory range is outside the selected space".into())
}

fn write_blob(bytes: &mut [u8], address: u32, width: u32, value: u32) -> Result<(), String> {
    let start = usize::try_from(address).map_err(|_| "address is outside the host range")?;
    let end = start
        .checked_add(width as usize)
        .ok_or("memory range overflows")?;
    let target = bytes
        .get_mut(start..end)
        .ok_or_else(|| "memory range is outside the selected space".to_owned())?;
    target.copy_from_slice(&value.to_le_bytes()[..width as usize]);
    Ok(())
}

impl NdsAdapter {
    fn apply_commands(&mut self, commands: &mut Vec<CommandRequest>) {
        let Some(control) = self.context.as_ref().map(|context| context.control.clone()) else {
            return;
        };
        control.take_requests(commands);
        for request in commands.drain(..) {
            let id = request.id;
            let generation = request.generation;
            let (outcome, response) = match self.command(request.command) {
                Ok(result) => result,
                Err(error) => {
                    if let Some(context) = self.context.as_mut() {
                        context.events.emit(CoreEvent::Error(error.to_string()));
                    }
                    (CommandOutcome::Failed(error.to_string()), None)
                }
            };
            self.report(generation, id, outcome, response);
        }
    }

    /// Advances the DS machine by one emulated frame and publishes its CPU
    /// presentation. The caller owns pacing and pause/stop policy so native
    /// loops and browser repaint loops can share the same frame path.
    fn run_frame(&mut self) -> CoreResult<bool> {
        let control = self
            .context
            .as_ref()
            .ok_or_else(|| CoreError::runtime("DS core was not loaded"))?
            .control
            .clone();
        let emu = self
            .emu
            .as_mut()
            .ok_or_else(|| CoreError::runtime("DS emulator was not initialized"))?;
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
        self.cheats.apply(emu);
        if emu.run() == RunOutput::Shutdown {
            return Ok(true);
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
            self.video = pixels;
        } else {
            let framebuffer = emu.gpu.renderer_2d().framebuffer();
            #[cfg(target_endian = "little")]
            {
                let source = unsafe {
                    std::slice::from_raw_parts(framebuffer.as_ptr().cast::<u8>(), self.video.len())
                };
                self.video.copy_from_slice(source);
            }
            #[cfg(not(target_endian = "little"))]
            {
                for (pixel, out) in framebuffer
                    .iter()
                    .flatten()
                    .zip(self.video.chunks_exact_mut(4))
                {
                    out.copy_from_slice(&pixel.to_le_bytes());
                }
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
        self.frames += 1;
        if std::env::var_os("FANPLAY_NDS_TRACE").is_some() && self.frames % 60 == 0 {
            eprintln!(
                "frame {}: arm9={:?} arm7={:?}",
                self.frames,
                self.emu.as_ref().unwrap().arm9.regs(),
                self.emu.as_ref().unwrap().arm7.regs()
            );
        }
        if self.frames % 300 == 0 {
            self.flush_save(false)?;
        }
        Ok(false)
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
            system_file(&content, &context, "FANPLAY_NDS_BIOS7", "bios7.bin")?,
            "bios7.bin",
        )?;
        self.bios9 = bios(
            system_file(&content, &context, "FANPLAY_NDS_BIOS9", "bios9.bin")?,
            "bios9.bin",
        )?;
        self.firmware = system_file(&content, &context, "FANPLAY_NDS_FIRMWARE", "firmware.bin")?
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
            std::env::var("FANPLAY_NDS_AUDIO_RATE")
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
            // (for example an in-memory store). Never probe `std::fs`,
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
        self.frames = 0;
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
        let mut frames = 0u64;
        let mut pacer = pacing::FramePacer::default();
        let mut audio_paused = false;
        while !control.should_stop() && control.frame_limit().is_none_or(|limit| frames < limit) {
            let mut commands = std::mem::take(&mut self.commands);
            self.apply_commands(&mut commands);
            self.commands = commands;
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
                pacer.reset();
                continue;
            }
            if audio_paused {
                if let Some(context) = self.context.as_mut() {
                    context.audio.pause(false);
                }
                audio_paused = false;
            }
            pacer.begin_frame(self.context.as_ref().unwrap().platform.now());
            if self.run_frame()? {
                break;
            }
            frames += 1;
            if control.frame_limit().is_some_and(|limit| frames >= limit) {
                break;
            }
            if control.fast_forward() {
                pacer.reset();
                continue;
            }
            let period = control
                .target_fps()
                .map_or(560190.0 / 33513982.0, |fps| 1.0 / f64::from(fps));
            let platform = &self.context.as_ref().unwrap().platform;
            let now = platform.now();
            if now <= 0.0 {
                continue;
            }
            let sleep = pacer.finish_frame(now, period);
            if !sleep.is_zero() {
                platform.sleep(sleep);
            }
        }
        self.flush_save(true)?;
        Ok(ExitStatus::success())
    }

    fn run_slice(&mut self) -> CoreResult<fanplay_core::RunSliceStatus> {
        let control = self
            .context
            .as_ref()
            .ok_or_else(|| CoreError::runtime("DS core was not loaded"))?
            .control
            .clone();
        if control.should_stop() {
            self.flush_save(true)?;
            return Ok(fanplay_core::RunSliceStatus::Exited(ExitStatus::success()));
        }
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
        if self.run_frame()? {
            self.flush_save(true)?;
            return Ok(fanplay_core::RunSliceStatus::Exited(ExitStatus::success()));
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
mod tests;
