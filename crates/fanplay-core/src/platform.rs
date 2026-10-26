// SPDX-License-Identifier: GPL-2.0-or-later
//! Filesystem, clock and logging services provided by the frontend.

use crate::events::LogLevel;
use std::error::Error;
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

/// Typed failure from a platform read or write.
///
/// Cores need to distinguish "the file is genuinely absent" from "the file
/// exists but cannot be read"; collapsing both into `None` hides permission and
/// I/O failures from the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlatformError {
    NotFound(String),
    PermissionDenied(String),
    Io(String),
    Unsupported(String),
}

impl PlatformError {
    pub fn not_found(path: impl Into<String>) -> Self {
        Self::NotFound(path.into())
    }
    pub fn permission_denied(path: impl Into<String>) -> Self {
        Self::PermissionDenied(path.into())
    }
    pub fn io(message: impl Into<String>) -> Self {
        Self::Io(message.into())
    }
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::Unsupported(message.into())
    }

    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::NotFound(_))
    }
}

impl fmt::Display for PlatformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(path) => write!(f, "{path}: file not found"),
            Self::PermissionDenied(path) => write!(f, "{path}: permission denied"),
            Self::Io(message) | Self::Unsupported(message) => f.write_str(message),
        }
    }
}

impl Error for PlatformError {}

impl From<std::io::Error> for PlatformError {
    fn from(error: std::io::Error) -> Self {
        match error.kind() {
            std::io::ErrorKind::NotFound => Self::NotFound(error.to_string()),
            std::io::ErrorKind::PermissionDenied => Self::PermissionDenied(error.to_string()),
            _ => Self::Io(error.to_string()),
        }
    }
}

pub type PlatformResult<T> = Result<T, PlatformError>;

/// Platform services used on cold paths or for pacing.
///
/// Paths are relative to the loaded content root unless absolute. A frontend
/// may serve them from a native filesystem, a virtual filesystem or an
/// in-memory game bundle; cores must not assume native path semantics beyond
/// the contract.
///
/// Content, system files and user saves are separate namespaces:
/// - [`read_content`](Self::read_content) reads game data.
/// - [`read_system_file`](Self::read_system_file) reads optional BIOS/firmware
///   files; absence is `Ok(None)` but an unreadable file is an error.
/// - [`read_save`](Self::read_save)/[`write_save`](Self::write_save) access
///   battery-backed user data without exposing a native path.
pub trait Platform: Send + Sync {
    /// Reads one content file. A missing file is an error, not `None`.
    ///
    /// Implementations should override this with a typed filesystem read. The
    /// default adapts the legacy [`read_file`](Self::read_file) helper.
    fn read_content(&self, path: &str) -> PlatformResult<Vec<u8>> {
        self.read_file(path)
            .ok_or_else(|| PlatformError::not_found(path))
    }

    /// Reads one optional firmware/system file.
    ///
    /// The default treats a missing content file as absent while propagating
    /// permission and other I/O failures.
    fn read_system_file(&self, path: &str) -> PlatformResult<Option<Vec<u8>>> {
        match self.read_content(path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.is_not_found() => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Reads a named user save blob. Absence is `Ok(None)`.
    fn read_save(&self, _name: &str) -> PlatformResult<Option<Vec<u8>>> {
        Ok(None)
    }

    /// Writes a named user save blob. The default adapts the legacy
    /// [`write_file`](Self::write_file) helper.
    fn write_save(&self, name: &str, bytes: &[u8]) -> PlatformResult<()> {
        self.write_file(name, bytes).map_err(PlatformError::io)
    }

    /// Directory for battery saves, save states and screenshots.
    ///
    /// Native backends that cannot go through [`write_save`](Self::write_save)
    /// use this path; it is an implementation detail, not a routing key.
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

    /// Legacy helper retained for small integrations. New code should use the
    /// typed methods above so unreadable files are not reported as missing.
    fn read_file(&self, _path: &str) -> Option<Vec<u8>> {
        None
    }

    /// Legacy helper retained for small integrations.
    fn write_file(&self, path: &str, _bytes: &[u8]) -> Result<(), String> {
        Err(format!("platform does not support writing {path}"))
    }
}
