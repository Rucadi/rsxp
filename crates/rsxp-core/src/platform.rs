// SPDX-License-Identifier: GPL-2.0-or-later
//! Filesystem, clock and logging services provided by the frontend.

use crate::events::LogLevel;
use std::path::PathBuf;
use std::time::Duration;

/// Platform services used on cold paths or for pacing.
///
/// Paths are relative to the loaded content root unless absolute. A frontend
/// may serve them from a native filesystem, a virtual filesystem or an
/// in-memory game bundle; cores must not assume native path semantics beyond
/// the contract.
pub trait Platform: Send + Sync {
    /// Reads one file. Returns `None` when it does not exist.
    fn read_file(&self, path: &str) -> Option<Vec<u8>>;

    /// Writes one file (saves, configuration). The default rejects writes.
    fn write_file(&self, path: &str, _bytes: &[u8]) -> Result<(), String> {
        Err(format!("platform does not support writing {path}"))
    }

    /// Directory for battery saves, save states and screenshots.
    fn save_dir(&self) -> PathBuf;

    /// Monotonic seconds. Used for pacing and UI counters, never for emulated
    /// cycle accounting.
    fn now(&self) -> f64 {
        0.0
    }

    /// Pauses the core thread. Browsers may make this a no-op.
    fn sleep(&self, _duration: Duration) {}

    /// Emits a diagnostic line. The frontend decides whether it reaches a
    /// terminal, the workspace log panel or a file.
    fn log(&self, level: LogLevel, message: &str) {
        match level {
            LogLevel::Error => eprintln!("error: {message}"),
            LogLevel::Warn => eprintln!("warning: {message}"),
            LogLevel::Info => eprintln!("{message}"),
            LogLevel::Debug => {}
        }
    }
}
