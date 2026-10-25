// SPDX-License-Identifier: GPL-2.0-or-later
//! Left-hand "Tools" panel: tool selection and the runtime-patch catalog.
//!
//! `draw_patches_panel` builds the resizable `workspace-game-panel` side panel.
//! It shows the logical game size and the native runtime tool tab, then
//! dispatches to the matching body:
//!
//! * `draw_patch_catalog(ui, user_patches)` lists patches from
//!   `crate::runtime_patch::patches()`, split into built-in and user patches by
//!   the `user/` id prefix. Each row is a checkbox — driving
//!   `Backend::patch_enabled` / `Backend::set_patch_enabled` — plus, when the
//!   patch exposes a `PatchParameter`, a slider that reads and writes
//!   `Backend::patch_value` / `Backend::set_patch_value`.
use super::*;

impl Workspace {
    pub(super) fn draw_patches_panel(
        &mut self,
        ui: &mut egui::Ui,
        _info: &WorkspaceInfo,
        draw_plugin_ui: &mut dyn FnMut(&mut egui::Ui),
    ) {
        egui::Panel::left("workspace-game-panel")
            .resizable(true)
            .default_size(300.0)
            .size_range(250.0..=520.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::SURFACE)
                    .inner_margin(egui::Margin::symmetric(14, 12)),
            )
            .show(ui, |ui| {
                theme::section_label(ui, "Game tools", theme::CYAN);
                ui.label(theme::display("Runtime tools", 20.0));
                ui.label(
                    egui::RichText::new("Changes apply live to the running game.")
                        .font(theme::body_font(12.0))
                        .color(theme::TEXT_MUTED),
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.tool, Tool::Runtime, "Runtime");
                });
                ui.add_space(8.0);
                theme::divider(ui);
                egui::ScrollArea::vertical()
                    .id_salt("game-tools-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.tool {
                        Tool::Runtime => {
                            self.draw_patch_catalog(ui, true);
                            ui.add_space(12.0);
                            theme::divider(ui);
                            draw_plugin_ui(ui);
                        }
                    });
            });
    }

    fn draw_patch_catalog(&mut self, ui: &mut egui::Ui, user_patches: bool) {
        let all_patches = crate::runtime_patch::patches();
        let patches: Vec<_> = all_patches
            .iter()
            .filter(|patch| patch.id.starts_with("user/") == user_patches)
            .collect();
        ui.horizontal(|ui| {
            theme::section_label(
                ui,
                if user_patches {
                    "Runtime patches"
                } else {
                    "Essentials patches"
                },
                theme::PINK,
            );
            theme::tag(ui, &format!("{}", patches.len()), theme::Tone::Neutral);
        });
        if user_patches {
            ui.label(
                egui::RichText::new(
                    "Put Ruby patches in the game’s patches folder, then restart the session.",
                )
                .font(theme::body_font(12.0))
                .color(theme::TEXT_MUTED),
            );
        }
        let mut enabled_count = 0usize;
        for patch in patches {
            let mut enabled = self.backend.patch_enabled(&patch.id);
            if enabled {
                enabled_count += 1;
            }
            egui::Frame::new()
                .fill(if enabled {
                    theme::SURFACE_RAISED
                } else {
                    theme::SURFACE
                })
                .stroke(egui::Stroke::new(
                    1.0,
                    if enabled {
                        theme::PINK.gamma_multiply(0.5)
                    } else {
                        theme::BORDER
                    },
                ))
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(egui::Margin::same(12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut enabled, patch.name.as_ref()).changed() {
                            self.backend.set_patch_enabled(&patch.id, enabled);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            theme::tag(
                                ui,
                                if enabled { "on" } else { "off" },
                                if enabled {
                                    theme::Tone::Lime
                                } else {
                                    theme::Tone::Neutral
                                },
                            );
                        });
                    });
                    if let Some(parameter) = &patch.parameter {
                        let mut value = self
                            .backend
                            .patch_value(&patch.id)
                            .unwrap_or(parameter.default);
                        ui.add_enabled_ui(enabled, |ui| {
                            let slider =
                                egui::Slider::new(&mut value, parameter.min..=parameter.max)
                                    .step_by(parameter.step)
                                    .text(parameter.label)
                                    .suffix(parameter.suffix);
                            if ui.add(slider).changed() {
                                self.backend.set_patch_value(&patch.id, value);
                            }
                        });
                    }
                    ui.label(
                        egui::RichText::new(patch.description.as_ref())
                            .font(theme::body_font(11.0))
                            .color(theme::TEXT_FAINT),
                    );
                });
            ui.add_space(6.0);
        }
        if enabled_count == 0 {
            ui.label(
                egui::RichText::new("No patches are enabled for this session.")
                    .font(theme::body_font(11.0))
                    .color(theme::TEXT_FAINT),
            );
        }
    }
}
