//! Direct WGPU presentation for the RGSS framebuffer.
//!
//! `PresentedFrame` already owns tightly packed RGBA/BGRA bytes.  This callback
//! keeps one GPU texture alive in egui-wgpu's callback resource map and updates
//! it with `Queue::write_texture` whenever the worker publishes a new frame.
//! The game is then drawn by a tiny fullscreen triangle in the same render pass
//! as the dashboard, with no `Color32` conversion or intermediate egui texture.

use super::PresentedFrame;
use eframe::egui_wgpu::{Callback, CallbackResources, CallbackTrait};
use eframe::wgpu;
use egui::PaintCallbackInfo;
use std::sync::{Arc, Mutex};

struct GameResources {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    texture: wgpu::Texture,
    format_uniform: wgpu::Buffer,
    width: u32,
    height: u32,
    revision: u64,
    bgra: bool,
}

struct GameCallback {
    frame: Arc<Mutex<PresentedFrame>>,
    target_format: wgpu::TextureFormat,
}

impl GameResources {
    fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("FANPLAY game frame shader"),
            source: wgpu::ShaderSource::Wgsl(
                r#"
                    struct VertexOut {
                        @builtin(position) position: vec4<f32>,
                        @location(0) uv: vec2<f32>,
                    };

                    @vertex
                    fn vs(@builtin(vertex_index) index: u32) -> VertexOut {
                        var positions = array<vec2<f32>, 3>(
                            vec2<f32>(-1.0, -1.0),
                            vec2<f32>( 3.0, -1.0),
                            vec2<f32>(-1.0,  3.0),
                        );
                        var uvs = array<vec2<f32>, 3>(
                            vec2<f32>(0.0, 1.0),
                            vec2<f32>(2.0, 1.0),
                            vec2<f32>(0.0, -1.0),
                        );
                        var out: VertexOut;
                        out.position = vec4<f32>(positions[index], 0.0, 1.0);
                        out.uv = uvs[index];
                        return out;
                    }

                    @group(0) @binding(0) var game_texture: texture_2d<f32>;
                    @group(0) @binding(1) var game_sampler: sampler;
                    @group(0) @binding(2) var<uniform> bgra: u32;

                    @fragment
                    fn fs(input: VertexOut) -> @location(0) vec4<f32> {
                        var color = textureSample(game_texture, game_sampler, input.uv);
                        if (bgra != 0u) {
                            color = color.bgra;
                        }
                        return color;
                    }
                "#
                .into(),
            ),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("FANPLAY game frame bindings"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: Some(std::num::NonZeroU64::new(4).unwrap()),
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("FANPLAY game frame pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("FANPLAY game frame pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        // A 1-word uniform is valid in a 256-byte uniform buffer allocation.
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("FANPLAY game frame format"),
            size: 256,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("FANPLAY game frame texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: if target_format.is_srgb() {
                wgpu::TextureFormat::Rgba8UnormSrgb
            } else {
                wgpu::TextureFormat::Rgba8Unorm
            },
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("FANPLAY game frame sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("FANPLAY game frame bind group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
        // Keep the uniform alive through the bind group; it selects the one
        // byte-order swizzle used by the fragment shader.
        Self {
            pipeline,
            bind_group,
            texture,
            format_uniform: uniform,
            width,
            height,
            revision: 0,
            bgra: false,
        }
    }
}

impl CallbackTrait for GameCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &eframe::egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if callback_resources.get::<GameResources>().is_none() {
            callback_resources.insert(GameResources::new(device, self.target_format, 1, 1));
        }
        let resources = callback_resources
            .get_mut::<GameResources>()
            .expect("game WGPU resources were inserted");
        let frame = self.frame.lock().unwrap_or_else(|error| error.into_inner());
        if frame.revision == 0 || frame.width == 0 || frame.height == 0 {
            return Vec::new();
        }
        let Some(byte_count) = (frame.width as usize)
            .checked_mul(frame.height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
        else {
            return Vec::new();
        };
        if frame.pixels.len() < byte_count {
            return Vec::new();
        }
        if resources.width != frame.width || resources.height != frame.height {
            // The bind group contains the texture view, so recreate the resource
            // only when the RGSS logical framebuffer changes dimensions.
            *resources = GameResources::new(device, self.target_format, frame.width, frame.height);
        }
        let bgra = frame.format == fanplay_core::PixelFormat::Bgra8;
        if resources.revision != frame.revision || resources.bgra != bgra {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &resources.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &frame.pixels[..byte_count],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(frame.width * 4),
                    rows_per_image: Some(frame.height),
                },
                wgpu::Extent3d {
                    width: frame.width,
                    height: frame.height,
                    depth_or_array_layers: 1,
                },
            );
            queue.write_buffer(
                &resources.format_uniform,
                0,
                bytemuck::bytes_of(&u32::from(bgra)),
            );
            resources.revision = frame.revision;
            resources.bgra = bgra;
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &CallbackResources,
    ) {
        let Some(resources) = callback_resources.get::<GameResources>() else {
            return;
        };
        render_pass.set_pipeline(&resources.pipeline);
        render_pass.set_bind_group(0, &resources.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

pub fn callback(
    frame: Arc<Mutex<PresentedFrame>>,
    target_format: wgpu::TextureFormat,
) -> egui::PaintCallback {
    Callback::new_paint_callback(
        egui::Rect::ZERO,
        GameCallback {
            frame,
            target_format,
        },
    )
}
