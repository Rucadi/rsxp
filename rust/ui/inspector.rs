// SPDX-License-Identifier: GPL-2.0-or-later
//! The API console and the Essentials inspector/tools for a running game.
//!
//! This file adds the request/response plumbing and the "pokeAPI · API +
//! testing" window to [`Workspace`]. Every backend operation is queued with
//! `Workspace::queue_api_request`/`queue_api_request_tagged`, which remembers
//! the returned id in `api_pending` (a single foreground slot), and is consumed
//! by [`Workspace::poll_api_response`] via `backend.take_response`. The window
//! provides an endpoint tester driven by the checked-in
//! `api/openapi.json` contract, a free-form game console, map/Pokémon
//! utilities, and the "Essentials" debug-mode toggle.
//!
//! ## Main functions
//! * [`Workspace::poll_api_response`] — drains the pending foreground response
//!   and routes it by operation. Notable results: `pokemon_essentials_status`
//!   updates the cached `$DEBUG` flag and map id; `world_map` replaces
//!   `world_map_data`, clears the layout, opens the window, and queues
//!   `map_preview` for each un-cached outdoor map; `map_preview` stores the
//!   preview and continues draining the tile queue; a map change while the
//!   encounter window is open refreshes `pokemon_essentials_encounters`.
//! * [`Workspace::queue_api_request`] / [`Workspace::queue_api_request_tagged`]
//!   — send an operation through the backend, tracking the response against a
//!   possibly different operation tag.
//! * [`Workspace::poll_pokemon_encounter_map`] — a *background* poller with its
//!   own request slot: every 750 ms it asks `pokemon_essentials_status`, and on
//!   a map change loads the new `pokemon_essentials_encounters` table without
//!   touching the foreground response, spinner, or `api_pending`.
//! * `draw_essentials_tools`, `draw_api_utilities`, `draw_api_console`,
//!   `draw_pokeapi_window`, `draw_openapi_operation`, `draw_api_response` — the
//!   UI, grouped into collapsible sections.
//! * `draw_debug_toggle` / `poll_essentials_debug` / `request_essentials_debug`
//!   — query and set the game's `$DEBUG` through `pokemon_essentials_debug`
//!   (empty argument queries; `"true"`/`"false"` sets).
//!
//! ## Backend and JSON use
//! `backend.send_request(operation, args)` returns a request id and
//! `backend.take_response(id)` yields `Ok(json_string)` or `Err(message)`.
//! Responses are parsed into `serde_json::Value` (`api_response_json`):
//! `pokemon_essentials_encounters` returns `{ "map_id", "tables": [...] }`;
//! `pokemon_essentials_status` returns `{ "map_id", "debug": bool }`;
//! `world_map` returns `{ "maps": [{ "id", "outdoor", ... }], "connections",
//! "transfers", "current_map_id" }`; `map_preview` returns
//! `{ "map_id", "width", "height", "tileset", "autotiles", "autotile_parts",
//! "layers" }`; `pokemon_essentials_debug` returns `{ "debug": bool }` or
//! `{ "error": "..." }`. An API response carrying `ivs`/`evs` is offered as a
//! collapsible "Pokémon details" view. The OpenAPI contract is embedded from
//! `api/openapi.json`, its `paths` are listed as operations whose parameter
//! defaults seed URL/query values, and "Send request" resolves the URL back to
//! an operation through `crate::api_bridge::route_http`.
use super::*;
impl Workspace {
    #[cfg(feature = "game-api")]
    pub(super) fn poll_api_response(&mut self) {
        if let Some(id) = self.api_pending
            && let Some(result) = self.backend.take_response(id)
        {
            self.api_pending = None;
            let operation = self.api_pending_operation.take();
            let mut load_next_outdoor_map = false;
            match result {
                Ok(response) => {
                    if operation.as_deref() == Some("cable_club_launch") {
                        self.cable_error = None;
                    }
                    let response_json = serde_json::from_str::<serde_json::Value>(&response).ok();
                    self.api_response_json = response_json.clone();
                    if operation.as_deref() == Some("pokemon_essentials_encounters") {
                        self.pokemon_encounters_data = self.api_response_json.clone();
                        self.pokemon_encounters_map_id = self
                            .api_response_json
                            .as_ref()
                            .and_then(|value| value.get("map_id"))
                            .and_then(serde_json::Value::as_i64);
                        self.pokemon_encounters_window_open = true;
                    }
                    let status_map_id = (operation.as_deref() == Some("pokemon_essentials_status"))
                        .then(|| {
                            response_json
                                .as_ref()
                                .and_then(|value| value.get("map_id"))
                                .and_then(serde_json::Value::as_i64)
                        })
                        .flatten();
                    let refresh_encounters = status_map_id.is_some_and(|map_id| {
                        self.pokemon_encounters_window_open
                            && self.pokemon_encounters_map_id != Some(map_id)
                    });
                    if operation.as_deref() == Some("pokemon_essentials_status")
                        && let Some(debug) = response_json
                            .as_ref()
                            .and_then(|value| value.get("debug"))
                            .and_then(serde_json::Value::as_bool)
                    {
                        self.essentials_debug = Some(debug);
                    }
                    if status_map_id.is_some() {
                        // Remember the latest map even if loading that map's
                        // encounter table fails; do not retry every poll.
                        self.pokemon_encounters_map_id = status_map_id;
                    }
                    if operation.as_deref() == Some("world_map") {
                        self.world_map_data = self.api_response_json.clone();
                        self.world_map_layout = None;
                        self.world_map_open = true;
                        if let Some(maps) = self
                            .world_map_data
                            .as_ref()
                            .and_then(|world| world.get("maps"))
                            .and_then(serde_json::Value::as_array)
                        {
                            self.world_map_tile_queue = maps
                                .iter()
                                .filter(|map| map["outdoor"].as_bool() == Some(true))
                                .filter_map(|map| map["id"].as_i64())
                                .filter(|map_id| !self.world_map_previews.contains_key(map_id))
                                .collect();
                            self.world_map_tile_total = self.world_map_tile_queue.len();
                            load_next_outdoor_map = !self.world_map_tile_queue.is_empty();
                        }
                    } else if operation.as_deref() == Some("map_preview") {
                        self.map_preview_data =
                            self.api_response_json.clone().map(std::sync::Arc::new);
                        if let Some(preview) = &self.map_preview_data
                            && let Some(map_id) =
                                preview.get("map_id").and_then(serde_json::Value::as_i64)
                        {
                            self.world_map_previews.insert(map_id, preview.clone());
                        }
                        load_next_outdoor_map = !self.world_map_tile_queue.is_empty();
                    }
                    self.api_response = response;
                    self.api_error = None;
                    if refresh_encounters {
                        self.queue_api_request("pokemon_essentials_encounters", Vec::new());
                    }
                }
                Err(error) => {
                    if operation.as_deref() == Some("cable_club_launch") {
                        self.cable_error = Some(error);
                    } else {
                        self.api_error = Some(error);
                    }
                    load_next_outdoor_map = !self.world_map_tile_queue.is_empty();
                }
            }
            if load_next_outdoor_map
                && self.api_pending.is_none()
                && let Some(map_id) = self.world_map_tile_queue.pop_front()
            {
                self.queue_api_request("map_preview", vec![map_id.to_string()]);
            }
        }
    }

    #[cfg(feature = "game-api")]
    pub(super) fn queue_api_request(&mut self, operation: &str, args: Vec<String>) {
        self.queue_api_request_tagged(operation, operation, args);
    }

    #[cfg(feature = "game-api")]
    pub(super) fn queue_api_request_tagged(
        &mut self,
        operation: &str,
        response_operation: &str,
        args: Vec<String>,
    ) {
        match self.backend.send_request(operation, args) {
            Ok(id) => {
                self.api_pending = Some(id);
                self.api_pending_operation = Some(response_operation.to_owned());
                self.api_error = None;
            }
            Err(error) => self.api_error = Some(error),
        }
    }

    #[cfg(feature = "game-api")]
    pub(super) fn poll_pokemon_encounter_map(&mut self) {
        // Map checks have their own request slot. They must not disable the
        // inspector/Refresh buttons, show a spinner, or replace API responses.
        // Defer consumption while a foreground request is active so a map
        // change cannot overwrite that request's pending ID.
        if self.api_pending.is_some() {
            return;
        }
        if let Some(id) = self.pokemon_encounters_status_pending {
            let Some(result) = self.backend.take_response(id) else {
                return;
            };
            self.pokemon_encounters_status_pending = None;
            let map_id = result.ok().and_then(|response| {
                serde_json::from_str::<serde_json::Value>(&response)
                    .ok()?
                    .get("map_id")?
                    .as_i64()
            });
            if self.pokemon_encounters_window_open
                && let Some(map_id) = map_id
                && self.pokemon_encounters_map_id != Some(map_id)
            {
                // Keep the old table and textures visible until its replacement
                // arrives. Remember the attempted map even if loading fails.
                self.pokemon_encounters_map_id = Some(map_id);
                self.queue_api_request("pokemon_essentials_encounters", Vec::new());
                return;
            }
        }
        if !self.pokemon_encounters_window_open
            || web_time::Instant::now() < self.pokemon_encounters_poll_at
        {
            return;
        }
        self.pokemon_encounters_poll_at =
            web_time::Instant::now() + std::time::Duration::from_millis(750);
        // Background failures leave the current content and foreground error
        // untouched. A later map check can recover without visual churn.
        self.pokemon_encounters_status_pending = self
            .backend
            .send_request("pokemon_essentials_status", Vec::new())
            .ok();
    }

    #[cfg(feature = "game-api")]
    pub(super) fn draw_essentials_tools(&mut self, ui: &mut egui::Ui) {
        self.poll_essentials_debug();
        let can_send = self.api_pending.is_none();
        if ui
            .add_enabled(can_send, egui::Button::new("Load world map"))
            .clicked()
        {
            self.world_map_open = true;
            self.queue_api_request("world_map", Vec::new());
        }
        if ui
            .add_enabled(can_send, egui::Button::new("Current-map encounters"))
            .clicked()
        {
            self.pokemon_encounters_window_open = true;
            self.queue_api_request("pokemon_essentials_encounters", Vec::new());
        }
        if ui.button("Pokédex · species explorer").clicked() {
            self.pokedex.open = true;
            if self.pokedex.catalog.is_empty() {
                self.request_pokedex(true);
            }
        }
        if ui.button("Movedex · move explorer").clicked() {
            self.movedex.open = true;
            self.movedex
                .load_if_needed(&*self.backend, registry_dex::DexKind::Moves);
        }
        if ui.button("Itemdex · item explorer").clicked() {
            self.itemdex.open = true;
            self.itemdex
                .load_if_needed(&*self.backend, registry_dex::DexKind::Items);
        }
        ui.separator();
        self.draw_debug_toggle(ui);
    }

    #[cfg(feature = "game-api")]
    fn draw_debug_toggle(&mut self, ui: &mut egui::Ui) {
        // Ask the game for its current `$DEBUG` value the first time the panel
        // is shown, then keep the checkbox in sync with the game.
        if !self.essentials_debug_queried && self.essentials_debug_pending.is_none() {
            self.essentials_debug_queried = true;
            self.request_essentials_debug("");
        }
        ui.horizontal_wrapped(|ui| {
            let busy = self.essentials_debug_pending.is_some();
            let known = self.essentials_debug.is_some();
            let mut enabled = self.essentials_debug.unwrap_or(false);
            let response = ui.add_enabled(
                !busy && known,
                egui::Checkbox::new(&mut enabled, "Debug mode"),
            );
            if response
                .on_hover_text("Enables the game's own debug menus, Ctrl-modifiers and shortcuts.")
                .changed()
            {
                self.request_essentials_debug(if enabled { "true" } else { "false" });
            }
            if busy {
                ui.spinner();
            } else if !known && ui.button("Query debug state").clicked() {
                self.request_essentials_debug("");
            }
        });
        if let Some((error, message)) = &self.essentials_debug_notice {
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

    #[cfg(feature = "game-api")]
    fn poll_essentials_debug(&mut self) {
        let Some(id) = self.essentials_debug_pending else {
            return;
        };
        let Some(result) = self.backend.take_response(id) else {
            return;
        };
        self.essentials_debug_pending = None;
        match result.and_then(|body| {
            serde_json::from_str::<serde_json::Value>(&body).map_err(|error| error.to_string())
        }) {
            Ok(value) if value["error"].is_string() => {
                self.essentials_debug_notice = Some((
                    true,
                    value["error"]
                        .as_str()
                        .unwrap_or("debug request failed")
                        .to_owned(),
                ));
            }
            Ok(value) => {
                self.essentials_debug = value["debug"].as_bool();
                self.essentials_debug_queried = true;
                let state = match self.essentials_debug {
                    Some(true) => "enabled",
                    Some(false) => "disabled",
                    None => "unknown",
                };
                self.essentials_debug_notice = Some((false, format!("Debug mode {state}.")));
            }
            Err(error) => self.essentials_debug_notice = Some((true, error)),
        }
    }

    #[cfg(feature = "game-api")]
    fn request_essentials_debug(&mut self, mode: &str) {
        if self.essentials_debug_pending.is_some() {
            return;
        }
        match self
            .backend
            .send_request("pokemon_essentials_debug", vec![mode.to_owned()])
        {
            Ok(id) => {
                self.essentials_debug_pending = Some(id);
                self.essentials_debug_notice = None;
            }
            Err(error) => self.essentials_debug_notice = Some((true, error)),
        }
    }

    #[cfg(feature = "game-api")]
    fn draw_api_utilities(&mut self, ui: &mut egui::Ui) {
        let can_send = self.api_pending.is_none();
        ui.collapsing("Map tools", |ui| {
            ui.separator();
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut self.api_map_id)
                        .prefix("Map ID ")
                        .range(1..=99999),
                );
                if ui
                    .add_enabled(can_send, egui::Button::new("Get map tiles"))
                    .clicked()
                {
                    self.queue_api_request("map_preview", vec![self.api_map_id.to_string()]);
                }
            });
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut self.api_map_x).prefix("X "));
                ui.add(egui::DragValue::new(&mut self.api_map_y).prefix("Y "));
                if ui
                    .add_enabled(can_send, egui::Button::new("Teleport"))
                    .clicked()
                {
                    self.queue_api_request(
                        "teleport",
                        vec![
                            self.api_map_id.to_string(),
                            self.api_map_x.to_string(),
                            self.api_map_y.to_string(),
                        ],
                    );
                }
            });
        });
        ui.collapsing("Pokémon Essentials", |ui| {
            ui.label(
                egui::RichText::new("Details include stats, base stats, IVs, EVs, moves, ability, nature, and item.")
                    .small()
                    .weak(),
            );
            if ui
                .add_enabled(can_send, egui::Button::new("Get party"))
                .clicked()
            {
                self.queue_api_request("pokemon_essentials_party", Vec::new());
            }
            if ui.add_enabled(can_send, egui::Button::new("Essentials status")).clicked() {
                self.queue_api_request("pokemon_essentials_status", Vec::new());
            }
            ui.horizontal(|ui| {
                ui.add(egui::Slider::new(&mut self.api_party_slot, 0..=5).text("Slot"));
                if ui
                    .add_enabled(can_send, egui::Button::new("Get details · IVs / EVs"))
                    .clicked()
                {
                    self.queue_api_request("pokemon_essentials_info", vec![self.api_party_slot.to_string()]);
                }
            });
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(can_send, egui::Button::new("Current-map NPCs")).clicked() {
                    self.queue_api_request("pokemon_essentials_npcs", Vec::new());
                }
                if ui.add_enabled(can_send, egui::Button::new("Battle state / options")).clicked() {
                    self.queue_api_request("pokemon_essentials_battle", Vec::new());
                }
            });
        });
    }

    #[cfg(feature = "game-api")]
    pub(super) fn draw_pokeapi_window(&mut self, context: &egui::Context) {
        if !self.api_tools_open {
            return;
        }
        let mut open = self.api_tools_open;
        egui::Window::new("pokeAPI · API + testing")
            .open(&mut open)
            .default_size(egui::vec2(1050.0, 650.0))
            .resizable(true)
            .show(context, |ui| {
                ui.columns(2, |columns| {
                    let height = columns[0].available_height().max(300.0);
                    egui::ScrollArea::vertical()
                        .id_salt("pokeapi-response")
                        .max_height(height)
                        .show(&mut columns[1], |ui| self.draw_api_response(ui));
                    egui::ScrollArea::vertical()
                        .id_salt("pokeapi-tools")
                        .max_height(height)
                        .show(&mut columns[0], |ui| {
                            self.draw_api_endpoint(ui);
                            self.draw_api_utilities(ui);
                            self.draw_api_console(ui);
                            ui.separator();
                            ui.heading("Full OpenAPI contract");
                            let spec = include_str!("../../api/openapi.json");
                            if ui.button("Copy OpenAPI JSON").clicked() {
                                ui.ctx().copy_text(spec.to_owned());
                            }
                            static CONTRACT: std::sync::LazyLock<serde_json::Value> =
                                std::sync::LazyLock::new(|| {
                                    serde_json::from_str(include_str!("../../api/openapi.json"))
                                        .expect("checked-in OpenAPI")
                                });
                            let contract = &*CONTRACT;
                            if let Some(paths) = contract["paths"].as_object() {
                                for (path, methods) in paths {
                                    if let Some(methods) = methods.as_object() {
                                        for (method, operation) in methods {
                                            ui.push_id((path, method), |ui| {
                                                ui.collapsing(
                                                    format!("{} {}", method.to_uppercase(), path),
                                                    |ui| {
                                                        self.draw_openapi_operation(
                                                            ui, path, method, operation,
                                                        );
                                                    },
                                                );
                                            });
                                        }
                                    }
                                }
                            }
                            ui.collapsing(
                                "Complete OpenAPI JSON · schemas and all endpoints",
                                |ui| {
                                    ui.label(egui::RichText::new(spec).monospace());
                                },
                            );
                        });
                });
            });
        self.api_tools_open = open;
    }

    #[cfg(feature = "game-api")]
    fn draw_openapi_operation(
        &mut self,
        ui: &mut egui::Ui,
        path: &str,
        method: &str,
        operation: &serde_json::Value,
    ) {
        let mut target = path.to_owned();
        let mut query = Vec::new();
        if let Some(parameters) = operation["parameters"].as_array() {
            for parameter in parameters {
                let name = parameter["name"].as_str().unwrap_or("");
                let key = format!("{method}:{path}:{name}");
                let value = self.api_parameters.entry(key).or_insert_with(|| {
                    parameter["schema"]["default"]
                        .as_str()
                        .map(str::to_owned)
                        .or_else(|| {
                            parameter["schema"]["default"]
                                .as_i64()
                                .map(|v| v.to_string())
                        })
                        .unwrap_or_default()
                });
                ui.horizontal(|ui| {
                    ui.label(name);
                    ui.text_edit_singleline(value);
                });
                let encoded: String = value
                    .bytes()
                    .map(|byte| {
                        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                            (byte as char).to_string()
                        } else {
                            format!("%{byte:02X}")
                        }
                    })
                    .collect();
                if parameter["in"] == "path" {
                    target = target.replace(&format!("{{{name}}}"), &encoded);
                } else if !value.is_empty() {
                    query.push(format!("{name}={encoded}"));
                }
            }
        }
        if !query.is_empty() {
            target.push('?');
            target.push_str(&query.join("&"));
        }
        if ui
            .add_enabled(
                self.api_pending.is_none(),
                egui::Button::new("Send request"),
            )
            .clicked()
        {
            match crate::api_bridge::route_http(&method.to_uppercase(), &target) {
                Ok((operation, args)) => self.queue_api_request_tagged(operation, "api_test", args),
                Err((_, error)) => self.api_error = Some(error),
            }
        }
        ui.label(egui::RichText::new(serde_json::to_string_pretty(operation).unwrap()).monospace());
    }

    #[cfg(feature = "game-api")]
    pub(super) fn draw_api_console(&mut self, ui: &mut egui::Ui) {
        const KEYS: [&str; 6] = ["up", "down", "left", "right", "action", "cancel"];
        let can_send = self.api_pending.is_none();

        ui.collapsing("Game status", |ui| {
            if ui
                .add_enabled(can_send, egui::Button::new("Get status"))
                .clicked()
            {
                self.queue_api_request("game_status", Vec::new());
            }
        });

        ui.collapsing("Map surroundings", |ui| {
            ui.add(egui::Slider::new(&mut self.api_radius, 1..=15).text("Radius"));
            if ui
                .add_enabled(can_send, egui::Button::new("Get surroundings"))
                .clicked()
            {
                self.queue_api_request("get_surroundings", vec![self.api_radius.to_string()]);
            }
        });

        ui.collapsing("Input", |ui| {
            egui::ComboBox::from_label("Key")
                .selected_text(KEYS[self.api_key])
                .show_ui(ui, |ui| {
                    for (index, key) in KEYS.iter().enumerate() {
                        ui.selectable_value(&mut self.api_key, index, *key);
                    }
                });
            ui.add(egui::Slider::new(&mut self.api_key_frames, 1..=60).text("Frames"));
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(can_send, egui::Button::new("Press key"))
                    .clicked()
                {
                    self.queue_api_request(
                        "press_key",
                        vec![KEYS[self.api_key].into(), self.api_key_frames.to_string()],
                    );
                }
                if ui
                    .add_enabled(can_send, egui::Button::new("Release all"))
                    .clicked()
                {
                    self.queue_api_request("release_all_keys", Vec::new());
                }
            });
        });
    }

    #[cfg(feature = "game-api")]
    fn draw_api_response(&mut self, ui: &mut egui::Ui) {
        ui.heading("API response");
        if self.api_pending.is_some() {
            ui.spinner();
            ui.label("Waiting for the game thread…");
        }
        if let Some(error) = &self.api_error {
            ui.add(egui::Label::new(error).selectable(true));
        }
        if self.api_response.is_empty() {
            ui.weak("Responses will appear here.");
            return;
        }
        let response = self
            .api_response_json
            .as_ref()
            .and_then(|value| serde_json::to_string_pretty(value).ok())
            .unwrap_or_else(|| self.api_response.clone());
        if ui.button("Copy response").clicked() {
            ui.ctx().copy_text(response.clone());
        }
        ui.add(egui::Label::new(egui::RichText::new(response).monospace()).selectable(true));
        if self
            .api_response_json
            .as_ref()
            .is_some_and(|value| value.get("ivs").is_some() || value.get("evs").is_some())
        {
            ui.collapsing("Pokémon details", |ui| self.draw_pokemon_details(ui));
        }
    }
}

#[cfg(all(test, feature = "game-api"))]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::HashMap, rc::Rc};

    #[derive(Clone, Default)]
    struct TestBackend {
        requests: Rc<RefCell<Vec<String>>>,
        request_args: Rc<RefCell<Vec<Vec<String>>>>,
        responses: Rc<RefCell<HashMap<u64, Result<String, String>>>>,
    }

    impl backend::Backend for TestBackend {
        fn fast_forward(&self) -> bool {
            false
        }
        fn set_fast_forward(&self, _: bool) {}
        fn current_rate(&self) -> u32 {
            40
        }
        fn set_rate_override(&self, _: u32) {}
        fn request_reset(&self) {}
        fn patch_enabled(&self, _: &str) -> bool {
            false
        }
        fn set_patch_enabled(&self, _: &str, _: bool) -> bool {
            false
        }
        fn patch_value(&self, _: &str) -> Option<f64> {
            None
        }
        fn set_patch_value(&self, _: &str, _: f64) -> bool {
            false
        }
        fn listening_port(&self) -> Option<u16> {
            None
        }
        fn start_api(&self, _: u16) -> Result<(), String> {
            Ok(())
        }
        fn take_response(&self, id: u64) -> Option<Result<String, String>> {
            self.responses.borrow_mut().remove(&id)
        }
        fn send_request(&self, operation: &str, args: Vec<String>) -> Result<u64, String> {
            let mut requests = self.requests.borrow_mut();
            requests.push(operation.into());
            self.request_args.borrow_mut().push(args);
            Ok(requests.len() as u64)
        }
        fn asset_image(&self, _: &str, _: &str) -> Option<std::sync::Arc<image::RgbaImage>> {
            None
        }
        #[cfg(not(target_arch = "wasm32"))]
        fn cable_status(&self) -> crate::cable_club::ServerStatus {
            Default::default()
        }
        #[cfg(not(target_arch = "wasm32"))]
        fn stop_cable(&self) {}
        #[cfg(not(target_arch = "wasm32"))]
        fn start_cable(&self, _: &str, _: u16) -> Result<(), String> {
            Ok(())
        }
        #[cfg(not(target_arch = "wasm32"))]
        fn save_endpoint(&self, _: &str, _: u16) -> Result<std::path::PathBuf, String> {
            Ok(Default::default())
        }
    }

    fn encounters() -> (Workspace, TestBackend) {
        let backend = TestBackend::default();
        let mut workspace = Workspace::new();
        workspace.backend = Box::new(backend.clone());
        workspace.pokemon_encounters_window_open = true;
        workspace.pokemon_encounters_map_id = Some(1);
        workspace.pokemon_encounters_data = Some(serde_json::json!({"map_id": 1, "tables": []}));
        workspace.api_response = "previous manual response".into();
        workspace.api_error = Some("previous manual error".into());
        (workspace, backend)
    }

    #[test]
    fn inventory_actions_are_single_flight_and_preserve_api_results() {
        let backend = TestBackend::default();
        let mut workspace = Workspace::new();
        workspace.backend = Box::new(backend.clone());
        workspace.api_response = "manual response".into();
        workspace.api_error = Some("manual error".into());
        workspace
            .itemdex
            .add_inventory(&*workspace.backend, "POTION", 7);
        workspace
            .itemdex
            .add_inventory(&*workspace.backend, "POTION", 7);
        assert_eq!(
            &*backend.requests.borrow(),
            &["pokemon_essentials_add_item"]
        );
        assert_eq!(
            backend.request_args.borrow()[0],
            vec!["POTION".to_owned(), "7".to_owned()]
        );
        backend.responses.borrow_mut().insert(1,Ok(r#"{"item":"POTION","name":"Poción","added":7,"requested":7,"total":9,"complete":true}"#.into()));
        let context = egui::Context::default();
        let _ = context.run(egui::RawInput::default(), |context| {
            workspace.draw_registry_dex_windows(context)
        });
        assert!(backend.responses.borrow().is_empty());
        assert_eq!(workspace.api_response, "manual response");
        assert_eq!(workspace.api_error.as_deref(), Some("manual error"));
        assert!(workspace.api_pending.is_none());
        workspace
            .itemdex
            .add_inventory(&*workspace.backend, "POTION", 1);
        assert_eq!(backend.requests.borrow().len(), 2);
    }

    #[test]
    fn pokedex_requests_do_not_overwrite_the_api_console() {
        let backend = TestBackend::default();
        let mut workspace = Workspace::new();
        workspace.backend = Box::new(backend.clone());
        workspace.pokedex.open = true;
        workspace.api_response = "manual response".into();
        workspace.api_error = Some("manual error".into());
        workspace.request_pokedex(true);
        backend
            .responses
            .borrow_mut()
            .insert(1, Ok(r#"{"species":[],"note":"Game data"}"#.into()));
        let context = egui::Context::default();
        let _ = context.run(egui::RawInput::default(), |context| {
            workspace.draw_pokedex_window(context)
        });
        assert_eq!(workspace.api_response, "manual response");
        assert_eq!(workspace.api_error.as_deref(), Some("manual error"));
        assert!(workspace.api_pending.is_none());
    }

    #[test]
    fn api_response_is_in_the_right_pane() {
        let mut workspace = Workspace::new();
        workspace.api_tools_open = true;
        workspace.api_response = "copyable response marker".into();
        let context = egui::Context::default();
        let mut positions = std::collections::HashMap::new();
        for _ in 0..2 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1500.0, 1000.0),
                )),
                ..Default::default()
            };
            let output = context.run(input, |context| workspace.draw_pokeapi_window(context));
            for shape in output.shapes {
                if let egui::Shape::Text(text) = shape.shape {
                    positions.insert(text.galley.text().to_owned(), text.pos.x);
                }
            }
        }
        assert!(positions["API response"] > positions["Full OpenAPI contract"]);
        assert!(positions.contains_key("copyable response marker"));
        assert!(!positions.contains_key("Open response window"));
    }

    #[test]
    fn unchanged_map_checks_do_not_change_content_or_foreground_request_state() {
        let (mut workspace, backend) = encounters();
        let data = workspace.pokemon_encounters_data.clone();
        for _ in 0..3 {
            workspace.pokemon_encounters_poll_at = web_time::Instant::now();
            workspace.poll_pokemon_encounter_map();
            assert!(workspace.api_pending.is_none());
            assert!(workspace.api_pending_operation.is_none());
            let id = workspace.pokemon_encounters_status_pending.unwrap();
            backend
                .responses
                .borrow_mut()
                .insert(id, Ok(r#"{"map_id":1}"#.into()));
            workspace.poll_pokemon_encounter_map();
            assert_eq!(workspace.pokemon_encounters_data, data);
            assert!(workspace.api_pending.is_none());
            assert_eq!(workspace.api_response, "previous manual response");
            assert_eq!(
                workspace.api_error.as_deref(),
                Some("previous manual error")
            );
        }
        assert_eq!(
            &*backend.requests.borrow(),
            &["pokemon_essentials_status"; 3]
        );
    }

    #[test]
    fn map_change_keeps_old_content_until_new_table_arrives_and_fetches_once() {
        let (mut workspace, backend) = encounters();
        let old_data = workspace.pokemon_encounters_data.clone();
        workspace.poll_pokemon_encounter_map();
        backend
            .responses
            .borrow_mut()
            .insert(1, Ok(r#"{"map_id":2}"#.into()));
        // A user request issued while the background check is in flight wins.
        workspace.queue_api_request("game_status", Vec::new());
        workspace.poll_pokemon_encounter_map();
        assert_eq!(workspace.api_pending, Some(2));
        assert_eq!(workspace.pokemon_encounters_status_pending, Some(1));
        backend
            .responses
            .borrow_mut()
            .insert(2, Ok(r#"{"ready":true}"#.into()));
        workspace.poll_api_response();
        workspace.poll_pokemon_encounter_map();
        assert_eq!(workspace.api_pending, Some(3));
        assert_eq!(workspace.pokemon_encounters_data, old_data);
        workspace.poll_pokemon_encounter_map();
        assert_eq!(backend.requests.borrow().len(), 3);
        let next = serde_json::json!({"map_id": 2, "tables": [{"name": "Grass"}]});
        backend
            .responses
            .borrow_mut()
            .insert(3, Ok(next.to_string()));
        workspace.poll_api_response();
        assert_eq!(workspace.pokemon_encounters_data, Some(next));
        workspace.pokemon_encounters_poll_at = web_time::Instant::now();
        workspace.poll_pokemon_encounter_map();
        backend
            .responses
            .borrow_mut()
            .insert(4, Ok(r#"{"map_id":2}"#.into()));
        workspace.poll_pokemon_encounter_map();
        assert!(workspace.api_pending.is_none());
        assert_eq!(
            backend
                .requests
                .borrow()
                .iter()
                .filter(|op| *op == "pokemon_essentials_encounters")
                .count(),
            1
        );
    }

    #[test]
    fn failed_background_check_preserves_diagnostics_and_closed_window_stays_closed() {
        let (mut workspace, backend) = encounters();
        workspace.poll_pokemon_encounter_map();
        backend
            .responses
            .borrow_mut()
            .insert(1, Err("temporarily unavailable".into()));
        workspace.poll_pokemon_encounter_map();
        assert_eq!(
            workspace.api_error.as_deref(),
            Some("previous manual error")
        );
        assert_eq!(
            workspace.pokemon_encounters_data.as_ref().unwrap()["map_id"],
            1
        );
        workspace.pokemon_encounters_poll_at = web_time::Instant::now();
        workspace.poll_pokemon_encounter_map();
        workspace.pokemon_encounters_window_open = false;
        backend
            .responses
            .borrow_mut()
            .insert(2, Ok(r#"{"map_id":2}"#.into()));
        workspace.poll_pokemon_encounter_map();
        assert!(!workspace.pokemon_encounters_window_open);
        assert!(workspace.api_pending.is_none());
        assert!(workspace.pokemon_encounters_status_pending.is_none());
    }

    #[test]
    fn debug_toggle_queries_the_game_and_tracks_its_state() {
        let backend = TestBackend::default();
        let mut workspace = Workspace::new();
        workspace.backend = Box::new(backend.clone());
        // Asking for the current value uses an empty mode argument.
        workspace.request_essentials_debug("");
        assert_eq!(backend.requests.borrow().len(), 1);
        assert_eq!(
            backend.requests.borrow()[0].as_str(),
            "pokemon_essentials_debug"
        );
        assert_eq!(backend.request_args.borrow()[0], vec![String::new()]);
        // The reply feeds the cached checkbox state.
        backend
            .responses
            .borrow_mut()
            .insert(1, Ok(r#"{"debug":true}"#.into()));
        workspace.poll_essentials_debug();
        assert_eq!(workspace.essentials_debug, Some(true));
        assert!(workspace.essentials_debug_notice.is_some());
        // A toggle sends the desired value explicitly rather than flipping blindly.
        workspace.request_essentials_debug("false");
        assert_eq!(backend.request_args.borrow()[1], vec!["false".to_string()]);
        // Errors surface as a notice without clearing the known state.
        backend
            .responses
            .borrow_mut()
            .insert(2, Err("game is not ready".into()));
        workspace.poll_essentials_debug();
        assert_eq!(workspace.essentials_debug, Some(true));
        assert!(workspace.essentials_debug_notice.as_ref().unwrap().0);
    }
}
