// SPDX-License-Identifier: GPL-2.0-or-later
//! Party-slot detail renderer for the Pokémon Essentials API window.
//!
//! `draw_pokemon_details` renders `Workspace::api_response_json` — the reply to
//! a party-slot detail request — into a scrolling `ui`. It requires that reply
//! to contain `ivs` or `evs`; otherwise it prompts the user to request a slot.
//! It first prints a fixed set of scalar fields (species name, nickname, level,
//! form, gender, shiny, HP, status), then a fixed set of nested tables (battle
//! stats, base stats, IVs, EVs, ability, nature, held item, moves, owner,
//! obtain, species data, extensions), rendering objects and arrays as
//! two-column grids through `display_json_value`. Only compiled under `game-api`.
use super::*;
impl Workspace {
    #[cfg(feature = "game-api")]
    pub(super) fn draw_pokemon_details(&self, ui: &mut egui::Ui) {
        let Some(details) = self.api_response_json.as_ref() else {
            return;
        };
        if details.get("ivs").is_none() && details.get("evs").is_none() {
            ui.label("Request details for a party slot to populate this window.");
            return;
        }

        for (label, key) in [
            ("Species", "species_name"),
            ("Nickname", "name"),
            ("Level", "level"),
            ("Form", "form"),
            ("Gender", "gender"),
            ("Shiny", "shiny"),
            ("HP", "hp"),
            ("Status", "status"),
        ] {
            if let Some(value) = details.get(key) {
                ui.label(format!("{label}: {}", display_json_value(value)));
            }
        }

        for (label, key) in [
            ("Battle stats", "stats"),
            ("Base stats", "base_stats"),
            ("IVs", "ivs"),
            ("EVs", "evs"),
            ("Ability", "ability"),
            ("Nature", "nature"),
            ("Held item", "held_item"),
            ("Moves", "moves"),
            ("Owner", "owner"),
            ("Obtained", "obtain"),
            ("Species data", "species_data"),
            ("Extensions", "extensions"),
        ] {
            if let Some(value) = details.get(key) {
                ui.add_space(6.0);
                ui.label(egui::RichText::new(label).strong());
                egui::Grid::new(("pokemon-detail", key))
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| match value {
                        serde_json::Value::Object(fields) => {
                            for (name, value) in fields {
                                ui.label(name);
                                ui.label(display_json_value(value));
                                ui.end_row();
                            }
                        }
                        serde_json::Value::Array(values) => {
                            for (index, value) in values.iter().enumerate() {
                                ui.label(index.to_string());
                                ui.label(display_json_value(value));
                                ui.end_row();
                            }
                        }
                        _ => {
                            ui.label(display_json_value(value));
                            ui.end_row();
                        }
                    });
            }
        }
    }
}
