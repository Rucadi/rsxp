// SPDX-License-Identifier: GPL-2.0-or-later
//! Content identity and load-time probing.

use std::path::{Path, PathBuf};

/// What the user asked to run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContentKind {
    /// A game directory (RGSS) or a directory containing a ROM set.
    Directory,
    /// A single ROM/script/archive file.
    File,
}

/// Content selected by the user or the game library.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Content {
    pub path: PathBuf,
    pub kind: ContentKind,
    /// Human-readable name for window titles and save paths. The frontend may
    /// replace this after inspecting metadata.
    pub label: String,
}

impl Content {
    pub fn directory(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let label = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        Self {
            path,
            kind: ContentKind::Directory,
            label,
        }
    }

    pub fn file(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let label = path
            .file_stem()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        Self {
            path,
            kind: ContentKind::File,
            label,
        }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    pub fn extension(&self) -> Option<&str> {
        self.path.extension().and_then(|ext| ext.to_str())
    }
}

/// Read-only facts used to route content to a core.
///
/// `header` should be a small prefix of the file (16-64 bytes is enough for
/// most ROM formats). Directory content passes an empty header.
#[derive(Clone, Copy, Debug)]
pub struct ContentProbe<'a> {
    pub path: &'a Path,
    pub is_dir: bool,
    pub header: &'a [u8],
}

impl<'a> ContentProbe<'a> {
    pub fn new(path: &'a Path, is_dir: bool, header: &'a [u8]) -> Self {
        Self {
            path,
            is_dir,
            header,
        }
    }

    pub fn file_name(&self) -> &str {
        self.path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
    }

    /// Lower-case extension without the leading dot.
    pub fn extension(&self) -> Option<&str> {
        self.path
            .extension()
            .and_then(|extension| extension.to_str())
    }

    /// Case-insensitive extension comparison without allocating.
    pub fn has_extension(&self, extension: &str) -> bool {
        self.path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case(extension))
    }

    /// Case-insensitive file-name comparison without allocating.
    pub fn has_file_name(&self, name: &str) -> bool {
        self.file_name().eq_ignore_ascii_case(name)
    }
}
