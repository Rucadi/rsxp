// SPDX-License-Identifier: GPL-2.0-or-later
//! Platform services the engine needs, so the same engine runs under winit on
//! desktop and under WebAssembly in the browser.
//!
//! [`Host`] is the object-safe backend trait: loose-file reads, image/font
//! decoding and text, input polling, presentation, window state and optional
//! audio/performance/GPU hooks, most with no-op defaults for headless or WASM.
//! [`AudioBackend`] is reached through `Host::audio` and plays the BGM/BGS/ME/SE
//! channels. [`InputEvent`] carries browser-side events into the runtime, and
//! `Host` exposes FPS display visibility hooks for the optional performance panel.
use crate::graphics::{Bitmap, Color};
use crate::input::Input;
use std::time::Duration;

/// An input event queued from a host (used by the browser frontend).
#[derive(Clone, Debug)]
pub enum InputEvent {
    KeyDown(i32),
    KeyUp(i32),
    Text(String),
    Mouse { x: i32, y: i32, inside: bool },
    Scroll(i32),
}

/// A logical audio stream. BGM is a single looping music stream; BGS and ME use
/// reserved channels; SEs share the remaining channels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioChannel {
    Bgm,
    Bgs,
    Me,
    Se,
}

/// Audio backend. The desktop frontend uses Kira/CPAL; the browser frontend
/// uses the browser media and Web Audio APIs.
pub trait AudioBackend {
    /// Starts playback. `volume` is 0..100, `pitch` 100 is normal, and
    /// `position` is the start offset in seconds.
    fn play(
        &mut self,
        channel: AudioChannel,
        path: &str,
        bytes: &[u8],
        volume: i32,
        pitch: f64,
        position: f64,
    );
    fn stop(&mut self, channel: AudioChannel);
    fn fade(&mut self, channel: AudioChannel, ms: i32);
    /// Playback position in seconds.
    fn position(&self, channel: AudioChannel) -> f64;
    fn set_volume(&mut self, channel: AudioChannel, volume: i32);
    fn volume(&self, channel: AudioChannel) -> i32;
}

/// Platform backend. Implemented by the winit desktop frontend and the wasm
/// browser frontend.
pub trait Host {
    /// Audio backend, or `None` where the platform cannot play sound.
    fn audio(&mut self) -> Option<&mut dyn AudioBackend> {
        None
    }
    /// Whether the host renders the FPS/performance panel itself (e.g. via egui).
    fn has_performance_ui(&self) -> bool {
        false
    }
    /// Whether the FPS/performance panel is currently visible in the host UI.
    fn performance_enabled(&self) -> Option<bool> {
        None
    }
    /// Optional native compositor. Software-only and browser hosts return None.
    #[cfg(feature = "gpu")]
    fn gpu(&mut self) -> Option<&mut crate::gpu::GpuRenderer> {
        None
    }
    #[cfg(feature = "gpu")]
    fn present_gpu(&mut self) -> Result<(), String> {
        Ok(())
    }
    /// Read a loose game file (case-insensitive on platforms that need it).
    /// Returns `None` when the file does not exist.
    fn read_file(&self, path: &str) -> Option<Vec<u8>>;
    /// Decode an image from its raw bytes.
    fn image(&self, bytes: &[u8]) -> Result<Bitmap, String>;
    /// Measure (render == false) or rasterize (render == true) a line of text.
    /// `font` is the raw TrueType data, or `None` to use the built-in font.
    #[allow(clippy::too_many_arguments)]
    fn text(
        &self,
        font: Option<&[u8]>,
        size: i32,
        bold: bool,
        italic: bool,
        text: &str,
        color: Color,
        render: bool,
    ) -> Result<(u32, u32, Option<Bitmap>), String>;
    /// Poll interactive input into `input`. Return false to request shutdown.
    /// Only called when the runtime is not headless.
    fn poll(&mut self, _input: &mut Input) -> bool {
        true
    }
    /// Present a finished frame. Only called when the runtime is not headless.
    fn present(&mut self, _frame: &Bitmap) -> Result<(), String> {
        Ok(())
    }
    /// Resize the presentation surface.
    fn resize(&mut self, _width: u32, _height: u32) -> Result<(), String> {
        Ok(())
    }
    fn set_title(&mut self, _title: &str) {}
    fn set_fullscreen(&mut self, _enabled: bool) -> Result<(), String> {
        Ok(())
    }
    fn set_scale(&mut self, _scale: f64) {}
    fn center(&mut self) {}
    fn show_cursor(&mut self, _shown: bool) {}
    fn text_input(&mut self, _enabled: bool) {}
    /// Battery/power state as `(seconds, percent, discharging)`; -1 means
    /// unknown.
    fn power_state(&self) -> (i32, i32, bool) {
        (-1, -1, false)
    }
    /// Pause for one frame of pacing. Default is a no-op so the browser build
    /// (where sleeping is unavailable) does not block.
    fn sleep(&self, _duration: Duration) {}
    /// Monotonic seconds since the host started, or 0.0 if unavailable. Used
    /// for the on-screen frame counter.
    fn now(&self) -> f64 {
        0.0
    }
}
