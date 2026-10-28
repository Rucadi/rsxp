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
                    ui.label(theme::display_color("FANPLAY", 20.0, theme::PINK));
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
                            self.ds_layout.selector(ui);
                        }
                    });
                    if theme::button_sized(ui, "Controls", ButtonKind::Ghost, ButtonSize::Sm)
                        .on_hover_text("Remap controller buttons and adjust stick sensitivity")
                        .clicked()
                    {
                        self.controller_settings_open = true;
                    }
                    if self.backend.supports_speed_control() {
                        ui.menu_button("Speed", |ui| {
                            ui.label(theme::eyebrow("Game speed"));
                            let mut fast_forward = self.backend.fast_forward();
                            if ui.checkbox(&mut fast_forward, "Uncapped (F11)").changed() {
                                self.backend.set_fast_forward(fast_forward);
                            }
                            ui.horizontal_wrapped(|ui| {
                                ui.label("Target");
                                for (label, multiplier) in [
                                    ("1×", 1u32),
                                    ("2×", 2),
                                    ("3×", 3),
                                    ("4×", 4),
                                    ("5×", 5),
                                    ("8×", 8),
                                ] {
                                    if ui.button(label).clicked() {
                                        self.backend.set_fast_forward(false);
                                        self.backend.set_rate_override(
                                            info.base_fps.saturating_mul(multiplier).max(1),
                                        );
                                    }
                                }
                                if ui.button("Game rate").clicked() {
                                    self.backend.set_fast_forward(false);
                                    self.backend.set_rate_override(0);
                                }
                            });
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
                    }
                    if info.supports_save_states {
                        ui.menu_button("States", |ui| {
                            ui.label(theme::eyebrow("Game snapshots"));
                            for slot in &info.state_slots {
                                ui.horizontal(|ui| {
                                    ui.label(format!("Slot {}", slot.slot));
                                    if let Some(saved_at) = &slot.saved_at {
                                        ui.label(
                                            egui::RichText::new(if slot.bytes >= 1024 {
                                                format!(
                                                    "{saved_at} · {:.1} KiB",
                                                    slot.bytes as f32 / 1024.0
                                                )
                                            } else {
                                                format!("{saved_at} · {} B", slot.bytes)
                                            })
                                            .font(theme::body_font(11.0))
                                            .color(theme::TEXT_MUTED),
                                        );
                                    } else {
                                        ui.label(
                                            egui::RichText::new("empty")
                                                .font(theme::body_font(11.0))
                                                .color(theme::TEXT_FAINT),
                                        );
                                    }
                                    if ui.button("Save").clicked() {
                                        output.save_state = Some(slot.slot.clone());
                                    }
                                    if (slot.saved_at.is_some() || slot.bytes > 0)
                                        && ui.button("Load").clicked()
                                    {
                                        output.load_state = Some(slot.slot.clone());
                                    }
                                });
                            }
                        });
                    }
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
        return self.draw_controller_settings_horizontal(context);
        #[allow(unreachable_code)]
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

    fn draw_controller_settings_horizontal(&mut self, context: &egui::Context) {
        if !self.controller_settings_open {
            return;
        }
        let mut open = true;
        let mut mapping = crate::controls::get();
        let keyboard_actions = [
            (82, "Up"),
            (81, "Down"),
            (80, "Left"),
            (79, "Right"),
            (4, "A / Confirm"),
            (5, "B / Cancel"),
            (6, "C / Confirm"),
            (27, "X"),
            (28, "Y"),
            (29, "Z"),
            (15, "L"),
            (21, "R"),
            (40, "Enter / Start"),
            (41, "Escape"),
            (225, "Shift / Run"),
            (224, "Ctrl"),
            (226, "Alt"),
            (44, "Space"),
            (68, "Fast-forward"),
        ];
        if let Some(target) = self.keyboard_capture {
            if context.input(|input| input.modifiers.shift) {
                self.backend.set_keyboard_binding(target, 225);
                self.keyboard_capture = None;
            }
            if let Some((code, cancel)) = context.input(|input| {
                input.events.iter().find_map(|event| {
                    let egui::Event::Key {
                        key,
                        physical_key,
                        pressed: true,
                        repeat: false,
                        ..
                    } = event
                    else {
                        return None;
                    };
                    if *key == egui::Key::Escape {
                        return Some((target, true));
                    }
                    Some((toolbar_scancode(physical_key.unwrap_or(*key))?, false))
                })
            }) {
                if !cancel {
                    self.backend.set_keyboard_binding(target, code);
                }
                self.keyboard_capture = None;
            }
        }
        egui::Window::new("Controls")
            .id(egui::Id::unique("controller-settings-horizontal"))
            .open(&mut open)
            .collapsible(false)
            .default_width(980.0)
            .min_width(760.0)
            .show(context, |ui| {
                ui.horizontal(|ui| {
                    theme::section_label(ui, "Controller", theme::PINK);
                    ui.add_space(12.0);
                    ui.label(format!("{} connected", mapping.connected));
                    ui.add_space(20.0);
                    ui.add(egui::Slider::new(&mut mapping.deadzone, 0.1..=0.9).text("Stick dead zone"));
                });
                ui.label("Choose actions for physical controls. Keyboard bindings can be changed from the same screen.");
                ui.add_space(10.0);
                ui.columns(2, |columns| {
                    columns[0].vertical(|ui| {
                        theme::section_label(ui, "Gamepad layout", theme::CYAN);
                        ui.add_space(6.0);
                        egui::Frame::new().fill(theme::surface()).stroke(egui::Stroke::new(1.0, theme::BORDER)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                            egui::Grid::new("controller-horizontal-grid").num_columns(4).spacing([10.0, 8.0]).striped(true).show(ui, |ui| {
                                for index in 0..20 {
                                    ui.label(egui::RichText::new(crate::controls::BUTTONS[index]).font(theme::body_font(12.0)).color(theme::TEXT_MUTED));
                                    egui::ComboBox::from_id_salt(("controller-horizontal", index)).selected_text(crate::controls::ACTIONS[mapping.buttons[index]]).show_ui(ui, |ui| {
                                        for (action, label) in crate::controls::ACTIONS.iter().enumerate() { ui.selectable_value(&mut mapping.buttons[index], action, *label); }
                                    });
                                    if index % 2 == 1 { ui.end_row(); }
                                }
                            });
                        });
                    });
                    columns[1].vertical(|ui| {
                        theme::section_label(ui, "Keyboard layout", theme::PINK);
                        ui.label("Click a binding, then press a key. Escape cancels.");
                        ui.add_space(6.0);
                        egui::Frame::new().fill(theme::surface()).stroke(egui::Stroke::new(1.0, theme::BORDER)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                            egui::Grid::new("keyboard-horizontal-grid").num_columns(2).spacing([18.0, 10.0]).striped(true).show(ui, |ui| {
                                for (action, label) in keyboard_actions {
                                    ui.label(egui::RichText::new(label).font(theme::body_font(13.0)).color(theme::TEXT_MUTED));
                                    let physical = self.backend.keyboard_binding(action);
                                    let button = if self.keyboard_capture == Some(action) { "Press a key…".into() } else { toolbar_key_name(physical) };
                                    if ui.button(button).clicked() { self.keyboard_capture = Some(action); }
                                    ui.end_row();
                                }
                            });
                            ui.add_space(12.0);
                            ui.horizontal(|ui| {
                                ui.label("Add game action");
                                egui::ComboBox::from_id_salt("keyboard-add-action")
                                    .selected_text(if self.keyboard_add_action == 0 { "Choose…".into() } else { toolbar_action_name(self.keyboard_add_action) })
                                    .show_ui(ui, |ui| {
                                        for (action, label) in keyboard_actions {
                                            ui.selectable_value(&mut self.keyboard_add_action, action, label);
                                        }
                                    });
                                if theme::button_sized(ui, "Add binding", ButtonKind::Secondary, ButtonSize::Sm).clicked() && self.keyboard_add_action != 0 {
                                    self.keyboard_capture = Some(self.keyboard_add_action);
                                }
                            });
                            ui.label("Add another RPG Maker action, then click its binding and press a key.");
                        });
                    });
                });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if theme::button(ui, "Restore controller defaults", ButtonKind::Secondary).clicked() {
                        let connected = mapping.connected;
                        mapping = crate::controls::Mapping::default();
                        mapping.connected = connected;
                    }
                    if theme::button(ui, "Restore keyboard defaults", ButtonKind::Ghost).clicked() {
                        for (action, _) in keyboard_actions { self.backend.set_keyboard_binding(action, action); }
                        self.keyboard_add_action = 0;
                    }
                });
            });
        crate::controls::set(mapping);
        self.controller_settings_open = open;
    }
}

fn toolbar_key_name(code: i32) -> String {
    match code {
        4..=29 => ((b'A' + (code - 4) as u8) as char).to_string(),
        40 => "Enter".into(),
        41 => "Escape".into(),
        42 => "Backspace".into(),
        44 => "Space".into(),
        58..=69 => format!("F{}", code - 57),
        79 => "Right".into(),
        80 => "Left".into(),
        81 => "Down".into(),
        82 => "Up".into(),
        225 | 229 => "Shift".into(),
        _ => format!("Key {code}"),
    }
}

fn toolbar_action_name(code: i32) -> &'static str {
    match code {
        4 => "A / Confirm",
        5 => "B / Cancel",
        6 => "C / Confirm",
        15 => "L",
        21 => "R",
        27 => "X",
        28 => "Y",
        29 => "Z",
        40 => "Enter / Start",
        41 => "Escape",
        44 => "Space",
        68 => "Fast-forward",
        79 => "Right",
        80 => "Left",
        81 => "Down",
        82 => "Up",
        224 => "Ctrl",
        225 => "Shift / Run",
        226 => "Alt",
        _ => "Other action",
    }
}

fn toolbar_scancode(key: egui::Key) -> Option<i32> {
    use egui::Key::*;
    Some(match key {
        A => 4,
        B => 5,
        C => 6,
        D => 7,
        E => 8,
        F => 9,
        G => 10,
        H => 11,
        I => 12,
        J => 13,
        K => 14,
        L => 15,
        M => 16,
        N => 17,
        O => 18,
        P => 19,
        Q => 20,
        R => 21,
        S => 22,
        T => 23,
        U => 24,
        V => 25,
        W => 26,
        X => 27,
        Y => 28,
        Z => 29,
        Enter => 40,
        Escape => 41,
        Backspace => 42,
        Space => 44,
        F1 => 58,
        F2 => 59,
        F3 => 60,
        F4 => 61,
        F5 => 62,
        F6 => 63,
        F7 => 64,
        F8 => 65,
        F9 => 66,
        F10 => 67,
        F11 => 68,
        F12 => 69,
        ArrowRight => 79,
        ArrowLeft => 80,
        ArrowDown => 81,
        ArrowUp => 82,
        _ => return None,
    })
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
