// SPDX-License-Identifier: GPL-2.0-or-later
//! Asset lookup and image/audio/font resolution through the platform Host.
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
