// SPDX-License-Identifier: GPL-2.0-or-later
//! Presentation-only launcher for opening games and managing save backups.
//!
//! [`SessionPanel`] holds all launcher state and is drawn by [`SessionPanel::draw`];
//! it performs no I/O itself. Instead `draw` returns a [`SessionActions`] value
//! describing the user's intents, which the platform adapter executes.
//!
//! While a session is inactive, the library/save screens fill the window under
//! a branded ArcadeNeon header. While a session is active the panel stays out of
//! the way: the workspace toolbar owns Pause, Restart, and Stop, and the
//! library disappears until the game is stopped. Destructive actions always
//! pass through a confirmation window before the corresponding
//! [`SessionActions`] field is set.
//!
//! The actual library and save widgets are defined in `library.rs` /
//! `saves.rs` (`SessionPanel::draw_library` and `SessionPanel::draw_saves`).
use super::theme::{self, Tone};

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
    /// Named save snapshots are available through an adapter.
    pub save_backups: bool,
    pub ruby_jit: bool,
    pub zip_import: bool,
    pub torrent_import: bool,
    /// A host-provided single-file picker for cartridge ROMs.
    pub rom_import: bool,
}
impl SessionCapabilities {
    pub const DESKTOP: Self = Self {
        paths: true,
        directory_backups: true,
        save_backups: true,
        ruby_jit: true,
        zip_import: false,
        torrent_import: false,
        rom_import: false,
    };

    pub const WEB: Self = Self {
        paths: false,
        directory_backups: false,
        save_backups: true,
        ruby_jit: false,
        zip_import: false,
        torrent_import: false,
        rom_import: true,
    };
}
impl Default for SessionCapabilities {
    fn default() -> Self {
        Self::DESKTOP
    }
}

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
    /// Control API state shown in the shared header. Desktop adapters sync
    /// these from the real listener.
    pub api_port: String,
    pub api_running: bool,
    pub api_status: String,
    pub api_error: Option<String>,
    pub api_open: bool,
    pub(super) library_query: String,
    pub(super) library_platform: String,
    pub(super) library_selected: Option<String>,
    pub(super) library_sort_title: bool,
    pub(super) library_compact: bool,
    confirm_stop: bool,
    confirm_restart: bool,
}
impl Default for SessionPanel {
    fn default() -> Self {
        Self {
            capabilities: SessionCapabilities::default(),
            ruby_jit: RubyJit::default(),
            source: String::new(),
            title: String::new(),
            status: String::new(),
            active: false,
            paused: false,
            error: None,
            open: false,
            show_saves: false,
            busy: false,
            recent: Vec::new(),
            save_directory: String::new(),
            backup_directory: String::new(),
            api_port: "47821".into(),
            api_running: false,
            api_status: String::new(),
            api_error: None,
            api_open: false,
            library_query: String::new(),
            library_platform: "All".into(),
            library_selected: None,
            library_sort_title: false,
            library_compact: false,
            confirm_stop: false,
            confirm_restart: false,
        }
    }
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
    pub create_backup: bool,
    pub restore_latest: bool,
    pub export_saves: bool,
    pub import_saves: bool,
    pub import_zip: bool,
    pub download: Option<String>,
    pub cancel: bool,
    pub start_api: Option<u16>,
    pub stop_api: bool,
}
impl SessionPanel {
    pub fn draw(&mut self, ui: &mut egui::Ui) -> SessionActions {
        let context = ui.ctx().clone();
        let mut actions = SessionActions::default();

        if !self.active {
            self.draw_library_header(ui, &mut actions);
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::new()
                        .fill(theme::BG)
                        .inner_margin(egui::Margin::symmetric(28, 20)),
                )
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("session-scroll")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.add_space(4.0);
                            if self.show_saves {
                                self.draw_saves(ui, &mut actions);
                            } else {
                                self.draw_library(ui, &mut actions);
                            }
                        });
                });
        }

        self.draw_api_window(&context, &mut actions);
        self.draw_confirmations(&context, &mut actions);
        actions
    }

    fn draw_library_header(&mut self, ui: &mut egui::Ui, actions: &mut SessionActions) {
        egui::Panel::top("session-header")
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::symmetric(28, 14)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    brand(ui);
                    ui.add_space(14.0);
                    ui.vertical(|ui| {
                        ui.label(theme::display("Game library", 24.0));
                        ui.label(
                            egui::RichText::new("RPG Maker · GB / GBC · GBA · NDS · One neon home")
                                .font(theme::mono_font(11.0))
                                .color(theme::TEXT_MUTED),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let api_label = if self.api_running {
                            "Control API · live"
                        } else {
                            "Control API"
                        };
                        let api_tone = if self.api_running {
                            Tone::Lime
                        } else {
                            Tone::Neutral
                        };
                        if theme::chip(ui, api_label, api_tone, self.api_open).clicked() {
                            self.api_open = !self.api_open;
                            self.api_error = None;
                        }
                        if self.capabilities.directory_backups || self.capabilities.save_backups {
                            if theme::chip(ui, "Saves", Tone::Cyan, self.show_saves).clicked() {
                                self.show_saves = true;
                            }
                        }
                        if theme::chip(ui, "Library", Tone::Pink, !self.show_saves).clicked() {
                            self.show_saves = false;
                        }
                    });
                });
                if let Some(error) = &self.error {
                    ui.add_space(8.0);
                    error_banner(ui, error);
                }
                if !self.status.is_empty() {
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(&self.status)
                            .font(theme::body_font(13.0))
                            .color(theme::TEXT_MUTED),
                    );
                }
                if self.busy {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(
                            egui::RichText::new("Preparing game…")
                                .font(theme::body_font(13.0))
                                .color(theme::CYAN),
                        );
                        if (self.capabilities.zip_import || self.capabilities.torrent_import)
                            && theme::button_sized(
                                ui,
                                "Cancel",
                                theme::ButtonKind::Ghost,
                                theme::ButtonSize::Sm,
                            )
                            .clicked()
                        {
                            actions.cancel = true;
                        }
                    });
                }
            });
    }

    /// Active-session controls live in the workspace toolbar so there is only
    /// one top bar. The confirmation dialogs still belong to the panel.
    pub(super) fn draw_active_controls(&mut self, ui: &mut egui::Ui) {
        if !self.active {
            return;
        }
        let pause_label = if self.paused { "Resume" } else { "Pause" };
        let pause_kind = if self.paused {
            theme::ButtonKind::Success
        } else {
            theme::ButtonKind::Secondary
        };
        if theme::button_sized(ui, pause_label, pause_kind, theme::ButtonSize::Sm)
            .on_hover_text("Pause execution. Inspector requests wait until resumed.")
            .clicked()
        {
            self.paused = !self.paused;
        }
        if theme::button_sized(
            ui,
            "Restart",
            theme::ButtonKind::Ghost,
            theme::ButtonSize::Sm,
        )
        .clicked()
        {
            self.confirm_restart = true;
        }
        if theme::button_sized(
            ui,
            "Stop",
            theme::ButtonKind::Destructive,
            theme::ButtonSize::Sm,
        )
        .clicked()
        {
            self.confirm_stop = true;
        }
    }

    fn draw_api_window(&mut self, context: &egui::Context, actions: &mut SessionActions) {
        if !self.api_open {
            return;
        }
        let mut open = true;
        egui::Window::new("Control API")
            .id(egui::Id::unique("session-control-api-window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(520.0)
            .show(context, |ui| {
                {
                    theme::section_label(ui, "Loopback server", theme::PINK);
                    ui.label(
                        "One server controls the launcher and the active game. Every endpoint and parameter type is listed at /v1/discovery.",
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label("Port");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.api_port)
                                .desired_width(88.0)
                                .font(theme::mono_font(14.0)),
                        );
                        if self.api_running {
                            if theme::button_sized(
                                ui,
                                "Stop API",
                                theme::ButtonKind::Ghost,
                                theme::ButtonSize::Sm,
                            )
                            .clicked()
                            {
                                actions.stop_api = true;
                            }
                            theme::tag(ui, "running", Tone::Lime);
                        } else if theme::button_sized(
                            ui,
                            "Start API",
                            theme::ButtonKind::Primary,
                            theme::ButtonSize::Sm,
                        )
                        .clicked()
                        {
                            match self.api_port.trim().parse::<u16>() {
                                Ok(port) if port != 0 => actions.start_api = Some(port),
                                _ => self.api_error = Some("Port must be 1–65535".into()),
                            }
                        }
                    });
                    if self.api_running {
                        let url = format!("http://127.0.0.1:{}/v1/discovery", self.api_port);
                        ui.add_space(8.0);
                        egui::Frame::new()
                            .fill(theme::SURFACE)
                            .stroke(egui::Stroke::new(1.0, theme::BORDER))
                            .corner_radius(egui::CornerRadius::same(6))
                            .inner_margin(egui::Margin::same(10))
                            .show(ui, |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(
                                        egui::RichText::new(&url)
                                            .font(theme::mono_font(13.0))
                                            .color(theme::CYAN),
                                    );
                                    if theme::button_sized(
                                        ui,
                                        "Copy",
                                        theme::ButtonKind::Ghost,
                                        theme::ButtonSize::Sm,
                                    )
                                    .clicked()
                                    {
                                        ui.ctx().copy_text(url.clone());
                                    }
                                });
                            });
                    }
                }
                if let Some(error) = &self.api_error {
                    ui.add_space(8.0);
                    error_banner(ui, error);
                }
                if !self.api_status.is_empty() {
                    ui.add_space(8.0);
                    ui.weak(&self.api_status);
                }
            });
        self.api_open = open && self.api_open;
    }

    fn draw_confirmations(&mut self, context: &egui::Context, actions: &mut SessionActions) {
        if !self.confirm_stop && !self.confirm_restart {
            return;
        }
        let restart = self.confirm_restart;
        let mut keep_playing = false;
        egui::Window::new(if restart {
            "Restart the game?"
        } else {
            "Stop the game?"
        })
        .id(egui::Id::unique("session-confirm"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(context, |ui| {
            ui.horizontal(|ui| {
                theme::status_dot(ui, theme::WARNING, 1.0);
                ui.label(theme::display(
                    if restart {
                        "Restart from the beginning?"
                    } else {
                        "End the current game?"
                    },
                    20.0,
                ));
            });
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("Unsaved progress will be lost. Save in the game first.")
                    .font(theme::body_font(14.0))
                    .color(theme::TEXT_MUTED),
            );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let confirm = if restart {
                    theme::button(ui, "Restart game", theme::ButtonKind::Destructive)
                } else {
                    theme::button(ui, "Stop game", theme::ButtonKind::Destructive)
                };
                if confirm.clicked() {
                    actions.restart = restart;
                    actions.stop = !restart;
                    self.confirm_stop = false;
                    self.confirm_restart = false;
                }
                if theme::button(ui, "Keep playing", theme::ButtonKind::Secondary).clicked() {
                    keep_playing = true;
                }
            });
        });
        if keep_playing {
            self.confirm_stop = false;
            self.confirm_restart = false;
        }
    }
}

fn brand(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(104.0, 38.0), egui::Sense::hover());
    theme::glow(ui.painter(), rect, 10, theme::PINK, 0.55);
    theme::centered_text(ui, rect, "FANPLAY", theme::display_font(28.0), theme::PINK);
}

/// Red error banner shared by the launcher and workspace.
pub(super) fn error_banner(ui: &mut egui::Ui, message: &str) {
    egui::Frame::new()
        .fill(theme::PINK.gamma_multiply(0.10))
        .stroke(egui::Stroke::new(1.0, theme::PINK.gamma_multiply(0.55)))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                theme::status_dot(ui, theme::PINK, 0.8);
                ui.label(
                    egui::RichText::new(message)
                        .font(theme::body_font(13.0))
                        .color(theme::TEXT),
                );
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_header_and_saves_fit_narrow_and_wide_windows() {
        for width in [360.0, 720.0, 1200.0] {
            for show_saves in [false, true] {
                let context = egui::Context::default();
                let mut panel = SessionPanel::default();
                panel.recent = vec!["/games/Alpha".into(), "/roms/Beta.gba".into()];
                panel.show_saves = show_saves;
                panel.error =
                    Some("A long error message that should wrap without overflowing".into());
                let mut actions = SessionActions::default();
                context
                    .run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(width, 900.0),
                            )),
                            ..Default::default()
                        },
                        |ui| actions = panel.draw(ui),
                    )
                    .drop_without_applying_deltas();
                assert!(actions.open.is_none());
            }
        }
    }

    #[test]
    fn active_session_does_not_draw_the_library() {
        let context = egui::Context::default();
        let mut panel = SessionPanel::default();
        panel.active = true;
        panel.title = "Running".into();
        panel.recent = vec!["/games/Alpha".into()];
        context
            .run_ui(egui::RawInput::default(), |ui| {
                panel.draw(ui);
            })
            .drop_without_applying_deltas();
        assert!(panel.library_selected.is_none());
    }
}
