// SPDX-License-Identifier: GPL-2.0-or-later
//! Runtime output panel.
//!
//! `draw_logs` renders the bottom, resizable `workspace-output-panel` over
//! `Workspace::logs`, one monospace line each, auto-scrolled to the newest
//! entry, with a Copy button that places the whole log on the clipboard. It is
//! drawn for the `panels_output` toggle and, when a session has stopped, even
//! with controls hidden so crash output stays visible (see `Workspace::draw`).
use super::*;
impl Workspace {
    pub(super) fn draw_logs(&mut self, context: &egui::Context) {
        context.root_ui(|root_ui| {
            egui::Panel::bottom("workspace-output-panel")
                .resizable(true)
                .default_size(150.0)
                .show(root_ui, |ui| {
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
                })
        });
    }
}
