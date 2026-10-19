// SPDX-License-Identifier: GPL-2.0-or-later
//! Right-hand FPS/performance panel.
//!
//! `draw_performance_panel` renders the resizable panel with FPS, total frame
//! count, and derived frame time.
use super::*;
impl Workspace {
    pub(super) fn draw_performance_panel(&mut self, context: &egui::Context, info: &WorkspaceInfo) {
        context.root_ui(|root_ui| {
            egui::Panel::right("workspace-performance-panel")
                .resizable(true)
                .default_size(250.0)
                .show(root_ui, |ui| {
                    ui.heading("Performance");
                    ui.separator();
                    ui.label(format!("FPS: {:.1}", info.fps));
                    ui.label(format!("Frame: {}", info.frames));
                    if info.fps > 0.0 {
                        ui.label(format!("Frame time: {:.2} ms", 1000.0 / info.fps));
                    }
                })
        });
    }
}
