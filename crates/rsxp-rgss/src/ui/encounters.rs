// SPDX-License-Identifier: GPL-2.0-or-later
//! Pokémon Essentials wild-encounter window (`game-api` builds only).
//!
//! `draw_api_windows` is the per-frame hook that opens auxiliary API windows;
//! today it only calls `draw_pokemon_encounters_window`. That window renders
//! `Workspace::pokemon_encounters_data`, the JSON reply to the
//! `pokemon_essentials_encounters` API operation, whose shape is:
//!
//! * `map_id` (int) — picks the title suffix, e.g. "… · Map 3";
//!   `available: false` short-circuits with a notice, and `probability_note`
//!   (string) is shown small beneath the header;
//! * `tables` (array) — each has `name`/`type`, an optional `step_chance`
//!   (percent), and `entries`;
//! * each entry has `species` (string or int), optional `dex_number`, `name`,
//!   `min_level`/`max_level`, and `selection_percent`/`step_percent`.
//!
//! The window is only shown while `pokemon_encounters_window_open` is set; egui
//! keeps its position/size by a fixed window id even as the title follows the
//! map. `pokemon_sprite_texture` lazily fetches and caches an egui texture for
//! an entry, keyed by `sprite_key|species|name`; it tries several asset
//! categories, uses the first frame of an animated horizontal strip, and
//! downscales to at most 512px per side before uploading (see the inline notes).
use super::*;
impl Workspace {
    #[cfg(feature = "game-api")]
    pub(super) fn draw_api_windows(&mut self, context: &egui::Context) {
        self.draw_pokemon_encounters_window(context);
    }

    #[cfg(feature = "game-api")]
    pub(super) fn draw_pokemon_encounters_window(&mut self, context: &egui::Context) {
        if !self.pokemon_encounters_window_open {
            return;
        }
        let data = self.pokemon_encounters_data.clone();
        let map_id = data.as_ref().and_then(|value| value["map_id"].as_i64());
        let title = map_id.map_or_else(
            || "Wild Pokémon encounters".to_owned(),
            |id| format!("Wild Pokémon encounters · Map {id}"),
        );
        let mut open = self.pokemon_encounters_window_open;
        egui::Window::new(title)
            // The displayed title follows the selected map, but the window's
            // identity must remain fixed so egui retains its position/size.
            .id(egui::Id::unique("pokemon_encounters_window"))
            .open(&mut open)
            .default_size(egui::vec2(620.0, 680.0))
            .min_size(egui::vec2(360.0, 280.0))
            .resizable(true)
            .show(context, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Encounter tables");
                    if ui
                        .add_enabled(self.api_pending.is_none(), egui::Button::new("Refresh"))
                        .clicked()
                    {
                        self.queue_api_request("pokemon_essentials_encounters", Vec::new());
                    }
                });
                let Some(data) = data else {
                    ui.weak("Load current-map encounters from the Pokémon Essentials API panel.");
                    return;
                };
                if data["available"].as_bool() == Some(false) {
                    ui.label("No Pokémon Essentials encounter data is available on this map.");
                    return;
                }
                if let Some(note) = data["probability_note"].as_str() {
                    ui.label(egui::RichText::new(note).small().weak());
                }
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let tables = data["tables"].as_array().cloned().unwrap_or_default();
                        if tables.is_empty() {
                            ui.weak("No encounter tables were reported for this map.");
                        }
                        for table in tables {
                            ui.separator();
                            let table_name = table["name"]
                                .as_str()
                                .or_else(|| table["type"].as_str())
                                .unwrap_or("Encounter type");
                            let chance = table["step_chance"]
                                .as_f64()
                                .map(|v| format!(" · step chance {v:.3}%"))
                                .unwrap_or_default();
                            ui.heading(format!("{table_name}{chance}"));
                            for entry in table["entries"].as_array().into_iter().flatten() {
                                let species = entry["species"]
                                    .as_str()
                                    .map(str::to_owned)
                                    .or_else(|| entry["species"].as_i64().map(|n| n.to_string()))
                                    .unwrap_or_default();
                                let sprite_key = entry["dex_number"]
                                    .as_i64()
                                    .map(|number| number.to_string())
                                    .unwrap_or_else(|| species.clone());
                                let name = entry["name"]
                                    .as_str()
                                    .unwrap_or(if species.is_empty() {
                                        "Unknown species"
                                    } else {
                                        &species
                                    })
                                    .to_owned();
                                let min_level = entry["min_level"].as_i64();
                                let max_level = entry["max_level"].as_i64();
                                let selection = entry["selection_percent"].as_f64();
                                let step = entry["step_percent"].as_f64();
                                ui.horizontal(|ui| {
                                    if let Some(texture) = self.pokemon_sprite_texture(
                                        context,
                                        &sprite_key,
                                        &species,
                                        &name,
                                    ) {
                                        ui.add(
                                            egui::Image::new((
                                                texture.id(),
                                                egui::vec2(52.0, 52.0),
                                            ))
                                            .fit_to_exact_size(egui::vec2(52.0, 52.0)),
                                        );
                                    } else {
                                        ui.allocate_ui(egui::vec2(96.0, 52.0), |ui| {
                                            ui.centered_and_justified(|ui| {
                                                ui.weak(format!("No sprite\n{sprite_key}"));
                                            });
                                        });
                                    }
                                    ui.vertical(|ui| {
                                        ui.strong(name);
                                        let level = match (min_level, max_level) {
                                            (Some(a), Some(b)) if a == b => format!("Level {a}"),
                                            (Some(a), Some(b)) => format!("Levels {a}–{b}"),
                                            _ => "Level unknown".to_owned(),
                                        };
                                        ui.label(level);
                                        ui.horizontal(|ui| {
                                            if let Some(value) = selection {
                                                ui.label(format!("Slot share: {value:.3}%"));
                                            }
                                            if let Some(value) = step {
                                                ui.label(format!("Est. per step: {value:.3}%"));
                                            }
                                        });
                                    });
                                });
                            }
                        }
                    });
            });
        self.pokemon_encounters_window_open = open;
    }

    #[cfg(feature = "game-api")]
    pub(super) fn pokemon_sprite_texture(
        &mut self,
        context: &egui::Context,
        sprite_key: &str,
        species: &str,
        name: &str,
    ) -> Option<&egui::TextureHandle> {
        if sprite_key.is_empty() && species.is_empty() && name.is_empty() {
            return None;
        }
        let cache_key = format!("{sprite_key}|{species}|{name}");
        if !self.pokemon_sprite_textures.contains_key(&cache_key) {
            let mut candidates = Vec::new();
            for value in [sprite_key, species, name] {
                if !value.is_empty() {
                    candidates.push(value.to_owned());
                    candidates.push(value.to_ascii_uppercase());
                }
            }
            if let Ok(number) = sprite_key
                .parse::<u32>()
                .or_else(|_| species.parse::<u32>())
            {
                candidates.push(format!("{number:03}"));
                candidates.push(format!("{number:04}"));
            }
            candidates.dedup();
            let image = [
                "Battlers",
                "Pokemon/Front",
                "Pokemon/Icons",
                "Pokemon",
                "Icons",
            ]
            .iter()
            .find_map(|category| {
                candidates
                    .iter()
                    .find_map(|candidate| self.map_asset_image(category, candidate))
            });
            if let Some(image) = image {
                // Encounter sprites are displayed at 52px. Keep uploaded egui
                // textures comfortably below wgpu's per-device texture limit,
                // even if a game's matching asset is a large sprite sheet.
                const MAX_SPRITE_TEXTURE_SIDE: u32 = 512;
                // Animated Essentials sprites are horizontal strips of square
                // frames. Use frame zero instead of squeezing the whole strip.
                let mut sprite = first_pokemon_sprite_frame(image.as_ref());
                if sprite.width().max(sprite.height()) > MAX_SPRITE_TEXTURE_SIDE {
                    let scale =
                        MAX_SPRITE_TEXTURE_SIDE as f32 / sprite.width().max(sprite.height()) as f32;
                    sprite = image::imageops::resize(
                        &sprite,
                        ((sprite.width() as f32 * scale).round() as u32).max(1),
                        ((sprite.height() as f32 * scale).round() as u32).max(1),
                        image::imageops::FilterType::Nearest,
                    );
                }
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    [sprite.width() as usize, sprite.height() as usize],
                    sprite.as_raw(),
                );
                let texture = context.load_texture(
                    format!("pokemon-encounter-{cache_key}"),
                    color_image,
                    egui::TextureOptions::NEAREST,
                );
                self.pokemon_sprite_textures
                    .insert(cache_key.clone(), texture);
            }
        }
        self.pokemon_sprite_textures.get(&cache_key)
    }
}
