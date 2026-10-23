// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use dust_core::utils::mem_prelude::*;
use rsxp_core::{AudioSink, CoreControl, EventSink, Platform, VideoSink};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
struct PlatformFs(PathBuf);
impl Platform for PlatformFs {
    fn read_file(&self, name: &str) -> Option<Vec<u8>> {
        std::fs::read(self.0.join(name)).ok()
    }
    fn write_file(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        std::fs::write(self.0.join(name), bytes).map_err(|e| e.to_string())
    }
    fn save_dir(&self) -> PathBuf {
        self.0.clone()
    }
}
#[derive(Default)]
struct Output {
    frames: Vec<u64>,
    audio: usize,
    states: Vec<Vec<u8>>,
    errors: Vec<String>,
}
struct Sink(Arc<Mutex<Output>>);
impl VideoSink for Sink {
    fn present(&mut self, frame: VideoFrame<'_>) {
        assert_eq!((frame.width, frame.height, frame.stride), (256, 384, 1024));
        assert_eq!(frame.format, PixelFormat::Rgba8);
        let hash = frame
            .pixels
            .iter()
            .fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(*b as u64));
        self.0.lock().unwrap().frames.push(hash);
    }
}
impl AudioSink for Sink {
    fn queue_pcm(&mut self, pcm: PcmBuffer<'_>) {
        assert_eq!((pcm.sample_rate, pcm.channels), (48000, 2));
        self.0.lock().unwrap().audio += pcm.samples.len();
    }
}
impl EventSink for Sink {
    fn emit(&mut self, event: CoreEvent) {
        match event {
            CoreEvent::SaveState { data, .. } => self.0.lock().unwrap().states.push(data),
            CoreEvent::Error(error) => self.0.lock().unwrap().errors.push(error),
            _ => {}
        }
    }
}
fn rom() -> Vec<u8> {
    let mut bytes = vec![0; 0x10000];
    // Both CPUs store a marker in RAM, then loop. No Nintendo code or BIOS.
    for (base, offset, ram) in [(0x20, 0x200u32, 0x02000000u32), (0x30, 0x300, 0x02000200)] {
        for (index, word) in [offset, ram, ram, 24].iter().enumerate() {
            bytes[base + index * 4..base + index * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        let code: [u32; 6] = [
            0xE59F000C,
            0xE3A01042,
            0xE5801000,
            0xEAFFFFFE,
            0,
            ram + 0x1000,
        ];
        for (i, word) in code.iter().enumerate() {
            bytes[offset as usize + i * 4..offset as usize + i * 4 + 4]
                .copy_from_slice(&word.to_le_bytes());
        }
    }
    bytes
}
struct Fixture {
    path: PathBuf,
    core: NdsAdapter,
    control: Arc<CoreControl>,
    output: Arc<Mutex<Output>>,
}
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "rsxp-nds-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("test.nds"), rom()).unwrap();
        // An existing 512-byte EEPROM also tests save-type inference for homebrew.
        std::fs::write(path.join("test.sav"), vec![0xFF; 512]).unwrap();
        let control = Arc::new(CoreControl::new());
        control.set_fast_forward(true);
        control.set_frame_limit(Some(3));
        let output = Arc::new(Mutex::new(Output::default()));
        let context = CoreContext::new(
            Arc::new(PlatformFs(path.clone())),
            Box::new(Sink(output.clone())),
            Box::new(Sink(output.clone())),
            Box::new(Sink(output.clone())),
            control.clone(),
        );
        let mut core = NdsAdapter::default();
        core.load(Content::file(path.join("test.nds")), context)
            .unwrap();
        Self {
            path,
            core,
            control,
            output,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
#[test]
fn boots_both_cpus_renders_audio_and_round_trips_state_and_save() {
    let mut f = Fixture::new();
    f.control.input().set_button(Button::North, true);
    f.control.input().set_button(Button::Extra0, true);
    f.control.input().set_pointer(100, 250, true);
    f.core.run().unwrap();
    let emu = f.core.emu.as_ref().unwrap();
    assert_eq!(emu.main_mem().read_le::<u32>(0x1000), 0x42);
    assert_eq!(emu.main_mem().read_le::<u32>(0x1200), 0x42);
    assert!(!emu.input.status().x());
    assert!(!emu.input.status().pen_down());
    let out = f.output.lock().unwrap();
    assert_eq!(out.frames.len(), 3);
    assert!(out.audio > 0);
    assert!(out.errors.is_empty());
    drop(out);
    f.core.emu.as_mut().unwrap().ds_slot.spi.contents_mut()[0] = 0x23;
    let state = f.core.save_state().unwrap();
    f.core.emu.as_mut().unwrap().ds_slot.spi.contents_mut()[0] = 0x45;
    f.core.load_state(&state).unwrap();
    assert_eq!(f.core.emu.as_ref().unwrap().ds_slot.spi.contents()[0], 0x23);
    let mut corrupt = state.clone();
    corrupt[30] ^= 1;
    assert!(f.core.load_state(&corrupt).is_err());
    assert!(f.core.load_state(b"other-core-state").is_err());
    assert_eq!(f.core.emu.as_ref().unwrap().ds_slot.spi.contents()[0], 0x23);
    f.core.command(CoreCommand::Reset).unwrap();
    assert_eq!(f.core.emu.as_ref().unwrap().ds_slot.spi.contents()[0], 0x23);
    f.core.run().unwrap();
    assert_eq!(std::fs::read(f.path.join("test.sav")).unwrap()[0], 0x23);
}
#[test]
fn frames_are_deterministic_and_touch_releases_outside_lower_screen() {
    let mut a = Fixture::new();
    let mut b = Fixture::new();
    a.control.input().set_button(Button::Extra0, true);
    a.control.input().set_pointer(100, 50, true);
    a.core.run().unwrap();
    b.core.run().unwrap();
    assert!(a.core.emu.as_ref().unwrap().input.status().pen_down());
    assert_eq!(
        a.output.lock().unwrap().frames,
        b.output.lock().unwrap().frames
    );
}
#[test]
fn rejects_bad_headers_and_bios_sizes() {
    assert!(validate_rom(&[]).is_err());
    assert!(validate_rom(&vec![0; 512]).is_err());
    let mut bytes = rom();
    bytes[0x2c..0x30].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(validate_rom(&bytes).is_err());
    assert!(bios::<16384>(Some(vec![0; 5]), "bios7").is_err());
}
