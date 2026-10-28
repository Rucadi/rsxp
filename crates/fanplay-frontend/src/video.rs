// SPDX-License-Identifier: GPL-2.0-or-later
//! Validated, allocation-reusing conversion of core frames for host displays.

use fanplay_core::{FrameError, PixelFormat, VideoFrame};

/// Converts a frame directly into packed `0xAARRGGBB` pixels.
pub fn convert_argb(frame: &VideoFrame<'_>, out: &mut Vec<u32>) -> Result<(), FrameError> {
    frame.validate()?;
    let width = frame.width as usize;
    out.resize(width.saturating_mul(frame.height as usize), 0);
    for row in 0..frame.height as usize {
        let source = visible_row(frame, row);
        let target = &mut out[row * width..(row + 1) * width];
        match frame.format {
            PixelFormat::Rgba8 => {
                let (pixels, _) = source.as_chunks::<4>();
                pixels.iter().zip(target).for_each(|(p, out)| {
                    *out = u32::from_be_bytes([p[3], p[0], p[1], p[2]]);
                });
            }
            PixelFormat::Bgra8 | PixelFormat::Bgrx8 => {
                let (pixels, _) = source.as_chunks::<4>();
                pixels.iter().zip(target).for_each(|(p, out)| {
                    let alpha = if frame.format == PixelFormat::Bgra8 {
                        p[3]
                    } else {
                        255
                    };
                    *out = u32::from_be_bytes([alpha, p[2], p[1], p[0]]);
                });
            }
            PixelFormat::Rgb24 => {
                let (pixels, _) = source.as_chunks::<3>();
                pixels.iter().zip(target).for_each(|(p, out)| {
                    *out = u32::from_be_bytes([255, p[0], p[1], p[2]]);
                });
            }
            PixelFormat::Rgb565 => {
                let (pixels, _) = source.as_chunks::<2>();
                pixels.iter().zip(target).for_each(|(p, out)| {
                    let [r, g, b, a] = rgb565(p);
                    *out = u32::from_be_bytes([a, r, g, b]);
                });
            }
            PixelFormat::Indexed8 => {
                let palette = frame.palette.ok_or(FrameError::PaletteRequired)?;
                source.iter().zip(target).for_each(|(index, out)| {
                    let offset = usize::from(*index) * 4;
                    let p = &palette[offset..offset + 4];
                    *out = u32::from_be_bytes([p[3], p[0], p[1], p[2]]);
                });
            }
        }
    }
    Ok(())
}

/// Converts a frame directly into tightly packed RGBA bytes.
pub fn convert_rgba(frame: &VideoFrame<'_>, out: &mut Vec<u8>) -> Result<(), FrameError> {
    frame.validate()?;
    let width = frame.width as usize;
    out.resize(
        width
            .saturating_mul(frame.height as usize)
            .saturating_mul(4),
        0,
    );
    for row in 0..frame.height as usize {
        let source = visible_row(frame, row);
        let target = &mut out[row * width * 4..(row + 1) * width * 4];
        match frame.format {
            PixelFormat::Rgba8 => target.copy_from_slice(source),
            PixelFormat::Bgra8 | PixelFormat::Bgrx8 => {
                let (pixels, _) = source.as_chunks::<4>();
                let (target, _) = target.as_chunks_mut::<4>();
                pixels.iter().zip(target).for_each(|(p, out)| {
                    out.copy_from_slice(&[
                        p[2],
                        p[1],
                        p[0],
                        if frame.format == PixelFormat::Bgra8 {
                            p[3]
                        } else {
                            255
                        },
                    ]);
                });
            }
            PixelFormat::Rgb24 => {
                let (pixels, _) = source.as_chunks::<3>();
                let (target, _) = target.as_chunks_mut::<4>();
                pixels.iter().zip(target).for_each(|(p, out)| {
                    out.copy_from_slice(&[p[0], p[1], p[2], 255]);
                });
            }
            PixelFormat::Rgb565 => {
                let (pixels, _) = source.as_chunks::<2>();
                let (target, _) = target.as_chunks_mut::<4>();
                pixels.iter().zip(target).for_each(|(p, out)| {
                    out.copy_from_slice(&rgb565(p));
                });
            }
            PixelFormat::Indexed8 => {
                let palette = frame.palette.ok_or(FrameError::PaletteRequired)?;
                let (target, _) = target.as_chunks_mut::<4>();
                source.iter().zip(target).for_each(|(index, out)| {
                    let offset = usize::from(*index) * 4;
                    out.copy_from_slice(&palette[offset..offset + 4]);
                });
            }
        }
    }
    Ok(())
}

fn visible_row<'a>(frame: &'a VideoFrame<'_>, row: usize) -> &'a [u8] {
    let start = row * frame.stride;
    &frame.pixels[start..start + frame.row_bytes()]
}

fn rgb565(pixel: &[u8]) -> [u8; 4] {
    let value = u16::from_le_bytes([pixel[0], pixel[1]]);
    [
        (((value >> 11) & 0x1f) as u8) << 3,
        (((value >> 5) & 0x3f) as u8) << 2,
        ((value & 0x1f) as u8) << 3,
        255,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_all_formats_and_ignores_row_padding() {
        let cases: &[(PixelFormat, &[u8], Option<&[u8]>)] = &[
            (PixelFormat::Rgba8, &[255, 0, 0, 128, 9], None),
            (PixelFormat::Bgra8, &[0, 0, 255, 128, 9], None),
            (PixelFormat::Bgrx8, &[0, 0, 255, 0, 9], None),
            (PixelFormat::Rgb24, &[255, 0, 0, 9], None),
            (PixelFormat::Rgb565, &[0, 248, 9], None),
            (
                PixelFormat::Indexed8,
                &[1, 9],
                Some(&[0, 0, 0, 0, 255, 0, 0, 128]),
            ),
        ];
        for &(format, pixels, palette) in cases {
            let frame = VideoFrame::try_new(1, 1, pixels.len(), format, pixels, palette).unwrap();
            let mut rgba = Vec::new();
            let mut argb = Vec::new();
            convert_rgba(&frame, &mut rgba).unwrap();
            convert_argb(&frame, &mut argb).unwrap();
            let alpha = if format == PixelFormat::Bgrx8
                || format == PixelFormat::Rgb24
                || format == PixelFormat::Rgb565
            {
                255
            } else {
                128
            };
            let red = if format == PixelFormat::Rgb565 {
                248
            } else {
                255
            };
            assert_eq!(rgba, [red, 0, 0, alpha]);
            assert_eq!(argb, [u32::from_be_bytes([alpha, red, 0, 0])]);
        }
    }

    #[test]
    fn rejects_invalid_frames_before_resizing_output() {
        let frame = VideoFrame {
            width: 2,
            height: 1,
            stride: 1,
            format: PixelFormat::Rgba8,
            pixels: &[0],
            palette: None,
        };
        let mut out = vec![7];
        assert_eq!(
            convert_rgba(&frame, &mut out),
            Err(FrameError::StrideTooSmall)
        );
        assert_eq!(out, [7]);
    }
}
