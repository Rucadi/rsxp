// SPDX-License-Identifier: GPL-2.0-or-later
//! Video output. Frames borrow core-owned pixels; the contract performs no
//! allocation and no copy on its side.

/// Pixel layout of a submitted frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    /// 4 bytes per pixel, red in the lowest addressed byte.
    Rgba8,
    /// 4 bytes per pixel, blue in the lowest addressed byte.
    Bgra8,
    /// 2 bytes per pixel, native little-endian RGB565.
    Rgb565,
    /// One byte per pixel plus a palette supplied by the core separately.
    Indexed8,
}

impl PixelFormat {
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Rgba8 | Self::Bgra8 => 4,
            Self::Rgb565 => 2,
            Self::Indexed8 => 1,
        }
    }
}

/// A completed frame. `stride` is the number of bytes between row starts and
/// may be larger than `width * bytes_per_pixel` for alignment/padding.
#[derive(Clone, Copy, Debug)]
pub struct VideoFrame<'a> {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub format: PixelFormat,
    pub pixels: &'a [u8],
}

impl<'a> VideoFrame<'a> {
    pub fn new(
        width: u32,
        height: u32,
        stride: usize,
        format: PixelFormat,
        pixels: &'a [u8],
    ) -> Self {
        debug_assert!(stride >= width as usize * format.bytes_per_pixel());
        debug_assert!(pixels.len() >= stride.saturating_mul(height as usize));
        Self {
            width,
            height,
            stride,
            format,
            pixels,
        }
    }

    pub fn rgba8(width: u32, height: u32, stride: usize, pixels: &'a [u8]) -> Self {
        Self::new(width, height, stride, PixelFormat::Rgba8, pixels)
    }

    pub fn bgra8(width: u32, height: u32, stride: usize, pixels: &'a [u8]) -> Self {
        Self::new(width, height, stride, PixelFormat::Bgra8, pixels)
    }

    pub fn rgb565(width: u32, height: u32, stride: usize, pixels: &'a [u8]) -> Self {
        Self::new(width, height, stride, PixelFormat::Rgb565, pixels)
    }

    /// Visible bytes of one row.
    pub fn row_bytes(&self) -> usize {
        self.width as usize * self.format.bytes_per_pixel()
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
}

impl VideoSink for NullVideoSink {
    fn present(&mut self, frame: VideoFrame<'_>) {
        self.frames += 1;
        self.bytes += frame.pixels.len() as u64;
    }
}
