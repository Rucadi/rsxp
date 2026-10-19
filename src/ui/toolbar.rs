// SPDX-License-Identifier: GPL-2.0-or-later
//! Top workspace toolbar.
//!
//! `draw_toolbar` renders the `workspace-toolbar` top panel: the Workspace menu
//! (panel-visibility toggles), the game-view mode combo (`ViewMode`), an
//! optional global-fullscreen button, the Controls menu (controller mapping
//! from `crate::controls`), and runtime frame pacing — an "Uncapped" checkbox
//! and an FPS drag value, both driven through the injected
//! `backend::Backend`. It also surfaces save export/import and reset buttons
//! when the frontend enables them, recording those requests in
//! `WorkspaceOutput` / the workspace's confirmation state.
use super::*;
impl Workspace {
    pub(super) fn draw_toolbar(
        &mut self,
        context: &egui::Context,
        info: &WorkspaceInfo,
        output: &mut WorkspaceOutput,
    ) {
        context.root_ui(|root_ui| {
            egui::Panel::top("workspace-toolbar").show(root_ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if self.session.is_none() {
                        ui.heading("rsxp");
                    }
                    ui.menu_button("Workspace", |ui| {
                        ui.checkbox(&mut self.panels_left, "Tools (F2)");
                        ui.checkbox(&mut self.panels_performance, "Performance (F10)");
                        ui.checkbox(&mut self.panels_output, "Runtime output (F3)");
                    });
                    egui::ComboBox::from_id_salt("game-view-mode")
                        .selected_text(self.view_mode.label())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.view_mode, ViewMode::Docked, "Docked");
                            ui.selectable_value(
                                &mut self.view_mode,
                                ViewMode::WorkspaceFullscreen,
                                "Workspace fullscreen",
                            );
                            ui.selectable_value(
                                &mut self.view_mode,
                                ViewMode::Floating,
                                "Floating window",
                            );
                        });
                    if self.is_workspace_fullscreen() {
                        self.controls_visible = false;
                    }
                    if info.show_fullscreen
                        && ui
                            .button(if info.global_fullscreen {
                                "Exit fullscreen (F4)"
                            } else {
                                "Global fullscreen (F4)"
                            })
                            .clicked()
                    {
                        output.fullscreen_request = Some(!info.global_fullscreen);
                        if !info.global_fullscreen {
                            self.controls_visible = false;
                        }
                    }
                    ui.menu_button("Controls", |ui| {
                        let mut mapping = crate::controls::get();
                        ui.label(format!("{} controller(s) connected", mapping.connected));
                        ui.label("Choose an action for each control.");
                        ui.add(
                            egui::Slider::new(&mut mapping.deadzone, 0.1..=0.9)
                                .text("Stick dead zone"),
                        );
                        egui::ScrollArea::vertical()
                            .max_height(360.0)
                            .show(ui, |ui| {
                                for (index, name) in crate::controls::BUTTONS.iter().enumerate() {
                                    ui.horizontal(|ui| {
                                        ui.label(*name);
                                        egui::ComboBox::from_id_salt(("controller", index))
                                            .selected_text(
                                                crate::controls::ACTIONS[mapping.buttons[index]],
                                            )
                                            .show_ui(ui, |ui| {
                                                for (action, label) in
                                                    crate::controls::ACTIONS.iter().enumerate()
                                                {
                                                    ui.selectable_value(
                                                        &mut mapping.buttons[index],
                                                        action,
                                                        *label,
                                                    );
                                                }
                                            });
                                    });
                                }
                            });
                        if ui.button("Restore defaults").clicked() {
                            let connected = mapping.connected;
                            mapping = crate::controls::Mapping::default();
                            mapping.connected = connected;
                        }
                        crate::controls::set(mapping);
                    });
                    ui.separator();
                    // Frame pacing is configured here at runtime instead of through
                    // command-line flags.
                    let mut fast_forward = self.backend.fast_forward();
                    if ui.checkbox(&mut fast_forward, "Uncapped").changed() {
                        self.backend.set_fast_forward(fast_forward);
                    }
                    let current_rate = self.backend.current_rate();
                    let mut fps = if current_rate == 0 { 60 } else { current_rate };
                    if ui
                        .add_enabled(
                            current_rate > 0,
                            egui::DragValue::new(&mut fps)
                                .range(1..=1000)
                                .speed(1.0)
                                .prefix("FPS "),
                        )
                        .changed()
                    {
                        self.backend.set_rate_override(fps);
                    }
                    if info.show_save_buttons {
                        if ui.button("Export save").clicked() {
                            output.export_saves = true;
                        }
                        if ui.button("Import save").clicked() {
                            output.import_saves = true;
                        }
                    }
                    if self.session.is_none() && ui.button("Reset game").clicked() {
                        self.reset_confirm = true;
                    }
                    if ui.small_button("Hide UI (F1)").clicked() {
                        self.controls_visible = false;
                    }
                    ui.weak(
                    "F1 UI · F2 tools · F3 output · F4 fullscreen · F10 performance · F11 speed",
                );
                });
            })
        });
    }
}
