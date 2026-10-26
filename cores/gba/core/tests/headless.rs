// SPDX-License-Identifier: GPL-2.0-or-later
//! Headless contract test for the GBA adapter using a hand-written test ROM.
//!
//! The ROM switches to mode 3, fills pixel (0, 0) with red and then spins.

use fanplay_core::audio::NullAudioSink;
use fanplay_core::{
    Content, CoreContext, CoreControl, CoreEvent, CoreLoadInfo, EventSink, PixelFormat, Platform,
    VideoSink,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "fanplay-gba-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct TestPlatform(PathBuf);

impl Platform for TestPlatform {
    fn read_file(&self, path: &str) -> Option<Vec<u8>> {
        std::fs::read(self.0.join(path)).ok()
    }
    fn save_dir(&self) -> PathBuf {
        self.0.clone()
    }
    fn now(&self) -> f64 {
        0.0
    }
    fn sleep(&self, _duration: Duration) {}
}

#[derive(Default)]
struct FrameRecorder {
    pixel: Mutex<Option<(PixelFormat, u8, u8, u8)>>,
    frames: Mutex<u32>,
}

impl VideoSink for FrameRecorder {
    fn present(&mut self, frame: fanplay_core::VideoFrame<'_>) {
        if let Some(px) = frame.pixels.get(0..3) {
            *self.pixel.lock().unwrap() = Some((frame.format, px[0], px[1], px[2]));
        }
        *self.frames.lock().unwrap() += 1;
    }
}

struct SharedVideo(Arc<FrameRecorder>);

impl VideoSink for SharedVideo {
    fn present(&mut self, frame: fanplay_core::VideoFrame<'_>) {
        if let Some(px) = frame.pixels.get(0..3) {
            *self.0.pixel.lock().unwrap() = Some((frame.format, px[0], px[1], px[2]));
        }
        *self.0.frames.lock().unwrap() += 1;
    }
}

struct Events {
    loaded: Arc<Mutex<Option<CoreLoadInfo>>>,
    saved_state: Arc<AtomicBool>,
}

impl EventSink for Events {
    fn emit(&mut self, event: CoreEvent) {
        match event {
            CoreEvent::Loaded(info) => *self.loaded.lock().unwrap() = Some(info),
            CoreEvent::SaveState { data, .. } => {
                assert!(!data.is_empty());
                self.saved_state.store(true, Ordering::Relaxed);
            }
            _ => {}
        }
    }
}

fn pixel_rom() -> Vec<u8> {
    let program: [u32; 8] = [
        0xE3A0_0003, // MOV r0, #3 (mode 3)
        0xE380_0B01, // ORR r0, r0, #0x400 (BG2 enable)
        0xE3A0_1301, // MOV r1, #0x04000000
        0xE581_0000, // STR r0, [r1] (DISPCNT)
        0xE3A0_2406, // MOV r2, #0x06000000
        0xE3A0_301F, // MOV r3, #0x1F (red)
        0xE1C2_30B0, // STRH r3, [r2] (pixel 0,0)
        0xEAFF_FFFE, // b $
    ];
    let mut rom = vec![0u8; 0x200];
    for (index, instruction) in program.iter().enumerate() {
        rom[index * 4..index * 4 + 4].copy_from_slice(&instruction.to_le_bytes());
    }
    rom
}

#[test]
fn gba_core_loads_presents_saves_state_and_stops() {
    check_pixel_rom(pixel_rom());
}

#[test]
fn gba_core_boots_valid_header_and_resets() {
    let mut rom = pixel_rom();
    rom.copy_within(0..32, 0xC0);
    rom[..4].copy_from_slice(&0xEA00_002Eu32.to_le_bytes());
    rom[4..0xC0].fill(0);
    rom[0xB2] = 0x96;
    rom[0xBD] = rom[0xA0..=0xBC]
        .iter()
        .fold(0u8, |sum, byte| sum.wrapping_sub(*byte))
        .wrapping_sub(0x19);
    check_pixel_rom(rom);
}

fn check_pixel_rom(rom: Vec<u8>) {
    let dir = TempDir::new("smoke");
    let rom_path = dir.0.join("pixel.gba");
    std::fs::write(&rom_path, rom).unwrap();

    let frames = Arc::new(FrameRecorder::default());
    let loaded = Arc::new(Mutex::new(None));
    let saved_state = Arc::new(AtomicBool::new(false));
    let control = Arc::new(CoreControl::new());
    control.set_frame_limit(Some(3));
    control.send(fanplay_core::CoreCommand::Reset);
    control.send(fanplay_core::CoreCommand::SaveState { slot: "1".into() });

    let context = CoreContext::new(
        Arc::new(TestPlatform(dir.0.clone())),
        Box::new(SharedVideo(Arc::clone(&frames))),
        Box::new(NullAudioSink),
        Box::new(Events {
            loaded: Arc::clone(&loaded),
            saved_state: Arc::clone(&saved_state),
        }),
        Arc::clone(&control),
    );

    let mut core = fanplay_core_gba::create();
    core.load(Content::file(&rom_path), context).unwrap();
    let info = loaded.lock().unwrap().unwrap();
    assert_eq!((info.width, info.height), (240, 160));
    assert!(core.run().unwrap().is_success());

    assert_eq!(*frames.frames.lock().unwrap(), 3);
    let (format, first, second, third) = frames.pixel.lock().unwrap().unwrap();
    #[cfg(target_endian = "little")]
    {
        assert_eq!(format, PixelFormat::Bgrx8);
        // Bgrx8 bytes for red: blue=0, green=0, red is 5-bit expanded.
        assert_eq!((first, second), (0, 0));
        assert!(third >= 240);
    }
    #[cfg(target_endian = "big")]
    {
        assert_eq!(format, PixelFormat::Rgba8);
        assert_eq!((first, second, third), (255, 0, 0));
    }
    assert!(saved_state.load(Ordering::Relaxed));
}
