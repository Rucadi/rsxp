// SPDX-License-Identifier: GPL-2.0-or-later
//! Compact, readable live session metrics.
use super::*;
impl Workspace {
    pub(super) fn draw_performance_panel(&mut self, ui: &mut egui::Ui, info: &WorkspaceInfo) {
        egui::Panel::right("workspace-performance-panel")
            .resizable(true)
            .default_size(220.0)
            .show(ui, |ui| {
                ui.add_space(8.0);
                ui.heading("Performance");
                ui.weak("Live session metrics");
                ui.add_space(16.0);
                metric(
                    ui,
                    "FRAMES PER SECOND",
                    if info.fps > 0.0 {
                        format!("{:.1}", info.fps)
                    } else {
                        "—".into()
                    },
                );
                metric(
                    ui,
                    "AVERAGE FRAME INTERVAL",
                    if info.fps > 0.0 {
                        format!("{:.2} ms", 1000.0 / info.fps)
                    } else {
                        "—".into()
                    },
                );
                metric(ui, "FRAMES RENDERED", info.frames.to_string());
                ui.add_space(8.0);
                ui.weak("F10 to hide this panel");
            });
    }
}

fn metric(ui: &mut egui::Ui, label: &str, value: String) {
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .corner_radius(12)
        .inner_margin(14.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(label).size(10.0).strong().weak());
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(value)
                    .size(26.0)
                    .strong()
                    .color(egui::Color32::from_rgb(127, 231, 200)),
            );
        });
    ui.add_space(10.0);
}
