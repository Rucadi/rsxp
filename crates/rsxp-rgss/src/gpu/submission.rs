// SPDX-License-Identifier: GPL-2.0-or-later
//! Frame submission: retained-frame reuse, buffer/bind-group upkeep, compute
//! dispatch and atlas reclamation.
//!
//! `submit` flushes pending native tiles, then either records a remote frame
//! (no GPU work) or compares the new commands, uploads, batching flag and tile
//! geometry against the previous frame. An identical frame is reused verbatim
//! (`reused_frames`), skipping the dispatch entirely. Otherwise it grows and
//! uploads the tile-geometry, command and batch-offset buffers as needed,
//! rebuilds the batch schedule via `draw_batches` when the layout changed
//! (outputs, targets or bounds), lazily recreates the bind group after any
//! buffer growth, and dispatches one workgroup per 8x8 destination tile with
//! the batch count on the z axis. Separate dispatches in one compute pass keep
//! ordered storage visibility, so barriers between batches hold. Finally
//! `reclaim_sources` frees atlas slots unused this frame, coalesces adjacent
//! free ranges and shrinks `atlas_end` when the tail is free.
use super::batching::draw_batches;
use super::GpuRenderer;
use std::num::NonZeroU64;

impl GpuRenderer {
    pub fn submit(&mut self) -> Result<(), String> {
        self.flush_tiles();
        if self.remote.is_some() {
            self.composed_frames += 1;
            self.reclaim_sources();
            return Ok(());
        }
        if self.commands == self.previous_commands
            && self.uploaded_bytes == self.previous_uploads
            && self.batching == self.previous_batching
            && self.tile_data == self.previous_tile_data
        {
            self.reused_frames += 1;
            return Ok(());
        }
        if self.tile_data != self.previous_tile_data {
            let bytes = self.tile_data.len() * 4;
            if bytes > self.tile_geometry_capacity {
                self.tile_geometry_capacity = bytes.next_power_of_two();
                if self.tile_geometry_capacity as u64
                    > u64::from(self.device.limits().max_storage_buffer_binding_size)
                {
                    return Err("native tile geometry exceeds adapter limits".into());
                }
                self.tile_geometry = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("native tile ground/priority geometry"),
                    size: self.tile_geometry_capacity as u64,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                self.bind_group = None;
            }
            if bytes > 0 {
                self.queue.write_buffer(
                    &self.tile_geometry,
                    0,
                    bytemuck::cast_slice(&self.tile_data),
                );
            }
        }
        let buffers_grew = self.commands.len() > self.param_capacity;
        if buffers_grew {
            self.param_capacity = self.commands.len().next_power_of_two();
            self.batch_offsets = Self::batch_offsets(&self.device, self.param_capacity);
            self.bind_group = None;
            self.params = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("draw commands"),
                size: (self.param_capacity * 256) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        self.queue
            .write_buffer(&self.params, 0, bytemuck::cast_slice(&self.commands));
        // Pixel content/source cells often animate while draw bounds remain
        // fixed. Reuse both the overlap schedule and its GPU offset buffer.
        let layout_changed = buffers_grew
            || self.batching != self.previous_batching
            || self.commands.len() != self.previous_commands.len()
            || self
                .commands
                .iter()
                .zip(&self.previous_commands)
                .any(|(a, b)| a.a[0][0] != b.a[0][0] || a.a[0][1] != b.a[0][1] || a.a[1] != b.a[1]);
        if layout_changed {
            self.batches = draw_batches(&self.commands, self.batching);
            let mut offsets = vec![[0u32; 64]; self.batches.len()];
            for (offset, batch) in offsets.iter_mut().zip(&self.batches) {
                offset[0] = batch.start as u32;
            }
            if !offsets.is_empty() {
                self.queue
                    .write_buffer(&self.batch_offsets, 0, bytemuck::cast_slice(&offsets));
            }
            self.previous_batching = self.batching;
        }
        if self.bind_group.is_none() {
            self.bind_group = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("frame buffers"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: self.tile_geometry.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.atlas.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.frame.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.params.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.batch_offsets,
                            offset: 0,
                            size: NonZeroU64::new(16),
                        }),
                    },
                ],
            }));
        }
        let group = self.bind_group.as_ref().unwrap();
        let mut encoder = self.device.create_command_encoder(&Default::default());
        // Separate dispatches in one compute pass have ordered storage visibility.
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("RGSS draws"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            for (i, batch) in self.batches.iter().enumerate() {
                pass.set_bind_group(0, group, &[(i * 256) as u32]);
                pass.dispatch_workgroups(
                    batch.width.div_ceil(8),
                    batch.height.div_ceil(8),
                    batch.len as u32,
                );
            }
            self.dispatches += self.batches.len() as u64;
        }
        self.queue.submit([encoder.finish()]);
        self.previous_commands.clone_from(&self.commands);
        self.previous_tile_data.clone_from(&self.tile_data);
        self.previous_uploads = self.uploaded_bytes;
        self.composed_frames += 1;
        self.reclaim_sources();
        Ok(())
    }

    fn reclaim_sources(&mut self) {
        if let Some(remote) = &mut self.remote {
            remote.mirrors.retain(|id, _| self.used.contains(id));
        }
        let dead: Vec<_> = self
            .slots
            .keys()
            .filter(|id| !self.used.contains(id))
            .copied()
            .collect();
        for id in dead {
            let slot = self.slots.remove(&id).unwrap();
            self.free.push((slot.offset, slot.len));
        }
        // Reuse adjacent slots when scenes dispose many small bitmaps and then
        // allocate larger ones, rather than growing the atlas indefinitely.
        self.free.sort_unstable_by_key(|slot| slot.0);
        let mut merged: Vec<(u32, u32)> = Vec::with_capacity(self.free.len());
        for &(offset, len) in &self.free {
            if let Some(last) = merged.last_mut()
                && last.0 + last.1 == offset
            {
                last.1 += len;
                continue;
            }
            merged.push((offset, len));
        }
        if let Some((offset, _)) = merged.pop_if(|s| s.0 + s.1 == self.atlas_end) {
            self.atlas_end = offset;
        }
        self.free = merged;
    }
}
