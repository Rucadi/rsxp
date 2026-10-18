// SPDX-License-Identifier: GPL-2.0-or-later
//! A wgpu surface on a winit-owned native window.
use rsxp::gpu::GpuRenderer;
use rsxp::graphics::Bitmap;
use rsxp::host::ProfileStats;
use std::time::{Duration, Instant};

/// Window and workspace state supplied for one presentation. Both software
/// framebuffer uploads and GPU composition use the same presentation contract.
pub struct Presentation<'a> {
    pub size: (u32, u32),
    pub context: &'a egui::Context,
    pub input: egui::RawInput,
    pub global_fullscreen: bool,
    pub profile: Option<ProfileStats>,
}

pub struct GpuDisplay {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    dimensions: wgpu::Buffer,
    cpu_frame: wgpu::Buffer,
    cpu_capacity: u64,
    cpu_revision: Option<u64>,
    egui_renderer: egui_wgpu::Renderer,
    fps_start: Instant,
    fps_frames: u32,
    fps: f64,
    game_metrics: Option<(f64, u64)>,
    frames: u64,
    bound_frame: Option<(wgpu::Buffer, wgpu::BindGroup)>,
    previous_dimensions: [f32; 8],
    // The workspace UI is shared with the browser frontend.
    workspace: rsxp::ui::Workspace,
    global_fullscreen_request: Option<bool>,
    game_pointer_rect: egui::Rect,
}

impl GpuDisplay {
    pub fn set_game_metrics(&mut self, fps: f64, frames: u64) {
        self.game_metrics = Some((fps, frames));
    }
    pub fn workspace_mut(&mut self) -> &mut rsxp::ui::Workspace {
        &mut self.workspace
    }
    pub fn profiler_visible(&self) -> bool {
        self.workspace.profiler_visible()
    }

    pub fn take_global_fullscreen_request(&mut self) -> Option<bool> {
        self.global_fullscreen_request.take()
    }

    pub fn game_pointer_rect(&self) -> egui::Rect {
        self.game_pointer_rect
    }

    pub fn new(
        gpu: &GpuRenderer,
        surface: wgpu::Surface<'static>,
        size: (u32, u32),
    ) -> Result<Self, String> {
        let caps = surface.get_capabilities(&gpu.adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .ok_or("GPU cannot present an encoded RGB framebuffer to this window")?;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.0,
            height: size.1,
            present_mode: wgpu::PresentMode::AutoNoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&gpu.device, &config);
        let shader=gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("packed framebuffer presentation"),source:wgpu::ShaderSource::Wgsl(r#"
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
"#.into())});
        let layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
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
                label: None,
                bind_group_layouts: &[&layout],
                push_constant_ranges: &[],
            });
        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("present"),
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
                multiview: None,
                cache: None,
            });
        let dimensions = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("presentation dimensions"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let cpu_capacity = (u64::from(size.0) * u64::from(size.1) * 4).max(4);
        let cpu_frame = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("software framebuffer upload"),
            size: cpu_capacity,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let egui_renderer =
            egui_wgpu::Renderer::new(&gpu.device, format, egui_wgpu::RendererOptions::default());
        Ok(Self {
            surface,
            config,
            pipeline,
            layout,
            dimensions,
            cpu_frame,
            cpu_capacity,
            cpu_revision: None,
            egui_renderer,
            fps_start: Instant::now(),
            fps_frames: 0,
            fps: 0.0,
            game_metrics: None,
            frames: 0,
            bound_frame: None,
            previous_dimensions: [0.0; 8],
            workspace: rsxp::ui::Workspace::new(),
            global_fullscreen_request: None,
            game_pointer_rect: egui::Rect::ZERO,
        })
    }
    pub fn present(
        &mut self,
        gpu: &GpuRenderer,
        presentation: Presentation<'_>,
    ) -> Result<egui::PlatformOutput, String> {
        self.present_buffer(
            gpu,
            gpu.frame_buffer_ref().clone(),
            gpu.size(),
            presentation,
        )
    }

    pub fn present_bitmap(
        &mut self,
        gpu: &GpuRenderer,
        bitmap: &Bitmap,
        presentation: Presentation<'_>,
    ) -> Result<egui::PlatformOutput, String> {
        let bytes = bytemuck::cast_slice(bitmap.pixels());
        let needed = bytes.len().max(4) as u64;
        if needed > self.cpu_capacity {
            self.cpu_capacity = needed.next_power_of_two();
            self.cpu_frame = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("software framebuffer upload"),
                size: self.cpu_capacity,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if self.cpu_revision != Some(bitmap.revision()) {
            gpu.queue.write_buffer(&self.cpu_frame, 0, bytes);
            self.cpu_revision = Some(bitmap.revision());
        }
        self.present_buffer(
            gpu,
            self.cpu_frame.clone(),
            (bitmap.width(), bitmap.height()),
            presentation,
        )
    }

    fn present_buffer(
        &mut self,
        gpu: &GpuRenderer,
        source: wgpu::Buffer,
        logical_size: (u32, u32),
        presentation: Presentation<'_>,
    ) -> Result<egui::PlatformOutput, String> {
        let Presentation {
            size,
            context,
            input,
            global_fullscreen,
            profile,
        } = presentation;
        if size.0 == 0 || size.1 == 0 {
            return Ok(Default::default());
        }
        if (self.config.width, self.config.height) != size {
            self.config.width = size.0;
            self.config.height = size.1;
            self.surface.configure(&gpu.device, &self.config);
        }
        let frame = match self.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.surface.configure(&gpu.device, &self.config);
                return Ok(Default::default());
            }
            Err(wgpu::SurfaceError::Timeout) => return Ok(Default::default()),
            Err(error) => return Err(error.to_string()),
        };

        self.fps_frames += 1;
        self.frames += 1;
        let fps_elapsed = self.fps_start.elapsed();
        if fps_elapsed >= Duration::from_millis(500) {
            self.fps = f64::from(self.fps_frames) / fps_elapsed.as_secs_f64();
            self.fps_frames = 0;
            self.fps_start = Instant::now();
        }
        let workspace_info = rsxp::ui::WorkspaceInfo {
            logical_size,
            fps: self.game_metrics.map_or(self.fps, |m| m.0),
            frames: self.game_metrics.map_or(self.frames, |m| m.1),
            global_fullscreen,
            show_fullscreen: true,
            show_save_buttons: false,
            show_profile_details: true,
            profile: profile.as_ref(),
        };
        let mut game_rect = egui::Rect::ZERO;
        let mut fullscreen_request = None;
        let output = context.run(input, |context| {
            let result = self.workspace.draw(context, &workspace_info);
            game_rect = result.game_rect;
            fullscreen_request = result.fullscreen_request;
        });
        self.global_fullscreen_request = fullscreen_request;
        self.game_pointer_rect = game_rect;
        let pixels_per_point = output.pixels_per_point;
        let x = (game_rect.min.x * pixels_per_point).clamp(0.0, size.0 as f32);
        let y = (game_rect.min.y * pixels_per_point).clamp(0.0, size.1 as f32);
        let target_rect = [
            x,
            y,
            (game_rect.width() * pixels_per_point)
                .max(0.0)
                .min(size.0 as f32 - x),
            (game_rect.height() * pixels_per_point)
                .max(0.0)
                .min(size.1 as f32 - y),
        ];
        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [size.0, size.1],
            pixels_per_point: output.pixels_per_point,
        };
        let paint_jobs = context.tessellate(output.shapes, output.pixels_per_point);
        let dimensions = [
            logical_size.0 as f32,
            logical_size.1 as f32,
            0.0,
            0.0,
            target_rect[0],
            target_rect[1],
            target_rect[2],
            target_rect[3],
        ];
        if dimensions != self.previous_dimensions {
            gpu.queue
                .write_buffer(&self.dimensions, 0, bytemuck::cast_slice(&dimensions));
            self.previous_dimensions = dimensions;
        }
        if self
            .bound_frame
            .as_ref()
            .is_none_or(|(buffer, _)| buffer != &source)
        {
            let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
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
            self.bound_frame = Some((source, group));
        }
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        for (id, delta) in &output.textures_delta.set {
            self.egui_renderer
                .update_texture(&gpu.device, &gpu.queue, *id, delta);
        }
        let user_commands = self.egui_renderer.update_buffers(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &paint_jobs,
            &screen_descriptor,
        );
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("winit surface"),
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
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bound_frame.as_ref().unwrap().1, &[]);
            pass.draw(0..3, 0..1);
            self.egui_renderer
                .render(&mut pass.forget_lifetime(), &paint_jobs, &screen_descriptor);
        }
        gpu.queue
            .submit(user_commands.into_iter().chain([encoder.finish()]));
        for id in &output.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }
        frame.present();
        Ok(output.platform_output)
    }
}
