// SPDX-License-Identifier: GPL-3.0-or-later
//! Synchronous software 3D bridge; all access stays on the core thread.
use dust_core::gpu::{
    engine_3d::{Polygon, RendererTx, RenderingState, ScreenVertex, SoftRendererRx},
    Scanline, SCREEN_HEIGHT,
};
use dust_core::utils::Bytes;
use dust_soft_3d::{Renderer, RenderingData};
use std::{cell::RefCell, rc::Rc};

pub struct Renderers {
    pub tx: Box<dyn dust_core::gpu::engine_3d::RendererTx>,
    pub rx: Box<dyn dust_core::gpu::engine_2d::Renderer>,
    pub gpu_output: Option<GpuOutput>,
}

pub struct GpuOutput {
    #[cfg(not(target_arch = "wasm32"))]
    channels: dust_wgpu_2d::FrontendChannels,
    #[cfg(not(target_arch = "wasm32"))]
    _three_d_channels: dust_wgpu_3d::threaded::FrontendChannels,
    #[cfg(not(target_arch = "wasm32"))]
    last_frame: Option<u64>,
}

impl GpuOutput {
    pub fn next_rgba(&mut self) -> Option<Vec<u8>> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (index, frame) = self.channels.wait_newer_frame(self.last_frame);
            self.last_frame = Some(index);
            Some(frame)
        }
        #[cfg(target_arch = "wasm32")]
        {
            None
        }
    }
}
struct Shared {
    data: Box<RenderingData>,
    renderer: Renderer,
    lines: Box<[Scanline<u32>; SCREEN_HEIGHT]>,
}
struct Tx(Rc<RefCell<Shared>>);
struct Rx {
    shared: Rc<RefCell<Shared>>,
    line: usize,
    out: Scanline<u32>,
}
fn new_software() -> (Box<dyn RendererTx>, Box<dyn SoftRendererRx>) {
    let shared = Rc::new(RefCell::new(Shared {
        // SAFETY: Dust initializes these numeric buffers identically in its frontends.
        data: unsafe { Box::new_zeroed().assume_init() },
        renderer: Renderer::new(),
        lines: unsafe { Box::new_zeroed().assume_init() },
    }));
    (
        Box::new(Tx(shared.clone())),
        Box::new(Rx {
            shared,
            line: 0,
            out: Scanline([0; 256]),
        }),
    )
}

pub fn new() -> Renderers {
    #[cfg(not(target_arch = "wasm32"))]
    if std::env::var("RSXP_NDS_RENDERER").as_deref() == Ok("wgpu") {
        match new_wgpu() {
            Ok(renderers) => {
                eprintln!("Nintendo DS: Dust wgpu 2D/3D renderers enabled");
                return renderers;
            }
            Err(error) => eprintln!("Dust wgpu renderer unavailable; using software: {error}"),
        }
    }
    let (tx, rx) = new_software();
    Renderers {
        tx,
        rx: Box::new(dust_soft_2d::sync::Renderer::new(rx)),
        gpu_output: None,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn new_wgpu() -> Result<Renderers, String> {
    use dust_wgpu_2d::Renderer3dRx;
    use std::sync::Arc;

    let instance = wgpu23::Instance::new(wgpu23::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu23::RequestAdapterOptions {
        power_preference: wgpu23::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
    }))
    .ok_or("no compatible wgpu adapter was found")?;
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu23::DeviceDescriptor::default(), None))
            .map_err(|error| error.to_string())?;
    let device = Arc::new(device);
    let queue = Arc::new(queue);
    let (tx_3d, rx_3d, three_d_channels, three_d_output) =
        dust_wgpu_3d::threaded::init(Arc::clone(&device), Arc::clone(&queue), 0);
    let (renderer_2d, _output_view, channels) =
        dust_wgpu_2d::threaded::lockstep_scanlines::Renderer::new(
            device,
            queue,
            0,
            Renderer3dRx::Accel {
                rx: Box::new(rx_3d),
                color_output_view: three_d_output.color_output_view,
                color_output_view_rx: three_d_output.color_output_view_rx,
                command_buffer_rx: three_d_output.command_buffer_rx,
                last_submitted_frame: three_d_output.last_submitted_frame,
            },
        );
    Ok(Renderers {
        tx: Box::new(tx_3d),
        rx: Box::new(renderer_2d),
        gpu_output: Some(GpuOutput {
            channels,
            _three_d_channels: three_d_channels,
            last_frame: None,
        }),
    })
}
impl RendererTx for Tx {
    fn set_capture_enabled(&mut self, _: bool) {}
    fn swap_buffers(&mut self, verts: &[ScreenVertex], polys: &[Polygon], state: &RenderingState) {
        self.0.borrow_mut().data.prepare(verts, polys, state);
    }
    fn repeat_last_frame(&mut self, state: &RenderingState) {
        self.0.borrow_mut().data.repeat_last_frame(state);
    }
    fn start_rendering(
        &mut self,
        texture: &Bytes<0x8_0000>,
        palette: &Bytes<0x1_8000>,
        state: &RenderingState,
    ) {
        let mut shared = self.0.borrow_mut();
        let Shared {
            data,
            renderer,
            lines,
        } = &mut *shared;
        data.copy_vram(texture, palette, state);
        renderer.start_frame(data);
        renderer.render_line(0, data);
        for y in 0..192u8 {
            if y < 191 {
                renderer.render_line(y + 1, data);
            }
            renderer.postprocess_line(y, &mut lines[y as usize], data);
        }
    }
    fn skip_rendering(&mut self) {}
}
impl SoftRendererRx for Rx {
    fn start_frame(&mut self) {
        self.line = 0;
    }
    fn read_scanline(&mut self) -> &Scanline<u32> {
        self.out
            .0
            .copy_from_slice(&self.shared.borrow().lines[self.line].0);
        self.line += 1;
        &self.out
    }
    fn skip_scanline(&mut self) {
        self.line += 1;
    }
}
