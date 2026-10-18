// SPDX-License-Identifier: GPL-2.0-or-later
use super::*;
impl Workspace {
    pub(super) fn draw_profiler_panel(&mut self, context: &egui::Context, info: &WorkspaceInfo) {
        egui::SidePanel::right("workspace-profiler-panel")
            .resizable(true)
            .default_width(250.0)
            .show(context, |ui| {
                ui.heading("Profiler");
                ui.separator();
                ui.label(format!("FPS: {:.1}", info.fps));
                ui.label(format!("Frame: {}", info.frames));
                if info.fps > 0.0 {
                    ui.label(format!("Frame time: {:.2} ms", 1000.0 / info.fps));
                }
                if info.show_profile_details
                    && let Some(profile) = info.profile
                {
                    ui.separator();
                    ui.label(format!("Ruby: {:.2} ms", profile.ruby_ms));
                    ui.label(format!("Bridge: {:.2} ms", profile.bridge_ms));
                    ui.label(format!("Native API: {:.2} ms", profile.native_ms));
                    ui.label(format!("Render: {:.2} ms", profile.render_ms));
                    ui.label(format!("Present: {:.2} ms", profile.present_ms));
                    ui.label(format!("Native calls: {}", profile.calls));
                    ui.separator();
                    ui.heading("Native API hotpaths · last 5s");
                    ui.label(format!("Updated {:.1}s ago", profile.hotspot_age_secs));
                    if profile.hotspots.is_empty() {
                        ui.weak("Collecting profile data…");
                    } else {
                        egui::Grid::new("profiler-hotpaths")
                            .num_columns(3)
                            .striped(true)
                            .show(ui, |ui| {
                                ui.strong("Operation");
                                ui.strong("Total");
                                ui.strong("Calls");
                                ui.end_row();
                                for hotspot in &profile.hotspots {
                                    ui.label(hotspot.operation.as_str());
                                    ui.label(format!("{:.1} ms", hotspot.total_ms));
                                    ui.label(hotspot.calls.to_string());
                                    ui.end_row();
                                }
                            });
                    }
                }
            });
    }
}
