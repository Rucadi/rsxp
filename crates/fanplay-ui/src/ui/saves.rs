// SPDX-License-Identifier: GPL-2.0-or-later
//! The `SessionPanel` "Save files" screen.
//!
//! Desktop hosts expose real save/backup paths and keep the directory-to-
//! directory copy controls. The panel only emits [`SessionActions`]; the
//! platform adapter performs the copy.
use super::session::{SessionActions, SessionPanel};
use super::theme::{self, ButtonKind, ButtonSize, Tone};

impl SessionPanel {
    pub(super) fn draw_saves(&mut self, ui: &mut egui::Ui, actions: &mut SessionActions) {
        ui.horizontal_wrapped(|ui| {
            ui.label(theme::display("Save files", 32.0));
            theme::tag(ui, "snapshots", Tone::Lime);
            if self.active {
                theme::tag(ui, "session running", Tone::Warning);
            }
        });
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("Back up or restore the save files for this game.")
                .font(theme::body_font(14.0))
                .color(theme::TEXT_MUTED),
        );
        ui.add_space(theme::LG);

        if self.capabilities.save_backups {
            theme::section_label(ui, "Snapshots", theme::LIME);
            ui.add_space(6.0);
            theme::card(false, None).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    theme::tag(ui, "safe to keep", Tone::Lime);
                    theme::tag(
                        ui,
                        if self.capabilities.directory_backups {
                            "FANPLAY/backups"
                        } else {
                            "session storage"
                        },
                        Tone::Neutral,
                    );
                });
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(if self.capabilities.directory_backups {
                        "Snapshots are stored under FANPLAY/backups and are safe to copy between machines."
                    } else {
                        "Snapshots are stored for the current session."
                    })
                    .font(theme::body_font(13.0))
                    .color(theme::TEXT_MUTED),
                );
                ui.add_space(12.0);
                ui.add_enabled_ui(!self.busy, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if theme::button(ui, "Create backup", ButtonKind::Primary).clicked() {
                            actions.create_backup = true;
                        }
                        if theme::button(ui, "Restore latest", ButtonKind::Secondary).clicked() {
                            actions.restore_latest = true;
                        }
                    });
                });
                if self.active {
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new("Stop the game before restoring save files.")
                            .font(theme::body_font(12.0))
                            .color(theme::WARNING),
                    );
                }
            });
            ui.add_space(theme::LG);
        }

        if self.capabilities.directory_backups {
            theme::section_label(ui, "Manual directory copy", theme::CYAN);
            ui.add_space(6.0);
            theme::card(false, None).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    egui::RichText::new(
                        "The entire selected directory is copied. Choose a save directory, not the game installation.",
                    )
                    .font(theme::body_font(13.0))
                    .color(theme::TEXT_MUTED),
                );
                ui.add_space(10.0);
                egui::Grid::new("save-directories")
                    .num_columns(2)
                    .spacing([14.0, 10.0])
                    .show(ui, |ui| {
                        ui.label(theme::eyebrow("Save directory"));
                        ui.add(
                            egui::TextEdit::singleline(&mut self.save_directory)
                                .desired_width(f32::INFINITY)
                                .hint_text("Directory containing only save data"),
                        );
                        ui.end_row();
                        ui.label(theme::eyebrow("Backup directory"));
                        ui.add(
                            egui::TextEdit::singleline(&mut self.backup_directory)
                                .desired_width(f32::INFINITY)
                                .hint_text("An empty directory for a new backup"),
                        );
                        ui.end_row();
                    });
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("Restore replaces matching files and keeps a safety copy of each original. Extra files remain in place.")
                        .font(theme::body_font(11.0))
                        .color(theme::TEXT_FAINT),
                );
                if self.active {
                    ui.label(
                        egui::RichText::new("Stop the game before changing save files.")
                            .font(theme::body_font(12.0))
                            .color(theme::WARNING),
                    );
                }
                if self.busy {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Copying save files…");
                    });
                }
                ui.add_space(10.0);
                ui.add_enabled_ui(
                    !self.active
                        && !self.busy
                        && !self.save_directory.is_empty()
                        && !self.backup_directory.is_empty(),
                    |ui| {
                        ui.horizontal_wrapped(|ui| {
                            if theme::button_sized(
                                ui,
                                "Back up saves",
                                ButtonKind::Secondary,
                                ButtonSize::Sm,
                            )
                            .clicked()
                            {
                                actions.backup = Some((
                                    self.save_directory.clone(),
                                    self.backup_directory.clone(),
                                ));
                            }
                            if theme::button_sized(
                                ui,
                                "Restore saves",
                                ButtonKind::Ghost,
                                ButtonSize::Sm,
                            )
                            .clicked()
                            {
                                actions.restore = Some((
                                    self.backup_directory.clone(),
                                    self.save_directory.clone(),
                                ));
                            }
                        });
                    },
                );
            });
        }
    }
}
