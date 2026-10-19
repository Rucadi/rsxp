// SPDX-License-Identifier: GPL-2.0-or-later
//! The `SessionPanel` "Save files" screen: a presentation-only backup/restore UI.
//!
//! [`SessionPanel::draw_saves`] edits a save directory and a backup directory
//! and offers "Back up saves" / "Restore saves". It emits
//! [`SessionActions::backup`] as `(save_directory, backup_directory)` and
//! [`SessionActions::restore`] as `(backup_directory, save_directory)` for the
//! frontend to perform; no copying happens here and no
//! [`super::backend::Backend`] is used. The whole selected directory is copied,
//! restore replaces matching files while keeping a safety copy of each
//! original, and the buttons are disabled while the game is active or busy.
use super::session::{SessionActions, SessionPanel};
impl SessionPanel {
    pub(super) fn draw_saves(&mut self, ui: &mut egui::Ui, actions: &mut SessionActions) {
        ui.heading("Keep your progress safe");
        ui.label("Back up or restore a directory containing save files.");
        ui.add_space(16.0);
        ui.strong("Save directory");
        ui.add(
            egui::TextEdit::singleline(&mut self.save_directory)
                .desired_width(460.0)
                .hint_text("Directory containing only save data"),
        );
        ui.strong("Backup directory");
        ui.add(
            egui::TextEdit::singleline(&mut self.backup_directory)
                .desired_width(460.0)
                .hint_text("An empty directory for a new backup"),
        );
        ui.add_space(12.0);
        ui.label("The entire selected directory is copied. Choose a save directory, not the game installation.");
        ui.weak("Restore replaces matching files and keeps a safety copy of each original. Extra files remain in place.");
        if self.active {
            ui.label("Stop the game before changing save files.");
        }
        if self.busy {
            ui.spinner();
            ui.label("Copying save files…");
        }
        ui.add_enabled_ui(
            !self.active
                && !self.busy
                && !self.save_directory.is_empty()
                && !self.backup_directory.is_empty(),
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Back up saves").clicked() {
                        actions.backup =
                            Some((self.save_directory.clone(), self.backup_directory.clone()));
                    }
                    if ui.button("Restore saves").clicked() {
                        actions.restore =
                            Some((self.backup_directory.clone(), self.save_directory.clone()));
                    }
                });
            },
        );
    }
}
