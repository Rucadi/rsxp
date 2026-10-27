// SPDX-License-Identifier: GPL-2.0-or-later
//! Animated GIF decoding shared by every frontend. Decoding lives in the
//! engine rather than the platform host so bitmap behavior is deterministic.
//!
//! [`is_gif`] sniffs the GIF87a/GIF89a signature. [`decode`] builds a
//! [`Bitmap`] from ARGB (`0xAARRGGBB`) pixels: a single-frame GIF stays static,
//! while an animated GIF composites every frame onto the logical screen
//! (honouring the disposal method) and installs them with per-frame delays in
//! seconds. A zero centisecond delay is treated as one centisecond so the frame
//! remains visible.
use crate::graphics::Bitmap;
use gif::{ColorOutput, DecodeOptions, DisposalMethod};
use std::io::Cursor;

/// True when `bytes` begin with a GIF signature.
pub fn is_gif(bytes: &[u8]) -> bool {
    bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")
}

/// Decodes a GIF into a bitmap. A single-frame GIF stays a static bitmap; an
/// animated GIF composites every frame onto the logical screen (honouring the
/// disposal method) and installs them with per-frame delays in seconds.
pub fn decode(bytes: &[u8]) -> Result<Bitmap, String> {
    let mut options = DecodeOptions::new();
    options.set_color_output(ColorOutput::RGBA);
    let mut decoder = options
        .read_info(Cursor::new(bytes))
        .map_err(|error| error.to_string())?;
    let width = u32::from(decoder.width()).max(1);
    let height = u32::from(decoder.height()).max(1);
    let (w, h) = (width as usize, height as usize);
    let mut canvas = vec![0u32; w * h];
    let mut frames: Vec<Vec<u32>> = Vec::new();
    let mut delays: Vec<f64> = Vec::new();
    let mut previous: Option<Vec<u32>> = None;
    while let Some(frame) = decoder
        .read_next_frame()
        .map_err(|error| error.to_string())?
    {
        let dispose = frame.dispose;
        if dispose == DisposalMethod::Previous {
            previous = Some(canvas.clone());
        }
        let frame_w = frame.width as usize;
        let frame_h = frame.height as usize;
        let left = frame.left as usize;
        let top = frame.top as usize;
        let buffer = frame.buffer.as_ref();
        for y in 0..frame_h {
            let dy = top + y;
            if dy >= h {
                break;
            }
            for x in 0..frame_w {
                let dx = left + x;
                if dx >= w {
                    break;
                }
                let offset = (y * frame_w + x) * 4;
                let alpha = buffer[offset + 3];
                if alpha == 0 {
                    continue;
                }
                canvas[dy * w + dx] = (u32::from(alpha) << 24)
                    | (u32::from(buffer[offset]) << 16)
                    | (u32::from(buffer[offset + 1]) << 8)
                    | u32::from(buffer[offset + 2]);
            }
        }
        frames.push(canvas.clone());
        // GIF delays are centiseconds; a zero delay still needs a visible frame.
        delays.push(f64::from(frame.delay.max(1)) / 100.0);
        match dispose {
            DisposalMethod::Background => {
                for y in 0..frame_h {
                    let dy = top + y;
                    if dy >= h {
                        break;
                    }
                    for x in 0..frame_w {
                        let dx = left + x;
                        if dx >= w {
                            break;
                        }
                        canvas[dy * w + dx] = 0;
                    }
                }
            }
            DisposalMethod::Previous => {
                if let Some(restored) = previous.take() {
                    canvas = restored;
                }
            }
            _ => {}
        }
    }
    if frames.is_empty() {
        return Err("gif contains no frames".into());
    }
    let mut bitmap = Bitmap::from_argb(width, height, &frames[0]).map_err(|e| e.to_string())?;
    if frames.len() > 1 {
        bitmap.set_gif_frames(frames, delays, 0, None);
    }
    Ok(bitmap)
}
