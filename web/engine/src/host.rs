// SPDX-License-Identifier: GPL-2.0-or-later
//! Browser platform adapter: virtual files, image/font decoding and audio.
//!
//! `WebHost` is the `rsxp::host::Host` implementation used by the WASM runtime.
//! `read_file` delegates to the virtual filesystem, `image` decodes any format
//! supported by the `image` crate into an RGBA `Bitmap`, and `text` rasterizes
//! with `fontdue` using the embedded `assets/liberation.ttf` when the game
//! supplies no font. `text` also measures the line (advance width plus ascent
//! and descenders) even when only metrics are requested, and rejects font sizes
//! outside `1..=512`. Audio comes from `BrowserAudio`.
use crate::{audio::BrowserAudio, files::read_via_host};
use rsxp::{
    graphics::{Bitmap, Color},
    host::Host,
};

pub(crate) struct WebHost {
    audio: BrowserAudio,
}

impl WebHost {
    pub(crate) fn new() -> Self {
        Self {
            audio: BrowserAudio::new(),
        }
    }
}

impl Host for WebHost {
    fn audio(&mut self) -> Option<&mut dyn rsxp::host::AudioBackend> {
        Some(&mut self.audio)
    }
    fn read_file(&self, path: &str) -> Option<Vec<u8>> {
        read_via_host(path)
    }
    fn image(&self, bytes: &[u8]) -> Result<Bitmap, String> {
        rsxp::portable_media::image(bytes)
    }
    fn text(
        &self,
        font: Option<&[u8]>,
        size: i32,
        bold: bool,
        italic: bool,
        text: &str,
        color: Color,
        render: bool,
    ) -> Result<(u32, u32, Option<Bitmap>), String> {
        rsxp::portable_media::text(font, size, bold, italic, text, color, render)
    }
}
