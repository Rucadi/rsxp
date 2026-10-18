// SPDX-License-Identifier: GPL-2.0-or-later
use super::*;
impl Workspace {
    #[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
    pub(super) fn draw_cable_club_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.separator();
        ui.heading("Cable Club");
        ui.label(
            egui::RichText::new(
                "Host a relay for two EricLostie clients, or point this game's Cable Club client at a relay.",
            )
            .small()
            .weak(),
        );

        let status = self.backend.cable_status();
        if let Some(address) = status.address {
            ui.label(format!("Relay listening: {address}"));
            if ui.button("Stop relay").clicked() {
                self.backend.stop_cable();
            }
            if status.clients.is_empty() {
                ui.weak("Waiting for game clients…");
            } else {
                for client in status.clients {
                    let room = client
                        .room_code
                        .map(|code| format!("room {code:08}"))
                        .unwrap_or_default();
                    ui.label(format!(
                        "{} · {} {} · {}",
                        client.name, client.state, room, client.address
                    ));
                }
            }
            ui.label(
                egui::RichText::new(
                    "For internet play, forward this TCP port on the host router and use the host's reachable address on both games.",
                )
                .small()
                .weak(),
            );
            ui.label(
                egui::RichText::new(
                    "The relay does not load PBS legality data or server battle presets yet; use it with trusted players.",
                )
                .small()
                .weak(),
            );
        } else {
            ui.horizontal(|ui| {
                ui.label("Bind");
                ui.add(egui::TextEdit::singleline(&mut self.cable_bind).desired_width(95.0));
            });
            ui.horizontal(|ui| {
                ui.label("Port");
                ui.add(egui::TextEdit::singleline(&mut self.cable_port).desired_width(72.0));
            });
            if ui.button("Start relay").clicked() {
                match self.cable_port.parse::<u16>() {
                    Ok(port) => {
                        self.cable_error = self.backend.start_cable(&self.cable_bind, port).err()
                    }
                    Err(_) => self.cable_error = Some("Port must be between 1 and 65535".into()),
                }
            }
        }

        ui.separator();
        ui.label(egui::RichText::new("Game client connection").strong());
        ui.label(
            egui::RichText::new(
                "Start the game's Cable Club menu directly, without finding its map NPC.",
            )
            .small()
            .weak(),
        );
        if ui
            .add_enabled(
                self.api_pending.is_none(),
                egui::Button::new("Start Cable Club in game"),
            )
            .clicked()
        {
            self.queue_api_request_tagged("start_cable_club", "cable_club_launch", Vec::new());
        }
        ui.horizontal(|ui| {
            ui.label("Host");
            ui.add(egui::TextEdit::singleline(&mut self.cable_client_host).desired_width(125.0));
        });
        ui.horizontal(|ui| {
            ui.label("Port");
            ui.add(egui::TextEdit::singleline(&mut self.cable_client_port).desired_width(72.0));
            if ui.button("Save for game").clicked() {
                match self.cable_client_port.parse::<u16>() {
                    Ok(port) => match self.backend.save_endpoint(&self.cable_client_host, port) {
                        Ok(path) => {
                            self.cable_saved = Some(path.display().to_string());
                            self.cable_error = None;
                        }
                        Err(error) => self.cable_error = Some(error),
                    },
                    Err(_) => self.cable_error = Some("Port must be between 1 and 65535".into()),
                }
            }
        });
        if let Some(path) = &self.cable_saved {
            ui.label(egui::RichText::new(format!("Saved: {path}")).small().weak());
        }
        if let Some(error) = status.error.or_else(|| self.cable_error.clone()) {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        if let Some(error) = &self.api_error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
    }
}
