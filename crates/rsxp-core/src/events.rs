// SPDX-License-Identifier: GPL-2.0-or-later
//! Low-frequency session events and presentation/window requests.

use crate::control::CommandOutcome;
use crate::video::PixelFormat;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

/// Initial format reported after content loads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoreLoadInfo {
    pub width: u32,
    pub height: u32,
    /// Native audio rate, or `0` when the core produces no audio.
    pub sample_rate: u32,
    pub audio_channels: u16,
}

/// Window state requested by the core. Frontends without a window ignore these.
#[derive(Clone, Debug, PartialEq)]
pub enum WindowCommand {
    SetTitle(String),
    SetFullscreen(bool),
    Resize { width: u32, height: u32 },
    SetScale(f64),
    Center,
    ShowCursor(bool),
    TextInput(bool),
}

/// Final process/game status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExitStatus {
    pub code: i32,
}

impl ExitStatus {
    pub const fn new(code: i32) -> Self {
        Self { code }
    }
    pub const fn success() -> Self {
        Self { code: 0 }
    }
    pub const fn is_success(self) -> bool {
        self.code == 0
    }
}

/// Session event emitted by a core. These are cold-path messages; none of them
/// occur per pixel, per sample or per CPU instruction.
#[derive(Debug)]
pub enum CoreEvent {
    Loaded(CoreLoadInfo),
    Window(WindowCommand),
    Log(LogLevel, String),
    /// Serialized save state for the frontend's storage.
    SaveState {
        slot: String,
        data: Vec<u8>,
    },
    /// Battery-backed save data.
    SaveRam {
        data: Vec<u8>,
    },
    /// A screenshot, already converted to tightly packed RGBA8.
    Screenshot {
        width: u32,
        height: u32,
        format: PixelFormat,
        pixels: Vec<u8>,
    },
    /// Human-readable progress/status for the workspace.
    Status(String),
    /// A non-fatal error the user should see without ending the session.
    Error(String),
    /// Result of a request queued with
    /// [`CoreControl::send_request`](crate::CoreControl::send_request).
    /// Cores must report every request they do not handle as
    /// [`CommandOutcome::Unsupported`].
    CommandResult {
        id: u64,
        outcome: CommandOutcome,
    },
    /// Final session status. The frontend emits this once when a worker ends,
    /// including for load failures and unwinds; cores should not emit it.
    Exited(ExitStatus),
}

/// Event endpoint supplied by the frontend.
pub trait EventSink: Send {
    fn emit(&mut self, event: CoreEvent);
}

/// Event sink that discards everything. Useful for headless tests.
#[derive(Default)]
pub struct NullEventSink;

impl EventSink for NullEventSink {
    fn emit(&mut self, _event: CoreEvent) {}
}
