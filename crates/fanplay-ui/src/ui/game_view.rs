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

    pub(super) fn draw_game_view(
        &mut self,
        ui: &mut egui::Ui,
        output: &mut WorkspaceOutput,
        info: &WorkspaceInfo,
    ) {
        if info.core_id == "nds" && info.logical_size == (256, 384) {
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(egui::Color32::BLACK))
                .show(ui, |ui| {
                    (output.game_rect, output.touch_rect) =
                        self.ds_layout
                            .paint(ui, self.frame_texture, self.controls_visible);
                    if self.show_fps_overlay() {
                        paint_fps_overlay(ui, output.game_rect, info.fps);
                    }
                });
            return;
        }
        let context = ui.ctx().clone();
        if self.controls_visible && self.view_mode == ViewMode::Floating {
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(egui::Color32::BLACK))
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
                .frame(egui::Frame::window(&context.global_style()).fill(egui::Color32::BLACK))
                .show(&context, |ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
                    let game_rect = aspect_rect(rect, info.logical_size);
                    output.game_rect = game_rect;
                    self.paint_game(ui, game_rect);
                });
        } else {
            output.game_rect = egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(egui::Color32::BLACK))
                .show(ui, |ui| {
                    // Always preserve the core's native aspect ratio. The native
                    // frontend may resize the window to this ratio while the UI
                    // is hidden; it never stretches the framebuffer.
                    let game_rect = aspect_rect(ui.max_rect(), info.logical_size);
                    self.paint_game(ui, game_rect);
                    if self.show_fps_overlay() {
                        paint_fps_overlay(ui, game_rect, info.fps);
                    }
                    game_rect
                })
                .inner;
        }
    }

    fn show_fps_overlay(&self) -> bool {
        !self.controls_visible || self.is_workspace_fullscreen()
    }
}

fn paint_fps_overlay(ui: &egui::Ui, game_rect: egui::Rect, fps: f64) {
    if !fps.is_finite() || fps <= 0.0 || game_rect.width() < 70.0 || game_rect.height() < 28.0 {
        return;
    }
    let badge =
        egui::Rect::from_min_size(game_rect.min + egui::vec2(8.0, 8.0), egui::vec2(78.0, 24.0));
    let painter = ui.painter();
    painter.rect_filled(
        badge,
        egui::CornerRadius::same(5),
        egui::Color32::from_black_alpha(190),
    );
    painter.rect_stroke(
        badge,
        egui::CornerRadius::same(5),
        egui::Stroke::new(1.0, theme::CYAN.gamma_multiply(0.8)),
        egui::StrokeKind::Inside,
    );
    painter.text(
        badge.center(),
        egui::Align2::CENTER_CENTER,
        format!("{:.1} FPS", fps),
        theme::mono_font(11.0),
        theme::LIME,
    );
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
