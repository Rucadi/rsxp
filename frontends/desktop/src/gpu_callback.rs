// SPDX-License-Identifier: GPL-2.0-or-later
//! Draws the runner's packed GPU framebuffer inside eframe's render pass.
use rsxp::gpu::GpuRenderer;
use std::sync::{Arc, Mutex};

pub(crate) struct GameGpu {
    pub renderer: Option<GpuRenderer>,
    pub rect: egui::Rect,
    pub logical_size: (u32, u32),
}

#[derive(Clone)]
pub(crate) struct GameCallback {
    state: Arc<Mutex<GameGpu>>,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    dimensions: wgpu::Buffer,
    bindings: Arc<Mutex<Option<((u32, u32), wgpu::BindGroup)>>>,
}

impl GameCallback {
    pub fn new(
        device: &wgpu::Device,
        state: Arc<Mutex<GameGpu>>,
        format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("RSXP eframe game surface"),
            source: wgpu::ShaderSource::Wgsl(
                r#"
struct Dims { source: vec2<f32>, rect: vec4<f32> };
@group(0) @binding(0) var<storage,read> pixels: array<u32>;
@group(0) @binding(1) var<uniform> dims: Dims;
@vertex fn vertex(@builtin(vertex_index) id:u32)->@builtin(position) vec4<f32> {
 let x=f32((id<<1u)&2u); let y=f32(id&2u); return vec4(x*2.0-1.0,y*2.0-1.0,0.0,1.0);
}
@fragment fn fragment(@builtin(position) pos:vec4<f32>)->@location(0) vec4<f32> {
 let rect=dims.rect; let source=dims.source;
 if any(pos.xy<rect.xy)||any(pos.xy>=rect.xy+rect.zw) { return vec4(0.0,0.0,0.0,1.0); }
 let scale=min(rect.z/source.x,rect.w/source.y);
 let origin=rect.xy+(rect.zw-source*scale)*0.5;
 let xy=floor((pos.xy-origin)/scale);
 if any(xy<vec2(0.0))||any(xy>=source) { return vec4(0.0,0.0,0.0,1.0); }
 let c=pixels[u32(xy.y)*u32(source.x)+u32(xy.x)];
 return vec4(f32((c>>16u)&255u),f32((c>>8u)&255u),f32(c&255u),255.0)/255.0;
}
"#
                .into(),
            ),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RSXP game framebuffer layout"),
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
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("RSXP game surface pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("RSXP game framebuffer pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
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
            multiview_mask: None,
            cache: None,
        });
        let dimensions = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RSXP game framebuffer dimensions"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            state,
            pipeline,
            layout,
            dimensions,
            bindings: Arc::new(Mutex::new(None)),
        }
    }

    pub fn paint_callback(self: &Arc<Self>, rect: egui::Rect) -> egui::PaintCallback {
        egui_wgpu::Callback::new_paint_callback(rect, self.as_ref().clone())
    }
}

impl egui_wgpu::CallbackTrait for GameCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        _resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let state = self.state.lock().unwrap();
        let Some(renderer) = &state.renderer else {
            return Vec::new();
        };
        let ppp = screen.pixels_per_point;
        let rect = state.rect;
        let vals: [f32; 8] = [
            state.logical_size.0 as f32,
            state.logical_size.1 as f32,
            0.0,
            0.0,
            rect.min.x * ppp,
            rect.min.y * ppp,
            rect.width() * ppp,
            rect.height() * ppp,
        ];
        queue.write_buffer(&self.dimensions, 0, bytemuck::cast_slice(&vals));
        let size = renderer.size();
        let mut bindings = self.bindings.lock().unwrap();
        if bindings
            .as_ref()
            .is_none_or(|(old_size, _)| *old_size != size)
        {
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("RSXP game framebuffer bind group"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: renderer.frame_buffer_ref().as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.dimensions.as_entire_binding(),
                    },
                ],
            });
            *bindings = Some((size, bind));
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        _resources: &egui_wgpu::CallbackResources,
    ) {
        let bindings = self.bindings.lock().unwrap();
        if let Some((_, bindings)) = bindings.as_ref() {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, bindings, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}
