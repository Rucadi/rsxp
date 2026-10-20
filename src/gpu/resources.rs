// SPDX-License-Identifier: GPL-2.0-or-later
//! Source bitmap uploads, atlas slot management and explicit framebuffer
//! readback.
//!
//! `upload` maps a caller-supplied bitmap id to a slot in the shared pixel
//! atlas. `Slot` records the atlas `offset`, `len`, and the last uploaded
//! `revision`, so a bitmap is re-sent only when `Bitmap::revision()` changes.
//! Slots whose length no longer matches are freed and reallocated; new or
//! resized bitmaps are placed in the first free hole large enough (splitting
//! the remainder) or appended at `atlas_end`, growing the buffer when needed.
//! While remote recording is active, changed pixels are queued instead of
//! written (`record_upload`), so the runner emits no GPU work.
//!
//! `readback` is the only path that observes composited pixels on the CPU.
//! It first materializes any recorded remote uploads, copies the frame buffer
//! into a mappable staging buffer, maps it, and returns a `Bitmap` via
//! `Bitmap::from_argb`, bumping the `readbacks` counter. Ordinary presentation
//! never calls it.
use super::{GpuRenderer, Slot};
use crate::graphics::Bitmap;

impl GpuRenderer {
    pub(super) fn upload(&mut self, id: u64, bitmap: &Bitmap) -> Result<u32, String> {
        self.used.insert(id);
        let len = bitmap.pixels().len() as u32;
        if self.slots.get(&id).is_some_and(|s| s.len != len) {
            let old = self.slots.remove(&id).unwrap();
            self.free.push((old.offset, old.len));
        }
        if !self.slots.contains_key(&id) {
            let offset = if let Some(i) = self.free.iter().position(|(_, size)| *size >= len) {
                let (offset, size) = self.free.swap_remove(i);
                if size > len {
                    self.free.push((offset + len, size - len));
                }
                offset
            } else {
                let offset = self.atlas_end;
                self.atlas_end = self
                    .atlas_end
                    .checked_add(len)
                    .ok_or("GPU bitmap atlas overflow")?;
                offset
            };
            let bytes = u64::from(offset + len) * 4;
            self.ensure_atlas_capacity(bytes)?;
            self.slots.insert(
                id,
                Slot {
                    offset,
                    len,
                    revision: 0,
                },
            );
        }
        let slot = &self.slots[&id];
        let offset = slot.offset;
        if slot.revision != bitmap.revision() {
            if self.remote.is_some() {
                self.record_upload(id, offset, bitmap);
            } else {
                self.queue.write_buffer(
                    &self.atlas,
                    u64::from(offset) * 4,
                    bytemuck::cast_slice(bitmap.pixels()),
                );
            }
            self.slots.get_mut(&id).unwrap().revision = bitmap.revision();
            self.uploaded_bytes += u64::from(len) * 4;
        }
        Ok(offset)
    }
    pub fn readback(&mut self) -> Result<Bitmap, String> {
        self.materialize_remote()?;
        let bytes = u64::from(self.size.0) * u64::from(self.size.1) * 4;
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("requested screenshot"),
            size: bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&self.frame, 0, &staging, 0, bytes);
        self.queue.submit([encoder.finish()]);
        let (send, recv) = std::sync::mpsc::channel();
        staging.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = send.send(r);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| e.to_string())?;
        recv.recv()
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        let mapped = staging
            .slice(..)
            .get_mapped_range()
            .map_err(|e| e.to_string())?;
        let result = Bitmap::from_argb(self.size.0, self.size.1, bytemuck::cast_slice(&mapped))
            .map_err(|e| e.to_string());
        drop(mapped);
        staging.unmap();
        self.readbacks += 1;
        result
    }
}
