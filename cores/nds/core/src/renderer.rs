// SPDX-License-Identifier: GPL-3.0-or-later
//! Synchronous software 3D bridge; all access stays on the core thread.
use dust_core::gpu::{
    engine_3d::{Polygon, RendererTx, RenderingState, ScreenVertex, SoftRendererRx},
    Scanline, SCREEN_HEIGHT,
};
use dust_core::utils::Bytes;
use dust_soft_3d::{Renderer, RenderingData};
use std::{cell::RefCell, rc::Rc};
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
pub fn new() -> (Box<dyn RendererTx>, Box<dyn SoftRendererRx>) {
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
