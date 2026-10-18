// SPDX-License-Identifier: GPL-2.0-or-later
use super::*;
impl Workspace {
    pub(super) fn draw_logs(&mut self, context: &egui::Context) {
        egui::TopBottomPanel::bottom("workspace-output-panel")
            .resizable(true)
            .default_height(150.0)
            .show(context, |ui| {
                ui.horizontal(|ui| {
                    ui.strong("Runtime output");
                    if ui.small_button("Copy").clicked() {
                        context.copy_text(self.logs.join("\n"));
                    }
                });
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        if self.logs.is_empty() {
                            ui.weak("No runtime messages yet.");
                        }
                        for line in &self.logs {
                            ui.monospace(line);
                        }
                    });
            });
    }
}
