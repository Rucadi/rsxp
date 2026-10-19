// SPDX-License-Identifier: GPL-2.0-or-later
//! Renderer construction: adapter/device selection, the packed-pixel compute
//! pipeline, and the initial GPU buffers.
//!
//! `new` creates its own `wgpu::Instance`; `new_for_surface` accepts an instance
//! and surface so the chosen adapter is presentation-compatible. `initialize`
//! requests a high-performance adapter (falling back to none), clamps
//! `max_storage_buffer_binding_size` to the adapter limit and at most 512 MiB,
//! and requests a device with no optional features (`Features::empty()`),
//! `MemoryHints::Performance` and tracing off. It builds the compute pipeline
//! from `shader.wgsl` and validates creation through an error scope.
//!
//! Bind group layout (compute-visible, matching `shader.wgsl`):
//! - `0` atlas: read-only storage, packed source pixels.
//! - `1` frame: read-write storage, main + scratch pixels.
//! - `2` commands: read-only storage, `Params` records (min 256 bytes).
//! - `3` batch offsets: dynamic-offset uniform (min 16 bytes) selecting a
//!   batch's first command.
//! - `4` tile geometry: read-only storage, native tile bins/quads.
//!
//! Initial buffers: a 64 KiB atlas, a width*height*8 frame (two `u32` per
//! pixel: main and scratch), a 256-byte command buffer holding one `Params`,
//! a 256-byte tile-geometry buffer, and `batch_offsets` sized capacity*256.
//! `batching` is enabled unless the `RSXP_GPU_BATCH` environment variable is
//! exactly `"0"` (used to A/B the serial and grouped dispatch paths).
//!
//! `frame_buffer` enforces adapter limits: each dimension must fit
//! `max_compute_workgroups_per_dimension * 8` and the total allocation must not
//! exceed `max_storage_buffer_binding_size`.
use super::GpuRenderer;
use crate::tilemap::TileAtlas;
use std::{
    collections::{HashMap, HashSet},
    num::NonZeroU64,
};

impl GpuRenderer {
    pub fn new(width: u32, height: u32) -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        pollster::block_on(Self::initialize(width, height, instance, None))
    }

    /// Initializes the compositor with an adapter compatible with a native
    /// presentation surface created from the same instance.
    pub fn new_for_surface(
        width: u32,
        height: u32,
        instance: wgpu::Instance,
        surface: &wgpu::Surface<'_>,
    ) -> Result<Self, String> {
        pollster::block_on(Self::initialize(width, height, instance, Some(surface)))
    }

    async fn initialize(
        width: u32,
        height: u32,
        instance: wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
    ) -> Result<Self, String> {
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: surface,
                ..Default::default()
            })
            .await
            .map_err(|e| e.to_string())?;
        let mut limits = wgpu::Limits::default();
        limits.max_storage_buffer_binding_size = adapter
            .limits()
            .max_storage_buffer_binding_size
            .min(adapter.limits().max_buffer_size.min(512 * 1024 * 1024));
        limits.max_buffer_size = limits
            .max_buffer_size
            .max(u64::from(limits.max_storage_buffer_binding_size));
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("RGSS compositor"),
                required_features: wgpu::Features::empty(),
                required_limits: limits,
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|e| e.to_string())?;
        let error_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("RGSS packed-pixel compositor"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RGSS buffers"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(256),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: NonZeroU64::new(16),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(4),
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("RGSS composition"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        if let Some(error) = error_scope.pop().await {
            return Err(error.to_string());
        }
        let atlas = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bitmap atlas"),
            size: 65536,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let frame = Self::frame_buffer(&device, width, height)?;
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("draw commands"),
            size: 256,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let tile_geometry = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("native tile ground/priority geometry"),
            size: 256,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let batch_offsets = Self::batch_offsets(&device, 1);
        let adapter_name = adapter.get_info().name;
        Ok(Self {
            remote: None,
            instance,
            adapter,
            device,
            queue,
            pipeline,
            layout,
            bind_group: None,
            tile_atlas: TileAtlas::default(),
            pending_tiles: Vec::new(),
            tile_runs: Vec::new(),
            tile_run_index: 0,
            collecting_tile: false,
            tile_data: Vec::new(),
            previous_tile_data: Vec::new(),
            tile_geometry,
            tile_geometry_capacity: 256,
            native_tile_batches: 0,
            native_tile_layers: 0,
            native_tile_quads: 0,
            atlas,
            atlas_capacity: 65536,
            atlas_end: 0,
            slots: HashMap::new(),
            free: Vec::new(),
            used: HashSet::new(),
            frame,
            params,
            batch_offsets,
            batching: std::env::var("RSXP_GPU_BATCH").as_deref() != Ok("0"),
            dispatches: 0,
            param_capacity: 1,
            commands: Vec::new(),
            previous_commands: Vec::new(),
            batches: Vec::new(),
            previous_batching: false,
            previous_uploads: 0,
            size: (width, height),
            adapter_name,
            uploaded_bytes: 0,
            readbacks: 0,
            composed_frames: 0,
            reused_frames: 0,
        })
    }
    pub(super) fn batch_offsets(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("draw batch offsets"),
            size: (capacity * 256) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }
    pub(super) fn frame_buffer(
        device: &wgpu::Device,
        w: u32,
        h: u32,
    ) -> Result<wgpu::Buffer, String> {
        let size = u64::from(w) * u64::from(h) * 8;
        let max_extent = device.limits().max_compute_workgroups_per_dimension * 8;
        if w == 0
            || h == 0
            || w > max_extent
            || h > max_extent
            || size > u64::from(device.limits().max_storage_buffer_binding_size)
        {
            return Err("GPU framebuffer exceeds adapter limits".into());
        }
        Ok(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screen and viewport pixels"),
            size,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }))
    }
}
