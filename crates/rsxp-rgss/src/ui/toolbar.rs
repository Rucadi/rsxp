// SPDX-License-Identifier: GPL-2.0-or-later
//! Workspace navigation and persistent controller settings.
use super::*;
impl Workspace {
    pub(super) fn draw_toolbar(
        &mut self,
        ui: &mut egui::Ui,
        info: &WorkspaceInfo,
        output: &mut WorkspaceOutput,
    ) {
        egui::Panel::top("workspace-toolbar").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if self.session.is_none() {
                    ui.heading("rsxp");
                }
                ui.menu_button("Layout", |ui| {
                    ui.checkbox(&mut self.panels_top, "Dashboard overview");
                    ui.checkbox(&mut self.panels_left, "Game tools (F2)");
                    ui.checkbox(&mut self.panels_performance, "Performance (F10)");
                    ui.checkbox(&mut self.panels_output, "Activity (F3)");
                    ui.separator();
                    ui.label("Game presentation");
                    ui.selectable_value(&mut self.view_mode, ViewMode::Docked, "Docked");
                    ui.selectable_value(
                        &mut self.view_mode,
                        ViewMode::WorkspaceFullscreen,
                        "Workspace fullscreen",
                    );
                    ui.selectable_value(&mut self.view_mode, ViewMode::Floating, "Floating window");
                });
                if ui
                    .button("Controls")
                    .on_hover_text("Remap controller buttons and adjust stick sensitivity")
                    .clicked()
                {
                    self.controller_settings_open = true;
                }
                ui.menu_button("Speed", |ui| {
                    ui.label("Game speed");
                    let mut fast_forward = self.backend.fast_forward();
                    if ui.checkbox(&mut fast_forward, "Uncapped (F11)").changed() {
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
                    ui.weak("Changing the frame rate may change gameplay speed.");
                });
                if info.show_save_buttons {
                    ui.menu_button("Save files", |ui| {
                        if ui.button("Export save").clicked() {
                            output.export_saves = true;
                        }
                        if ui.button("Import save").clicked() {
                            output.import_saves = true;
                        }
                    });
                }
                if info.show_fullscreen
                    && ui
                        .button(if info.global_fullscreen {
                            "Exit fullscreen · F4"
                        } else {
                            "Fullscreen · F4"
                        })
                        .clicked()
                {
                    output.fullscreen_request = Some(!info.global_fullscreen);
                    if !info.global_fullscreen {
                        self.controls_visible = false;
                    }
                }
                if self.session.is_none() && ui.button("Reset game").clicked() {
                    self.reset_confirm = true;
                }
                if ui.button("Return to game · F1").clicked() {
                    self.controls_visible = false;
                }
            });
        });
    }

    pub(super) fn draw_controller_settings(&mut self, context: &egui::Context) {
        if !self.controller_settings_open {
            return;
        }
        let mut open = self.controller_settings_open;
        egui::Window::new("Controller settings")
            .id(egui::Id::unique("controller-settings"))
            .open(&mut open)
            .collapsible(false)
            .default_width(440.0)
            .show(context, |ui| {
                let mut mapping = crate::controls::get();
                ui.heading("Make it feel right");
                ui.label(format!("{} controller(s) connected", mapping.connected));
                if mapping.connected == 0 {
                    ui.weak("Connect a controller to play. You can configure its layout now.");
                }
                ui.add_space(8.0);
                ui.add(egui::Slider::new(&mut mapping.deadzone, 0.1..=0.9).text("Stick dead zone"))
                    .on_hover_text(
                        "Increase this if your character moves when the stick is at rest.",
                    );
                ui.weak(
                    "Choose the game action for each physical control. Changes apply immediately.",
                );
                ui.add_space(12.0);
                egui::ScrollArea::vertical()
                    .id_salt("controller-mapping-scroll")
                    .max_height(360.0)
                    .show(ui, |ui| {
                        for (group, start, end) in [
                            ("Face buttons", 0, 4),
                            ("Shoulders & triggers", 4, 8),
                            ("Menu & stick clicks", 8, 12),
                            ("Directional pad", 12, 16),
                            ("Left stick", 16, 20),
                        ] {
                            ui.label(
                                egui::RichText::new(group)
                                    .strong()
                                    .color(egui::Color32::from_rgb(127, 231, 200)),
                            );
                            egui::Grid::new(("controller-group", start))
                                .num_columns(2)
                                .spacing([20.0, 8.0])
                                .striped(true)
                                .show(ui, |ui| {
                                    for index in start..end {
                                        ui.label(crate::controls::BUTTONS[index]);
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
                                        ui.end_row();
                                    }
                                });
                            ui.add_space(12.0);
                        }
                    });
                ui.separator();
                if ui.button("Restore defaults").clicked() {
                    let connected = mapping.connected;
                    mapping = crate::controls::Mapping::default();
                    mapping.connected = connected;
                }
                crate::controls::set(mapping);
            });
        self.controller_settings_open = open;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remapping_dropdown_applies_action_without_closing_settings() {
        let original = crate::controls::get();
        crate::controls::set(crate::controls::Mapping::default());
        let context = egui::Context::default();
        let mut workspace = Workspace::new();
        workspace.controller_settings_open = true;
        let mut frame = |events: Vec<egui::Event>| {
            context.run_pass(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000.0, 900.0),
                    )),
                    events,
                    ..Default::default()
                },
                |context| workspace.draw_controller_settings(context),
            )
        };
        fn label_position(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
            output
                .shapes
                .iter()
                .filter_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape {
                        if text.galley.text() == label {
                            return Some(text.pos + text.galley.size() * 0.5);
                        }
                    }
                    None
                })
                .reduce(|first, next| if label == "C / Confirm" { first } else { next })
                .unwrap_or_else(|| panic!("missing label {label}"))
        }
        fn pointer(pos: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        }
        frame(vec![]).drop_without_applying_deltas();
        let output = frame(vec![]);
        let combo = label_position(&output, "C / Confirm");
        output.drop_without_applying_deltas();
        frame(pointer(combo, true)).drop_without_applying_deltas();
        frame(pointer(combo, false)).drop_without_applying_deltas();
        let output = frame(vec![]);
        let action = label_position(&output, "A / Run");
        output.drop_without_applying_deltas();
        frame(pointer(action, true)).drop_without_applying_deltas();
        frame(pointer(action, false)).drop_without_applying_deltas();
        frame(vec![]).drop_without_applying_deltas();
        assert_eq!(crate::controls::get().buttons[0], 5);
        assert!(workspace.controller_settings_open);
        crate::controls::set(original);
    }
}
