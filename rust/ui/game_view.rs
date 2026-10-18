// SPDX-License-Identifier: GPL-2.0-or-later
use super::*;
impl Workspace {
    pub(super) fn draw_game_view(&mut self, context: &egui::Context, output: &mut WorkspaceOutput) {
        if self.controls_visible && self.view_mode == ViewMode::Floating {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(context, |ui| {
                    ui.centered_and_justified(|ui| {
                        ui.label("Game view is floating. Choose Docked to return it here.");
                    });
                });
            egui::Window::new("Game")
                .id(egui::Id::new("floating-game-view"))
                .default_size(egui::vec2(640.0, 480.0))
                .min_size(egui::vec2(320.0, 240.0))
                .movable(true)
                .resizable(true)
                .frame(egui::Frame::window(&context.style()).fill(egui::Color32::TRANSPARENT))
                .show(context, |ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
                    output.game_rect = rect;
                });
        } else {
            output.game_rect = egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(context, |ui| ui.max_rect())
                .inner;
        }
    }
}
