// SPDX-License-Identifier: GPL-2.0-or-later
//! The `SessionPanel` "Library" screen: a presentation-only game launcher.
//!
//! [`SessionPanel::draw_library`] renders the landing page shown before a game
//! is running. It offers a text field for a game directory (or a dropped game
//! folder / `Game.ini`), a "Launch game" button, and a grid of recently played
//! games read from `self.recent`. Every action is emitted as a
//! [`SessionActions::open`] path for the frontend to execute — this file never
//! touches the filesystem or the VM, and it uses no [`super::backend::Backend`].
//! The buttons are disabled while `self.busy` and while the directory field is
//! empty; launching while a game is active warns that the current game will be
//! replaced.
use super::session::{SessionActions, SessionPanel};
impl SessionPanel {
    pub(super) fn draw_library(&mut self, ui: &mut egui::Ui, actions: &mut SessionActions) {
        ui.label(
            egui::RichText::new("Your next adventure")
                .size(30.0)
                .strong(),
        );
        ui.weak("One game. A whole workspace of possibilities.");
        ui.add_space(20.0);
        egui::Frame::group(ui.style())
            .inner_margin(16.0)
            .show(ui, |ui| {
                ui.strong("Open a game");
                ui.label("Enter a game directory, or drop a game folder / Game.ini here.");
                ui.add(
                    egui::TextEdit::singleline(&mut self.source)
                        .desired_width(460.0)
                        .hint_text("Game directory"),
                );
                if self.active {
                    ui.label("Launching replaces the current game. Save your progress first.");
                }
                if ui
                    .add_enabled(
                        !self.busy && !self.source.trim().is_empty(),
                        egui::Button::new("Launch game").min_size(egui::vec2(140.0, 36.0)),
                    )
                    .clicked()
                {
                    actions.open = Some(self.source.trim().into());
                    self.open = false;
                }
            });
        ui.add_space(24.0);
        if self.recent.is_empty() {
            ui.strong("Make yourself at home");
            ui.label(
                "Games you open will appear here. Your tools follow the active game automatically.",
            );
        } else {
            ui.strong("Recently played");
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                for path in &self.recent {
                    egui::Frame::group(ui.style())
                        .inner_margin(14.0)
                        .show(ui, |ui| {
                            ui.set_width(220.0);
                            let title = path
                                .trim_end_matches(['/', '\\'])
                                .rsplit(['/', '\\'])
                                .next()
                                .unwrap_or(path);
                            ui.strong(title);
                            ui.add(
                                egui::Label::new(egui::RichText::new(path).small().weak())
                                    .truncate(),
                            )
                            .on_hover_text(path);
                            if ui
                                .add_enabled(!self.busy, egui::Button::new("Play"))
                                .clicked()
                            {
                                actions.open = Some(path.clone());
                                self.open = false;
                            }
                        });
                }
            });
        }
    }
}
