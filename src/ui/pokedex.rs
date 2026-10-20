// SPDX-License-Identifier: GPL-2.0-or-later
//! Game-backed species explorer for Pokémon Essentials projects.
//!
//! The Pokedex window lists every species and form the running game exposes,
//! then renders a detail view with Overview, Locations, Moves, Evolution, and
//! Cheats tabs. Its requests and cached results live in `Workspace::pokedex`,
//! entirely separate from the API console's foreground request slot, so opening
//! or refreshing the dex never overwrites `api_response`/`api_error`.
//!
//! ## Main types and functions
//! * `Pokedex` (`pub(super)`) — cached catalog (`Vec<Value>`), the query and
//!   its memoized matches, `known_only` filter, the selected species id, the
//!   active [`DetailTab`], per-species detail cache, and load/error state.
//! * `DetailTab` — which detail view is shown.
//! * `EncounterCheats` — options for the Cheats tab's "start a wild encounter".
//! * [`Workspace::request_pokedex`] (sends `pokemon_essentials_pokedex` with
//!   `["refresh"]` or no args) and `poll_pokedex`, which consumes the index or a
//!   single `pokemon_essentials_species_entry`, and lazily requests details for
//!   the current selection.
//! * [`Workspace::draw_pokedex_window`] plus the per-tab drawing helpers.
//! * `type_color` / `types` — shared type-chip rendering (also used by the
//!   RegistryDex for moves).
//! * `start_cheat_encounter` / `poll_cheats` — queue
//!   `pokemon_essentials_wild_encounter` and report its outcome.
//!
//! ## Backend use
//! All data comes from `backend.send_request` / `backend.take_response`, with
//! the pending id + species kept in `pokedex.pending`; sprite textures for the
//! header use `Workspace::pokemon_sprite_texture`. Search reuses the shared
//! `dex_search::{fuzzy_score, normalized}` helpers (name, id, and dex number).
//!
//! ## JSON schema
//! The index response is `{ "species": [{ "id", "name", "number", "form",
//! "form_name", "types": [...], "source", "map_count" }], "note" }`; errors come
//! back as `{ "error": "..." }`. A species entry adds `entry`, `stats`
//! (HP/ATTACK/DEFENSE/SPECIAL_ATTACK/SPECIAL_DEFENSE/SPEED), `facts`
//! (`height_m`, `weight_kg`, `evs`, `base_exp`, `happiness`, ...),
//! `abilities`/`hidden_abilities` (`[{ id, name, description }]`), `locations`
//! (`[{ map_id, map_name, type, version, min_level, max_level, percent }]`),
//! `moves` (`[{ id, name, method, level, type, category, power, accuracy, pp,
//! priority, description, function_code }]`), `evolutions`/`previous`
//! (`[{ id, name, condition, method, parameter }]`), and `raw`. The cheats
//! response reports `{ "name", "level", "shiny" }` or `{ "error": "..." }`.
use super::dex_search::{fuzzy_score, normalized};
use super::*;
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

#[derive(Debug, Default, PartialEq, Eq, Clone, Copy, Hash)]
enum DetailTab {
    #[default]
    Overview,
    Locations,
    Moves,
    Evolution,
    Cheats,
}

#[derive(Default)]
pub(super) struct Pokedex {
    pub open: bool,
    pub catalog: Vec<Value>,
    query: String,
    matches: Vec<usize>,
    matched_query: Option<(String, bool)>,
    known_only: bool,
    selected: String,
    tab: DetailTab,
    details: HashMap<String, Arc<Value>>,
    pending: Option<(u64, String)>,
    requested: String,
    error: Option<String>,
    note: String,
    move_query: String,
    move_method: String,
    cheats: EncounterCheats,
}

/// Options for the Cheats tab's "start a wild encounter" action.
struct EncounterCheats {
    species: String,
    level: i32,
    shiny: bool,
    form: i32,
    gender: String,
    nature: String,
    ability: String,
    item: String,
    max_ivs: bool,
    set_friendship: bool,
    friendship: i32,
    pending: Option<u64>,
    notice: Option<(bool, String)>,
}
impl Default for EncounterCheats {
    fn default() -> Self {
        Self {
            species: String::new(),
            level: 5,
            shiny: false,
            form: 0,
            gender: String::new(),
            nature: String::new(),
            ability: String::new(),
            item: String::new(),
            max_ivs: false,
            set_friendship: false,
            friendship: 255,
            pending: None,
            notice: None,
        }
    }
}
impl EncounterCheats {
    fn reset(&mut self, data: &Value) {
        self.species = text(data, "id").to_owned();
        self.level = 5;
        self.shiny = false;
        self.form = number(data, "form") as i32;
        self.gender.clear();
        self.nature.clear();
        self.ability = species_abilities(data)
            .first()
            .map(|(id, _)| id.clone())
            .unwrap_or_default();
        self.item.clear();
        self.max_ivs = false;
        self.set_friendship = false;
        self.friendship = 255;
        self.pending = None;
        self.notice = None;
    }
}

const NATURES: [&str; 25] = [
    "HARDY", "LONELY", "BRAVE", "ADAMANT", "NAUGHTY", "BOLD", "DOCILE", "RELAXED", "IMPISH", "LAX",
    "TIMID", "HASTY", "SERIOUS", "JOLLY", "NAIVE", "MODEST", "MILD", "QUIET", "BASHFUL", "RASH",
    "CALM", "GENTLE", "SASSY", "CAREFUL", "QUIRKY",
];

/// Reads a game ID that may be a string (newer Essentials) or a number (older PBS data).
fn id_text(value: &Value) -> String {
    match &value["id"] {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        _ => String::new(),
    }
}

fn species_abilities(data: &Value) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    for key in ["abilities", "hidden_abilities"] {
        for ability in data[key].as_array().into_iter().flatten() {
            let id = id_text(ability);
            if !id.is_empty() {
                rows.push((id, text(ability, "name").to_owned()));
            }
        }
    }
    rows
}

fn gender_label(value: &str) -> &str {
    match value {
        "male" => "Male",
        "female" => "Female",
        _ => "Random",
    }
}

fn nature_label(value: &str) -> &str {
    if value.is_empty() {
        "Random"
    } else {
        value
    }
}

fn ability_label<'a>(value: &'a str, abilities: &'a [(String, String)]) -> &'a str {
    if value.is_empty() {
        "Species default"
    } else {
        abilities
            .iter()
            .find(|(id, _)| id == value)
            .map(|(_, name)| name.as_str())
            .unwrap_or(value)
    }
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
fn number(value: &Value, key: &str) -> i64 {
    value[key].as_i64().unwrap_or(0)
}
fn species_score(query: &str, row: &Value) -> Option<usize> {
    let query = normalized(query);
    let dex = number(row, "number").to_string();
    let numeric = query.trim_start_matches('0');
    let number_match = if !query.is_empty() && query.chars().all(|c| c.is_ascii_digit()) {
        fuzzy_score(if numeric.is_empty() { "0" } else { numeric }, &dex)
    } else {
        None
    };
    [
        fuzzy_score(&query, &normalized(text(row, "name"))),
        fuzzy_score(&query, &normalized(text(row, "id"))),
        number_match,
    ]
    .into_iter()
    .flatten()
    .min()
}

pub(super) fn type_color(kind: &str) -> egui::Color32 {
    use egui::Color32 as C;
    match kind.to_ascii_uppercase().as_str() {
        "FIRE" => C::from_rgb(222, 100, 66),
        "WATER" => C::from_rgb(70, 140, 220),
        "GRASS" => C::from_rgb(92, 164, 91),
        "ELECTRIC" => C::from_rgb(214, 181, 57),
        "PSYCHIC" | "FAIRY" => C::from_rgb(203, 107, 156),
        "ICE" => C::from_rgb(87, 177, 187),
        "POISON" | "GHOST" => C::from_rgb(155, 113, 187),
        "DRAGON" => C::from_rgb(106, 119, 208),
        "DARK" | "STEEL" => C::from_rgb(126, 143, 163),
        "GROUND" | "ROCK" => C::from_rgb(174, 144, 91),
        "FIGHTING" => C::from_rgb(193, 100, 94),
        "BUG" => C::from_rgb(143, 160, 66),
        _ => C::from_rgb(143, 157, 173),
    }
}
fn types(ui: &mut egui::Ui, row: &Value) {
    if let Some(kinds) = row["types"].as_array() {
        for kind in kinds.iter().filter_map(Value::as_str) {
            egui::Frame::new()
                .fill(type_color(kind).gamma_multiply(0.22))
                .corner_radius(5)
                .inner_margin(egui::Margin::symmetric(8, 3))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(kind)
                            .color(type_color(kind))
                            .small()
                            .strong(),
                    );
                });
        }
    }
}
fn pretty(value: &Value) -> String {
    match value {
        Value::Null => "Not supplied".into(),
        Value::String(s) => s.clone(),
        Value::Array(rows) => rows.iter().map(pretty).collect::<Vec<_>>().join(", "),
        Value::Object(_) => serde_json::to_string(value).unwrap_or_default(),
        _ => value.to_string(),
    }
}
fn fact_value(key: &str, value: &Value) -> String {
    match key {
        "height_m" => format!("{} m", pretty(value)),
        "weight_kg" => format!("{} kg", pretty(value)),
        "evs" => value
            .as_object()
            .map(|stats| {
                stats
                    .iter()
                    .filter(|(_, value)| value.as_i64().unwrap_or(0) > 0)
                    .map(|(stat, value)| {
                        format!(
                            "{} +{}",
                            match stat.as_str() {
                                "HP" => "HP",
                                "ATTACK" => "Attack",
                                "DEFENSE" => "Defense",
                                "SPECIAL_ATTACK" => "Sp. Attack",
                                "SPECIAL_DEFENSE" => "Sp. Defense",
                                "SPEED" => "Speed",
                                other => other,
                            },
                            pretty(value)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| "None".into()),
        _ => pretty(value),
    }
}

fn section(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.add_space(12.0);
    ui.heading(title);
    if !subtitle.is_empty() {
        ui.weak(subtitle);
    }
    ui.add_space(6.0);
}

impl Workspace {
    pub(super) fn request_pokedex(&mut self, refresh: bool) {
        if self.pokedex.pending.is_some() {
            return;
        }
        let args = if refresh {
            vec!["refresh".into()]
        } else {
            Vec::new()
        };
        match self
            .backend
            .send_request("pokemon_essentials_pokedex", args)
        {
            Ok(id) => {
                self.pokedex.pending = Some((id, String::new()));
                self.pokedex.error = None;
            }
            Err(error) => self.pokedex.error = Some(error),
        }
    }

    fn poll_pokedex(&mut self) {
        if let Some((id, species)) = self.pokedex.pending.clone()
            && let Some(result) = self.backend.take_response(id)
        {
            self.pokedex.pending = None;
            match result.and_then(|body| {
                serde_json::from_str::<Value>(&body).map_err(|error| error.to_string())
            }) {
                Ok(value) if value["error"].is_string() => {
                    self.pokedex.error = Some(text(&value, "error").into())
                }
                Ok(value) if species.is_empty() => {
                    self.pokedex.catalog = value["species"].as_array().cloned().unwrap_or_default();
                    self.pokedex.note = text(&value, "note").into();
                    self.pokedex.matched_query = None;
                    self.pokedex.details.clear();
                    self.pokedex.requested.clear();
                    if !self
                        .pokedex
                        .catalog
                        .iter()
                        .any(|row| text(row, "id") == self.pokedex.selected)
                    {
                        self.pokedex.selected = self
                            .pokedex
                            .catalog
                            .first()
                            .map(|row| text(row, "id").to_owned())
                            .unwrap_or_default();
                    }
                    self.pokedex.error = None;
                }
                Ok(value) => {
                    self.pokedex.details.insert(species, Arc::new(value));
                    self.pokedex.error = None;
                }
                Err(error) => self.pokedex.error = Some(error),
            }
        }
        let dex = &mut self.pokedex;
        if dex.open
            && dex.pending.is_none()
            && !dex.selected.is_empty()
            && !dex.details.contains_key(&dex.selected)
            && dex.requested != dex.selected
        {
            dex.requested = dex.selected.clone();
            match self.backend.send_request(
                "pokemon_essentials_species_entry",
                vec![dex.selected.clone()],
            ) {
                Ok(id) => {
                    dex.pending = Some((id, dex.selected.clone()));
                    dex.error = None;
                }
                Err(error) => dex.error = Some(error),
            }
        }
    }

    pub(super) fn draw_pokedex_window(&mut self, context: &egui::Context) {
        self.poll_pokedex();
        self.poll_cheats();
        if !self.pokedex.open {
            return;
        }
        let mut open = true;
        egui::Window::new("Pokédex · game species explorer")
            .id(egui::Id::unique("essentials-pokedex"))
            .open(&mut open)
            .default_size(egui::vec2(1110.0, 760.0))
            .min_size(egui::vec2(780.0, 500.0))
            .resizable(true)
            .show(context, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.heading("Discover your game’s Pokémon");
                        ui.weak("Game data · custom species, forms, moves and evolution rules");
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(
                                self.pokedex.pending.is_none(),
                                egui::Button::new("Refresh data"),
                            )
                            .clicked()
                        {
                            self.request_pokedex(true);
                        }
                    });
                });
                ui.separator();
                if let Some(error) = self.pokedex.error.clone() {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                        if ui.button("Try again").clicked() {
                            self.pokedex.requested.clear();
                            if self.pokedex.catalog.is_empty() {
                                self.request_pokedex(true);
                            }
                        }
                    });
                }
                let (body, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
                let gap = 12.0;
                let list_width = 285.0_f32.min((body.width() - gap) * 0.4);
                let list_rect =
                    egui::Rect::from_min_size(body.min, egui::vec2(list_width, body.height()));
                let detail_rect = egui::Rect::from_min_max(
                    egui::pos2(list_rect.right() + gap, body.top()),
                    body.max,
                );
                ui.painter().vline(
                    list_rect.right() + gap / 2.0,
                    body.y_range(),
                    ui.visuals().widgets.noninteractive.bg_stroke,
                );
                self.draw_pokedex_list(&mut bounded_tool_pane(ui, "pokedex-list-pane", list_rect));
                self.draw_species_detail(&mut bounded_tool_pane(
                    ui,
                    "pokedex-detail-pane",
                    detail_rect,
                ));
            });
        self.pokedex.open = open;
    }

    fn draw_pokedex_list(&mut self, ui: &mut egui::Ui) {
        ui.add(
            egui::TextEdit::singleline(&mut self.pokedex.query)
                .hint_text("Search name or ID · e.g. Pikachu, 025")
                .desired_width(f32::INFINITY),
        );
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.pokedex.known_only, "Known sources only");
            if !self.pokedex.query.is_empty() && ui.small_button("Clear").clicked() {
                self.pokedex.query.clear();
            }
        });
        let signature = (self.pokedex.query.clone(), self.pokedex.known_only);
        if self.pokedex.matched_query.as_ref() != Some(&signature) {
            let mut matches: Vec<_> = self
                .pokedex
                .catalog
                .iter()
                .enumerate()
                .filter(|(_, row)| !self.pokedex.known_only || row["source"].is_string())
                .filter_map(|(index, row)| {
                    species_score(&self.pokedex.query, row)
                        .map(|score| (score, number(row, "number"), number(row, "form"), index))
                })
                .collect();
            matches.sort();
            self.pokedex.matches = matches.into_iter().map(|row| row.3).collect();
            self.pokedex.matched_query = Some(signature);
        }
        let matches = &self.pokedex.matches;
        ui.weak(format!(
            "{} results · {} species & forms",
            matches.len(),
            self.pokedex.catalog.len()
        ));
        if self.pokedex.catalog.is_empty() {
            if self.pokedex.pending.is_some() {
                ui.spinner();
                ui.label("Reading the game’s species and encounter tables…");
            } else {
                ui.label("Start an Essentials game, then refresh data.");
            }
        } else if matches.is_empty() {
            ui.add_space(20.0);
            ui.heading("No matches");
            ui.label("Try a shorter name or species ID, or turn off the source filter.");
        }
        egui::ScrollArea::vertical()
            .id_salt("dex-list")
            .auto_shrink([false, false])
            .show_rows(ui, 60.0, matches.len(), |ui, range| {
                for index in range {
                    let row = &self.pokedex.catalog[matches[index]];
                    let id = text(row, "id");
                    let form = text(row, "form_name");
                    let title = if form.is_empty() {
                        text(row, "name").to_owned()
                    } else {
                        format!("{} · {}", text(row, "name"), form)
                    };
                    let source = text(row, "source");
                    let selected = self.pokedex.selected == id;
                    let fill = if selected {
                        ui.visuals().selection.bg_fill
                    } else {
                        ui.visuals().faint_bg_color
                    };
                    let response = egui::Frame::new()
                        .fill(fill)
                        .corner_radius(7)
                        .inner_margin(8)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.horizontal(|ui| {
                                ui.weak(format!("#{:03}", number(row, "number")));
                                ui.strong(&title);
                            });
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} · {}",
                                    id,
                                    if source.is_empty() {
                                        "Source not indexed"
                                    } else {
                                        source
                                    }
                                ))
                                .small()
                                .weak(),
                            );
                        })
                        .response;
                    if ui
                        .interact(
                            response.rect,
                            response.id.with("select"),
                            egui::Sense::click(),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        self.pokedex.selected = id.into();
                        self.pokedex.move_query.clear();
                    }
                }
            });
    }

    fn draw_species_detail(&mut self, ui: &mut egui::Ui) {
        let Some(data) = self.pokedex.details.get(&self.pokedex.selected).cloned() else {
            ui.add_space(60.0);
            if self.pokedex.pending.is_some() {
                ui.spinner();
                ui.heading("Loading Pokémon…");
            } else {
                ui.heading("Choose a Pokémon");
                ui.label("Its game stats, locations, moves and evolutions will appear here.");
            }
            return;
        };
        egui::Frame::new()
            .fill(egui::Color32::from_rgb(34, 45, 57))
            .corner_radius(12)
            .inner_margin(16)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let sprite_key = number(&data, "number").to_string();
                    let id = text(&data, "id");
                    if let Some(texture) =
                        self.pokemon_sprite_texture(ui.ctx(), id, &sprite_key, text(&data, "name"))
                    {
                        ui.add(
                            egui::Image::new(texture).fit_to_exact_size(egui::vec2(100.0, 100.0)),
                        );
                    } else {
                        ui.allocate_ui(egui::vec2(100.0, 100.0), |ui| {
                            ui.centered_and_justified(|ui| {
                                ui.label(
                                    egui::RichText::new(format!("#{:03}", number(&data, "number")))
                                        .size(48.0)
                                        .color(egui::Color32::from_rgb(91, 173, 156)),
                                );
                            });
                        });
                    }
                    ui.vertical(|ui| {
                        ui.weak(format!("#{:03} · {}", number(&data, "number"), id));
                        ui.label(egui::RichText::new(text(&data, "name")).size(30.0).strong());
                        if !text(&data, "form_name").is_empty() {
                            ui.label(text(&data, "form_name"));
                        }
                        ui.horizontal(|ui| types(ui, &data));
                        let source = text(&data, "source");
                        ui.weak(if source.is_empty() {
                            "Acquisition source not indexed".to_owned()
                        } else {
                            format!(
                                "{} · {} wild map{}",
                                source,
                                number(&data, "map_count"),
                                if number(&data, "map_count") == 1 {
                                    ""
                                } else {
                                    "s"
                                }
                            )
                        });
                    });
                });
            });
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            for (tab, label) in [
                (DetailTab::Overview, "Overview"),
                (DetailTab::Locations, "Locations"),
                (DetailTab::Moves, "Moves"),
                (DetailTab::Evolution, "Evolution"),
                (DetailTab::Cheats, "Cheats"),
            ] {
                ui.selectable_value(&mut self.pokedex.tab, tab, label);
            }
            if ui.small_button("Copy Pokémon data").clicked() {
                ui.ctx()
                    .copy_text(serde_json::to_string_pretty(&*data).unwrap_or_default());
            }
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt((
                "dex-detail",
                self.pokedex.tab,
                self.pokedex.selected.clone(),
            ))
            .auto_shrink([false, false])
            .show(ui, |ui| match self.pokedex.tab {
                DetailTab::Overview => self.draw_species_overview(ui, &data),
                DetailTab::Locations => self.draw_species_locations(ui, &data),
                DetailTab::Moves => self.draw_species_moves(ui, &data),
                DetailTab::Evolution => self.draw_species_evolutions(ui, &data),
                DetailTab::Cheats => self.draw_species_cheats(ui, &data),
            });
    }

    fn draw_species_overview(&self, ui: &mut egui::Ui, data: &Value) {
        if !text(data, "entry").is_empty() {
            ui.label(text(data, "entry"));
        }
        ui.columns(2, |columns| {
            let bar_width = (columns[0].available_width() - 130.0).max(70.0);
            {
                let ui = &mut columns[0];
                section(
                    ui,
                    "Base stats",
                    "The species’ base stats, before levels, IVs, EVs and nature.",
                );
                let stats = &data["stats"];
                egui::Grid::new("dex-base-stats")
                    .num_columns(3)
                    .spacing([14.0, 9.0])
                    .show(ui, |ui| {
                        for (key, label) in [
                            ("HP", "HP"),
                            ("ATTACK", "Attack"),
                            ("DEFENSE", "Defense"),
                            ("SPECIAL_ATTACK", "Sp. Attack"),
                            ("SPECIAL_DEFENSE", "Sp. Defense"),
                            ("SPEED", "Speed"),
                        ] {
                            let value = stats[key]
                                .as_f64()
                                .or_else(|| stats[key.to_lowercase()].as_f64());
                            ui.label(label);
                            ui.label(value.map(|v| v.to_string()).unwrap_or_else(|| "—".into()));
                            ui.add(
                                egui::ProgressBar::new((value.unwrap_or(0.0) / 255.0) as f32)
                                    .desired_width(bar_width)
                                    .fill(egui::Color32::from_rgb(75, 167, 147)),
                            );
                            ui.end_row();
                        }
                    });
                let total: f64 = stats
                    .as_object()
                    .map(|fields| fields.values().filter_map(Value::as_f64).sum())
                    .unwrap_or(0.0);
                ui.strong(format!("Base stat total  {}", total));
            }
            {
                let ui = &mut columns[1];
                section(
                    ui,
                    "Abilities",
                    "Descriptions from this game’s ability data.",
                );
                for (field, label) in [
                    ("abilities", "Ability"),
                    ("hidden_abilities", "Hidden ability"),
                ] {
                    for ability in data[field].as_array().into_iter().flatten() {
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.strong(format!("{} · {}", text(ability, "name"), label));
                            if !text(ability, "description").is_empty() {
                                ui.label(text(ability, "description"));
                            }
                        });
                    }
                }
            }
        });
        section(ui, "Species information", "");
        egui::Grid::new("dex-facts").striped(true).show(ui, |ui| {
            if let Some(facts) = data["facts"].as_object() {
                for (key, value) in facts {
                    if value.as_array().is_some_and(Vec::is_empty) {
                        continue;
                    }
                    ui.label(match key.as_str() {
                        "height_m" => "Height".into(),
                        "weight_kg" => "Weight".into(),
                        "evs" => "EV yield".into(),
                        "base_exp" => "Base experience".into(),
                        "happiness" => "Base friendship".into(),
                        _ => {
                            let label = key.replace('_', " ");
                            let mut chars = label.chars();
                            chars
                                .next()
                                .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
                                .unwrap_or_default()
                        }
                    });
                    ui.label(fact_value(key, value));
                    ui.end_row();
                }
            }
        });
        ui.collapsing("Additional game fields", |ui| {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(
                        serde_json::to_string_pretty(&data["raw"]).unwrap_or_default(),
                    )
                    .monospace(),
                )
                .selectable(true),
            );
        });
    }

    fn draw_species_locations(&self, ui: &mut egui::Ui, data: &Value) {
        section(
            ui,
            "Where to find it",
            "Wild encounters across every stored encounter version.",
        );
        let source = text(data, "source");
        ui.label(if source.is_empty() {
            "No known acquisition source is indexed.".into()
        } else {
            format!("Known source: {source}")
        });
        let rows = data["locations"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if rows.is_empty() {
            ui.add_space(20.0);
            ui.heading("No wild encounter locations");
            ui.label("Check Evolution for related species. Gifts, trades and scripted encounters can exist outside wild encounter tables.");
        }
        let mut maps = std::collections::BTreeMap::<(i64, String), Vec<&Value>>::new();
        for row in rows {
            maps.entry((number(row, "map_id"), text(row, "map_name").to_owned()))
                .or_default()
                .push(row);
        }
        ui.weak(format!(
            "{} map{} · {} encounter table{}",
            maps.len(),
            if maps.len() == 1 { "" } else { "s" },
            rows.len(),
            if rows.len() == 1 { "" } else { "s" }
        ));
        for ((map_id, name), tables) in maps {
            egui::Frame::group(ui.style())
                .inner_margin(12)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.strong(format!("{name} · Map {map_id}"));
                    for row in tables {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(text(row, "type"));
                            ui.separator();
                            ui.label(format!(
                                "Lv. {}–{}",
                                number(row, "min_level"),
                                number(row, "max_level")
                            ));
                            ui.separator();
                            ui.label(format!("{}% slot share", pretty(&row["percent"])));
                            ui.separator();
                            ui.weak(format!("Version {}", number(row, "version")));
                        });
                    }
                });
        }
        ui.collapsing("About acquisition sources", |ui| {
            ui.label(&self.pokedex.note);
        });
        ui.weak("Slot share is the chance within this encounter table. Terrain, time, encounter versions and game logic affect availability.");
    }

    fn draw_species_moves(&mut self, ui: &mut egui::Ui, data: &Value) {
        section(
            ui,
            "Moves & learnsets",
            "Level-up, TM / tutor compatibility and egg moves from the game.",
        );
        ui.horizontal_wrapped(|ui| {
            for method in ["", "Level-up", "TM / tutor", "Egg"] {
                ui.selectable_value(
                    &mut self.pokedex.move_method,
                    method.to_owned(),
                    if method.is_empty() {
                        "All moves"
                    } else {
                        method
                    },
                );
            }
        });
        ui.add(
            egui::TextEdit::singleline(&mut self.pokedex.move_query)
                .hint_text("Filter moves by name or ID"),
        );
        let rows: Vec<_> = data["moves"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|row| {
                (self.pokedex.move_method.is_empty()
                    || text(row, "method") == self.pokedex.move_method)
                    && (fuzzy_score(
                        &normalized(&self.pokedex.move_query),
                        &normalized(text(row, "name")),
                    )
                    .is_some()
                        || fuzzy_score(
                            &normalized(&self.pokedex.move_query),
                            &normalized(text(row, "id")),
                        )
                        .is_some())
            })
            .collect();
        ui.weak(format!(
            "{} learnset entries · select a move for its description",
            rows.len()
        ));
        if rows.is_empty() {
            ui.label("No moves match this filter.");
        }
        for row in rows {
            let learned = if row["level"].is_number() {
                format!("Lv. {}", pretty(&row["level"]))
            } else {
                text(row, "method").into()
            };
            ui.push_id((text(row, "id"), &learned), |ui| {
                ui.collapsing(
                    format!(
                        "{}   ·   {}   ·   {}",
                        text(row, "name"),
                        text(row, "type"),
                        learned
                    ),
                    |ui| {
                        ui.label(text(row, "description"));
                        let category = match row["category"].as_i64() {
                            Some(0) => "Physical",
                            Some(1) => "Special",
                            Some(2) => "Status",
                            _ => "Category not supplied",
                        };
                        ui.label(format!(
                            "{} · Power {} · Accuracy {} · PP {} · Priority {}",
                            category,
                            pretty(&row["power"]),
                            if row["accuracy"].as_i64() == Some(0) {
                                "Always hits".into()
                            } else {
                                pretty(&row["accuracy"])
                            },
                            pretty(&row["pp"]),
                            pretty(&row["priority"])
                        ));
                        ui.weak(format!(
                            "{} · Function {}",
                            text(row, "id"),
                            text(row, "function_code")
                        ));
                    },
                );
            });
        }
    }

    fn draw_species_evolutions(&mut self, ui: &mut egui::Ui, data: &Value) {
        section(
            ui,
            "Evolution",
            "Explore related Pokémon and how they evolve in this game.",
        );
        for (field, title) in [("previous", "Evolves from"), ("evolutions", "Evolves into")] {
            ui.add_space(8.0);
            ui.strong(title);
            let rows = data[field].as_array().map(Vec::as_slice).unwrap_or(&[]);
            if rows.is_empty() {
                ui.weak("None listed in this game’s data.");
            }
            for row in rows {
                egui::Frame::group(ui.style())
                    .inner_margin(12)
                    .show(ui, |ui| {
                        if ui.button(format!("View {}", text(row, "name"))).clicked() {
                            self.pokedex.selected = text(row, "id").into();
                            self.pokedex.tab = DetailTab::Overview;
                        }
                        ui.label(text(row, "condition"));
                        ui.weak(if row["parameter"].is_null() {
                            format!("Rule: {}", text(row, "method"))
                        } else {
                            format!(
                                "Rule: {} · {}",
                                text(row, "method"),
                                pretty(&row["parameter"])
                            )
                        });
                        ui.weak(text(row, "id"));
                    });
            }
        }
    }

    fn poll_cheats(&mut self) {
        let Some(id) = self.pokedex.cheats.pending else {
            return;
        };
        let Some(result) = self.backend.take_response(id) else {
            return;
        };
        self.pokedex.cheats.pending = None;
        self.pokedex.cheats.notice = Some(match result {
            Ok(body) => match serde_json::from_str::<Value>(&body) {
                Ok(value) if value["error"].is_string() => (true, text(&value, "error").to_owned()),
                Ok(value) => {
                    let shiny = if value["shiny"].as_bool() == Some(true) {
                        " · shiny"
                    } else {
                        ""
                    };
                    (
                        false,
                        format!(
                            "Encounter scheduled: {} Lv {}{shiny}.",
                            text(&value, "name"),
                            number(&value, "level")
                        ),
                    )
                }
                Err(error) => (true, error.to_string()),
            },
            Err(error) => (true, error),
        });
    }

    fn start_cheat_encounter(&mut self, data: &Value) {
        if self.pokedex.cheats.pending.is_some() {
            return;
        }
        let args = {
            let cheats = &self.pokedex.cheats;
            vec![
                text(data, "id").to_owned(),
                cheats.level.to_string(),
                cheats.shiny.to_string(),
                cheats.form.to_string(),
                cheats.gender.clone(),
                cheats.nature.clone(),
                cheats.ability.clone(),
                cheats.item.clone(),
                cheats.max_ivs.to_string(),
                if cheats.set_friendship {
                    cheats.friendship.to_string()
                } else {
                    String::new()
                },
            ]
        };
        match self
            .backend
            .send_request("pokemon_essentials_wild_encounter", args)
        {
            Ok(id) => {
                self.pokedex.cheats.pending = Some(id);
                self.pokedex.cheats.notice = None;
            }
            Err(error) => self.pokedex.cheats.notice = Some((true, error)),
        }
    }

    fn draw_species_cheats(&mut self, ui: &mut egui::Ui, data: &Value) {
        if self.pokedex.cheats.species != text(data, "id") {
            self.pokedex.cheats.reset(data);
        }
        section(
            ui,
            "Cheats · start a wild encounter",
            "Starts a wild battle with this Pokémon. Requires the overworld with no battle, menu or dialogue open.",
        );
        let abilities = species_abilities(data);
        let mut start = false;
        {
            let cheats = &mut self.pokedex.cheats;
            egui::Grid::new("cheat-encounter-fields")
                .num_columns(2)
                .min_col_width(140.0)
                .spacing([20.0, 10.0])
                .show(ui, |ui| {
                    ui.label("Level");
                    ui.add(
                        egui::DragValue::new(&mut cheats.level)
                            .range(1..=100)
                            .speed(1),
                    );
                    ui.end_row();

                    ui.label("Shiny");
                    ui.checkbox(&mut cheats.shiny, "Force shiny");
                    ui.end_row();

                    ui.label("Form");
                    ui.add(
                        egui::DragValue::new(&mut cheats.form)
                            .range(0..=9999)
                            .speed(1),
                    );
                    ui.end_row();

                    ui.label("Gender");
                    egui::ComboBox::from_id_salt("cheat-gender")
                        .selected_text(gender_label(&cheats.gender))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut cheats.gender, String::new(), "Random");
                            ui.selectable_value(&mut cheats.gender, "male".into(), "Male");
                            ui.selectable_value(&mut cheats.gender, "female".into(), "Female");
                        });
                    ui.end_row();

                    ui.label("Nature");
                    egui::ComboBox::from_id_salt("cheat-nature")
                        .selected_text(nature_label(&cheats.nature))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut cheats.nature, String::new(), "Random");
                            for nature in NATURES {
                                ui.selectable_value(&mut cheats.nature, nature.to_owned(), nature);
                            }
                        });
                    ui.end_row();

                    ui.label("Ability");
                    egui::ComboBox::from_id_salt("cheat-ability")
                        .selected_text(ability_label(&cheats.ability, &abilities))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut cheats.ability,
                                String::new(),
                                "Species default",
                            );
                            for (id, name) in &abilities {
                                ui.selectable_value(&mut cheats.ability, id.clone(), name.clone());
                            }
                        });
                    ui.end_row();

                    ui.label("Held item");
                    ui.add(
                        egui::TextEdit::singleline(&mut cheats.item)
                            .hint_text("Item ID, e.g. LEFTOVERS")
                            .desired_width(180.0),
                    );
                    ui.end_row();

                    ui.label("IVs");
                    ui.checkbox(&mut cheats.max_ivs, "Set all to 31");
                    ui.end_row();

                    ui.label("Friendship");
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut cheats.set_friendship, "Set");
                        ui.add_enabled(
                            cheats.set_friendship,
                            egui::DragValue::new(&mut cheats.friendship)
                                .range(0..=255)
                                .speed(1),
                        );
                    });
                    ui.end_row();
                });
            ui.add_space(8.0);
            ui.add_enabled_ui(cheats.pending.is_none(), |ui| {
                start = ui.button("Start encounter").clicked();
            });
            if cheats.pending.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Scheduling encounter…");
                });
            }
            if let Some((error, message)) = &cheats.notice {
                ui.colored_label(
                    if *error {
                        ui.visuals().error_fg_color
                    } else {
                        egui::Color32::from_rgb(100, 205, 168)
                    },
                    message,
                );
            }
        }
        ui.add_space(6.0);
        ui.weak("The battle starts on the next game frame, so a long battle never blocks this response.");
        if start {
            self.start_cheat_encounter(data);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explorer_and_world_map_widths_stay_stable_across_frames() {
        for screen in [egui::vec2(3000.0, 1800.0), egui::vec2(1100.0, 800.0)] {
            for world_map in [false, true] {
                let mut workspace = Workspace::new();
                let fixture: Value =
                    serde_json::from_str(include_str!("../testdata/pokedex.json")).unwrap();
                workspace.pokedex.open = !world_map;
                workspace.pokedex.catalog = fixture["index"]["species"].as_array().unwrap().clone();
                workspace.pokedex.selected = "BULBASAUR".into();
                workspace
                    .pokedex
                    .details
                    .insert("BULBASAUR".into(), Arc::new(fixture["entry"].clone()));
                workspace.world_map_open = world_map;
                workspace.world_map_data = Some(
                    serde_json::json!({"current_map_id":1,"maps":[{"id":1,"name":"A map with a long name to exercise destination layout", "outdoor":true,"width":64,"height":64,"world_x":0,"world_y":0}],"connections":[],"transfers":[]}),
                );
                let context = egui::Context::default();
                theme::apply(&context);
                let id = egui::Id::unique(if world_map {
                    "essentials-world-map"
                } else {
                    "essentials-pokedex"
                });
                let mut settled_width = 0.0;
                for frame in 0..80 {
                    workspace.pokedex.tab = match frame {
                        0..=25 => DetailTab::Overview,
                        26..=45 => DetailTab::Moves,
                        46..=60 => DetailTab::Evolution,
                        _ => DetailTab::Locations,
                    };
                    let input = egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, screen)),
                        ..Default::default()
                    };
                    context
                        .run_pass(input, |context| {
                            if world_map {
                                workspace.draw_world_map_window(context);
                            } else {
                                workspace.draw_pokedex_window(context);
                            }
                        })
                        .drop_without_applying_deltas();
                    let width = context.memory(|memory| memory.area_rect(id).unwrap().width());
                    if frame == 8 {
                        settled_width = width;
                    }
                    if frame > 8 {
                        assert!((width-settled_width).abs()<1.0,"world_map={world_map}, frame={frame}: width grew from {settled_width} to {width}");
                    }
                }
            }
        }
    }
    #[test]
    fn explorer_renders_all_detail_tabs() {
        let source = std::env::var("RSXP_POKEDEX_FIXTURE")
            .ok()
            .map(|path| std::fs::read_to_string(path).unwrap())
            .unwrap_or_else(|| include_str!("../testdata/pokedex.json").to_owned());
        let fixture: Value = serde_json::from_str(&source).unwrap();
        let mut workspace = Workspace::new();
        workspace.pokedex.open = true;
        workspace.pokedex.catalog = fixture["index"]["species"].as_array().unwrap().clone();
        workspace.pokedex.note = text(&fixture["index"], "note").into();
        workspace.pokedex.selected = text(&fixture["entry"], "id").into();
        workspace.pokedex.query = std::env::var("RSXP_POKEDEX_QUERY").unwrap_or_default();
        workspace.pokedex.details.insert(
            workspace.pokedex.selected.clone(),
            Arc::new(fixture["entry"].clone()),
        );
        let context = egui::Context::default();
        theme::apply(&context);
        if std::env::var("RSXP_POKEDEX_SNAPSHOT").is_ok() {
            if let Ok(image) = image::open(format!(
                "ericlostie/Graphics/Pokemon/Front/{}.png",
                workspace.pokedex.selected
            )) {
                let image = first_pokemon_sprite_frame(&image.to_rgba8());
                let texture = context.load_texture(
                    "preview-species",
                    egui::ColorImage::from_rgba_unmultiplied(
                        [image.width() as usize, image.height() as usize],
                        image.as_raw(),
                    ),
                    egui::TextureOptions::NEAREST,
                );
                workspace.pokemon_sprite_textures.insert(
                    format!(
                        "{}|{}|{}",
                        workspace.pokedex.selected,
                        number(&fixture["entry"], "number"),
                        text(&fixture["entry"], "name")
                    ),
                    texture,
                );
            }
        }
        let mut textures: HashMap<u64, image::RgbaImage> = HashMap::new();
        for (tab, title) in [
            (DetailTab::Overview, "Base stats"),
            (DetailTab::Locations, "Where to find it"),
            (DetailTab::Moves, "Moves & learnsets"),
            (DetailTab::Evolution, "Evolves into"),
            (DetailTab::Cheats, "start a wild encounter"),
        ] {
            workspace.pokedex.tab = tab;
            for frame in 0..3 {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 900.0),
                    )),
                    ..Default::default()
                };
                let mut output =
                    context.run_pass(input, |context| workspace.draw_pokedex_window(context));
                let texture_updates = std::mem::take(&mut output.textures_delta.set);
                output.textures_delta.clear();
                fn has_text(shape: &egui::Shape, title: &str) -> bool {
                    match shape {
                        egui::Shape::Text(text) => text.galley.text().contains(title),
                        egui::Shape::Vec(shapes) => {
                            shapes.iter().any(|shape| has_text(shape, title))
                        }
                        _ => false,
                    }
                }
                if frame == 2 {
                    assert!(
                        output
                            .shapes
                            .iter()
                            .any(|shape| has_text(&shape.shape, title)),
                        "Missing {title}"
                    );
                }
                if let Ok(directory) = std::env::var("RSXP_POKEDEX_SNAPSHOT") {
                    for (id, delta) in texture_updates
                        .iter()
                        .flat_map(|(id, deltas)| deltas.iter().map(move |delta| (id, delta)))
                    {
                        let egui::TextureId::Managed(id) = id else {
                            continue;
                        };
                        let egui::ImageData::Color(data) = &delta.image;
                        let pixels: Vec<u8> = data
                            .pixels
                            .iter()
                            .flat_map(|pixel| pixel.to_array())
                            .collect();
                        let patch = image::RgbaImage::from_raw(
                            data.size[0] as u32,
                            data.size[1] as u32,
                            pixels,
                        )
                        .unwrap();
                        if let Some([x, y]) = delta.pos {
                            image::imageops::replace(
                                textures.get_mut(id).unwrap(),
                                &patch,
                                x as i64,
                                y as i64,
                            );
                        } else {
                            textures.insert(*id, patch);
                        }
                    }
                    if frame == 2 {
                        std::fs::create_dir_all(&directory).unwrap();
                        let primitives = context.tessellate(output.shapes, output.pixels_per_point);
                        let mut meshes = Vec::new();
                        for primitive in primitives {
                            if let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive {
                                let egui::TextureId::Managed(texture) = mesh.texture_id else {
                                    continue;
                                };
                                meshes.push(serde_json::json!({"texture":texture,"clip":[primitive.clip_rect.min.x,primitive.clip_rect.min.y,primitive.clip_rect.max.x,primitive.clip_rect.max.y],"indices":mesh.indices,
                                    "vertices":mesh.vertices.iter().map(|v| { let c=v.color.to_array(); serde_json::json!([v.pos.x,v.pos.y,v.uv.x,v.uv.y,c[0],c[1],c[2],c[3]]) }).collect::<Vec<_>>() }));
                            }
                        }
                        let name = format!("{tab:?}");
                        std::fs::write(
                            format!("{directory}/{name}.json"),
                            serde_json::to_vec(&meshes).unwrap(),
                        )
                        .unwrap();
                        for (id, texture) in &textures {
                            texture
                                .save(format!("{directory}/{name}-texture-{id}.png"))
                                .unwrap();
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn fuzzy_search_matches_names_ids_typos_and_dex_numbers() {
        let row = serde_json::json!({"id":"PIKACHU","name":"Pikachu","number":25});
        for query in ["pika", "pkchu", "pikchu", "pikacu", "PIKACHU", "25", "#025"] {
            assert!(species_score(query, &row).is_some(), "{query}");
        }
        assert!(species_score("charizard", &row).is_none());
        assert!(species_score("pikachu", &row) < species_score("pikacu", &row));
        assert_eq!(normalized("Flabébé"), "flabebe");
    }
}
