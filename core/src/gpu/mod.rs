// SPDX-License-Identifier: GPL-2.0-or-later
//! Persistent wgpu compositor. CPU bitmaps remain authoritative for RGSS pixel
//! access; only changed source revisions are uploaded. Presentation has no
//! framebuffer readback; snapshots explicitly synchronize when requested.
//!
//! State lives in `GpuRenderer`, which owns the device, pipeline, atlas,
//! retained frame and command list. `Params` is the 256-byte command record
//! shared with `shader.wgsl`: an operation tag in `a[0][0]` (0 clear, 1
//! sprite/plane, 2 viewport, 3 brightness, 4 native tile geometry) plus source
//! transform, color/tone and pattern fields, with floats stored as bit
//! patterns. `Slot` tracks each uploaded bitmap's atlas placement and revision.
//! See `commands` for the full field layout.
//!
//! `begin(w, h)` starts a frame: it reallocates the frame buffer on resize and
//! resets tile collection, the atlas frame, commands, used ids and the retained
//! comparison. `clear`, `sprite`, `tile`, `plane`, `viewport` and `brightness`
//! are defined in submodules; `submit` dispatches or reuses the frame.
//!
//! Module map: `initialize` creates the pipeline and buffers, `resources`
//! transfers pixels and owns atlas slots, `commands` encodes draws, `batching`
//! groups independent writes, `submission` dispatches or reuses the retained
//! frame, and `remote` produces/consumes portable `GpuFrame` submissions.
use crate::tilemap::{TileAtlas, TileGeometryCache, TileQuad};
use std::collections::{HashMap, HashSet};

mod batching;
mod commands;
mod initialize;
mod remote;
mod resources;
pub use remote::GpuFrame;
mod submission;
#[cfg(test)]
mod tests;

use batching::DrawBatch;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    a: [[u32; 4]; 16],
}
impl Params {
    fn floats(&mut self, i: usize, v: [f32; 4]) {
        self.a[i] = v.map(f32::to_bits);
    }
}
struct Slot {
    offset: u32,
    len: u32,
    revision: u64,
}
/// Persistent compositor state. Submodules handle setup, resource transfer,
/// draw encoding and submission; the frame lifecycle stays here.
pub struct GpuRenderer {
    remote: Option<remote::RemoteState>,
    // Device ownership and the compute pipeline.
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    bind_group: Option<wgpu::BindGroup>,

    // Native tile collection and retained geometry.
    tile_atlas: TileAtlas,
    pending_tiles: Vec<TileQuad>,
    tile_runs: Vec<TileGeometryCache>,
    tile_run_index: usize,
    collecting_tile: bool,
    tile_data: Vec<u32>,
    previous_tile_data: Vec<u32>,
    tile_geometry: wgpu::Buffer,
    tile_geometry_capacity: usize,
    pub native_tile_batches: u64,
    pub native_tile_layers: u64,
    pub native_tile_quads: u64,

    // Revision-tracked source bitmaps in the reusable pixel atlas.
    atlas: wgpu::Buffer,
    atlas_capacity: u64,
    atlas_end: u32,
    slots: HashMap<u64, Slot>,
    free: Vec<(u32, u32)>,
    used: HashSet<u64>,

    // Frame targets, encoded commands and retained-frame comparison.
    frame: wgpu::Buffer,
    params: wgpu::Buffer,
    batch_offsets: wgpu::Buffer,
    batching: bool,
    pub dispatches: u64,
    param_capacity: usize,
    commands: Vec<Params>,
    previous_commands: Vec<Params>,
    batches: Vec<DrawBatch>,
    previous_batching: bool,
    previous_uploads: u64,
    size: (u32, u32),

    // Adapter identity and cumulative profiler counters.
    pub adapter_name: String,
    pub uploaded_bytes: u64,
    pub readbacks: u64,
    pub composed_frames: u64,
    pub reused_frames: u64,
}
impl GpuRenderer {
    pub fn begin(&mut self, w: u32, h: u32) -> Result<(), String> {
        if self.size != (w, h) {
            self.bind_group = None;
            self.frame = Self::frame_buffer(&self.device, w, h)?;
            self.size = (w, h);
            self.previous_commands.clear();
        }
        self.pending_tiles.clear();
        self.tile_run_index = 0;
        self.tile_data.clear();
        self.tile_atlas.begin_frame();
        self.commands.clear();
        self.used.clear();
        self.clear(false, 0xff000000);
        Ok(())
    }
    pub fn frame_buffer_ref(&self) -> &wgpu::Buffer {
        &self.frame
    }
    pub fn size(&self) -> (u32, u32) {
        self.size
    }
}
