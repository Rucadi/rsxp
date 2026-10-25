// SPDX-License-Identifier: GPL-2.0-or-later
//! Workspace navigation, live session controls, and persistent controller settings.
//!
//! There is exactly one top bar. While a game is running it carries the game
//! title, a pulsing live indicator, and Pause / Restart / Stop; the layout,
//! speed, folder, and save menus sit beside them. Destructive actions still
//! open the confirmation window owned by the session panel.
use super::theme::{self, ButtonKind, ButtonSize, Tone};
use super::*;

impl Workspace {
    pub(super) fn draw_toolbar(
        &mut self,
        ui: &mut egui::Ui,
        info: &WorkspaceInfo,
        output: &mut WorkspaceOutput,
    ) {
        egui::Panel::top("workspace-toolbar")
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::symmetric(16, 10)),
            )
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(theme::display_color("RSXP", 20.0, theme::PINK));
                    ui.separator();
                    if let Some(session) = &self.session
                        && session.active
                    {
                        let pulse = theme::pulse(ui.ctx());
                        let color = if session.paused {
                            theme::WARNING
                        } else {
                            theme::LIME
                        };
                        theme::status_dot(ui, color, pulse);
                        ui.label(theme::display(
                            if session.title.is_empty() {
                                "Running game"
                            } else {
                                &session.title
                            },
                            18.0,
                        ));
                        theme::tag(
                            ui,
                            if session.paused { "paused" } else { "live" },
                            if session.paused {
                                Tone::Warning
                            } else {
                                Tone::Lime
                            },
                        );
                        if !session.status.is_empty() {
                            ui.label(
                                egui::RichText::new(&session.status)
                                    .font(theme::body_font(12.0))
                                    .color(theme::TEXT_MUTED),
                            );
                        }
                    } else {
                        ui.label(theme::display("Workspace", 18.0));
                    }
                    ui.add_space(4.0);
                    if info.fps > 0.0 {
                        ui.label(
                            egui::RichText::new(format!("{:.0} fps", info.fps))
                                .font(theme::mono_font(12.0))
                                .color(theme::CYAN),
                        );
                    }
                });
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.menu_button("Layout", |ui| {
                        ui.checkbox(&mut self.panels_top, "Session overview");
                        ui.checkbox(&mut self.panels_left, "Game tools (F2)");
                        ui.checkbox(&mut self.panels_performance, "Performance (F10)");
                        ui.checkbox(&mut self.panels_output, "Activity (F3)");
                        ui.separator();
                        ui.label(theme::eyebrow("Game presentation"));
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
                        if info.core_id == "nds" && info.logical_size == (256, 384) {
                            ui.separator();
                            ui.label(theme::eyebrow("Nintendo DS screens"));
                            ui.selectable_value(&mut self.ds_layout, DsLayout::Vertical, "Vertical (natural)");
                            ui.selectable_value(&mut self.ds_layout, DsLayout::Horizontal, "Horizontal");
                            ui.selectable_value(&mut self.ds_layout, DsLayout::TopLarge, "Top large · touch small");
                            ui.selectable_value(&mut self.ds_layout, DsLayout::TouchLarge, "Touch large · top small");
                            ui.selectable_value(&mut self.ds_layout, DsLayout::TopOnly, "Top screen only");
                            ui.selectable_value(&mut self.ds_layout, DsLayout::TouchOnly, "Touch screen only");
                        }
                    });
                    if theme::button_sized(ui, "Controls", ButtonKind::Ghost, ButtonSize::Sm)
                        .on_hover_text("Remap controller buttons and adjust stick sensitivity")
                        .clicked()
                    {
                        self.controller_settings_open = true;
                    }
                    ui.menu_button("Speed", |ui| {
                        ui.label(theme::eyebrow("Game speed"));
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
                    ui.menu_button("Folders", |ui| {
                        if ui.button("Game folder").clicked() {
                            output.open_game_folder = true;
                        }
                        if ui.button("Save folder").clicked() {
                            output.open_save_folder = true;
                        }
                        if ui.button("Patches folder").clicked() {
                            output.open_patches_folder = true;
                        }
                    });
                    if info.show_save_buttons {
                        ui.menu_button("Save files", |ui| {
                            if ui.button("Export save").clicked() {
                                output.export_saves = true;
                            }
                            if ui.button("Import save").clicked() {
                                output.import_saves = true;
                            }
                            ui.separator();
                            if ui.button("Create backup").clicked() {
                                output.create_backup = true;
                            }
                            if ui.button("Restore latest backup").clicked() {
                                output.restore_latest_backup = true;
                            }
                        });
                    }
                    if info.show_fullscreen
                        && theme::button_sized(
                            ui,
                            if info.global_fullscreen {
                                "Exit fullscreen"
                            } else {
                                "Fullscreen"
                            },
                            ButtonKind::Ghost,
                            ButtonSize::Sm,
                        )
                        .on_hover_text("F4")
                        .clicked()
                    {
                        output.fullscreen_request = Some(!info.global_fullscreen);
                        if !info.global_fullscreen {
                            self.controls_visible = false;
                        }
                    }
                    if self.session.is_none()
                        && theme::button_sized(ui, "Reset game", ButtonKind::Ghost, ButtonSize::Sm)
                            .clicked()
                    {
                        self.reset_confirm = true;
                    }
                    if theme::button_sized(ui, "Shortcuts", ButtonKind::Ghost, ButtonSize::Sm)
                        .on_hover_text("Keyboard and controller reference")
                        .clicked()
                    {
                        self.shortcuts_open = true;
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if theme::button_sized(ui, "Hide UI", ButtonKind::Secondary, ButtonSize::Sm)
                            .on_hover_text("Return to the game · F1")
                            .clicked()
                        {
                            self.controls_visible = false;
                        }
                        if let Some(session) = &mut self.session
                            && session.active
                        {
                            session.draw_active_controls(ui);
                        }
                    });
                });
                if let Some(session) = &self.session {
                    if let Some(error) = &session.error {
                        ui.add_space(6.0);
                        super::session::error_banner(ui, error);
                    }
                }
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
            .default_width(460.0)
            .show(context, |ui| {
                let mut mapping = crate::controls::get();
                theme::section_label(ui, "Make it feel right", theme::PINK);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "{} controller(s) connected",
                            mapping.connected
                        ))
                        .font(theme::body_font(13.0))
                        .color(theme::TEXT_MUTED),
                    );
                    if mapping.connected == 0 {
                        theme::tag(ui, "keyboard ready", Tone::Warning);
                    } else {
                        theme::tag(ui, "ready", Tone::Lime);
                    }
                });
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
                    .max_height(380.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (group, start, end) in [
                            ("Face buttons", 0, 4),
                            ("Shoulders & triggers", 4, 8),
                            ("Menu & stick clicks", 8, 12),
                            ("Directional pad", 12, 16),
                            ("Left stick", 16, 20),
                        ] {
                            theme::section_label(ui, group, theme::CYAN);
                            egui::Frame::new()
                                .fill(theme::SURFACE)
                                .stroke(egui::Stroke::new(1.0, theme::BORDER))
                                .corner_radius(egui::CornerRadius::same(8))
                                .inner_margin(egui::Margin::same(10))
                                .show(ui, |ui| {
                                    egui::Grid::new(("controller-group", start))
                                        .num_columns(2)
                                        .spacing([20.0, 8.0])
                                        .striped(true)
                                        .show(ui, |ui| {
                                            for index in start..end {
                                                ui.label(
                                                    egui::RichText::new(
                                                        crate::controls::BUTTONS[index],
                                                    )
                                                    .font(theme::mono_font(12.0))
                                                    .color(theme::TEXT_MUTED),
                                                );
                                                egui::ComboBox::from_id_salt(("controller", index))
                                                    .selected_text(
                                                        crate::controls::ACTIONS
                                                            [mapping.buttons[index]],
                                                    )
                                                    .show_ui(ui, |ui| {
                                                        for (action, label) in
                                                            crate::controls::ACTIONS
                                                                .iter()
                                                                .enumerate()
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
                                });
                            ui.add_space(12.0);
                        }
                    });
                ui.add_space(8.0);
                if theme::button(ui, "Restore defaults", ButtonKind::Secondary).clicked() {
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
