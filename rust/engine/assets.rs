// SPDX-License-Identifier: GPL-2.0-or-later
//! Asset lookup and image/audio/font resolution through the platform Host.
//!
//! `Runtime::asset` is the single entry point for reading game data: it
//! normalizes `\` to `/`, asks the host for a loose file first, then scans the
//! encrypted `archives` (raw `Archive` blobs) for a case-insensitive name match.
//!
//! The typed helpers retry with a fixed extension list when the exact name is
//! missing: images (`png`,`jpg`,`jpeg`,`bmp`,`gif`), audio
//! (`ogg`,`wav`,`mp3`,`mid`,`midi`,`flac`,`opus`) and movies (preferring
//! Theora `ogv`). `font_bytes` splits a U+001F-separated fallback list and
//! loads the first existing `Fonts/<name>.ttf`; hits and misses are memoized in
//! a 16-entry cache, which is why it needs `&mut self`.
use super::Runtime;
use crate::archive::Archive;
use std::sync::Arc;

impl Runtime {
    /// Reads a game asset through the host files and encrypted archive fallback.
    pub fn asset(&self, name: &str) -> Result<Vec<u8>, String> {
        let normalized = name.replace('\\', "/");
        if let Some(bytes) = self.host.read_file(&normalized) {
            return Ok(bytes);
        }
        for bytes in &self.archives {
            let archive = Archive::parse(bytes).map_err(|e| e.to_string())?;
            if let Some(index) = archive
                .entries()
                .iter()
                .position(|entry| entry.name.eq_ignore_ascii_case(normalized.as_bytes()))
            {
                return archive
                    .read(index, 0, usize::MAX)
                    .map_err(|e| e.to_string());
            }
        }
        Err(format!("asset not found: {name}"))
    }

    /// Fast loose-file existence check with encrypted-archive fallback.
    pub fn asset_exists(&self, name: &str) -> bool {
        let normalized = name.replace('\\', "/");
        if self
            .host
            .file_exists(&normalized)
            .unwrap_or_else(|| self.host.read_file(&normalized).is_some())
        {
            return true;
        }
        self.archives.iter().any(|bytes| {
            Archive::parse(bytes).is_ok_and(|archive| {
                archive
                    .entries()
                    .iter()
                    .any(|entry| entry.name.eq_ignore_ascii_case(normalized.as_bytes()))
            })
        })
    }

    /// Lists immediate game-folder children and archive-backed virtual paths.
    pub fn asset_directory(&self, name: &str) -> Vec<String> {
        let normalized = name.trim_matches('/').replace('\\', "/");
        let prefix = if normalized.is_empty() {
            String::new()
        } else {
            format!("{normalized}/")
        };
        let mut entries = self.host.read_dir(&normalized).unwrap_or_default();
        for bytes in &self.archives {
            let Ok(archive) = Archive::parse(bytes) else {
                continue;
            };
            for entry in archive.entries() {
                let Ok(path) = std::str::from_utf8(&entry.name) else {
                    continue;
                };
                let normalized_path = path.replace('\\', "/");
                let Some(remainder) = normalized_path.strip_prefix(&prefix) else {
                    continue;
                };
                if let Some(child) = remainder.split('/').next().filter(|part| !part.is_empty()) {
                    entries.push(child.to_owned());
                }
            }
        }
        let mut seen = std::collections::HashSet::new();
        entries.retain(|entry| seen.insert(entry.to_lowercase()));
        entries
    }

    /// Checks whether a game path is a directory in the host tree or archives.
    pub fn asset_is_dir(&self, name: &str) -> bool {
        let normalized = name.trim_matches('/').replace('\\', "/");
        if self.host.is_dir(&normalized).unwrap_or(false) {
            return true;
        }
        let prefix = if normalized.is_empty() {
            String::new()
        } else {
            format!("{normalized}/")
        };
        self.archives.iter().any(|bytes| {
            Archive::parse(bytes).is_ok_and(|archive| {
                archive.entries().iter().any(|entry| {
                    std::str::from_utf8(&entry.name)
                        .is_ok_and(|path| path.replace('\\', "/").starts_with(&prefix))
                })
            })
        })
    }
    pub(super) fn image_bytes(&self, name: &str) -> Result<Vec<u8>, String> {
        if let Ok(bytes) = self.asset(name) {
            return Ok(bytes);
        }
        for ext in ["png", "jpg", "jpeg", "bmp", "gif"] {
            if let Ok(bytes) = self.asset(&format!("{name}.{ext}")) {
                return Ok(bytes);
            }
        }
        Err(format!("image not found: {name}"))
    }
    pub(super) fn audio_bytes(&self, name: &str) -> Result<Vec<u8>, String> {
        if let Ok(bytes) = self.asset(name) {
            return Ok(bytes);
        }
        for ext in ["ogg", "wav", "mp3", "mid", "midi", "flac", "opus"] {
            if let Ok(bytes) = self.asset(&format!("{name}.{ext}")) {
                return Ok(bytes);
            }
        }
        Err(format!("audio not found: {name}"))
    }
    /// Resolves a movie by name, preferring Theora Ogg (`.ogv`).
    pub(super) fn movie_bytes(&self, name: &str) -> Result<Vec<u8>, String> {
        if let Ok(bytes) = self.asset(name) {
            return Ok(bytes);
        }
        for ext in ["ogv", "theora", "ogg"] {
            if let Ok(bytes) = self.asset(&format!("{name}.{ext}")) {
                return Ok(bytes);
            }
        }
        Err(format!("movie not found: {name}"))
    }
    pub(super) fn font_bytes(&mut self, names: &str) -> Option<Arc<Vec<u8>>> {
        if let Some(bytes) = self.font_cache.get(names) {
            return bytes.clone();
        }
        let mut result = None;
        for name in names.split('\u{1f}') {
            if let Ok(bytes) = self.asset(&format!("Fonts/{name}.ttf")) {
                result = Some(Arc::new(bytes));
                break;
            }
        }
        if self.font_cache.len() >= 16 {
            self.font_cache.clear();
        }
        self.font_cache.insert(names.to_owned(), result.clone());
        result
    }
}
