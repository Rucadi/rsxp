//! Per-game settings editor, independent of a running session.
//!
//! The editor is deliberately a single resizable window so it works the same
//! from the library and the running workspace. Controls follow the ArcadeNeon
//! design system and the window itself is safe to open before a game starts.
use fanplay_provision::settings::{Preferences, Settings};
use fanplay_ui::theme::{self, ButtonKind, ButtonSize};
use std::path::PathBuf;
pub struct Editor {
    pub key: String,
    /// True edits the installation-wide settings instead of a game override.
    pub common: bool,
    /// Optional game source for the folder shortcuts.
    pub game_path: Option<PathBuf>,
    pub title: String,
    pub root: PathBuf,
    pub prefs: Preferences,
    pub settings: Settings,
    pub patches: String,
    pub error: Option<String>,
    pub capturing_key: Option<i32>,
}
pub enum Action {
    Save,
    Reset,
    Cover,
    ClearCover,
}
impl Editor {
    pub fn draw(&mut self, ctx: &egui::Context, open: &mut bool) -> Option<Action> {
        theme::apply(ctx);
        let mut action = None;
        if let Some(target) = self.capturing_key {
            if ctx.input(|input| input.modifiers.shift) {
                self.settings.keyboard.insert(target, 225);
                self.capturing_key = None;
            }
            let captured = ctx.input(|input| {
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
                    egui_scancode(physical_key.unwrap_or(*key)).map(|code| (target, code))
                })
            });
            if let Some((target, code)) = captured {
                if code == 41 || code == target {
                    self.settings.keyboard.remove(&target);
                } else {
                    self.settings.keyboard.insert(target, code);
                }
                self.capturing_key = None;
            }
        }
        egui::Window::new(if self.common {
            "Common settings".to_string()
        } else {
            format!("{} · Settings", self.title)
        })
        .id(egui::Id::unique("game-settings"))
        .open(open)
        .default_width(560.0)
        .min_width(360.0)
        .resizable(true)
        .show(ctx, |ui| {
            ui.label(theme::display(
                if self.common {
                    "All games"
                } else {
                    &self.title
                },
                24.0,
            ));
            ui.label(
                egui::RichText::new(if self.common {
                    "These settings apply to every game that does not override them."
                } else {
                    "Overrides apply only to this game, the next time it starts."
                })
                .font(theme::body_font(13.0))
                .color(theme::TEXT_MUTED),
            );
            if let Some(error) = &self.error {
                ui.add_space(6.0);
                egui::Frame::new()
                    .fill(theme::PINK.gamma_multiply(0.10))
                    .stroke(egui::Stroke::new(1.0, theme::PINK.gamma_multiply(0.5)))
                    .corner_radius(egui::CornerRadius::same(8))
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(error)
                                .font(theme::body_font(12.0))
                                .color(theme::TEXT),
                        );
                    });
            }
            if let Some(path) = &self.game_path {
                let game = fanplay_provision::folders::game_directory(path);
                let root = fanplay_provision::root(Some(path));
                let save = fanplay_provision::folders::save_directory(
                    &root,
                    &fanplay_provision::folders::title(path),
                );
                let patches = fanplay_provision::folders::patches_directory(path);
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    if theme::button_sized(ui, "Game folder", ButtonKind::Ghost, ButtonSize::Sm)
                        .clicked()
                    {
                        let _ = fanplay_provision::folders::open(&game);
                    }
                    if theme::button_sized(ui, "Save folder", ButtonKind::Ghost, ButtonSize::Sm)
                        .clicked()
                    {
                        let _ = fanplay_provision::folders::open(&save);
                    }
                    if theme::button_sized(ui, "Patches folder", ButtonKind::Ghost, ButtonSize::Sm)
                        .clicked()
                    {
                        let _ = fanplay_provision::folders::open(&patches);
                    }
                });
                ui.label(
                    egui::RichText::new("Patch files are .rb files in the game's patches folder.")
                        .font(theme::body_font(11.0))
                        .color(theme::TEXT_FAINT),
                );
            }
            ui.add_space(10.0);
            egui::ScrollArea::vertical()
                .id_salt("settings-scroll")
                .max_height(540.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Frame::new()
                        .fill(theme::SURFACE)
                        .stroke(egui::Stroke::new(1.0, theme::BORDER))
                        .corner_radius(egui::CornerRadius::same(8))
                        .inner_margin(egui::Margin::same(12))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            theme::section_label(ui, "Cover art", theme::PINK);
                            ui.add_space(4.0);
                            ui.horizontal_wrapped(|ui| {
                                if theme::button_sized(
                                    ui,
                                    "Choose cover image…",
                                    ButtonKind::Secondary,
                                    ButtonSize::Sm,
                                )
                                .clicked()
                                {
                                    action = Some(Action::Cover);
                                }
                                if theme::button_sized(
                                    ui,
                                    "Use repository cover",
                                    ButtonKind::Ghost,
                                    ButtonSize::Sm,
                                )
                                .clicked()
                                {
                                    action = Some(Action::ClearCover);
                                }
                            });
                        });
                    ui.add_space(10.0);
                    egui::Frame::new()
                        .fill(theme::SURFACE)
                        .stroke(egui::Stroke::new(1.0, theme::BORDER))
                        .corner_radius(egui::CornerRadius::same(8))
                        .inner_margin(egui::Margin::same(12))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            theme::section_label(ui, "Performance & rendering", theme::CYAN);
                            ui.add_space(4.0);
                            choice(
                                ui,
                                "Renderer",
                                &mut self.settings.renderer,
                                &["auto", "wgpu", "software"],
                            );
                            choice(
                                ui,
                                "Ruby JIT (RPG Maker)",
                                &mut self.settings.jit,
                                &["yjit", "zjit", "off"],
                            );
                            ui.horizontal(|ui| {
                                ui.label("RGSS version");
                                egui::ComboBox::from_id_salt("rgss-version")
                                    .selected_text(
                                        self.settings
                                            .rgss
                                            .map(|n| n.to_string())
                                            .unwrap_or("Automatic".into()),
                                    )
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut self.settings.rgss,
                                            None,
                                            "Automatic",
                                        );
                                        for n in 1..=3 {
                                            ui.selectable_value(
                                                &mut self.settings.rgss,
                                                Some(n),
                                                n.to_string(),
                                            );
                                        }
                                    });
                            });
                            ui.horizontal(|ui| {
                                let mut enabled = self.settings.fps.is_some();
                                if ui
                                    .checkbox(&mut enabled, "Override RPG Maker FPS")
                                    .changed()
                                {
                                    self.settings.fps = enabled.then_some(60);
                                }
                                if let Some(fps) = &mut self.settings.fps {
                                    ui.add(egui::DragValue::new(fps).range(1..=240));
                                }
                            });
                            ui.checkbox(
                                &mut self.settings.fast_forward,
                                "Start with fast-forward enabled",
                            );
                        });
                    ui.add_space(10.0);
                    egui::CollapsingHeader::new("Keyboard controls")
                        .id_salt("settings-keyboard")
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(
                                    "Map game keys to physical keys. Function-key shortcuts stay unchanged.",
                                )
                                .font(theme::body_font(11.0))
                                .color(theme::TEXT_FAINT),
                            );
                            for (target, label) in [
                                (82, "Up"),
                                (81, "Down"),
                                (80, "Left"),
                                (79, "Right"),
                                (27, "X / cartridge A"),
                                (29, "Z / cartridge B"),
                                (22, "S / cartridge X"),
                                (4, "A / cartridge Y"),
                                (40, "Enter / Start"),
                                (42, "Backspace / Select"),
                                (20, "Q / L"),
                                (26, "W / R"),
                                (44, "Space"),
                                (68, "Fast-forward"),
                            ] {
                                ui.horizontal(|ui| {
                                    let key = *self.settings.keyboard.get(&target).unwrap_or(&target);
                                    ui.label(
                                        egui::RichText::new(label)
                                            .font(theme::body_font(12.0))
                                            .color(theme::TEXT_MUTED),
                                    );
                                    let capturing = self.capturing_key == Some(target);
                                    let label = if capturing { "Press a key…".to_string() } else { key_name(key) };
                                    if ui.button(label).clicked() { self.capturing_key = Some(target); }
                                });
                            }
                        });
                    ui.add_space(6.0);
                    egui::CollapsingHeader::new("Gamepad controls")
                        .id_salt("settings-gamepad")
                        .show(ui, |ui| {
                            let mut enabled = self.settings.gamepad.is_some();
                            if ui
                                .checkbox(&mut enabled, "Use a game-specific mapping")
                                .changed()
                            {
                                self.settings.gamepad = enabled.then(|| {
                                    "0.25;7,6,8,9,11,12,5,10,6,7,0,0,4,1,2,3,2,3,4,1".into()
                                });
                            }
                            let mut reset_gamepad = false;
                            if let Some(encoded) = &mut self.settings.gamepad {
                                match parse_gamepad(encoded) {
                                    Some((mut zone, mut buttons)) => {
                                        ui.add(
                                            egui::Slider::new(&mut zone, 0.1..=0.9)
                                                .text("Stick deadzone"),
                                        );
                                        let names = [
                                            "South",
                                            "East",
                                            "West",
                                            "North",
                                            "L shoulder",
                                            "R shoulder",
                                            "L trigger",
                                            "R trigger",
                                            "Select",
                                            "Start",
                                            "L stick click",
                                            "R stick click",
                                            "D-pad up",
                                            "D-pad down",
                                            "D-pad left",
                                            "D-pad right",
                                            "Stick left",
                                            "Stick right",
                                            "Stick up",
                                            "Stick down",
                                        ];
                                        let actions = [
                                            "Unbound",
                                            "Down",
                                            "Left",
                                            "Right",
                                            "Up",
                                            "A / Run",
                                            "B / Cancel",
                                            "C / Confirm",
                                            "X",
                                            "Y",
                                            "Z",
                                            "L",
                                            "R",
                                        ];
                                        for (index, value) in buttons.iter_mut().enumerate() {
                                            ui.horizontal(|ui| {
                                                ui.label(
                                                    egui::RichText::new(names[index])
                                                        .font(theme::body_font(12.0))
                                                        .color(theme::TEXT_MUTED),
                                                );
                                                egui::ComboBox::from_id_salt(("pad", index))
                                                    .selected_text(
                                                        actions
                                                            .get(*value)
                                                            .copied()
                                                            .unwrap_or("Unbound"),
                                                    )
                                                    .show_ui(ui, |ui| {
                                                        for (n, name) in actions.iter().enumerate() {
                                                            ui.selectable_value(
                                                                value, n, *name,
                                                            );
                                                        }
                                                    });
                                            });
                                        }
                                        *encoded = format!(
                                            "{};{}",
                                            zone,
                                            buttons
                                                .iter()
                                                .map(|n| n.to_string())
                                                .collect::<Vec<_>>()
                                                .join(",")
                                        );
                                    }
                                    None => {
                                        ui.label(
                                            egui::RichText::new(
                                                "This mapping is malformed. Reset it to continue.",
                                            )
                                            .font(theme::body_font(12.0))
                                            .color(theme::WARNING),
                                        );
                                        if theme::button_sized(
                                            ui,
                                            "Reset mapping",
                                            ButtonKind::Ghost,
                                            ButtonSize::Sm,
                                        )
                                        .clicked()
                                        {
                                            reset_gamepad = true;
                                        }
                                    }
                                }
                            }
                            if reset_gamepad {
                                self.settings.gamepad = None;
                            }
                        });
                    ui.add_space(6.0);
                    egui::CollapsingHeader::new("Runtime patches (RPG Maker)")
                        .id_salt("settings-patches")
                        .show(ui, |ui| {
                            ui.label("Enabled patch IDs, separated by commas");
                            ui.text_edit_singleline(&mut self.patches);
                            ui.label(
                                egui::RichText::new(
                                    "Use IDs from this game's patches folder / runtime patch panel. No patch code is downloaded here.",
                                )
                                .font(theme::body_font(11.0))
                                .color(theme::TEXT_FAINT),
                            );
                            for id in self
                                .patches
                                .split(',')
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                            {
                                ui.horizontal(|ui| {
                                    let mut enabled =
                                        self.settings.patch_values.contains_key(id);
                                    if ui
                                        .checkbox(&mut enabled, format!("Parameter: {id}"))
                                        .changed()
                                    {
                                        if enabled {
                                            self.settings.patch_values.insert(id.into(), 1.0);
                                        } else {
                                            self.settings.patch_values.remove(id);
                                        }
                                    }
                                    if let Some(value) = self.settings.patch_values.get_mut(id) {
                                        ui.add(egui::DragValue::new(value));
                                    }
                                });
                            }
                        });
                    ui.add_space(6.0);
                    egui::CollapsingHeader::new("Emulator options")
                        .id_salt("settings-emulator")
                        .show(ui, |ui| {
                            choice(
                                ui,
                                "Nintendo DS renderer",
                                &mut self.settings.nds_renderer,
                                &["wgpu", "software"],
                            );
                            for (label, path) in [
                                ("GBA BIOS", &mut self.settings.gba_bios),
                                ("DS BIOS 7", &mut self.settings.nds_bios7),
                                ("DS BIOS 9", &mut self.settings.nds_bios9),
                                ("DS firmware", &mut self.settings.nds_firmware),
                            ] {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(label)
                                            .font(theme::body_font(12.0))
                                            .color(theme::TEXT_MUTED),
                                    );
                                    if ui.button("Choose…").clicked()
                                        && let Some(file) = rfd::FileDialog::new().pick_file()
                                    {
                                        *path = Some(file.to_string_lossy().into_owned());
                                    }
                                    if ui.small_button("Clear").clicked() {
                                        *path = None;
                                    }
                                });
                                if let Some(path) = path {
                                    ui.label(
                                        egui::RichText::new(path.as_str())
                                            .font(theme::mono_font(10.0))
                                            .color(theme::TEXT_FAINT),
                                    );
                                }
                            }
                        });
                });
            ui.add_space(10.0);
            theme::divider(ui);
            ui.horizontal_wrapped(|ui| {
                if theme::button(ui, "Save settings", ButtonKind::Primary).clicked() {
                    self.settings.patches = self
                        .patches
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .collect();
                    action = Some(Action::Save);
                }
                if theme::button(
                    ui,
                    if self.common {
                        "Reset common settings"
                    } else {
                        "Reset to repository defaults"
                    },
                    ButtonKind::Ghost,
                )
                .clicked()
                {
                    action = Some(Action::Reset);
                }
            });
        });
        action
    }
}
fn parse_gamepad(encoded: &str) -> Option<(f32, Vec<usize>)> {
    let (zone, list) = encoded.split_once(';')?;
    let zone = zone.parse().ok()?;
    let buttons: Vec<usize> = list
        .split(',')
        .map(|s| s.parse().ok())
        .collect::<Option<_>>()?;
    (buttons.len() == 20).then_some((zone, buttons))
}
fn choice(ui: &mut egui::Ui, label: &str, value: &mut Option<String>, values: &[&str]) {
    ui.horizontal(|ui| {
        ui.label(label);
        egui::ComboBox::from_id_salt(label)
            .selected_text(value.as_deref().unwrap_or("Default"))
            .show_ui(ui, |ui| {
                ui.selectable_value(value, None, "Default");
                for option in values {
                    ui.selectable_value(value, Some((*option).into()), *option);
                }
            });
    });
}

fn key_name(code: i32) -> String {
    match code {
        4..=29 => ((b'A' + (code - 4) as u8) as char).to_string(),
        40 => "Enter".into(),
        41 => "Escape".into(),
        42 => "Backspace".into(),
        43 => "Tab".into(),
        44 => "Space".into(),
        58..=69 => format!("F{}", code - 57),
        79 => "Right".into(),
        80 => "Left".into(),
        81 => "Down".into(),
        82 => "Up".into(),
        225 | 229 => "Shift".into(),
        224 | 228 => "Ctrl".into(),
        226 | 230 => "Alt".into(),
        _ => format!("Key {code}"),
    }
}

fn egui_scancode(key: egui::Key) -> Option<i32> {
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
        Tab => 43,
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
