// SPDX-License-Identifier: GPL-2.0-or-later
//! Placement of the game view within the workspace.
//!
//! `draw_game_view` decides where the frontend should render the game and
//! reports that rectangle through `WorkspaceOutput::game_rect`:
//!
//! * when controls are hidden, or `view_mode` is not `ViewMode::Floating`, the
//!   game fills a borderless `CentralPanel` and its `max_rect` becomes the game
//!   rect (the common docked and workspace-fullscreen path);
//! * when controls are visible and `view_mode == ViewMode::Floating`, a
//!   placeholder is drawn in the central panel and the game instead occupies a
//!   movable, resizable egui window ("Game") whose available rect is recorded.
//!
//! The frontend reads `game_rect` each frame to position its own rendering and
//! input mapping.
use super::*;
impl Workspace {
    fn paint_game(&self, ui: &egui::Ui, rect: egui::Rect) {
        if let Some(callback) = &self.game_paint_callback {
            let mut callback = callback.clone();
            callback.rect = rect;
            ui.painter().add(egui::Shape::Callback(callback));
        } else if let Some(texture) = self.frame_texture {
            ui.painter().image(
                texture,
                rect,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
    }

    fn paint_ds_screen(&self, ui: &egui::Ui, rect: egui::Rect, top: bool) {
        if let Some(texture) = self.frame_texture {
            let uv = if top {
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 0.5))
            } else {
                egui::Rect::from_min_max(egui::pos2(0.0, 0.5), egui::pos2(1.0, 1.0))
            };
            ui.painter().image(texture, rect, uv, egui::Color32::WHITE);
        }
    }

    fn record_ds_rect(output: &mut WorkspaceOutput, rect: egui::Rect, top: bool) {
        output.game_rect = if output.game_rect == egui::Rect::ZERO {
            rect
        } else {
            output.game_rect.union(rect)
        };
        if !top {
            output.touch_rect = rect;
        }
    }

    fn ds_layout_rects(
        &self,
        available: egui::Rect,
    ) -> (Option<egui::Rect>, Option<egui::Rect>) {
        let (width, height) = match self.ds_layout {
            DsLayout::Vertical => (256, 384),
            DsLayout::Horizontal => (512, 192),
            DsLayout::TopLarge | DsLayout::TouchLarge => (768, 384),
            DsLayout::TopOnly | DsLayout::TouchOnly => (256, 192),
        };
        let canvas = if self.ds_layout == DsLayout::Horizontal && !self.controls_visible {
            // Horizontal DS mode is intentionally edge-to-edge vertically:
            // both LCDs fill the window height and meet its top and bottom.
            let scale = available.height() / height as f32;
            let size = egui::vec2(width as f32 * scale, available.height());
            egui::Rect::from_center_size(
                egui::pos2(available.center().x, available.min.y + size.y * 0.5),
                size,
            )
        } else {
            aspect_rect(available, (width, height))
        };
        let scale = (canvas.width() / width as f32).min(canvas.height() / height as f32);
        let at = |x: u32, y: u32, w: u32, h: u32| {
            egui::Rect::from_min_size(
                canvas.min + egui::vec2(x as f32 * scale, y as f32 * scale),
                egui::vec2(w as f32 * scale, h as f32 * scale),
            )
        };
        match self.ds_layout {
            DsLayout::Vertical => (Some(at(0, 0, 256, 192)), Some(at(0, 192, 256, 192))),
            DsLayout::Horizontal => (Some(at(0, 0, 256, 192)), Some(at(256, 0, 256, 192))),
            DsLayout::TopLarge => (Some(at(0, 0, 512, 384)), Some(at(512, 96, 256, 192))),
            DsLayout::TouchLarge => (Some(at(0, 96, 256, 192)), Some(at(256, 0, 512, 384))),
            DsLayout::TopOnly => (Some(at(0, 0, 256, 192)), None),
            DsLayout::TouchOnly => (None, Some(at(0, 0, 256, 192))),
        }
    }

    fn draw_ds_docked(&self, ui: &mut egui::Ui, output: &mut WorkspaceOutput) {
        let available = ui.max_rect();
        let (top, touch) = self.ds_layout_rects(available);
        if let Some(rect) = top {
            self.paint_ds_screen(ui, rect, true);
            Self::record_ds_rect(output, rect, true);
        }
        if let Some(rect) = touch {
            self.paint_ds_screen(ui, rect, false);
            Self::record_ds_rect(output, rect, false);
        }
    }

    pub(super) fn draw_game_view(
        &mut self,
        ui: &mut egui::Ui,
        output: &mut WorkspaceOutput,
        core_id: &str,
        logical_size: (u32, u32),
    ) {
        if core_id == "nds" && logical_size == (256, 384) {
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(theme::BG))
                .show(ui, |ui| {
                    self.draw_ds_docked(ui, output);
                });
            return;
        }
        let context = ui.ctx().clone();
        if self.controls_visible && self.view_mode == ViewMode::Floating {
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(theme::BG))
                .show(ui, |ui| {
                    ui.add_space(ui.available_height() * 0.18);
                    ui.vertical_centered(|ui| {
                        theme::section_label(ui, "Floating game", theme::CYAN);
                        ui.add_space(6.0);
                        ui.label(theme::display("The game is in its own window", 24.0));
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new("Drag or resize it anywhere. Choose Docked in Layout to bring it back here.")
                                .font(theme::body_font(13.0))
                                .color(theme::TEXT_MUTED),
                        );
                        ui.add_space(16.0);
                        if theme::button(ui, "Dock the game view", theme::ButtonKind::Secondary)
                            .clicked()
                        {
                            self.view_mode = ViewMode::Docked;
                        }
                    });
                });
            egui::Window::new("Game")
                .id(egui::Id::unique("floating-game-view"))
                .default_size(egui::vec2(640.0, 480.0))
                .min_size(egui::vec2(320.0, 240.0))
                .movable(true)
                .resizable(true)
                .frame(egui::Frame::window(&context.global_style()).fill(theme::BG))
                .show(&context, |ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
                    let game_rect = aspect_rect(rect, logical_size);
                    output.game_rect = game_rect;
                    self.paint_game(ui, game_rect);
                });
        } else {
            output.game_rect = egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(theme::BG))
                .show(ui, |ui| {
                    // Always preserve the core's native aspect ratio. The native
                    // frontend may resize the window to this ratio while the UI
                    // is hidden; it never stretches the framebuffer.
                    let game_rect = aspect_rect(ui.max_rect(), logical_size);
                    self.paint_game(ui, game_rect);
                    game_rect
                })
                .inner;
        }
    }
}

/// Fits the native framebuffer inside its available region without changing
/// its aspect ratio. The unused space is intentionally left as letterbox
/// margins so GBA (3:2) and DS (stacked 256:384) frames are not stretched.
fn aspect_rect(available: egui::Rect, logical_size: (u32, u32)) -> egui::Rect {
    let (width, height) = logical_size;
    if width == 0 || height == 0 || available.width() <= 0.0 || available.height() <= 0.0 {
        return available;
    }
    let scale = (available.width() / width as f32).min(available.height() / height as f32);
    let size = egui::vec2(width as f32 * scale, height as f32 * scale);
    egui::Rect::from_center_size(available.center(), size)
}

#[cfg(test)]
mod tests {
    use super::aspect_rect;

    #[test]
    fn aspect_rect_preserves_native_ratio_and_centers_letterbox() {
        let available =
            egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(1000.0, 700.0));
        let rect = aspect_rect(available, (240, 160));
        assert!((rect.width() / rect.height() - 1.5).abs() < 0.0001);
        assert!((rect.center().x - available.center().x).abs() < 0.0001);
        assert!((rect.center().y - available.center().y).abs() < 0.0001);
        assert!(rect.height() < available.height());
    }
}
