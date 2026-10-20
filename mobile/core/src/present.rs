// SPDX-License-Identifier: GPL-2.0-or-later
//! Native GPU presentation for the mobile shells.
//!
//! The engine composites RGSS scenes with the shared wgpu compute renderer. This
//! module owns the platform surface (a `CAMetalLayer` on iOS) and blits the
//! composited packed-pixel buffer to it, letterboxed and nearest-neighbour
//! scaled, so no RGBA readback or per-frame image conversion is needed.
use rsxp::gpu::GpuRenderer;
use std::ffi::c_void;

const SHADER: &str = r#"
@group(0) @binding(0) var<storage,read> pixels:array<u32>;
@group(0) @binding(1) var<uniform> dimensions:array<vec4<f32>,2>;
@vertex fn vertex(@builtin(vertex_index) id:u32)->@builtin(position) vec4<f32> {
 let x=f32((id<<1u)&2u);let y=f32(id&2u);return vec4(x*2.0-1.0,y*2.0-1.0,0.0,1.0);
}
@fragment fn fragment(@builtin(position) pos:vec4<f32>)->@location(0) vec4<f32> {
 let source=dimensions[0].xy;let rect=dimensions[1];
 if any(pos.xy<rect.xy)||any(pos.xy>=rect.xy+rect.zw){return vec4(0.0,0.0,0.0,1.0);}
 let scale=min(rect.z/source.x,rect.w/source.y);let origin=rect.xy+(rect.zw-source*scale)*0.5;let xy=floor((pos.xy-origin)/scale);
 if any(xy<vec2(0.0))||any(xy>=source){return vec4(0.0,0.0,0.0,1.0);}
 let c=pixels[u32(xy.y)*u32(source.x)+u32(xy.x)];return vec4(f32((c>>16u)&255u),f32((c>>8u)&255u),f32(c&255u),255.0)/255.0;
}
"#;

/// A presentation surface plus the pipeline that draws the composited frame.
pub struct Presenter {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    dimensions: wgpu::Buffer,
    bound: Option<(wgpu::Buffer, wgpu::BindGroup)>,
    previous: [f32; 8],
    size: (u32, u32),
    /// Upload target for software-rendered frames (scene fallbacks).
    cpu: wgpu::Buffer,
    cpu_capacity: u64,
}

impl Presenter {
    /// Creates a compositor and presenter for a platform layer.
    ///
    /// # Safety
    /// `layer` must be a live `CAMetalLayer` that outlives the returned values.
    pub unsafe fn attach(
        layer: *mut c_void,
        width: u32,
        height: u32,
    ) -> Result<(GpuRenderer, Self), String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(layer))
        }
        .map_err(|e| e.to_string())?;
        let (width, height) = (width.max(1), height.max(1));
        let gpu = GpuRenderer::new_for_surface(width, height, instance, &surface)?;
        gpu.device.on_uncaptured_error(std::sync::Arc::new(|error| {
            crate::debug_log(&format!("wgpu: {error}"))
        }));
        let caps = surface.get_capabilities(&gpu.adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|format| !format.is_srgb())
            .ok_or("GPU cannot present an encoded RGB framebuffer to this layer")?;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            // Native iOS presentation is display-synced; the shared UI already
            // paces game steps.
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            color_space: wgpu::SurfaceColorSpace::Auto,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&gpu.device, &config);
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("mobile packed framebuffer presentation"),
                source: wgpu::ShaderSource::Wgsl(SHADER.into()),
            });
        let layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("mobile presentation buffers"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("mobile presentation layout"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("mobile present"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vertex"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fragment"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            });
        let dimensions = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mobile presentation dimensions"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let cpu_capacity = (u64::from(width) * u64::from(height) * 4).max(4);
        let cpu = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mobile software framebuffer upload"),
            size: cpu_capacity,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok((
            gpu,
            Self {
                surface,
                config,
                pipeline,
                layout,
                dimensions,
                bound: None,
                previous: [0.0; 8],
                size: (width, height),
                cpu,
                cpu_capacity,
            },
        ))
    }

    /// Resizes the drawable surface to the game view in physical pixels.
    pub fn resize(&mut self, gpu: &GpuRenderer, width: u32, height: u32) {
        let (width, height) = (width.max(1), height.max(1));
        if self.size == (width, height) {
            return;
        }
        self.size = (width, height);
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&gpu.device, &self.config);
        self.bound = None;
    }

    /// Draws the latest composited frame and presents it.
    pub fn present(&mut self, gpu: &mut GpuRenderer) -> Result<(), String> {
        let source = gpu.frame_buffer_ref().clone();
        let (width, height) = gpu.size();
        self.draw(gpu, &source, width, height)
    }

    /// Uploads and presents a CPU-composited frame, used when a scene needs the
    /// software reference renderer.
    pub fn present_bitmap(
        &mut self,
        gpu: &mut GpuRenderer,
        bitmap: &rsxp::graphics::Bitmap,
    ) -> Result<(), String> {
        let bytes = bitmap.pixels().len() as u64 * 4;
        if bytes > self.cpu_capacity {
            self.cpu_capacity = bytes.next_power_of_two();
            self.cpu = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("mobile software framebuffer upload"),
                size: self.cpu_capacity,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.bound = None;
        }
        gpu.queue
            .write_buffer(&self.cpu, 0, bytemuck::cast_slice(bitmap.pixels()));
        let (width, height) = (bitmap.width(), bitmap.height());
        self.draw(gpu, &self.cpu.clone(), width, height)
    }

    fn draw(
        &mut self,
        gpu: &mut GpuRenderer,
        source: &wgpu::Buffer,
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        let dimensions = [
            width as f32,
            height as f32,
            0.0,
            0.0,
            0.0,
            0.0,
            self.config.width as f32,
            self.config.height as f32,
        ];
        if dimensions != self.previous {
            gpu.queue
                .write_buffer(&self.dimensions, 0, bytemuck::cast_slice(&dimensions));
            self.previous = dimensions;
        }
        if self
            .bound
            .as_ref()
            .is_none_or(|(buffer, _)| buffer != source)
        {
            let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("mobile presentation bind group"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: source.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.dimensions.as_entire_binding(),
                    },
                ],
            });
            self.bound = Some((source.clone(), group));
        }
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(_) | wgpu::CurrentSurfaceTexture::Outdated => {
                crate::debug_log("draw surface lost/outdated");
                self.surface.configure(&gpu.device, &self.config);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Lost => return Err("GPU presentation surface lost".into()),
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err("GPU presentation surface validation failed".into());
            }
        };
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("mobile surface"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bound.as_ref().unwrap().1, &[]);
            pass.draw(0..3, 0..1);
        }
        gpu.queue.submit([encoder.finish()]);
        gpu.queue.present(frame);
        Ok(())
    }
}
