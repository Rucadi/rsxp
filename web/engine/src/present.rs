// SPDX-License-Identifier: GPL-2.0-or-later
//! wgpu presentation and the egui workspace for the browser frontend.
//!
//! The game is composed by the shared CPU renderer; wgpu uploads the finished
//! frame to a texture, draws it letterboxed into the workspace's game view, and
//! renders the shared egui workspace on top. On the web wgpu uses the browser's
//! WebGL2 backend, which the browser software-emulates when the GPU is disabled,
//! so no GPU is required (exactly like the egui.rs demo).
//!
//! `Presenter` owns the wgpu surface, the frame texture and the egui renderer,
//! and reuses the shared `rsxp_desktop::ui::Workspace` from the desktop app. This module
//! drains queued pointer/keyboard events into egui, tracks FPS, letterboxes the
//! game frame into the workspace's game rect, and exposes that rect so the page
//! routes pointer input to the game only when it is over the game view.
//! `upload_frame` pads rows to 256 bytes as the WebGPU backend requires for
//! `Queue::write_texture`.

use rsxp_core::graphics::{Bitmap, Rect};
use std::cell::RefCell;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

thread_local! {
    static PRESENTER: RefCell<Option<Presenter>> = const { RefCell::new(None) };
}

pub(crate) fn install(presenter: Presenter) {
    PRESENTER.with(|slot| *slot.borrow_mut() = Some(presenter));
}

pub(crate) fn is_active() -> bool {
    PRESENTER.with(|slot| slot.borrow().is_some())
}

pub(crate) fn with_presenter<R>(f: impl FnOnce(&mut Presenter) -> R) -> Option<R> {
    PRESENTER.with(|slot| slot.borrow_mut().as_mut().map(f))
}

fn call_global(name: &str) {
    if let Some(function) = web_sys::window()
        .and_then(|window| window.get(name))
        .and_then(|value| value.dyn_into::<js_sys::Function>().ok())
    {
        let _ = function.call0(&JsValue::NULL);
    }
}

#[derive(Default)]
struct InputState {
    pointer: Option<egui::Pos2>,
    events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
}

pub struct Presenter {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    dimensions: wgpu::Buffer,
    sampler: wgpu::Sampler,
    bind_group: Option<wgpu::BindGroup>,
    frame: Option<(u32, u32, wgpu::Texture, wgpu::TextureView)>,
    frame_bytes: Vec<u8>,
    context: egui::Context,
    renderer: egui_wgpu::Renderer,
    workspace: rsxp_desktop::ui::Workspace,
    input: InputState,
    size: (u32, u32),
    pixels_per_point: f32,
    game_rect: (u32, u32, u32, u32),
    frames: u64,
    fps: f64,
    fps_window: f64,
    fps_frames: u32,
    adapter: String,
}

impl Presenter {
    pub(crate) async fn new(
        canvas: HtmlCanvasElement,
        size: (u32, u32),
    ) -> Result<Self, String> {
        // Render through WebGL2. The workspace needs no compute, so the WebGL2
        // backend is sufficient and it is software-backed when the browser's GPU
        // is disabled (exactly like the egui.rs demo). WebGPU is deliberately not
        // requested: wgpu's web dispatch would claim the canvas with a `webgpu`
        // context, which then makes `getContext("webgl2")` fail ("canvas already
        // in use"), so there is no way to fall back on the same canvas.
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::GL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
            .map_err(|error| error.to_string())?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .map_err(|error| error.to_string())?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("rsxp browser presentation"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                    .using_resolution(adapter.limits()),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|error| error.to_string())?;
        let adapter_name = adapter.get_info().name;

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|format| !format.is_srgb())
            .ok_or("GPU cannot present an encoded RGB framebuffer to this canvas")?;
        let size = (size.0.max(1), size.1.max(1));
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.0,
            height: size.1,
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            color_space: wgpu::SurfaceColorSpace::Auto,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("framebuffer presentation"),
            source: wgpu::ShaderSource::Wgsl(
                r#"
@group(0) @binding(0) var frame: texture_2d<f32>;
@group(0) @binding(1) var frame_sampler: sampler;
// dimensions[0] = source size, dimensions[1] = target rect (x, y, w, h).
@group(0) @binding(2) var<uniform> dimensions: array<vec4<f32>, 2>;
@vertex fn vertex(@builtin(vertex_index) id: u32) -> @builtin(position) vec4<f32> {
  let x = f32((id << 1u) & 2u);
  let y = f32(id & 2u);
  return vec4(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}
@fragment fn fragment(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
  let source = dimensions[0].xy;
  let rect = dimensions[1];
  if any(pos.xy < rect.xy) || any(pos.xy >= rect.xy + rect.zw) {
    return vec4(0.0, 0.0, 0.0, 1.0);
  }
  let scale = min(rect.z / source.x, rect.w / source.y);
  let origin = rect.xy + (rect.zw - source * scale) * 0.5;
  let xy = (pos.xy - origin) / scale;
  if any(xy < vec2(0.0)) || any(xy >= source) {
    return vec4(0.0, 0.0, 0.0, 1.0);
  }
  return textureSampleLevel(frame, frame_sampler, xy / source, 0.0);
}
"#
                .into(),
            ),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
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
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
            multiview_mask: None,
            cache: None,
        });
        let dimensions = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("presentation dimensions"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("framebuffer sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let renderer = egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());

        Ok(Self {
            surface,
            config,
            device,
            queue,
            pipeline,
            layout,
            dimensions,
            sampler,
            bind_group: None,
            frame: None,
            frame_bytes: Vec::new(),
            context: egui::Context::default(),
            renderer,
            workspace: {
                let mut workspace = rsxp_desktop::ui::Workspace::new();
                let mut services = rsxp_desktop::workspace_services::LocalServices::default();
                services.image_reader = Some(|path| {
                    rsxp_core::engine::with_runtime(|runtime| runtime.asset(path).ok()).flatten()
                });
                workspace.backend = Box::new(services);
                workspace
            },
            input: InputState::default(),
            size,
            pixels_per_point: 1.0,
            game_rect: (0, 0, size.0, size.1),
            frames: 0,
            fps: 0.0,
            fps_window: 0.0,
            fps_frames: 0,
            adapter: adapter_name,
        })
    }

    pub(crate) fn game_stopped(&mut self, message: &str) {
        self.workspace.game_stopped(message);
    }
    pub(crate) fn log(&mut self, text: &str) {
        self.workspace.logs.push(text.into());
        if self.workspace.logs.len() > 2000 { self.workspace.logs.drain(..1000); }
    }

    pub(crate) fn adapter_name(&self) -> &str {
        &self.adapter
    }

    pub(crate) fn set_size(&mut self, size: (u32, u32)) {
        self.size = (size.0.max(1), size.1.max(1));
    }

    pub(crate) fn set_pixels_per_point(&mut self, pixels_per_point: f32) {
        if pixels_per_point.is_finite() && pixels_per_point > 0.0 {
            self.pixels_per_point = pixels_per_point;
        }
    }

    pub(crate) fn set_loading(&mut self, done: u32, total: u32) {
        self.workspace.set_loading(done, total);
    }

    pub(crate) fn game_rect(&self) -> (u32, u32, u32, u32) {
        self.game_rect
    }

    fn logical_point(&self, x: f32, y: f32) -> egui::Pos2 {
        let scale = if self.pixels_per_point > 0.0 {
            self.pixels_per_point
        } else {
            1.0
        };
        egui::pos2(x / scale, y / scale)
    }

    pub(crate) fn queue_pointer(&mut self, x: f32, y: f32) {
        let point = self.logical_point(x, y);
        self.input.pointer = Some(point);
        self.input.events.push(egui::Event::PointerMoved(point));
    }

    pub(crate) fn queue_button(
        &mut self,
        button: egui::PointerButton,
        pressed: bool,
        x: f32,
        y: f32,
    ) {
        let point = self.logical_point(x, y);
        self.input.pointer = Some(point);
        self.input.events.push(egui::Event::PointerButton {
            pos: point,
            button,
            pressed,
            modifiers: self.input.modifiers,
        });
    }

    pub(crate) fn queue_scroll(&mut self, delta: f32) {
        let point = self.input.pointer.unwrap_or(egui::Pos2::ZERO);
        self.input.events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, delta),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: self.input.modifiers,
        });
        self.input.events.push(egui::Event::PointerMoved(point));
    }

    pub(crate) fn queue_key(&mut self, key: egui::Key, pressed: bool) {
        self.input.events.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: self.input.modifiers,
        });
    }

    pub(crate) fn present(&mut self, frame: &Bitmap) -> Result<(), String> {
        if (self.config.width, self.config.height) != self.size {
            self.config.width = self.size.0;
            self.config.height = self.size.1;
            self.surface.configure(&self.device, &self.config);
        }
        self.frames += 1;
        self.fps_frames += 1;
        let now = js_sys::Date::now() / 1000.0;
        if self.fps_window == 0.0 {
            self.fps_window = now;
        } else if now - self.fps_window >= 0.5 {
            self.fps = f64::from(self.fps_frames) / (now - self.fps_window);
            self.fps_frames = 0;
            self.fps_window = now;
        }

        let surface_texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture) => texture,
            wgpu::CurrentSurfaceTexture::Suboptimal(_) | wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
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

        let (mut output, ui) = self.run_ui((frame.width(), frame.height()));
        if ui.export_saves {
            call_global("__rsxp_exportSaves");
        }
        if ui.import_saves {
            call_global("__rsxp_importSaves");
        }
        if let Err(error) = self.upload_frame(frame, ui.game_rect) {
            output.drop_without_applying_deltas();
            return Err(error);
        }
        let paint_jobs = self
            .context
            .tessellate(output.shapes, output.pixels_per_point);

        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.size.0, self.size.1],
            pixels_per_point: output.pixels_per_point,
        };
        for (id, delta) in output
            .textures_delta
            .set
            .iter()
            .flat_map(|(id, deltas)| deltas.iter().map(move |delta| (id, delta)))
        {
            self.renderer
                .update_texture(&self.device, &self.queue, *id, delta);
        }
        let user_commands = self.renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            &paint_jobs,
            &screen_descriptor,
        );
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("canvas"),
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
            if let Some(bind_group) = &self.bind_group {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
            self.renderer
                .render(&mut pass.forget_lifetime(), &paint_jobs, &screen_descriptor);
        }
        self.queue
            .submit(user_commands.into_iter().chain([encoder.finish()]));
        for id in &output.textures_delta.free {
            self.renderer.free_texture(id);
        }
        output.textures_delta.clear();
        self.queue.present(surface_texture);
        Ok(())
    }

    fn run_ui(
        &mut self,
        logical_size: (u32, u32),
    ) -> (egui::FullOutput, rsxp_desktop::ui::WorkspaceOutput) {
        let pixels_per_point = self.pixels_per_point;
        let logical = egui::vec2(
            self.size.0 as f32 / pixels_per_point,
            self.size.1 as f32 / pixels_per_point,
        );
        let events = std::mem::take(&mut self.input.events);
        let mut raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, logical)),
            time: Some(js_sys::Date::now() / 1000.0),
            predicted_dt: 1.0 / 60.0,
            events,
            ..Default::default()
        };
        let mut viewport = egui::ViewportInfo::default();
        viewport.native_pixels_per_point = Some(pixels_per_point);
        raw_input
            .viewports
            .insert(egui::ViewportId::ROOT, viewport);
        let info = rsxp_desktop::ui::WorkspaceInfo {
            logical_size,
            fps: self.fps,
            frames: self.frames,
            global_fullscreen: web_sys::window()
                .and_then(|window| window.document())
                .is_some_and(|document| document.fullscreen_element().is_some()),
            show_fullscreen: false,
            show_save_buttons: true,
            // The browser host has no per-frame Ruby/bridge timings.
            show_profile_details: false,
            profile: None,
        };
        let context = self.context.clone();
        let workspace = &mut self.workspace;
        let mut result = None;
        let output = context.run_pass(raw_input, |context| {
            result = Some(workspace.draw(context, &info));
        });
        (output, result.unwrap_or_default())
    }

    fn upload_frame(&mut self, frame: &Bitmap, panel: egui::Rect) -> Result<(), String> {
        let (width, height) = (frame.width(), frame.height());
        frame.write_rgba(&mut self.frame_bytes);
        let pixels_per_point = self.pixels_per_point;
        let frame_width = width as f32;
        let frame_height = height as f32;
        let scale = (panel.width() / frame_width)
            .min(panel.height() / frame_height)
            .max(0.0);
        let draw_width = frame_width * scale;
        let draw_height = frame_height * scale;
        let draw_x = panel.min.x + (panel.width() - draw_width) * 0.5;
        let draw_y = panel.min.y + (panel.height() - draw_height) * 0.5;
        let rect = Rect::new(
            (draw_x * pixels_per_point).round() as i32,
            (draw_y * pixels_per_point).round() as i32,
            (draw_width * pixels_per_point).round().max(0.0) as u32,
            (draw_height * pixels_per_point).round().max(0.0) as u32,
        );
        self.game_rect = (rect.x.max(0) as u32, rect.y.max(0) as u32, rect.width, rect.height);
        let dimensions = [
            width as f32,
            height as f32,
            0.0,
            0.0,
            rect.x as f32,
            rect.y as f32,
            rect.width as f32,
            rect.height as f32,
        ];
        self.queue
            .write_buffer(&self.dimensions, 0, bytemuck::cast_slice(&dimensions));
        match &self.frame {
            Some((w, h, _, _)) if *w == width && *h == height => {}
            _ => {
                let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("game frame"),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                self.bind_group = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: self.dimensions.as_entire_binding(),
                        },
                    ],
                }));
                self.frame = Some((width, height, texture, view));
            }
        }
        if let Some((_, _, texture, _)) = &self.frame {
            // `Queue::write_texture` rows must be aligned to 256 bytes on the
            // WebGPU backend (the browser API enforces it), so pad the copy when
            // the frame width is not a multiple of 64 pixels.
            let row_bytes = (width * 4) as usize;
            let aligned_row_bytes =
                (row_bytes as u32).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) as usize
                    * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
            let mut padded;
            let source: &[u8] = if aligned_row_bytes == row_bytes {
                &self.frame_bytes
            } else {
                padded = vec![0u8; aligned_row_bytes * height as usize];
                for row in 0..height as usize {
                    padded[row * aligned_row_bytes..row * aligned_row_bytes + row_bytes]
                        .copy_from_slice(&self.frame_bytes[row * row_bytes..(row + 1) * row_bytes]);
                }
                &padded
            };
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                source,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(aligned_row_bytes as u32),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }
        Ok(())
    }
}

/// Maps a DOM `KeyboardEvent.code` to an egui key for the workspace shortcuts.
pub(crate) fn egui_key_from_code(code: &str) -> Option<egui::Key> {
    use egui::Key;
    Some(match code {
        "F1" => Key::F1,
        "F2" => Key::F2,
        "F3" => Key::F3,
        "F10" => Key::F10,
        "F11" => Key::F11,
        &_ => return None,
    })
}
