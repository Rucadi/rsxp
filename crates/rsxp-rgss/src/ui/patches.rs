// SPDX-License-Identifier: GPL-2.0-or-later
//! Left-hand "Tools" panel: tool selection and the runtime-patch catalog.
//!
//! `draw_patches_panel` builds the resizable `workspace-game-panel` side panel.
//! It shows the logical game size and the native runtime tool tab;
//! and dispatches to the matching body:
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
            .default_size(280.0)
            .size_range(230.0..=500.0)
            .show(ui, |ui| {
                ui.add_space(8.0);
                ui.heading("Game tools");
                ui.weak("Customize your session");
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.style_mut().spacing.button_padding = egui::vec2(12.0, 8.0);
                    ui.selectable_value(&mut self.tool, Tool::Runtime, "Runtime");
                });
                ui.add_space(12.0);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.separator();
                    ui.add_space(8.0);
                    match self.tool {
                        Tool::Runtime => {
                            self.draw_patch_catalog(ui, true);
                            ui.separator();
                            draw_plugin_ui(ui);
                        }
                    }
                });
            });
    }

    fn draw_patch_catalog(&mut self, ui: &mut egui::Ui, user_patches: bool) {
        ui.heading(if user_patches {
            "Runtime patches"
        } else {
            "Essentials patches"
        });
        if user_patches {
            ui.label("Put Ruby patches in the game’s patches folder, then restart the session.");
        }
        ui.weak("Applied live to the running game.");
        let patches = crate::runtime_patch::patches();
        let mut count = 0;
        for patch in patches
            .iter()
            .filter(|patch| patch.id.starts_with("user/") == user_patches)
        {
            count += 1;
            egui::Frame::new()
                .fill(ui.visuals().faint_bg_color)
                .corner_radius(10)
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let mut enabled = self.backend.patch_enabled(&patch.id);
                    if ui.checkbox(&mut enabled, patch.name.as_ref()).changed() {
                        self.backend.set_patch_enabled(&patch.id, enabled);
                    }
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
                            .small()
                            .weak(),
                    );
                });
            ui.add_space(8.0);
        }
        if count == 0 {
            ui.weak("No patches found.");
        }
    }
}
