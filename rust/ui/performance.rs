// SPDX-License-Identifier: GPL-2.0-or-later
//! Right-hand profiler panel.
//!
//! `draw_profiler_panel` renders the resizable `workspace-profiler-panel` with
//! per-frame figures from `WorkspaceInfo`: FPS, total frame count, and the
//! derived frame time. When `WorkspaceInfo::show_profile_details` is set and a
//! `host::ProfileStats` is supplied, it adds the Ruby/bridge/native/render/
//! present milliseconds, the native call count, and a grid of the native
//! hotpaths seen in the latest five-second window (with its age), or a
//! "Collecting profile data…" placeholder while the window is still empty. The
//! browser frontend leaves `show_profile_details` false, so only the FPS shows.
use super::*;
impl Workspace {
    pub(super) fn draw_profiler_panel(&mut self, context: &egui::Context, info: &WorkspaceInfo) {
        context.root_ui(|root_ui| {
            egui::Panel::right("workspace-profiler-panel")
                .resizable(true)
                .default_size(250.0)
                .show(root_ui, |ui| {
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
                })
        });
    }
}
