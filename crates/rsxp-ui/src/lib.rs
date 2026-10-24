// SPDX-License-Identifier: GPL-2.0-or-later
//! Shared eframe application and egui frontend for native and browser runners.
use rsxp::{
    graphics::Bitmap,
    ui::{Workspace, WorkspaceInfo, WorkspaceOutput},
};

/// Common frontend state. Platform runners update the frame and [`WorkspaceInfo`],
/// and execute the platform-neutral actions returned by the workspace.
pub struct SharedApp {
    pub workspace: Workspace,
    info: WorkspaceInfo,
    frame: Option<egui::TextureHandle>,
    frame_size: Option<[usize; 2]>,
    frame_revision: Option<u64>,
    context: Option<egui::Context>,
    output: WorkspaceOutput,
    rgba: Vec<u8>,
}

impl Default for SharedApp {
    fn default() -> Self {
        Self::new(
            Workspace::new(),
            WorkspaceInfo {
                logical_size: (640, 480),
                fps: 0.0,
                frames: 0,
                global_fullscreen: false,
                show_fullscreen: true,
                show_save_buttons: false,
            },
        )
    }
}

impl SharedApp {
    pub fn new(workspace: Workspace, info: WorkspaceInfo) -> Self {
        Self {
            workspace,
            info,
            frame: None,
            frame_size: None,
            frame_revision: None,
            context: None,
            output: WorkspaceOutput::default(),
            rgba: Vec::new(),
        }
    }

    pub fn set_info(&mut self, info: WorkspaceInfo) {
        self.info = info;
    }

    /// Updates the live performance figures shown by the workspace.
    ///
    /// Runners that don't have a complete [`WorkspaceInfo`] still need to
    /// report the FPS counters; keeping this separate avoids overwriting the
    /// frontend-specific flags when a frame arrives.
    pub fn set_performance(&mut self, fps: f64, frames: u64) {
        self.info.fps = fps;
        self.info.frames = frames;
    }

    /// True while an egui text field or other widget owns the keyboard.
    pub fn wants_keyboard(&self) -> bool {
        self.context
            .as_ref()
            .is_some_and(egui::Context::egui_wants_keyboard_input)
    }

    pub fn set_game_paint_callback(&mut self, callback: Option<egui::PaintCallback>) {
        self.workspace.set_game_paint_callback(callback);
    }

    pub fn set_logical_size(&mut self, size: (u32, u32)) {
        self.info.logical_size = size;
    }

    pub fn draw(&mut self, ui: &mut egui::Ui) {
        self.context = Some(ui.ctx().clone());
        self.workspace
            .set_frame_texture(self.frame.as_ref().map(egui::TextureHandle::id));
        self.output = self.workspace.draw(ui, &self.info);
        if let Some(fullscreen) = self.output.fullscreen_request {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Fullscreen(fullscreen));
        }
    }

    pub fn set_bitmap(&mut self, context: &egui::Context, bitmap: &Bitmap) {
        let revision = bitmap.revision();
        let size = [bitmap.width() as usize, bitmap.height() as usize];
        if self.frame_revision == Some(revision) && self.frame_size == Some(size) {
            return;
        }
        bitmap.write_rgba(&mut self.rgba);
        let image = egui::ColorImage::from_rgba_unmultiplied(size, &self.rgba);
        if self.frame_size != Some(size) {
            self.frame =
                Some(context.load_texture("rsxp-game-frame", image, egui::TextureOptions::NEAREST));
            self.frame_size = Some(size);
        } else if let Some(frame) = &mut self.frame {
            frame.set(image, egui::TextureOptions::NEAREST);
        }
        self.frame_revision = Some(revision);
        self.workspace
            .set_frame_texture(self.frame.as_ref().map(egui::TextureHandle::id));
        context.request_repaint();
    }

    pub fn update_bitmap(&mut self, bitmap: &Bitmap) {
        if let Some(context) = self.context.clone() {
            self.set_bitmap(&context, bitmap);
        }
    }

    pub fn game_rect(&self) -> egui::Rect {
        self.output.game_rect
    }

    pub fn request_repaint(&self) {
        if let Some(context) = &self.context {
            context.request_repaint();
        }
    }

    pub fn take_output(&mut self) -> WorkspaceOutput {
        std::mem::take(&mut self.output)
    }
}

impl eframe::App for SharedApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }

    #[cfg(target_arch = "wasm32")]
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub fn android_main(app: winit::platform::android::activity::AndroidApp) {
    let options = eframe::NativeOptions {
        android_app: Some(app),
        ..Default::default()
    };
    eframe::run_native(
        "RSXP",
        options,
        Box::new(|_| {
            let mut app = SharedApp::default();
            let mut panel = rsxp::ui::session::SessionPanel::default();
            panel.capabilities = rsxp::ui::session::SessionCapabilities::MOBILE;
            app.workspace.session = Some(panel);
            Ok(Box::new(app))
        }),
    )
    .unwrap();
}
