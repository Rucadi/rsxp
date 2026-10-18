// SPDX-License-Identifier: GPL-2.0-or-later
//! Cross-platform Theora (`.ogv`) movie decoding for `Graphics.play_movie`.
//!
//! A pure-Rust Ogg demuxer feeds the Theora decoder so movies work on every
//! frontend (desktop, Android, iOS and WebAssembly) without system codecs.
//! Playback is driven by the game's normal update loop: the caller asks for the
//! frame that should be visible at a given elapsed time, and this module decodes
//! ahead only as far as needed.
//!
//! [`Movie::open`] parses the Theora headers so `width`/`height` are known
//! before the first frame; [`Movie::frame_at`] returns the frame visible at a
//! given elapsed time, and [`Movie::finished`] reports end of stream.
use crate::graphics::Bitmap;
use ogg::reading::PacketReader;
use std::io::{BufReader, Cursor};
use theora_rs::{
    DecoderContext, HeaderParser, PixelFmt, YCbCrBuffer, th_decode_packetin, th_decode_ycbcr_out,
    th_granule_time,
};

pub struct Movie {
    reader: PacketReader<BufReader<Cursor<Vec<u8>>>>,
    parser: HeaderParser,
    decoder: Option<DecoderContext>,
    serial: Option<u32>,
    packet_index: i64,
    /// Presentation time of the most recently decoded frame (seconds).
    next_time: f64,
    /// A decoded frame waiting to be shown, with its presentation time.
    pending: Option<(f64, Bitmap)>,
    /// The frame currently being shown.
    current: Option<Bitmap>,
    finished: bool,
}

impl Movie {
    /// Opens a movie, parsing the Theora headers so the dimensions are known
    /// before the first frame is requested.
    pub fn open(bytes: Vec<u8>) -> Result<Self, String> {
        let mut movie = Self {
            reader: PacketReader::new(BufReader::new(Cursor::new(bytes))),
            parser: HeaderParser::new(),
            decoder: None,
            serial: None,
            packet_index: 0,
            next_time: 0.0,
            pending: None,
            current: None,
            finished: false,
        };
        loop {
            let Some(packet) = movie.next_theora_packet()? else {
                return Err("movie ended before its Theora headers".into());
            };
            movie
                .parser
                .push(&packet)
                .map_err(|error| error.to_string())?;
            if movie.parser.is_ready() {
                break;
            }
        }
        movie.decoder = Some(
            movie
                .parser
                .decoder()
                .map_err(|error| error.to_string())?,
        );
        Ok(movie)
    }

    pub fn width(&self) -> u32 {
        self.parser.info.pic_width.max(1)
    }

    pub fn height(&self) -> u32 {
        self.parser.info.pic_height.max(1)
    }

    pub fn finished(&self) -> bool {
        self.finished
    }

    /// Returns the frame that should be visible at `elapsed` seconds, decoding
    /// ahead as needed. `None` only before the first frame is decoded.
    pub fn frame_at(&mut self, elapsed: f64) -> Result<Option<&Bitmap>, String> {
        loop {
            if self.pending.is_none() {
                while self.pending.is_none() && !self.finished {
                    if !self.decode_next()? {
                        break;
                    }
                }
                if self.pending.is_none() {
                    break;
                }
            }
            let due = self.current.is_none()
                || matches!(&self.pending, Some((time, _)) if *time <= elapsed);
            if !due {
                break;
            }
            if let Some((_, bitmap)) = self.pending.take() {
                self.current = Some(bitmap);
            }
        }
        Ok(self.current.as_ref())
    }

    /// Reads Ogg pages until the next packet belonging to the Theora stream.
    fn next_theora_packet(&mut self) -> Result<Option<theora_rs::OggPacket>, String> {
        loop {
            let packet = match self.reader.read_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => {
                    self.finished = true;
                    return Ok(None);
                }
                Err(error) => return Err(error.to_string()),
            };
            if self.serial.is_none() {
                let data = &packet.data;
                let is_theora =
                    packet.first_in_stream() && data.len() >= 7 && data[0] == 0x80 && &data[1..7] == b"theora";
                if !is_theora {
                    continue;
                }
                self.serial = Some(packet.stream_serial());
            }
            if Some(packet.stream_serial()) != self.serial {
                continue;
            }
            let op = theora_rs::OggPacket {
                packet: packet.data.clone(),
                b_o_s: packet.first_in_stream(),
                e_o_s: packet.last_in_stream(),
                granulepos: packet.absgp_page() as i64,
                packetno: self.packet_index,
            };
            self.packet_index += 1;
            return Ok(Some(op));
        }
    }

    /// Processes exactly one Theora packet. Returns false once the stream ends.
    fn decode_next(&mut self) -> Result<bool, String> {
        let Some(packet) = self.next_theora_packet()? else {
            return Ok(false);
        };
        let Some(decoder) = self.decoder.as_mut() else {
            return Ok(false);
        };
        th_decode_packetin(decoder, &packet).map_err(|error| error.to_string())?;
        let frame = match th_decode_ycbcr_out(decoder) {
            Ok(frame) => frame,
            // Some packets do not produce a displayable frame; move on.
            Err(_) => return Ok(true),
        };
        let time = {
            let info = &self.parser.info;
            let time = th_granule_time(info, packet.granulepos);
            if time < 0.0 {
                self.next_time
            } else {
                time
            }
        };
        let bitmap = {
            let info = &self.parser.info;
            ycbcr_to_bitmap(&frame, info)?
        };
        self.next_time = time;
        self.pending = Some((time, bitmap));
        Ok(true)
    }
}

fn chroma_decimation(format: PixelFmt) -> (usize, usize) {
    match format {
        PixelFmt::Pf444 => (0, 0),
        PixelFmt::Pf422 => (1, 0),
        PixelFmt::Pf420 | PixelFmt::Reserved => (1, 1),
    }
}

fn sample_plane(plane: &theora_rs::ImgPlane, x: usize, y: usize) -> u8 {
    let width = plane.width.max(1) as usize;
    if x >= width {
        return 0;
    }
    let index = plane.data_offset as isize + y as isize * plane.stride as isize + x as isize;
    if index < 0 {
        return 0;
    }
    plane.data.get(index as usize).copied().unwrap_or(0)
}

fn clamp_channel(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

/// Converts the decoder's YCbCr planes to an ARGB bitmap cropped to the visible
/// picture, mirroring libtheora's `th_decode_ycbcr_out` sample layout.
pub(crate) fn ycbcr_to_bitmap(
    frame: &YCbCrBuffer,
    info: &theora_rs::Info,
) -> Result<Bitmap, String> {
    let width = info.pic_width.max(1);
    let height = info.pic_height.max(1);
    let (hdec, vdec) = chroma_decimation(info.pixel_fmt);
    let crop_x = info.pic_x as usize;
    let crop_y = info.pic_y as usize;
    let mut pixels = vec![0u32; width as usize * height as usize];
    for y in 0..height as usize {
        for x in 0..width as usize {
            let yy = i32::from(sample_plane(&frame[0], crop_x + x, crop_y + y));
            let cb = i32::from(sample_plane(&frame[1], (crop_x + x) >> hdec, (crop_y + y) >> vdec));
            let cr = i32::from(sample_plane(&frame[2], (crop_x + x) >> hdec, (crop_y + y) >> vdec));
            let red = clamp_channel((1904000 * yy + 2609823 * cr - 363703744) / 1635200);
            let green =
                clamp_channel((3827562 * yy - 1287801 * cb - 2672387 * cr + 447306710) / 3287200);
            let blue = clamp_channel((952000 * yy + 1649289 * cb - 225932192) / 817600);
            pixels[y * width as usize + x] =
                (255u32 << 24) | (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue);
        }
    }
    Bitmap::from_argb(width, height, &pixels).map_err(|error| error.to_string())
}
