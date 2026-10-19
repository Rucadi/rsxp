// SPDX-License-Identifier: GPL-2.0-or-later
//! Presentation-only launcher for opening games and managing save backups.
//!
//! [`SessionPanel`] holds all launcher state and is drawn by [`SessionPanel::draw`];
//! it performs no I/O itself. Instead `draw` returns a [`SessionActions`] value
//! describing the user's intents, which the platform adapter executes.
//!
//! While a session is inactive, the library/save screens fill a central panel;
//! while one is active they hide behind the header's Library/Saves toggle and
//! instead appear as a floating window, and the header offers Pause, Stop, and
//! Restart. Stop and Restart route through a confirmation window before the
//! corresponding [`SessionActions`] field is set. The actual library and save
//! widgets are defined in `library.rs` / `saves.rs`
//! (`SessionPanel::draw_library` and `SessionPanel::draw_saves`).
#[derive(Default)]
pub struct SessionPanel {
    pub source: String,
    pub title: String,
    pub status: String,
    pub active: bool,
    pub paused: bool,
    pub error: Option<String>,
    pub open: bool,
    pub show_saves: bool,
    pub busy: bool,
    pub recent: Vec<String>,
    pub save_directory: String,
    pub backup_directory: String,
    confirm_stop: bool,
    confirm_restart: bool,
}
/// Intents produced by [`SessionPanel::draw`] for the platform adapter to run.
#[derive(Default)]
pub struct SessionActions {
    pub open: Option<String>,
    pub stop: bool,
    pub restart: bool,
    pub backup: Option<(String, String)>,
    pub restore: Option<(String, String)>,
}
impl SessionPanel {
    pub fn draw(&mut self, context: &egui::Context) -> SessionActions {
        let mut actions = SessionActions::default();
        context.root_ui(|root_ui| {
            egui::Panel::top("session-header").show(root_ui, |ui| {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("rsxp")
                            .size(23.0)
                            .strong()
                            .color(egui::Color32::from_rgb(104, 211, 176)),
                    );
                    ui.add_space(12.0);
                    ui.label(if self.title.is_empty() {
                        "Game workspace"
                    } else {
                        &self.title
                    });
                    if !self.status.is_empty() {
                        ui.label(
                            egui::RichText::new(&self.status)
                                .small()
                                .color(egui::Color32::from_rgb(104, 211, 176)),
                        );
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Library").clicked() {
                        self.open = !self.open || self.show_saves;
                        self.show_saves = false;
                    }
                    if ui.button("Saves").clicked() {
                        self.open = !self.open || !self.show_saves;
                        self.show_saves = true;
                    }
                    if self.active {
                        ui.toggle_value(&mut self.paused, "Pause").on_hover_text(
                            "Pause game execution; inspector requests wait until resumed",
                        );
                        if ui.button("Stop").clicked() {
                            self.confirm_stop = true;
                        }
                        if ui.button("Restart").clicked() {
                            self.confirm_restart = true;
                        }
                    }
                });
                if let Some(error) = &self.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                ui.add_space(6.0);
            })
        });
        if !self.active {
            context.root_ui(|root_ui| {
                egui::CentralPanel::default().show(root_ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.add_space(28.0);
                        ui.set_max_width(960.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.selectable_value(&mut self.show_saves, false, "Library");
                            ui.selectable_value(&mut self.show_saves, true, "Save files");
                        });
                        ui.separator();
                        if self.show_saves {
                            self.draw_saves(ui, &mut actions);
                        } else {
                            self.draw_library(ui, &mut actions);
                        }
                    });
                })
            });
        } else if self.open {
            egui::Window::new(if self.show_saves {
                "Save files"
            } else {
                "Library"
            })
            .id(egui::Id::unique("session-library"))
            .default_width(520.0)
            .collapsible(false)
            .show(context, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if self.show_saves {
                        self.draw_saves(ui, &mut actions);
                    } else {
                        self.draw_library(ui, &mut actions);
                    }
                });
            });
        }
        if self.confirm_stop || self.confirm_restart {
            egui::Window::new("End the current game?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(context, |ui| {
                    ui.label("Unsaved progress will be lost. Save in the game before continuing.");
                    ui.horizontal(|ui| {
                        if ui
                            .button(if self.confirm_restart {
                                "Restart game"
                            } else {
                                "Stop game"
                            })
                            .clicked()
                        {
                            actions.restart = self.confirm_restart;
                            actions.stop = self.confirm_stop;
                            self.confirm_stop = false;
                            self.confirm_restart = false;
                        }
                        if ui.button("Keep playing").clicked() {
                            self.confirm_stop = false;
                            self.confirm_restart = false;
                        }
                    });
                });
        }
        actions
    }
}
