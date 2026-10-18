// SPDX-License-Identifier: GPL-2.0-or-later
//! Move and item explorers backed by the running game's registries.
//!
//! One [`RegistryDex`] instance backs each of `Workspace::movedex` and
//! `Workspace::itemdex`; [`Workspace::draw_registry_dex_windows`] draws both and
//! lets the itemdex open a taught move in the movedex. Both windows share the
//! same filter/search/list/detail layout and hit their own registry through the
//! backend.
//!
//! ## Main types and functions
//! * [`DexKind`] — which registry a `RegistryDex` shows, mapping to the
//!   `pokemon_essentials_movedex` / `pokemon_essentials_itemdex` operations.
//! * `RegistryDex` (`pub(super)`) — the entry cache, filters, memoized matches,
//!   selection, texture cache, and load/error state.
//! * `Filters` / `NumberFilter` / `Sort` — the search and filter model
//!   (`accepts` decides which entries pass; `Sort` covers relevance, name,
//!   power/accuracy/PP, and price).
//! * `InventoryAction` — the shared "add to bag / add TM" action, including the
//!   per-move machine picker.
//! * `RegistryDex::load_if_needed` / `request` / `poll` — fetch and cache the
//!   catalog; `add_inventory` / `poll_inventory` — queue
//!   `pokemon_essentials_add_item`.
//! * `draw_filters`, `draw_list`, `draw_detail`, `draw_facts`,
//!   `draw_inventory_action`, `item_texture` — the UI.
//!
//! ## Backend use
//! `load_if_needed` and `request` send the kind's operation with `["refresh"]`;
//! responses are consumed through `backend.take_response`. Item icons are loaded
//! with `backend.asset_image(category, name)` (falling back to the `Items`
//! category) using the entry's `icon` path with the leading `Graphics/`
//! stripped. Search reuses `dex_search::{fuzzy_score, normalized}`.
//!
//! ## JSON schema
//! A catalog response is `{ "entries": [ ... ] }` or `{ "error": "..." }`.
//! Move entries carry `id`, `name`, `type`, `type_name`, `category`,
//! `category_name`, `power`, `accuracy`, `pp`, `priority`, `description`,
//! `function_code`, `number`, `target`, `effect_chance`, `machines`
//! (`[{ id, name }]`), `flags`, and `raw`. Item entries carry `id`, `number`,
//! `name`, `description`, `pocket`, `pocket_name`, `price`, `sell_price`,
//! `bp_price`, `consumable`, `field_use`/`field_use_label`,
//! `battle_use`/`battle_use_label`, `tags`, `flags`, `move`, `icon`, and `raw`.
//! The add-item response reports `{ "added", "total", "complete", "name" }` or
//! `{ "error": "..." }`.
use super::{
    dex_search::{fuzzy_score, normalized},
    *,
};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DexKind {
    Moves,
    Items,
}
impl DexKind {
    fn title(self) -> &'static str {
        match self {
            Self::Moves => "Movedex · game moves",
            Self::Items => "Itemdex · game items",
        }
    }
    fn operation(self) -> &'static str {
        match self {
            Self::Moves => "pokemon_essentials_movedex",
            Self::Items => "pokemon_essentials_itemdex",
        }
    }
}
#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Sort {
    #[default]
    Relevance,
    Name,
    Power,
    Accuracy,
    Pp,
    Price,
}
impl Sort {
    fn label(self) -> &'static str {
        match self {
            Self::Relevance => "Best match",
            Self::Name => "Name A–Z",
            Self::Power => "Highest power",
            Self::Accuracy => "Highest accuracy",
            Self::Pp => "Highest PP",
            Self::Price => "Highest price",
        }
    }
}
#[derive(Clone, PartialEq, Eq)]
struct NumberFilter {
    active: bool,
    min: i64,
    max: i64,
}
impl NumberFilter {
    fn new(min: i64, max: i64) -> Self {
        Self {
            active: false,
            min,
            max,
        }
    }
    fn matches(&self, value: &Value) -> bool {
        !self.active
            || value
                .as_i64()
                .is_some_and(|value| value >= self.min && value <= self.max)
    }
    fn ui(&mut self, ui: &mut egui::Ui, label: &str, bounds: std::ops::RangeInclusive<i64>) {
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.active, label);
            ui.add_enabled_ui(self.active, |ui| {
                ui.add(
                    egui::DragValue::new(&mut self.min)
                        .range(bounds.clone())
                        .speed(1),
                );
                ui.label("to");
                ui.add(egui::DragValue::new(&mut self.max).range(bounds).speed(1));
            });
        });
    }
}
#[derive(Clone, PartialEq, Eq)]
struct Filters {
    query: String,
    group: String,
    category: String,
    flag: String,
    usage: String,
    consumable: i8,
    has_tm: bool,
    power: NumberFilter,
    accuracy: NumberFilter,
    pp: NumberFilter,
    priority: NumberFilter,
    price: NumberFilter,
    sort: Sort,
}
impl Default for Filters {
    fn default() -> Self {
        Self {
            query: String::new(),
            group: String::new(),
            category: String::new(),
            flag: String::new(),
            usage: String::new(),
            consumable: -1,
            has_tm: false,
            power: NumberFilter::new(0, 999),
            accuracy: NumberFilter::new(0, 100),
            pp: NumberFilter::new(0, 99),
            priority: NumberFilter::new(-9, 9),
            price: NumberFilter::new(0, 999999),
            sort: Sort::default(),
        }
    }
}
struct InventoryAction {
    quantity: u32,
    pending: Option<u64>,
    notice: Option<(bool, String)>,
    machines: HashMap<String, String>,
}
impl Default for InventoryAction {
    fn default() -> Self {
        Self {
            quantity: 1,
            pending: None,
            notice: None,
            machines: HashMap::new(),
        }
    }
}
#[derive(Default)]
pub(super) struct RegistryDex {
    pub open: bool,
    inventory: InventoryAction,
    entries: Vec<Value>,
    loaded: bool,
    pending: Option<u64>,
    error: Option<String>,
    filters: Filters,
    cached_filters: Option<Filters>,
    matches: Vec<usize>,
    selected: String,
    textures: HashMap<String, egui::TextureHandle>,
}
fn text<'a>(row: &'a Value, key: &str) -> &'a str {
    row[key].as_str().unwrap_or("")
}
fn value(row: &Value, key: &str) -> String {
    match &row[key] {
        Value::Null => "Not supplied".into(),
        Value::String(v) => v.clone(),
        v => v.to_string(),
    }
}
fn score(query: &str, row: &Value) -> Option<usize> {
    let query = normalized(query);
    let mut result = [
        fuzzy_score(&query, &normalized(text(row, "name"))),
        fuzzy_score(&query, &normalized(text(row, "id"))),
    ]
    .into_iter()
    .flatten()
    .min();
    if let Some(number) = row["number"].as_i64() {
        if !query.is_empty() && query.chars().all(|c| c.is_ascii_digit()) {
            let numeric = query.trim_start_matches('0');
            let number_match = fuzzy_score(
                if numeric.is_empty() { "0" } else { numeric },
                &number.to_string(),
            );
            result = result.into_iter().chain(number_match).min();
        }
    }
    result
}
fn accepts(kind: DexKind, filters: &Filters, row: &Value) -> bool {
    if !filters.flag.is_empty()
        && !row["flags"].as_array().is_some_and(|flags| {
            flags
                .iter()
                .any(|flag| flag.as_str() == Some(&filters.flag))
        })
    {
        return false;
    }
    if kind == DexKind::Moves {
        (filters.group.is_empty() || text(row, "type") == filters.group)
            && (filters.category.is_empty() || text(row, "category_name") == filters.category)
            && (!filters.has_tm
                || row["machines"].as_array().is_some_and(|machines| {
                    machines
                        .iter()
                        .any(|machine| !text(machine, "id").is_empty())
                }))
            && filters.power.matches(&row["power"])
            && filters.accuracy.matches(&row["accuracy"])
            && filters.pp.matches(&row["pp"])
            && filters.priority.matches(&row["priority"])
    } else {
        (filters.group.is_empty() || text(row, "pocket_name") == filters.group)
            && (filters.category.is_empty()
                || row["tags"].as_array().is_some_and(|tags| {
                    tags.iter()
                        .any(|tag| tag.as_str() == Some(&filters.category))
                }))
            && (filters.consumable < 0
                || row["consumable"].as_bool() == Some(filters.consumable == 1))
            && (filters.usage.is_empty()
                || row[if filters.usage == "Field" {
                    "field_use"
                } else {
                    "battle_use"
                }]
                .as_i64()
                .is_some_and(|mode| mode > 0))
            && filters.price.matches(&row["price"])
    }
}
fn options(rows: &[Value], key: &str, array: bool) -> Vec<String> {
    let mut values = BTreeSet::new();
    for row in rows {
        if array {
            for value in row[key]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                values.insert(value.to_owned());
            }
        } else if !text(row, key).is_empty() {
            values.insert(text(row, key).to_owned());
        }
    }
    values.into_iter().collect()
}
fn combo(ui: &mut egui::Ui, id: &str, label: &str, selected: &mut String, options: Vec<String>) {
    egui::ComboBox::from_id_salt(id)
        .width(138.0)
        .wrap_mode(egui::TextWrapMode::Truncate)
        .selected_text(if selected.is_empty() {
            format!("All {label}")
        } else {
            selected.clone()
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(selected, String::new(), format!("All {label}"));
            for option in options {
                ui.selectable_value(selected, option.clone(), option);
            }
        });
}

fn draw_facts(ui: &mut egui::Ui, kind: DexKind, row: &Value) {
    let fields = if kind == DexKind::Moves {
        vec![
            ("Power", "power"),
            ("Accuracy", "accuracy"),
            ("Base PP", "pp"),
            ("Priority", "priority"),
            ("Effect chance", "effect_chance"),
            ("Target", "target"),
        ]
    } else {
        vec![
            ("Buy price", "price"),
            ("Sell price", "sell_price"),
            ("BP price", "bp_price"),
            ("Field use", "field_use_label"),
            ("Battle use", "battle_use_label"),
            ("Consumable", "consumable"),
        ]
    };
    egui::Grid::new("registry-facts")
        .num_columns(2)
        .min_col_width(140.0)
        .striped(true)
        .spacing([24.0, 10.0])
        .show(ui, |ui| {
            for (label, key) in fields {
                ui.add(
                    egui::Label::new(egui::RichText::new(label).strong())
                        .wrap_mode(egui::TextWrapMode::Extend),
                );
                let display = if kind == DexKind::Moves
                    && key == "accuracy"
                    && row[key].as_i64() == Some(0)
                {
                    "No normal accuracy check (0)".into()
                } else if kind == DexKind::Moves
                    && (key == "accuracy" || key == "effect_chance")
                    && !row[key].is_null()
                {
                    format!("{}%", value(row, key))
                } else if key == "consumable" {
                    match row[key].as_bool() {
                        Some(true) => "Yes".into(),
                        Some(false) => "No".into(),
                        None => "Not supplied".into(),
                    }
                } else {
                    value(row, key)
                };
                ui.label(display);
                ui.end_row();
            }
        });
}

impl RegistryDex {
    pub(super) fn load_if_needed(&mut self, backend: &dyn backend::Backend, kind: DexKind) {
        if !self.loaded && self.pending.is_none() {
            self.request(backend, kind);
        }
    }
    fn request(&mut self, backend: &dyn backend::Backend, kind: DexKind) {
        if self.pending.is_some() {
            return;
        }
        match backend.send_request(kind.operation(), vec!["refresh".into()]) {
            Ok(id) => {
                self.pending = Some(id);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
    fn poll(&mut self, backend: &dyn backend::Backend) {
        self.poll_inventory(backend);
        let Some(id) = self.pending else {
            return;
        };
        let Some(result) = backend.take_response(id) else {
            return;
        };
        self.pending = None;
        match result
            .and_then(|body| serde_json::from_str::<Value>(&body).map_err(|e| e.to_string()))
        {
            Ok(data) if data["error"].is_string() => self.error = Some(text(&data, "error").into()),
            Ok(data) => {
                if let Some(entries) = data["entries"].as_array() {
                    self.entries = entries.clone();
                    self.loaded = true;
                    self.cached_filters = None;
                    self.error = None;
                } else {
                    self.error =
                        Some("The game returned an invalid catalog. Try refreshing data.".into());
                }
            }
            Err(error) => self.error = Some(error),
        }
    }
    pub(super) fn add_inventory(
        &mut self,
        backend: &dyn backend::Backend,
        item: &str,
        quantity: u32,
    ) {
        if self.inventory.pending.is_some() {
            return;
        }
        self.inventory.quantity = quantity.clamp(1, 9999);
        match backend.send_request(
            "pokemon_essentials_add_item",
            vec![item.to_owned(), self.inventory.quantity.to_string()],
        ) {
            Ok(id) => {
                self.inventory.pending = Some(id);
                self.inventory.notice = None;
            }
            Err(error) => self.inventory.notice = Some((true, error)),
        }
    }
    fn poll_inventory(&mut self, backend: &dyn backend::Backend) {
        let Some(id) = self.inventory.pending else {
            return;
        };
        let Some(result) = backend.take_response(id) else {
            return;
        };
        self.inventory.pending = None;
        let result = result.and_then(|body| {
            serde_json::from_str::<Value>(&body).map_err(|error| error.to_string())
        });
        self.inventory.notice = Some(match result {
            Ok(data) if data["error"].is_string() => (true, text(&data, "error").into()),
            Ok(data) if data["added"].as_i64().is_some_and(|added| added > 0) => {
                let complete = data["complete"].as_bool() == Some(true);
                let mut message = format!(
                    "{} {} × {}.",
                    if complete {
                        "Añadido:"
                    } else {
                        "Añadido parcialmente:"
                    },
                    value(&data, "added"),
                    text(&data, "name")
                );
                if !data["total"].is_null() {
                    message.push_str(&format!(" Total en la bolsa: {}.", value(&data, "total")));
                }
                (!complete, message)
            }
            Ok(_) => (
                true,
                "El juego no ha confirmado que se haya añadido el objeto.".into(),
            ),
            Err(error) => (true, error),
        });
    }
    fn draw_inventory_action(
        &mut self,
        ui: &mut egui::Ui,
        backend: &dyn backend::Backend,
        row: &Value,
        kind: DexKind,
    ) {
        let target = if kind == DexKind::Items {
            Some(text(row, "id").to_owned())
        } else {
            let machines = row["machines"].as_array().map(Vec::as_slice).unwrap_or(&[]);
            if machines.is_empty() {
                None
            } else {
                let selected = self
                    .inventory
                    .machines
                    .entry(text(row, "id").to_owned())
                    .or_insert_with(|| text(&machines[0], "id").into());
                if !machines
                    .iter()
                    .any(|machine| text(machine, "id") == selected)
                {
                    *selected = text(&machines[0], "id").into();
                }
                if machines.len() > 1 {
                    egui::ComboBox::from_id_salt("move-machine")
                        .width(230.0)
                        .wrap_mode(egui::TextWrapMode::Truncate)
                        .selected_text(
                            machines
                                .iter()
                                .find(|machine| text(machine, "id") == selected)
                                .map(|machine| text(machine, "name"))
                                .unwrap_or("Elegir MT"),
                        )
                        .show_ui(ui, |ui| {
                            for machine in machines {
                                ui.selectable_value(
                                    selected,
                                    text(machine, "id").to_owned(),
                                    format!("{} · {}", text(machine, "name"), text(machine, "id")),
                                );
                            }
                        });
                } else {
                    ui.weak(format!("MT disponible: {}", text(&machines[0], "name")));
                }
                Some(selected.clone())
            }
        };
        if let Some(item) = target {
            let mut add = false;
            ui.add_enabled_ui(self.inventory.pending.is_none(), |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Cantidad");
                    ui.add(
                        egui::DragValue::new(&mut self.inventory.quantity)
                            .range(1..=9999)
                            .speed(1),
                    );
                    add = ui
                        .button(if kind == DexKind::Items {
                            "Añadir al inventario"
                        } else {
                            "Añadir MT"
                        })
                        .clicked();
                });
            });
            if add {
                self.add_inventory(backend, &item, self.inventory.quantity);
            }
        }
        if self.inventory.pending.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Añadiendo a la bolsa…");
            });
        }
        if let Some((error, message)) = &self.inventory.notice {
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
    fn update_matches(&mut self, kind: DexKind) {
        if self.cached_filters.as_ref() == Some(&self.filters) {
            return;
        }
        let mut matches: Vec<_> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, row)| accepts(kind, &self.filters, row))
            .filter_map(|(i, row)| score(&self.filters.query, row).map(|score| (i, score)))
            .collect();
        matches.sort_by(|(a, sa), (b, sb)| {
            let a = &self.entries[*a];
            let b = &self.entries[*b];
            let by_name = || {
                text(a, "name")
                    .to_lowercase()
                    .cmp(&text(b, "name").to_lowercase())
            };
            match self.filters.sort {
                Sort::Relevance => sa.cmp(sb),
                Sort::Name => by_name(),
                sort => {
                    let key = match sort {
                        Sort::Power => "power",
                        Sort::Accuracy => "accuracy",
                        Sort::Pp => "pp",
                        _ => "price",
                    };
                    b[key].as_i64().cmp(&a[key].as_i64())
                }
            }
            .then_with(by_name)
            .then_with(|| text(a, "id").cmp(text(b, "id")))
        });
        self.matches = matches.into_iter().map(|(index, _)| index).collect();
        if !self
            .matches
            .iter()
            .any(|&index| text(&self.entries[index], "id") == self.selected)
        {
            self.selected = self
                .matches
                .first()
                .map(|&index| text(&self.entries[index], "id").to_owned())
                .unwrap_or_default();
        }
        self.cached_filters = Some(self.filters.clone());
    }
    fn show(
        &mut self,
        context: &egui::Context,
        backend: &dyn backend::Backend,
        kind: DexKind,
    ) -> Option<String> {
        self.poll(backend);
        if !self.open {
            return None;
        }
        let mut open = true;
        let mut inspect_move = None;
        egui::Window::new(kind.title())
            .id(egui::Id::new(kind.title()))
            .open(&mut open)
            .default_size(egui::vec2(1100.0, 760.0))
            .min_size(egui::vec2(780.0, 520.0))
            .resizable(true)
            .show(context, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.heading(if kind == DexKind::Moves {
                            "Explore your game’s moves"
                        } else {
                            "Explore your game’s items"
                        });
                        ui.weak("Loaded game data · custom entries and translated descriptions");
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(self.pending.is_none(), egui::Button::new("Refresh data"))
                            .clicked()
                        {
                            self.request(backend, kind);
                        }
                    });
                });
                ui.add(
                    egui::TextEdit::singleline(&mut self.filters.query)
                        .hint_text("Fuzzy search by name or ID · partial names and typos work too")
                        .desired_width(ui.available_width()),
                );
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    self.draw_filters(ui, kind);
                });
                if let Some(error) = self.error.clone() {
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                        if ui.button("Try again").clicked() {
                            self.request(backend, kind);
                        }
                    });
                }
                if self.pending.is_some() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Reading the game’s catalog…");
                    });
                }
                self.update_matches(kind);
                ui.weak(format!(
                    "{} results · {} total {}",
                    self.matches.len(),
                    self.entries.len(),
                    if kind == DexKind::Moves {
                        "moves"
                    } else {
                        "items"
                    }
                ));
                let (body, _) = ui.allocate_exact_size(
                    ui.available_size().max(egui::vec2(0.0, 0.0)),
                    egui::Sense::hover(),
                );
                let left_width = 310.0_f32.min((body.width() - 12.0) * 0.42);
                let list_rect =
                    egui::Rect::from_min_size(body.min, egui::vec2(left_width, body.height()));
                let detail_rect = egui::Rect::from_min_max(
                    egui::pos2(list_rect.right() + 12.0, body.top()),
                    body.max,
                );
                ui.painter().vline(
                    list_rect.right() + 6.0,
                    body.y_range(),
                    ui.visuals().widgets.noninteractive.bg_stroke,
                );
                self.draw_list(&mut bounded_tool_pane(ui, "registry-list", list_rect), kind);
                inspect_move = self.draw_detail(
                    &mut bounded_tool_pane(ui, "registry-detail", detail_rect),
                    backend,
                    kind,
                );
            });
        self.open = open;
        inspect_move
    }
    fn draw_filters(&mut self, ui: &mut egui::Ui, kind: DexKind) {
        ui.horizontal_wrapped(|ui| {
            if kind == DexKind::Moves {
                combo(
                    ui,
                    "move-type",
                    "types",
                    &mut self.filters.group,
                    options(&self.entries, "type", false),
                );
                combo(
                    ui,
                    "move-category",
                    "categories",
                    &mut self.filters.category,
                    options(&self.entries, "category_name", false),
                );
                ui.checkbox(&mut self.filters.has_tm, "Has TM")
                    .on_hover_text("Only moves with an existing MT/TM item in this game.");
            } else {
                combo(
                    ui,
                    "item-pocket",
                    "pockets",
                    &mut self.filters.group,
                    options(&self.entries, "pocket_name", false),
                );
                combo(
                    ui,
                    "item-kind",
                    "item kinds",
                    &mut self.filters.category,
                    options(&self.entries, "tags", true),
                );
                egui::ComboBox::from_id_salt("consumable")
                    .width(130.0)
                    .selected_text(match self.filters.consumable {
                        0 => "Reusable",
                        1 => "Consumable",
                        _ => "Any consumption",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.filters.consumable, -1, "Any consumption");
                        ui.selectable_value(&mut self.filters.consumable, 1, "Consumable");
                        ui.selectable_value(&mut self.filters.consumable, 0, "Reusable");
                    });
            }
            egui::ComboBox::from_id_salt("registry-sort")
                .width(140.0)
                .selected_text(self.filters.sort.label())
                .show_ui(ui, |ui| {
                    let sorts = if kind == DexKind::Moves {
                        vec![
                            Sort::Relevance,
                            Sort::Name,
                            Sort::Power,
                            Sort::Accuracy,
                            Sort::Pp,
                        ]
                    } else {
                        vec![Sort::Relevance, Sort::Name, Sort::Price]
                    };
                    for sort in sorts {
                        ui.selectable_value(&mut self.filters.sort, sort, sort.label());
                    }
                });
            if ui.button("Reset all").clicked() {
                self.filters = Filters::default();
            }
        });
        if kind == DexKind::Moves {
            ui.horizontal_wrapped(|ui| {
                self.filters.power.ui(ui, "Power", 0..=9999);
                self.filters.accuracy.ui(ui, "Accuracy", 0..=100);
                self.filters.pp.ui(ui, "PP", 0..=999);
            });
            ui.collapsing("More filters",|ui| {ui.horizontal_wrapped(|ui| {
                self.filters.priority.ui(ui,"Priority",-99..=99);
                combo(ui,"move-flag","flags",&mut self.filters.flag,options(&self.entries,"flags",true));
            });ui.weak("Enable a range to filter it. Accuracy 0 means the move skips the normal accuracy check.");});
        } else {
            ui.horizontal_wrapped(|ui| {
                self.filters.price.ui(ui, "Buy price", 0..=999999999);
                combo(
                    ui,
                    "item-usage",
                    "use modes",
                    &mut self.filters.usage,
                    vec!["Field".into(), "Battle".into()],
                );
                combo(
                    ui,
                    "item-flag",
                    "flags",
                    &mut self.filters.flag,
                    options(&self.entries, "flags", true),
                );
            });
        }
    }
    fn draw_list(&mut self, ui: &mut egui::Ui, kind: DexKind) {
        if self.matches.is_empty() && self.pending.is_none() {
            ui.heading(if self.loaded {
                "No matches"
            } else {
                "Catalog not loaded"
            });
            ui.label(if self.loaded {
                "Try a shorter search or reset the filters."
            } else {
                "Start an Essentials game and refresh its data."
            });
        }
        egui::ScrollArea::vertical()
            .id_salt("registry-rows")
            .auto_shrink([false, false])
            .show_rows(ui, 66.0, self.matches.len(), |ui, range| {
                for index in range {
                    let row = &self.entries[self.matches[index]];
                    let id = text(row, "id");
                    let selected = self.selected == id;
                    let frame = egui::Frame::new()
                        .fill(if selected {
                            ui.visuals().selection.bg_fill
                        } else {
                            ui.visuals().faint_bg_color
                        })
                        .corner_radius(7)
                        .inner_margin(8)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.add(
                                egui::Label::new(egui::RichText::new(text(row, "name")).strong())
                                    .truncate(),
                            );
                            ui.add(
                                egui::Label::new(egui::RichText::new(id).small().weak()).truncate(),
                            );
                            let summary = if kind == DexKind::Moves {
                                format!(
                                    "{} · {} · Power {}",
                                    text(row, "type"),
                                    text(row, "category_name"),
                                    value(row, "power")
                                )
                            } else {
                                format!(
                                    "{} · Buy {}",
                                    text(row, "pocket_name"),
                                    value(row, "price")
                                )
                            };
                            ui.add(
                                egui::Label::new(egui::RichText::new(summary).small()).truncate(),
                            );
                        })
                        .response;
                    if ui
                        .interact(frame.rect, frame.id.with("choose"), egui::Sense::click())
                        .on_hover_text(format!("{} · {}", text(row, "name"), id))
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        self.selected = id.into();
                    }
                }
            });
    }
    fn item_texture(
        &mut self,
        context: &egui::Context,
        backend: &dyn backend::Backend,
        row: &Value,
    ) -> Option<egui::TextureHandle> {
        let id = text(row, "id");
        if let Some(texture) = self.textures.get(id) {
            return Some(texture.clone());
        }
        let path = text(row, "icon").trim_start_matches("Graphics/");
        let image = path
            .rsplit_once('/')
            .and_then(|(category, name)| backend.asset_image(category, name))
            .or_else(|| backend.asset_image("Items", id));
        let image = image?;
        let texture = context.load_texture(
            format!("itemdex-{id}"),
            egui::ColorImage::from_rgba_unmultiplied(
                [image.width() as usize, image.height() as usize],
                image.as_raw(),
            ),
            egui::TextureOptions::NEAREST,
        );
        self.textures.insert(id.into(), texture.clone());
        Some(texture)
    }
    fn draw_detail(
        &mut self,
        ui: &mut egui::Ui,
        backend: &dyn backend::Backend,
        kind: DexKind,
    ) -> Option<String> {
        let Some(row) = self
            .matches
            .iter()
            .map(|&index| &self.entries[index])
            .find(|row| text(row, "id") == self.selected)
            .cloned()
        else {
            ui.add_space(32.0);
            ui.heading(if self.pending.is_some() {
                "Loading catalog…"
            } else {
                "No entry selected"
            });
            ui.label("Use the search and filters to find an entry, then choose it from the list.");
            return None;
        };
        egui::Frame::new()
            .fill(egui::Color32::from_rgb(34, 45, 57))
            .corner_radius(12)
            .inner_margin(16)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    if kind == DexKind::Items {
                        if let Some(texture) = self.item_texture(ui.ctx(), backend, &row) {
                            ui.add(
                                egui::Image::new(&texture)
                                    .fit_to_exact_size(egui::vec2(64.0, 64.0)),
                            );
                        }
                    } else {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(12.0, 64.0), egui::Sense::hover());
                        ui.painter().rect_filled(
                            rect,
                            4.0,
                            pokedex::type_color(text(&row, "type")),
                        );
                    }
                    ui.vertical(|ui| {
                        ui.weak(text(&row, "id"));
                        ui.label(egui::RichText::new(text(&row, "name")).size(27.0).strong());
                        ui.label(if kind == DexKind::Moves {
                            format!(
                                "{} · {}",
                                text(&row, "type_name"),
                                text(&row, "category_name")
                            )
                        } else {
                            text(&row, "pocket_name").into()
                        });
                    });
                });
            });
        if ui
            .button(if kind == DexKind::Moves {
                "Copy move data"
            } else {
                "Copy item data"
            })
            .clicked()
        {
            ui.ctx()
                .copy_text(serde_json::to_string_pretty(&row).unwrap_or_default());
        }
        self.draw_inventory_action(ui, backend, &row, kind);
        ui.separator();
        let mut inspect_move = None;
        egui::ScrollArea::vertical().id_salt(("registry-description",self.selected.clone())).auto_shrink([false,false]).show(ui,|ui| {
            ui.add_space(6.0);ui.label(text(&row,"description"));ui.add_space(12.0);
            draw_facts(ui,kind,&row);
            if kind==DexKind::Items&&row["move"].is_object() {
                ui.add_space(12.0);ui.heading("Teaches a move");
                ui.label(format!("{} · {}",text(&row["move"],"name"),text(&row["move"],"type")));
                ui.label(text(&row["move"],"description"));
                if ui.button("Open in Movedex").clicked(){inspect_move=Some(text(&row["move"],"id").into());}
            }
            for (key,label) in [("tags","Item kinds"),("flags","Game flags")] {
                if let Some(flags)=row[key].as_array().filter(|rows|!rows.is_empty()) {
                    ui.add_space(12.0);ui.strong(label);
                    ui.horizontal_wrapped(|ui| {for flag in flags.iter().filter_map(Value::as_str){egui::Frame::group(ui.style()).inner_margin(5).show(ui,|ui|{ui.label(flag);});}});
                }
            }
            if kind==DexKind::Moves {ui.add_space(12.0);ui.collapsing("Game mechanics",|ui|{ui.label(format!("Function code: {}",text(&row,"function_code")));ui.label("Power and accuracy are the game’s base values; effects and battle conditions can alter the result.");});}
            ui.add_space(12.0);ui.collapsing("Additional game fields",|ui|{ui.add(egui::Label::new(egui::RichText::new(serde_json::to_string_pretty(&row["raw"]).unwrap_or_default()).monospace()).selectable(true));});
        });
        inspect_move
    }
}
impl Workspace {
    pub(super) fn draw_registry_dex_windows(&mut self, context: &egui::Context) {
        self.movedex.show(context, &*self.backend, DexKind::Moves);
        if let Some(id) = self.itemdex.show(context, &*self.backend, DexKind::Items) {
            self.movedex.open = true;
            self.movedex.filters = Filters {
                query: id.clone(),
                ..Default::default()
            };
            self.movedex.selected = id;
            self.movedex.load_if_needed(&*self.backend, DexKind::Moves);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Value {
        serde_json::from_str(include_str!("../testdata/registry_dex.json")).unwrap()
    }
    #[test]
    fn move_filters_combine_and_preserve_zero_and_missing_values() {
        let data = fixture();
        let move_row = &data["moves"]["entries"][0];
        let status = &data["moves"]["entries"][1];
        let mut filter = Filters::default();
        filter.group = "GRASS".into();
        filter.category = "Special".into();
        filter.power = NumberFilter {
            active: true,
            min: 80,
            max: 100,
        };
        filter.accuracy = NumberFilter {
            active: true,
            min: 0,
            max: 0,
        };
        filter.pp = NumberFilter {
            active: true,
            min: 15,
            max: 20,
        };
        filter.priority = NumberFilter {
            active: true,
            min: -1,
            max: -1,
        };
        filter.flag = "Contact".into();
        assert!(accepts(DexKind::Moves, &filter, move_row));
        assert!(!accepts(DexKind::Moves, &filter, status));
        filter.power.min = 94;
        assert!(!accepts(DexKind::Moves, &filter, move_row));
        let mut filter = Filters::default();
        filter.power = NumberFilter {
            active: true,
            min: 0,
            max: 0,
        };
        assert!(accepts(DexKind::Moves, &filter, status));
        let mut missing = status.clone();
        missing["power"] = Value::Null;
        assert!(!accepts(DexKind::Moves, &filter, &missing));
    }
    #[test]
    fn item_filters_combine_pockets_kind_usage_price_and_consumption() {
        let data = fixture();
        let potion = &data["items"]["entries"][0];
        let machine = &data["items"]["entries"][1];
        let mut filter = Filters::default();
        filter.group = "Medicine".into();
        filter.category = "TM / HM / TR".into();
        filter.consumable = 0;
        filter.price = NumberFilter {
            active: true,
            min: 0,
            max: 0,
        };
        filter.usage = "Field".into();
        assert!(accepts(DexKind::Items, &filter, machine));
        assert!(!accepts(DexKind::Items, &filter, potion));
        filter.usage = "Battle".into();
        assert!(!accepts(DexKind::Items, &filter, machine));
        filter = Filters::default();
        filter.flag = "Healing".into();
        filter.consumable = 1;
        assert!(accepts(DexKind::Items, &filter, potion));
        assert!(!accepts(DexKind::Items, &filter, machine));
    }
    #[test]
    fn has_tm_filter_matches_existing_items_and_combines_with_search() {
        let data = fixture();
        let mut dex = RegistryDex::default();
        dex.entries = data["moves"]["entries"].as_array().unwrap().clone();
        dex.filters.has_tm = true;
        dex.filters.group = "GRASS".into();
        dex.filters.query = "costommove".into();
        dex.update_matches(DexKind::Moves);
        assert_eq!(dex.matches.len(), 1);
        assert_eq!(dex.selected, "CUSTOMMOVE");
        dex.filters.query = "status".into();
        dex.update_matches(DexKind::Moves);
        assert!(dex.matches.is_empty());
        dex.filters.has_tm = false;
        dex.update_matches(DexKind::Moves);
        assert_eq!(dex.selected, "STATUSMOVE");
        let mut missing = data["moves"]["entries"][0].clone();
        missing["machines"] = Value::Null;
        let filters = Filters {
            has_tm: true,
            ..Default::default()
        };
        assert!(!accepts(DexKind::Moves, &filters, &missing));
    }
    #[test]
    fn registry_search_uses_pokedex_fuzzy_matching_and_sorts_filtered_results() {
        let data = fixture();
        let row = &data["moves"]["entries"][0];
        for query in ["custommove", "cstmmo", "costommove", "25", "#025"] {
            assert!(score(query, row).is_some(), "{query}");
        }
        assert!(score("000", row).is_none());
        assert!(score("unrelated", row).is_none());
        let mut dex = RegistryDex::default();
        dex.entries = data["moves"]["entries"].as_array().unwrap().clone();
        dex.filters.sort = Sort::Power;
        dex.update_matches(DexKind::Moves);
        assert_eq!(text(&dex.entries[dex.matches[0]], "id"), "CUSTOMMOVE");
        dex.filters.category = "Status".into();
        dex.update_matches(DexKind::Moves);
        assert_eq!(dex.matches.len(), 1);
        assert_eq!(dex.selected, "STATUSMOVE");
    }
    #[test]
    fn registry_windows_render_and_keep_their_width() {
        for kind in [DexKind::Moves, DexKind::Items] {
            for screen in [egui::vec2(1280.0, 900.0), egui::vec2(1000.0, 720.0)] {
                let data = fixture();
                let key = if kind == DexKind::Moves {
                    "moves"
                } else {
                    "items"
                };
                let mut dex = RegistryDex {
                    open: true,
                    loaded: true,
                    entries: data[key]["entries"].as_array().unwrap().clone(),
                    ..Default::default()
                };
                if let Ok(root) = std::env::var("RSXP_REGISTRY_REAL") {
                    dex.entries = serde_json::from_str::<Value>(
                        &std::fs::read_to_string(format!("{root}/{key}.json")).unwrap(),
                    )
                    .unwrap()["entries"]
                        .as_array()
                        .unwrap()
                        .clone();
                    if kind == DexKind::Moves {
                        dex.filters.has_tm = true;
                        dex.filters.group = "FIRE".into();
                        dex.filters.category = "Special".into();
                        dex.filters.power = NumberFilter {
                            active: true,
                            min: 80,
                            max: 100,
                        };
                        dex.selected = "FLAMETHROWER".into();
                    } else {
                        dex.filters.group = "Medicinas".into();
                        dex.selected = "POTION".into();
                    }
                }
                let context = egui::Context::default();
                theme::apply(&context);
                let backend = crate::workspace_services::LocalServices::new(Some(
                    std::path::PathBuf::from("ericlostie"),
                ));
                let mut textures: HashMap<u64, image::RgbaImage> = HashMap::new();
                let mut settled = 0.0;
                for frame in 0..48 {
                    let input = egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, screen)),
                        ..Default::default()
                    };
                    let output = context.run(input, |context| {
                        dex.show(context, &backend, kind);
                    });
                    let width = context.memory(|memory| {
                        memory
                            .area_rect(egui::Id::new(kind.title()))
                            .unwrap()
                            .width()
                    });
                    if frame == 8 {
                        settled = width;
                    }
                    if frame > 8 {
                        assert!(
                            (width - settled).abs() < 1.0,
                            "{} expanded from {settled} to {width}",
                            kind.title()
                        );
                    }
                    if frame == 47 {
                        let expected = if kind == DexKind::Moves {
                            "Copy move data"
                        } else {
                            "Copy item data"
                        };
                        fn contains(shape: &egui::Shape, needle: &str) -> bool {
                            match shape {
                                egui::Shape::Text(text) => text.galley.text().contains(needle),
                                egui::Shape::Vec(shapes) => {
                                    shapes.iter().any(|shape| contains(shape, needle))
                                }
                                _ => false,
                            }
                        }
                        assert!(
                            output
                                .shapes
                                .iter()
                                .any(|shape| contains(&shape.shape, expected)),
                            "Missing details in {}",
                            kind.title()
                        );
                    }
                    if screen.x == 1280.0 {
                        if let Ok(directory) = std::env::var("RSXP_REGISTRY_SNAPSHOT") {
                            for (id, delta) in &output.textures_delta.set {
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
                            if frame == 47 {
                                std::fs::create_dir_all(&directory).unwrap();
                                let mut meshes = Vec::new();
                                for primitive in
                                    context.tessellate(output.shapes, output.pixels_per_point)
                                {
                                    if let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive
                                    {
                                        let egui::TextureId::Managed(texture) = mesh.texture_id
                                        else {
                                            continue;
                                        };
                                        meshes.push(serde_json::json!({"texture":texture,"clip":[primitive.clip_rect.min.x,primitive.clip_rect.min.y,primitive.clip_rect.max.x,primitive.clip_rect.max.y],"indices":mesh.indices,
                                    "vertices":mesh.vertices.iter().map(|v|{let c=v.color.to_array();serde_json::json!([v.pos.x,v.pos.y,v.uv.x,v.uv.y,c[0],c[1],c[2],c[3]])}).collect::<Vec<_>>() }));
                                    }
                                }
                                std::fs::write(
                                    format!("{directory}/{key}.json"),
                                    serde_json::to_vec(&meshes).unwrap(),
                                )
                                .unwrap();
                                for (id, texture) in &textures {
                                    texture
                                        .save(format!("{directory}/{key}-texture-{id}.png"))
                                        .unwrap();
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
