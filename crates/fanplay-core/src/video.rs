// SPDX-License-Identifier: GPL-2.0-or-later
//! Video output. Frames borrow core-owned pixels; the contract performs no
//! allocation and no copy on its side.

use std::error::Error;
use std::fmt;

/// Pixel layout of a submitted frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    /// 4 bytes per pixel, red in the lowest addressed byte.
    Rgba8,
    /// 4 bytes per pixel, blue in the lowest addressed byte.
    Bgra8,
    /// 4 bytes per pixel, blue in the lowest addressed byte. The fourth byte
    /// is padding and must be treated as opaque.
    Bgrx8,
    /// 3 bytes per pixel in red, green, blue order.
    Rgb24,
    /// 2 bytes per pixel, native little-endian RGB565.
    Rgb565,
    /// One byte per pixel plus a palette supplied with the frame. Palette
    /// entries are RGBA quadruples in index order.
    Indexed8,
}

impl PixelFormat {
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Rgba8 | Self::Bgra8 | Self::Bgrx8 => 4,
            Self::Rgb24 => 3,
            Self::Rgb565 => 2,
            Self::Indexed8 => 1,
        }
    }

    /// Returns true when the format requires a palette.
    pub const fn is_indexed(self) -> bool {
        matches!(self, Self::Indexed8)
    }
}

/// Why a [`VideoFrame`] cannot be presented.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    Empty,
    StrideTooSmall,
    BufferTooSmall,
    PaletteRequired,
    PaletteTooLarge,
    PaletteMissingColors,
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("frame has zero width or height"),
            Self::StrideTooSmall => f.write_str("frame stride is smaller than one row"),
            Self::BufferTooSmall => f.write_str("frame buffer is smaller than stride * height"),
            Self::PaletteRequired => f.write_str("indexed frame has no palette"),
            Self::PaletteTooLarge => f.write_str("frame palette has more than 256 entries"),
            Self::PaletteMissingColors => f.write_str("frame palette is too short for its indices"),
        }
    }
}

impl Error for FrameError {}

/// A completed frame. `stride` is the number of bytes between row starts and
/// may be larger than `width * bytes_per_pixel` for alignment/padding.
///
/// Fields remain public so cores can build a frame without a trait-object
/// call, but [`VideoFrame::try_new`] and [`VideoFrame::validate`] perform the
/// same checks in release builds that the old debug assertions did.
#[derive(Clone, Copy, Debug)]
pub struct VideoFrame<'a> {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub format: PixelFormat,
    pub pixels: &'a [u8],
    /// RGBA quadruples for [`PixelFormat::Indexed8`]; ignored otherwise.
    pub palette: Option<&'a [u8]>,
}

impl<'a> VideoFrame<'a> {
    /// Builds a frame, panicking on a contract violation.
    ///
    /// Core code should use [`try_new`](Self::try_new) when the dimensions or
    /// buffer come from untrusted content.
    pub fn new(
        width: u32,
        height: u32,
        stride: usize,
        format: PixelFormat,
        pixels: &'a [u8],
    ) -> Self {
        Self::try_new(width, height, stride, format, pixels, None)
            .expect("core submitted an invalid video frame")
    }

    /// Builds a validated frame, reporting why it is invalid.
    pub fn try_new(
        width: u32,
        height: u32,
        stride: usize,
        format: PixelFormat,
        pixels: &'a [u8],
        palette: Option<&'a [u8]>,
    ) -> Result<Self, FrameError> {
        let frame = Self {
            width,
            height,
            stride,
            format,
            pixels,
            palette,
        };
        frame.validate()?;
        Ok(frame)
    }

    pub fn rgba8(width: u32, height: u32, stride: usize, pixels: &'a [u8]) -> Self {
        Self::new(width, height, stride, PixelFormat::Rgba8, pixels)
    }

    pub fn bgra8(width: u32, height: u32, stride: usize, pixels: &'a [u8]) -> Self {
        Self::new(width, height, stride, PixelFormat::Bgra8, pixels)
    }

    pub fn bgrx8(width: u32, height: u32, stride: usize, pixels: &'a [u8]) -> Self {
        Self::new(width, height, stride, PixelFormat::Bgrx8, pixels)
    }

    pub fn rgb24(width: u32, height: u32, stride: usize, pixels: &'a [u8]) -> Self {
        Self::new(width, height, stride, PixelFormat::Rgb24, pixels)
    }

    pub fn rgb565(width: u32, height: u32, stride: usize, pixels: &'a [u8]) -> Self {
        Self::new(width, height, stride, PixelFormat::Rgb565, pixels)
    }

    /// Builds an indexed frame with an RGBA palette.
    pub fn indexed8(
        width: u32,
        height: u32,
        stride: usize,
        pixels: &'a [u8],
        palette: &'a [u8],
    ) -> Self {
        Self::try_new(
            width,
            height,
            stride,
            PixelFormat::Indexed8,
            pixels,
            Some(palette),
        )
        .expect("core submitted an invalid indexed video frame")
    }

    /// Visible bytes of one row.
    pub fn row_bytes(&self) -> usize {
        self.width as usize * self.format.bytes_per_pixel()
    }

    /// Checks dimensions, stride, buffer length and palette in release builds.
    pub fn validate(&self) -> Result<(), FrameError> {
        if self.width == 0 || self.height == 0 {
            return Err(FrameError::Empty);
        }
        let row = self.row_bytes();
        if self.stride < row {
            return Err(FrameError::StrideTooSmall);
        }
        if self.pixels.len() < self.stride.saturating_mul(self.height as usize) {
            return Err(FrameError::BufferTooSmall);
        }
        if self.format.is_indexed() {
            let palette = self.palette.ok_or(FrameError::PaletteRequired)?;
            if palette.len() % 4 != 0 || palette.len() > 1024 {
                return Err(FrameError::PaletteTooLarge);
            }
            let colors = palette.len() / 4;
            if colors == 0 {
                return Err(FrameError::PaletteRequired);
            }
            for row in 0..self.height as usize {
                let start = row * self.stride;
                if self.pixels[start..start + self.width as usize]
                    .iter()
                    .any(|index| usize::from(*index) >= colors)
                {
                    return Err(FrameError::PaletteMissingColors);
                }
            }
        }
        Ok(())
    }
}

/// Presentation endpoint supplied by the frontend.
///
/// Calls are made from the core thread. A sink may block briefly to apply
/// backpressure (for example when writing an isolated runner packet), but must
/// not run UI code or wait for a user interaction.
pub trait VideoSink: Send {
    /// Presents a complete frame. Implementations must consume the bytes before
    /// returning; the core may reuse its buffer immediately afterwards.
    fn present(&mut self, frame: VideoFrame<'_>);

    /// Submits a core-specific acceleration packet.
    ///
    /// The meaning of `kind` belongs to the core. A frontend that does not know
    /// the kind returns `false`; the core may then submit a CPU frame through
    /// [`Self::present`] if it advertised a CPU fallback. No packet may contain
    /// pointers or process-local addresses.
    fn present_packet(&mut self, _kind: &str, _packet: &[u8]) -> bool {
        false
    }
}

/// Convenience sink for headless tests and benchmarks. It verifies that frames
/// are well-formed without retaining them.
#[derive(Default)]
pub struct NullVideoSink {
    pub frames: u64,
    pub bytes: u64,
    /// Frames rejected by [`VideoFrame::validate`].
    pub invalid: u64,
}

impl VideoSink for NullVideoSink {
    fn present(&mut self, frame: VideoFrame<'_>) {
        if frame.validate().is_err() {
            self.invalid += 1;
            return;
        }
        self.frames += 1;
        self.bytes += frame.pixels.len() as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_dimensions_and_palettes() {
        let pixels = [0u8; 16];
        assert!(VideoFrame::rgba8(2, 2, 8, &pixels).validate().is_ok());
        assert_eq!(
            VideoFrame::try_new(2, 2, 4, PixelFormat::Rgba8, &pixels, None).unwrap_err(),
            FrameError::StrideTooSmall
        );
        assert_eq!(
            VideoFrame::try_new(2, 2, 8, PixelFormat::Rgba8, &[0; 8], None).unwrap_err(),
            FrameError::BufferTooSmall
        );
        let indices = [0u8, 1, 1, 0];
        let palette = [0u8, 0, 0, 255, 255, 255, 255, 255];
        assert!(VideoFrame::indexed8(2, 2, 2, &indices, &palette)
            .validate()
            .is_ok());
        assert_eq!(
            VideoFrame::try_new(2, 2, 2, PixelFormat::Indexed8, &indices, None).unwrap_err(),
            FrameError::PaletteRequired
        );
        assert_eq!(
            VideoFrame::try_new(
                2,
                2,
                2,
                PixelFormat::Indexed8,
                &[0, 4, 0, 0],
                Some(&palette)
            )
            .unwrap_err(),
            FrameError::PaletteMissingColors
        );
    }
}
