// SPDX-License-Identifier: GPL-2.0-or-later
//! Left-hand "Tools" panel: tool selection and the runtime-patch catalog.
//!
//! `draw_patches_panel` builds the resizable `workspace-game-panel` side panel.
//! It shows the logical game size, a row of tool tabs (`Tool::Runtime`,
//! `Tool::Essentials`, and the feature-gated `Tool::PokeApi` / `Tool::OnlinePlay`),
//! and dispatches to the matching body:
//!
//! * `draw_patch_catalog(ui, user_patches)` lists patches from
//!   `rsxp_core::runtime_patch::patches()`, split into built-in and user patches by
//!   the `user/` id prefix. Each row is a checkbox — driving
//!   `Backend::patch_enabled` / `Backend::set_patch_enabled` — plus, when the
//!   patch exposes a `PatchParameter`, a slider that reads and writes
//!   `Backend::patch_value` / `Backend::set_patch_value`.
//! * `draw_api_endpoint` (`game-api` only) shows the external HTTP API status
//!   and starts it via `Backend::start_api`; on wasm it explains that external
//!   networking is unavailable.
use super::*;
impl Workspace {
    pub(super) fn draw_patches_panel(&mut self, context: &egui::Context, info: &WorkspaceInfo) {
        context.root_ui(|root_ui| {
            egui::Panel::left("workspace-game-panel")
                .resizable(true)
                .default_size(280.0)
                .size_range(230.0..=500.0)
                .show(root_ui, |ui| {
                    ui.heading("Tools");
                    ui.horizontal_wrapped(|ui| {
                        ui.selectable_value(&mut self.tool, Tool::Runtime, "Runtime");
                        ui.selectable_value(&mut self.tool, Tool::Essentials, "Pokémon Essentials");
                        #[cfg(feature = "game-api")]
                        ui.selectable_value(&mut self.tool, Tool::PokeApi, "pokeAPI");
                        #[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
                        ui.selectable_value(&mut self.tool, Tool::OnlinePlay, "Online play");
                    });
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.separator();
                        ui.label("Game view");
                        ui.label(format!("{} × {}", info.logical_size.0, info.logical_size.1));
                        ui.add_space(8.0);
                        ui.separator();
                        match self.tool {
                            Tool::Runtime => self.draw_patch_catalog(ui, true),
                            #[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
                            Tool::OnlinePlay => self.draw_cable_club_panel(ui),
                            Tool::Essentials => {
                                self.draw_patch_catalog(ui, false);
                                #[cfg(feature = "game-api")]
                                self.draw_essentials_tools(ui);
                            }
                            #[cfg(feature = "game-api")]
                            Tool::PokeApi => {
                                if ui.button("Open API + testing window").clicked() {
                                    self.api_tools_open = true;
                                }
                            }
                        }
                    });
                })
        });
    }

    fn draw_patch_catalog(&mut self, ui: &mut egui::Ui, user_patches: bool) {
        ui.heading(if user_patches {
            "Runtime patches"
        } else {
            "Essentials patches"
        });
        if user_patches {
            ui.label("Put Ruby patches in the game’s patches folder, then restart the session.");
        }
        ui.weak("Applied live to the running game.");
        let patches = rsxp_core::runtime_patch::patches();
        let mut count = 0;
        for patch in patches
            .iter()
            .filter(|patch| patch.id.starts_with("user/") == user_patches)
        {
            count += 1;
            let mut enabled = self.backend.patch_enabled(&patch.id);
            if ui.checkbox(&mut enabled, patch.name.as_ref()).changed() {
                self.backend.set_patch_enabled(&patch.id, enabled);
            }
            if let Some(parameter) = &patch.parameter {
                let mut value = self
                    .backend
                    .patch_value(&patch.id)
                    .unwrap_or(parameter.default);
                ui.add_enabled_ui(enabled, |ui| {
                    let slider = egui::Slider::new(&mut value, parameter.min..=parameter.max)
                        .step_by(parameter.step)
                        .text(parameter.label)
                        .suffix(parameter.suffix);
                    if ui.add(slider).changed() {
                        self.backend.set_patch_value(&patch.id, value);
                    }
                });
            }
            ui.label(
                egui::RichText::new(patch.description.as_ref())
                    .small()
                    .weak(),
            );
            ui.add_space(2.0);
        }
        if count == 0 {
            ui.weak("No patches found.");
        }
    }
    #[cfg(feature = "game-api")]
    pub(super) fn draw_api_endpoint(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.separator();
        ui.heading("Game API");
        if cfg!(target_arch = "wasm32") {
            ui.label("Game tools use the local game connection. External networking is not available in this browser host.");
            return;
        }
        let port = self.backend.listening_port();
        if let Some(port) = port {
            ui.label(format!("HTTP endpoint: 127.0.0.1:{port}"));
        } else {
            ui.label(
                            egui::RichText::new(
                                "The local inspector works without HTTP. Start this endpoint for MCP and external clients.",
                            )
                            .small()
                            .weak(),
                        );
            ui.horizontal(|ui| {
                ui.label("Port");
                ui.add(egui::TextEdit::singleline(&mut self.api_port).desired_width(72.0));
            });
            if ui.button("Start HTTP API").clicked() {
                match self.api_port.parse::<u16>() {
                    Ok(port) if port != 0 => {
                        self.api_error = self.backend.start_api(port).err();
                    }
                    _ => self.api_error = Some("Port must be between 1 and 65535".into()),
                }
            }
            if let Some(error) = &self.api_error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
        }
        ui.label(
            egui::RichText::new(
                "Optional external endpoint · loopback only · stays active until exit.",
            )
            .small()
            .weak(),
        );
    }
}
