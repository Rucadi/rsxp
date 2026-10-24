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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RubyJit {
    #[default]
    Yjit,
    Zjit,
    Disabled,
}

impl RubyJit {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Yjit => "yjit",
            Self::Zjit => "zjit",
            Self::Disabled => "off",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Yjit => "YJIT",
            Self::Zjit => "ZJIT",
            Self::Disabled => "Disabled",
        }
    }
}

/// Host services, independent of OS and window runner (winit/eframe).
/// A sandboxed mobile host can enable document/archive import without exposing
/// filesystem paths, directory backups, or Ruby JIT controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionCapabilities {
    pub paths: bool,
    pub directory_backups: bool,
    pub ruby_jit: bool,
    pub zip_import: bool,
    pub torrent_import: bool,
    /// A host-provided single-file picker for GBA/DS ROMs.
    pub rom_import: bool,
}
impl SessionCapabilities {
    pub const DESKTOP: Self = Self {
        paths: true,
        directory_backups: true,
        ruby_jit: true,
        zip_import: false,
        torrent_import: false,
        rom_import: false,
    };
    pub const BROWSER: Self = Self {
        paths: false,
        directory_backups: false,
        ruby_jit: false,
        zip_import: true,
        torrent_import: true,
        rom_import: true,
    };
}
impl Default for SessionCapabilities {
    fn default() -> Self {
        Self::DESKTOP
    }
}

#[derive(Default)]
pub struct SessionPanel {
    pub capabilities: SessionCapabilities,
    pub ruby_jit: RubyJit,
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
    pub(super) library_query: String,
    pub(super) library_selected: Option<String>,
    pub(super) library_sort_title: bool,
    pub(super) library_compact: bool,
    confirm_stop: bool,
    confirm_restart: bool,
}
/// Intents produced by [`SessionPanel::draw`] for the platform adapter to run.
#[derive(Default)]
pub struct SessionActions {
    pub open: Option<String>,
    pub browse: bool,
    pub browse_game_file: bool,
    pub stop: bool,
    pub restart: bool,
    pub backup: Option<(String, String)>,
    pub restore: Option<(String, String)>,
    pub import_zip: bool,
    pub download: Option<String>,
    pub cancel: bool,
}
impl SessionPanel {
    pub fn draw(&mut self, ui: &mut egui::Ui) -> SessionActions {
        let context = ui.ctx().clone();
        let mut actions = SessionActions::default();
        egui::Panel::top("session-header").show(ui, |ui| {
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
                if ui
                    .selectable_label(!self.show_saves && (!self.active || self.open), "Library")
                    .clicked()
                {
                    self.open = !self.open || self.show_saves;
                    self.show_saves = false;
                }
                if self.capabilities.directory_backups
                    && ui
                        .selectable_label(self.show_saves && (!self.active || self.open), "Saves")
                        .clicked()
                {
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
        });
        if !self.active {
            egui::CentralPanel::default().show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_space(8.0);
                    if self.show_saves {
                        self.draw_saves(ui, &mut actions);
                    } else {
                        self.draw_library(ui, &mut actions);
                    }
                });
            });
        } else if self.open {
            egui::Window::new(if self.show_saves {
                "Save files"
            } else {
                "Library"
            })
            .id(egui::Id::unique("session-library"))
            .default_width(860.0)
            .collapsible(false)
            .show(&context, |ui| {
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
                .show(&context, |ui| {
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
