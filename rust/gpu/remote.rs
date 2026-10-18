// SPDX-License-Identifier: GPL-2.0-or-later
//! Portable compositor submissions: ordered commands plus revision-based uploads.
//! The runner builds these without executing GPU work. The presentation device
//! applies them directly, so normal game frames never pass through readback.
use super::{GpuRenderer, Params};
use crate::graphics::Bitmap;
use std::{collections::HashMap, sync::Arc};

const MAX_PACKET: usize = 512 * 1024 * 1024;
const MAGIC: u32 = 0x52534731;
pub(super) struct Upload {
    offset: u32,
    pixels: Arc<Vec<u32>>,
}
#[derive(Default)]
pub(super) struct RemoteState {
    pub uploads: Vec<Upload>,
    pub mirrors: HashMap<u64, Arc<Vec<u32>>>,
}
/// CPU-side render description. Merging keeps every asset update in order while
/// retaining only the newest draw list; dropping intermediate views is safe.
pub struct GpuFrame {
    size: (u32, u32),
    atlas_capacity: u64,
    batching: bool,
    commands: Vec<Params>,
    tiles: Vec<u32>,
    uploads: Vec<Upload>,
}
impl GpuFrame {
    pub fn size(&self) -> (u32, u32) {
        self.size
    }
    pub fn upload_bytes(&self) -> usize {
        self.uploads.iter().map(|u| u.pixels.len() * 4).sum()
    }
    pub fn merge(&mut self, mut latest: Self) {
        self.size = latest.size;
        self.atlas_capacity = latest.atlas_capacity;
        self.batching = latest.batching;
        self.commands = latest.commands;
        self.tiles = latest.tiles;
        self.uploads.append(&mut latest.uploads);
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(
            36 + self.commands.len() * 256
                + self.tiles.len() * 4
                + self.upload_bytes()
                + self.uploads.len() * 8,
        );
        for v in [
            MAGIC,
            self.size.0,
            self.size.1,
            u32::from(self.batching),
            self.commands.len() as u32,
            self.tiles.len() as u32,
            self.uploads.len() as u32,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&self.atlas_capacity.to_le_bytes());
        // Both peers run the same native executable/ABI. Params contains only
        // u32 values (including float bit patterns), with no pointers or padding.
        #[cfg(target_endian = "little")]
        {
            out.extend_from_slice(bytemuck::cast_slice(&self.commands));
            out.extend_from_slice(bytemuck::cast_slice(&self.tiles));
        }
        #[cfg(not(target_endian = "little"))]
        {
            for p in &self.commands {
                for row in p.a {
                    for v in row {
                        out.extend_from_slice(&v.to_le_bytes());
                    }
                }
            }
            for v in &self.tiles {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        for upload in &self.uploads {
            out.extend_from_slice(&upload.offset.to_le_bytes());
            out.extend_from_slice(&(upload.pixels.len() as u32).to_le_bytes());
            #[cfg(target_endian = "little")]
            out.extend_from_slice(bytemuck::cast_slice(upload.pixels.as_slice()));
            #[cfg(not(target_endian = "little"))]
            for pixel in upload.pixels.iter() {
                out.extend_from_slice(&pixel.to_le_bytes());
            }
        }
        out
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_PACKET {
            return Err("GPU submission exceeds transport limit".into());
        }
        let mut reader = Reader(bytes);
        if reader.word()? != MAGIC {
            return Err("Unsupported GPU submission version".into());
        }
        let size = (reader.word()?, reader.word()?);
        if size.0 == 0 || size.1 == 0 || u64::from(size.0) * u64::from(size.1) > 16_777_216 {
            return Err("Invalid GPU submission dimensions".into());
        }
        let batching = reader.word()? != 0;
        let command_count = reader.word()? as usize;
        let tile_count = reader.word()? as usize;
        let upload_count = reader.word()? as usize;
        let atlas_capacity = u64::from_le_bytes(reader.take(8)?.try_into().unwrap());
        if atlas_capacity > MAX_PACKET as u64 || !atlas_capacity.is_multiple_of(4) {
            return Err("Invalid GPU atlas capacity".into());
        }
        let mut commands = Vec::new();
        for bytes in reader
            .take(
                command_count
                    .checked_mul(256)
                    .ok_or("GPU command count overflow")?,
            )?
            .chunks_exact(256)
        {
            let mut p = Params { a: [[0; 4]; 16] };
            for (word, bytes) in p.a.iter_mut().flatten().zip(bytes.chunks_exact(4)) {
                *word = u32::from_le_bytes(bytes.try_into().unwrap());
            }
            commands.push(p);
        }
        let tiles = reader
            .take(tile_count.checked_mul(4).ok_or("GPU tile count overflow")?)?
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        if upload_count > reader.0.len() / 8 {
            return Err("Invalid GPU upload count".into());
        }
        let mut uploads = Vec::new();
        for _ in 0..upload_count {
            let offset = reader.word()?;
            let len = reader.word()? as usize;
            let byte_count = len.checked_mul(4).ok_or("GPU upload size overflow")?;
            if u64::from(offset) * 4 + byte_count as u64 > atlas_capacity {
                return Err("GPU upload exceeds atlas".into());
            }
            let pixels = Arc::new(
                reader
                    .take(byte_count)?
                    .chunks_exact(4)
                    .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
                    .collect(),
            );
            uploads.push(Upload { offset, pixels });
        }
        if !reader.0.is_empty() {
            return Err("Trailing GPU submission data".into());
        }
        Ok(Self {
            size,
            atlas_capacity,
            batching,
            commands,
            tiles,
            uploads,
        })
    }
}
struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], String> {
        if len > self.0.len() {
            return Err("Truncated GPU submission".into());
        }
        let (data, rest) = self.0.split_at(len);
        self.0 = rest;
        Ok(data)
    }
    fn word(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
}
impl GpuRenderer {
    pub fn enable_remote(&mut self) {
        self.remote = Some(RemoteState::default());
    }
    pub(super) fn record_upload(&mut self, id: u64, offset: u32, bitmap: &Bitmap) {
        let remote = self.remote.as_mut().unwrap();
        let pixels = Arc::new(bitmap.pixels().to_vec());
        remote.uploads.push(Upload {
            offset,
            pixels: pixels.clone(),
        });
        remote.mirrors.insert(id, pixels);
    }
    pub fn take_remote_frame(&mut self) -> Result<GpuFrame, String> {
        let remote = self
            .remote
            .as_mut()
            .ok_or("Renderer is not recording remote submissions")?;
        Ok(GpuFrame {
            size: self.size,
            atlas_capacity: self.atlas_capacity,
            batching: self.batching,
            commands: self.commands.clone(),
            tiles: self.tile_data.clone(),
            uploads: std::mem::take(&mut remote.uploads),
        })
    }
    pub fn apply_remote_frame(&mut self, frame: GpuFrame) -> Result<(), String> {
        self.begin(frame.size.0, frame.size.1)?;
        self.ensure_atlas_capacity(frame.atlas_capacity)?;
        for upload in frame.uploads {
            self.queue.write_buffer(
                &self.atlas,
                u64::from(upload.offset) * 4,
                bytemuck::cast_slice(&upload.pixels),
            );
            self.uploaded_bytes += upload.pixels.len() as u64 * 4;
        }
        self.batching = frame.batching;
        self.commands = frame.commands;
        self.tile_data = frame.tiles;
        self.submit()
    }
    pub(super) fn materialize_remote(&mut self) -> Result<(), String> {
        let Some(remote) = self.remote.take() else {
            return Ok(());
        };
        for (id, bitmap) in &remote.mirrors {
            if let Some(slot) = self.slots.get(id) {
                self.queue.write_buffer(
                    &self.atlas,
                    u64::from(slot.offset) * 4,
                    bytemuck::cast_slice(bitmap.as_slice()),
                );
            }
        }
        let result = self.submit();
        self.remote = Some(remote);
        result
    }
    pub(super) fn ensure_atlas_capacity(&mut self, bytes: u64) -> Result<(), String> {
        if bytes <= self.atlas_capacity {
            return Ok(());
        }
        let capacity = bytes
            .checked_next_power_of_two()
            .ok_or("GPU atlas size overflow")?;
        if capacity > u64::from(self.device.limits().max_storage_buffer_binding_size) {
            return Err("GPU bitmap atlas exceeds adapter limit; use --renderer software".into());
        }
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bitmap atlas"),
            size: capacity,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        if self.remote.is_none() {
            let mut encoder = self.device.create_command_encoder(&Default::default());
            encoder.copy_buffer_to_buffer(&self.atlas, 0, &buffer, 0, self.atlas_capacity);
            self.queue.submit([encoder.finish()]);
        }
        self.bind_group = None;
        self.atlas = buffer;
        self.atlas_capacity = capacity;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_merge_preserves_updates_and_latest_draws() {
        let frame = |color| GpuFrame {
            size: (2, 2),
            atlas_capacity: 65536,
            batching: true,
            commands: vec![Params {
                a: [[color; 4]; 16],
            }],
            tiles: vec![color],
            uploads: vec![Upload {
                offset: 0,
                pixels: Arc::new(vec![color; 4]),
            }],
        };
        let mut first = GpuFrame::decode(&frame(1).encode()).unwrap();
        first.merge(GpuFrame::decode(&frame(2).encode()).unwrap());
        let merged = GpuFrame::decode(&first.encode()).unwrap();
        assert_eq!(merged.commands[0].a[0][0], 2);
        assert_eq!(merged.tiles, [2]);
        assert_eq!(merged.uploads.len(), 2);
        assert_eq!(merged.uploads[0].pixels.as_slice(), [1; 4]);
        assert_eq!(merged.uploads[1].pixels.as_slice(), [2; 4]);
        let bytes = merged.encode();
        for n in 0..bytes.len() {
            assert!(GpuFrame::decode(&bytes[..n]).is_err());
        }
    }
}

#[cfg(test)]
mod parity_tests {
    use super::*;
    use crate::graphics::{Color, Rect, SpriteFx, SpriteTransform, Tone};
    #[test]
    #[ignore = "requires a wgpu adapter"]
    fn remote_frames_match_direct_gpu_after_merging_resizing_and_snapshots() {
        let mut direct = GpuRenderer::new(64, 64).unwrap();
        let mut recorder = GpuRenderer::new(64, 64).unwrap();
        recorder.enable_remote();
        let mut display = GpuRenderer::new(64, 64).unwrap();
        let mut bitmap = Bitmap::new(32, 32).unwrap();
        let mut pending: Option<GpuFrame> = None;
        for phase in 0..6 {
            let (width, height) = if phase < 2 { (64, 64) } else { (96, 80) };
            if phase == 1 {
                bitmap = Bitmap::new(256, 128).unwrap();
            } // grow the atlas
            if phase == 3 {
                bitmap = Bitmap::new(48, 32).unwrap();
            } // recycle its slot
            bitmap.clear(Color::rgba(30 + phase * 20, 180, 90, 210));
            for gpu in [&mut direct, &mut recorder] {
                gpu.begin(width, height).unwrap();
                gpu.clear(false, 0xff102030);
                gpu.clear(true, 0);
                for index in 0..4 {
                    gpu.tile(
                        0,
                        1,
                        &bitmap,
                        Rect::new(0, 0, 32, 32),
                        Rect::new(0, 0, width, height),
                        SpriteTransform {
                            x: (index % 2 * 32) as f64,
                            y: (index / 2 * 32) as f64,
                            ..Default::default()
                        },
                        (0, 0),
                        &SpriteFx::default(),
                        0,
                        true,
                    )
                    .unwrap();
                }
                gpu.sprite(
                    2,
                    &bitmap,
                    bitmap.rect(),
                    Rect::new(0, 0, width, height),
                    SpriteTransform {
                        x: phase as f64 * 2.0,
                        zoom_x: 0.5,
                        zoom_y: 0.5,
                        ..Default::default()
                    },
                    (0, 0),
                    &SpriteFx {
                        opacity: 170,
                        ..Default::default()
                    },
                    0,
                    true,
                )
                .unwrap();
                gpu.viewport(Color::rgba(20, 30, 40, 50), Tone::default());
                gpu.brightness(230);
                gpu.submit().unwrap();
            }
            assert_eq!(
                recorder.dispatches, 0,
                "recording must never dispatch GPU work"
            );
            assert_eq!(
                recorder.readbacks, 0,
                "ordinary frames must never read back pixels"
            );
            let frame = GpuFrame::decode(&recorder.take_remote_frame().unwrap().encode()).unwrap();
            if let Some(pending) = &mut pending {
                pending.merge(frame);
            } else {
                pending = Some(frame);
            }
            if phase % 2 == 1 {
                display.apply_remote_frame(pending.take().unwrap()).unwrap();
                let expected = direct.readback().unwrap();
                let actual = display.readback().unwrap();
                assert_eq!(
                    actual.pixels(),
                    expected.pixels(),
                    "remote frame differs in phase {phase}"
                );
                // Explicit game screenshots still materialize the current frame.
                let capture = recorder.readback().unwrap();
                assert_eq!(capture.pixels(), expected.pixels());
                // Start the next phase with fresh diagnostic counters.
                recorder.dispatches = 0;
                recorder.readbacks = 0;
            }
        }
    }
}
