// SPDX-License-Identifier: GPL-2.0-or-later
//! The "Connected world" window: an interactive canvas of RPG Maker maps,
//! Essentials connections, linked interiors, and a teleport target picker.
//!
//! [`Workspace::draw_world_map_window`] is the window itself, split into a left
//! canvas (drawn by [`Workspace::draw_world_canvas`]) and a right pane that
//! chooses either a map/tile destination or the interiors linked to the current
//! overworld map. Selecting a tile sets the `teleport` target; "Teleport to
//! tile" queues it. The canvas supports pan (drag) and zoom (scroll or ±), and
//! `build_world_map_layout` in the parent module positions the maps.
//!
//! ## Main functions
//! * [`Workspace::draw_world_map_window`] — the window, destination/indoors
//!   pane, and tile-preview placement.
//! * [`Workspace::draw_world_canvas`] — paints map rectangles and connection
//!   edges, handles pan/zoom, and lets a click select a map and tile.
//! * [`Workspace::draw_map_tiles`] / [`Workspace::draw_map_tiles_preview`] —
//!   render a single map's real tile layers with a selectable tile.
//! * [`Workspace::paint_map_preview`] — uploads/uses a composed texture for a
//!   preview; [`Workspace::compose_map_texture`] rasterizes the layers.
//! * [`Workspace::map_asset_image`] / [`Workspace::map_asset_image_scaled`] —
//!   fetch and cache tileset/autotile bitmaps at a requested tile resolution.
//!
//! ## Backend use
//! This file reads `world_map_data`, `world_map_previews`, and
//! `map_preview_data` (populated by `poll_api_response` in `inspector`) and
//! queues `map_preview` / `teleport` through `queue_api_request`. Asset bytes
//! come from `backend.asset_image(category, name)`.
//!
//! ## JSON schema
//! The world map is `{ "maps": [{ "id", "name", "width", "height",
//! "tileset_id", "outdoor", "region", "world_x", "world_y" }], "connections":
//! [[6 ints]], "transfers": [[6 ints]], "current_map_id" }`; each edge is
//! `[map, x, y, map, x, y]`. A map preview is `{ "map_id", "width", "height",
//! "tileset", "autotiles": [names], "autotile_parts": [[...]], "layers":
//! [[[tile ids]]] }`. While composing, tile ids below 48 are empty; ids
//! `48..384` are autotiles (index `id/48 - 1`, quadrant layout `id % 48` from
//! `autotile_parts`); ids `>= 384` index tileset cells `(id-384) % 8` by
//! `(id-384) / 8`. Tiles are 32 px in RGSS and each autotile is a 6-column
//! sheet of half-tile quadrants.
use super::*;
impl Workspace {
    #[cfg(feature = "game-api")]
    pub(super) fn draw_world_map_window(&mut self, context: &egui::Context) {
        let Some(world) = self.world_map_data.clone() else {
            return;
        };
        let maps = world
            .get("maps")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let connections = world
            .get("connections")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let transfers = world
            .get("transfers")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        if !self.world_map_open {
            return;
        }
        if self.world_map_selected_map.is_none() {
            self.world_map_selected_map = world
                .get("current_map_id")
                .and_then(serde_json::Value::as_i64)
                .filter(|id| {
                    maps.iter().any(|map| {
                        map["id"].as_i64() == Some(*id) && map["outdoor"].as_bool() == Some(true)
                    })
                })
                .or_else(|| {
                    maps.iter()
                        .find(|map| map["outdoor"].as_bool() == Some(true))
                        .and_then(|map| map["id"].as_i64())
                });
        }
        let mut open = self.world_map_open;
        egui::Window::new("Connected world · actual maps and connections")
            .open(&mut open)
            .default_size(egui::vec2(1440.0, 960.0))
            .min_size(egui::vec2(480.0, 280.0))
            .resizable(egui::Vec2b::TRUE)
            .show(context, |ui| {
                if maps.is_empty() {
                    ui.label("No RPG Maker maps were found in this game.");
                    return;
                }
                let can_send = self.api_pending.is_none();
                ui.horizontal_wrapped(|ui| {
                    if ui.button("−").clicked() { self.world_map_zoom = (self.world_map_zoom / 1.25).clamp(0.01, 256.0); }
                    if ui.button("+").clicked() { self.world_map_zoom = (self.world_map_zoom * 1.25).clamp(0.01, 256.0); }
                    if ui.button("Fit world").clicked() { self.world_map_zoom = 1.0; self.world_map_pan = egui::Vec2::ZERO; }
                        ui.label(format!("{:.1}× · drag to pan · scroll to zoom · {} overworld maps · {} stitched", self.world_map_zoom, maps.iter().filter(|map| map["outdoor"].as_bool() == Some(true)).count(), self.world_map_previews.len()));
                });
                ui.horizontal(|ui| {
                    if !self.world_map_tile_queue.is_empty() {
                        ui.label(format!("Stitching Essentials overworld tiles · {} maps remaining of {}", self.world_map_tile_queue.len(), self.world_map_tile_total));
                    } else {
                        ui.label("Click a tile in the stitched world, then choose Teleport to tile.");
                    }
                });
                let body_size = ui.available_size();
                let gap = 12.0;
                let canvas_width = ((body_size.x-gap)*0.68).max(0.0);
                let (body, _) = ui.allocate_exact_size(body_size,egui::Sense::hover());
                let canvas_rect = egui::Rect::from_min_size(body.min,egui::vec2(canvas_width,body.height()));
                let destination_rect = egui::Rect::from_min_max(egui::pos2(canvas_rect.right()+gap,body.top()),body.max);
                ui.painter().vline(canvas_rect.right()+gap/2.0,body.y_range(),ui.visuals().widgets.noninteractive.bg_stroke);
                let overworld: Vec<serde_json::Value> = maps.iter().filter(|map|map["outdoor"].as_bool()==Some(true)).cloned().collect();
                self.draw_world_canvas(&mut bounded_tool_pane(ui,"world-canvas-pane",canvas_rect),context,&overworld,&connections,&transfers);
                let mut destination_ui = bounded_tool_pane(ui,"world-destination-pane",destination_rect);
                {
                    let ui = &mut destination_ui;
                            let selected_id = self.world_map_selected_map;
                            let mut indoor_ids = Vec::new();
                            if let Some(id) = selected_id {
                                for edge in &transfers {
                                    let Some(values) = edge.as_array() else { continue; };
                                    let (Some(from), Some(to)) = (values.first().and_then(serde_json::Value::as_i64),
                                        values.get(3).and_then(serde_json::Value::as_i64)) else { continue; };
                                    let Some(indoor_id) = (if from == id { Some(to) } else if to == id { Some(from) } else { None }) else { continue; };
                                    if maps.iter().any(|candidate| candidate["id"].as_i64() == Some(indoor_id)
                                        && candidate["outdoor"].as_bool() != Some(true)) {
                                        indoor_ids.push(indoor_id);
                                    }
                                }
                            }
                            indoor_ids.sort_unstable();
                            indoor_ids.dedup();
                            if self.world_map_indoors_parent != selected_id {
                                self.world_map_indoors_parent = selected_id;
                                self.world_map_indoors_selected = indoor_ids.first().copied();
                            } else if !self.world_map_indoors_selected.is_some_and(|id| indoor_ids.contains(&id)) {
                                self.world_map_indoors_selected = indoor_ids.first().copied();
                            }

                            ui.horizontal(|ui| {
                                if ui.selectable_label(!self.world_map_right_pane_indoors, "Destination").clicked() {
                                    self.world_map_right_pane_indoors = false;
                                }
                                let indoors_label = if indoor_ids.is_empty() {
                                    "Indoors".to_owned()
                                } else {
                                    format!("Indoors · {}", indoor_ids.len())
                                };
                                if ui.add_enabled(!indoor_ids.is_empty(), egui::Button::selectable(
                                    self.world_map_right_pane_indoors, indoors_label,
                                )).clicked() {
                                    self.world_map_right_pane_indoors = true;
                                }
                            });
                            ui.separator();
                            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                                if self.world_map_right_pane_indoors {
                                    if let Some(parent_id) = selected_id {
                                        let parent_name = maps.iter().find(|map| map["id"].as_i64() == Some(parent_id))
                                            .and_then(|map| map["name"].as_str()).unwrap_or("Map");
                                        ui.heading(format!("Indoors · {parent_name}"));
                                    } else {
                                        ui.heading("Indoors");
                                    }
                                    if indoor_ids.is_empty() {
                                        ui.label("Select an overworld map with linked interiors.");
                                    } else {
                                        ui.label("Linked interior maps · select a tile in Destination to set the teleport target.");
                                        let card_width = ((ui.available_width() - 8.0) * 0.5).max(120.0);
                                        ui.horizontal_wrapped(|ui| {
                                            for id in &indoor_ids {
                                                let map_name = maps.iter().find(|map| map["id"].as_i64() == Some(*id))
                                                    .and_then(|map| map["name"].as_str()).unwrap_or("Interior");
                                                let card_size = egui::vec2(card_width, 148.0);
                                                ui.allocate_ui(card_size, |ui| {
                                                    let (card, response) = ui.allocate_exact_size(card_size, egui::Sense::click());
                                                    let painter = ui.painter().with_clip_rect(card);
                                                    painter.rect_filled(card, 3.0, egui::Color32::from_gray(18));
                                                    let mut map_rect = None;
                                                    if let Some(preview) = self.world_map_previews.get(id).cloned() {
                                                        if let (Some(width), Some(height)) = (preview["width"].as_u64(), preview["height"].as_u64()) {
                                                            let image_area = card.shrink2(egui::vec2(5.0, 5.0));
                                                            let image_area = egui::Rect::from_min_max(
                                                                image_area.min,
                                                                egui::pos2(image_area.max.x, image_area.max.y - 27.0),
                                                            );
                                                            let fit = (image_area.width() / (width as f32 * 32.0))
                                                                .min(image_area.height() / (height as f32 * 32.0));
                                                            let image_size = egui::vec2(width as f32 * 32.0 * fit, height as f32 * 32.0 * fit);
                                                            let image_rect = egui::Rect::from_center_size(image_area.center(), image_size);
                                                            map_rect = Some((image_rect, width as i32, height as i32, fit, preview.clone()));
                                                            self.paint_map_preview(context, &painter, card, image_rect, 32.0 * fit, &preview, false);
                                                        }
                                                    } else {
                                                        painter.text(card.center(), egui::Align2::CENTER_CENTER, "Loading tiles…",
                                                            egui::FontId::proportional(12.0), egui::Color32::LIGHT_GRAY);
                                                        if self.api_pending.is_none() && self.world_map_tile_queue.is_empty() {
                                                            self.queue_api_request("map_preview", vec![id.to_string()]);
                                                        }
                                                    }
                                                    let title_rect = egui::Rect::from_min_max(
                                                        egui::pos2(card.left(), card.bottom() - 27.0), card.max,
                                                    );
                                                    painter.rect_filled(title_rect, 0.0, egui::Color32::from_black_alpha(210));
                                                    painter.text(title_rect.shrink2(egui::vec2(5.0, 0.0)).left_center(),
                                                        egui::Align2::LEFT_CENTER, format!("{} · {}", id, map_name),
                                                        egui::FontId::proportional(12.0), egui::Color32::WHITE);
                                                    if response.clicked() {
                                                        self.world_map_selected_map = Some(*id);
                                                        if let Some((image_rect, width, height, fit, preview)) = map_rect {
                                                            if let Some(pointer) = response.interact_pointer_pos() {
                                                                self.world_map_x = (((pointer.x - image_rect.left()) / (32.0 * fit)).floor() as i32).clamp(0, width - 1);
                                                                self.world_map_y = (((pointer.y - image_rect.top()) / (32.0 * fit)).floor() as i32).clamp(0, height - 1);
                                                            } else {
                                                                self.world_map_x = 0;
                                                                self.world_map_y = 0;
                                                            }
                                                            self.map_preview_data = Some(preview);
                                                            self.world_map_right_pane_indoors = false;
                                                        }
                                                    }
                                                    response.on_hover_text("Click to use this interior as the teleport destination");
                                                });
                                            }
                                        });
                                    }
                                } else {
                                    ui.heading("Destination");
                                    egui::ComboBox::from_id_salt("world-map-selected-map")
                                        .selected_text(self.world_map_selected_map.and_then(|id| maps.iter().find(|map| map["id"].as_i64() == Some(id)))
                                            .map(|map| format!("{} · {}", map["id"], map["name"].as_str().unwrap_or("Map")))
                                            .unwrap_or_else(|| "Select map".into()))
                                        .show_ui(ui, |ui| {
                                            for map in &maps {
                                                let id = map["id"].as_i64().unwrap_or_default();
                                                let name = map["name"].as_str().unwrap_or("Map");
                                                if ui.selectable_value(&mut self.world_map_selected_map, Some(id), format!("{id} · {name}")).clicked() {
                                                    self.world_map_x = 0;
                                                    self.world_map_y = 0;
                                                    if let Some(preview) = self.world_map_previews.get(&id).cloned() {
                                                        self.map_preview_data = Some(preview);
                                                    } else if can_send && self.world_map_tile_queue.is_empty() {
                                                        self.queue_api_request("map_preview", vec![id.to_string()]);
                                                    }
                                                }
                                            }
                                        });
                                    if let Some(id) = self.world_map_selected_map {
                                        if self.map_preview_data.as_ref().and_then(|value| value["map_id"].as_i64()) != Some(id)
                                            && let Some(preview) = self.world_map_previews.get(&id).cloned()
                                        {
                                            self.map_preview_data = Some(preview);
                                        }
                                        if let Some(map) = maps.iter().find(|map| map["id"].as_i64() == Some(id)) {
                                            let width = map["width"].as_i64().unwrap_or(1).max(1) as i32;
                                            let height = map["height"].as_i64().unwrap_or(1).max(1) as i32;
                                            self.world_map_x = self.world_map_x.clamp(0, width - 1);
                                            self.world_map_y = self.world_map_y.clamp(0, height - 1);
                                            ui.label(format!("{} × {} tiles · tileset {}", width, height, map["tileset_id"]));
                                            if !indoor_ids.is_empty() && ui.button(format!("Show {} linked interiors", indoor_ids.len())).clicked() {
                                                self.world_map_right_pane_indoors = true;
                                            }
                                            ui.horizontal(|ui| {
                                                ui.add(egui::DragValue::new(&mut self.world_map_x).range(0..=width - 1).prefix("X "));
                                                ui.add(egui::DragValue::new(&mut self.world_map_y).range(0..=height - 1).prefix("Y "));
                                            });
                                            if ui.add_enabled(can_send, egui::Button::new("Load actual tiles")).clicked() {
                                                self.queue_api_request("map_preview", vec![id.to_string()]);
                                            }
                                            if ui.add_enabled(can_send, egui::Button::new("Teleport to tile")).clicked() {
                                                self.queue_api_request("teleport", vec![id.to_string(), self.world_map_x.to_string(), self.world_map_y.to_string()]);
                                            }
                                        }
                                        if self.map_preview_data.as_ref().and_then(|value| value["map_id"].as_i64()) == Some(id) {
                                            self.draw_map_tiles(ui, context);
                                        } else {
                                            ui.label("Select a map or load its actual tile layers.");
                                        }
                                    }
                                }
                            });
                }
            });
        self.world_map_open = open;
    }

    #[cfg(feature = "game-api")]
    pub(super) fn draw_world_canvas(
        &mut self,
        ui: &mut egui::Ui,
        context: &egui::Context,
        maps: &[serde_json::Value],
        connections: &[serde_json::Value],
        transfers: &[serde_json::Value],
    ) {
        // This UI is allocated to the full left-hand region by the window, so
        // the map canvas tracks both horizontal and vertical window resizing.
        let (viewport, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::drag());
        if response.dragged() {
            self.world_map_pan += response.drag_delta();
        }
        let zoom_scroll = if response.hovered() {
            ui.input(|input| input.raw_scroll_delta.y)
        } else {
            0.0
        };
        let layout = self
            .world_map_layout
            .get_or_insert_with(|| build_world_map_layout(maps, connections, transfers))
            .clone();
        let origins = &layout.origins;
        let edges = &layout.edges;
        let bounds = maps
            .iter()
            .filter_map(|map| {
                let id = map["id"].as_i64()?;
                let origin = *origins.get(&id)?;
                let width = map["width"].as_i64().unwrap_or(1) as f32;
                let height = map["height"].as_i64().unwrap_or(1) as f32;
                Some(egui::Rect::from_min_size(
                    egui::pos2(origin.x, origin.y),
                    egui::vec2(width, height),
                ))
            })
            .reduce(|a, b| a.union(b))
            .unwrap_or(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1.0, 1.0),
            ));
        let fit = (viewport.width() / bounds.width().max(1.0))
            .min(viewport.height() / bounds.height().max(1.0))
            * 0.92;
        if zoom_scroll != 0.0 {
            let old_scale = fit * self.world_map_zoom;
            let new_zoom = (self.world_map_zoom * (zoom_scroll * 0.001).exp()).clamp(0.01, 256.0);
            if old_scale > 0.0
                && let Some(pointer) = response.hover_pos()
            {
                let center = viewport.center();
                let world_offset = (pointer - center - self.world_map_pan) / old_scale;
                self.world_map_pan = pointer - center - world_offset * (fit * new_zoom);
            }
            self.world_map_zoom = new_zoom;
        }
        let scale = fit * self.world_map_zoom;
        let world_center = bounds.center().to_vec2();
        let screen_center = viewport.center() + self.world_map_pan;
        let map_screen = |point: egui::Vec2| screen_center + (point - world_center) * scale;
        let painter = ui.painter().with_clip_rect(viewport);
        painter.rect_filled(viewport, 0.0, egui::Color32::from_gray(18));
        for &(a, b, a_point, b_point) in edges {
            if let (Some(a_origin), Some(b_origin)) = (origins.get(&a), origins.get(&b)) {
                let pa = map_screen(*a_origin + a_point);
                let pb = map_screen(*b_origin + b_point);
                painter.line_segment(
                    [pa, pb],
                    egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(75, 190, 225)),
                );
            }
        }
        for map in maps {
            let Some(id) = map["id"].as_i64() else {
                continue;
            };
            let Some(origin) = origins.get(&id) else {
                continue;
            };
            let w = map["width"].as_i64().unwrap_or(1).max(1) as f32;
            let h = map["height"].as_i64().unwrap_or(1).max(1) as f32;
            let rect = egui::Rect::from_min_max(
                map_screen(*origin),
                map_screen(*origin + egui::vec2(w, h)),
            );
            if !rect.intersects(viewport) {
                continue;
            }
            let selected = self.world_map_selected_map == Some(id);
            let preview = self.world_map_previews.get(&id).cloned();
            if let Some(preview) = preview {
                self.paint_map_preview(context, &painter, viewport, rect, scale, &preview, false);
            } else {
                let fill = if selected {
                    egui::Color32::from_rgb(48, 95, 112)
                } else if map["outdoor"].as_bool() == Some(true) {
                    egui::Color32::from_rgb(50, 92, 61)
                } else {
                    egui::Color32::from_rgb(73, 70, 83)
                };
                painter.rect_filled(rect, 0.0, fill);
            }
            painter.rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(
                    if selected { 2.0_f32 } else { 0.6_f32 },
                    if selected {
                        egui::Color32::YELLOW
                    } else {
                        egui::Color32::from_gray(180)
                    },
                ),
                egui::StrokeKind::Inside,
            );
            if scale > 0.13 && !self.world_map_previews.contains_key(&id) {
                let name = map["name"].as_str().unwrap_or("Map");
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    format!("{id} {name}"),
                    egui::FontId::proportional((11.0 * scale).clamp(7.0, 14.0)),
                    egui::Color32::WHITE,
                );
            }
            let map_response = ui.interact(
                rect,
                egui::Id::new(("world-map-node", id)),
                egui::Sense::click(),
            );
            if map_response.clicked() {
                self.world_map_selected_map = Some(id);
                if let Some(position) = map_response.interact_pointer_pos() {
                    self.world_map_x = (((position.x - rect.left()) / scale).floor() as i32)
                        .clamp(0, w as i32 - 1);
                    self.world_map_y =
                        (((position.y - rect.top()) / scale).floor() as i32).clamp(0, h as i32 - 1);
                } else {
                    self.world_map_x = 0;
                    self.world_map_y = 0;
                }
                if let Some(preview) = self.world_map_previews.get(&id).cloned() {
                    self.map_preview_data = Some(preview);
                } else if self.api_pending.is_none() && self.world_map_tile_queue.is_empty() {
                    self.queue_api_request("map_preview", vec![id.to_string()]);
                }
            }
            if selected {
                let cell = egui::Rect::from_min_size(
                    rect.min
                        + egui::vec2(
                            self.world_map_x as f32 * scale,
                            self.world_map_y as f32 * scale,
                        ),
                    egui::vec2(scale, scale),
                );
                painter.rect_stroke(
                    cell,
                    0.0,
                    egui::Stroke::new(2.0_f32, egui::Color32::YELLOW),
                    egui::StrokeKind::Inside,
                );
            }
        }
    }

    #[cfg(feature = "game-api")]
    pub(super) fn draw_map_tiles(&mut self, ui: &mut egui::Ui, context: &egui::Context) {
        let Some(preview) = self.map_preview_data.clone() else {
            return;
        };
        self.draw_map_tiles_preview(ui, context, &preview);
    }

    #[cfg(feature = "game-api")]
    pub(super) fn draw_map_tiles_preview(
        &mut self,
        ui: &mut egui::Ui,
        context: &egui::Context,
        preview: &serde_json::Value,
    ) {
        let (Some(width), Some(height)) = (preview["width"].as_u64(), preview["height"].as_u64())
        else {
            return;
        };
        let width = width as usize;
        let height = height as usize;
        let available = ui.available_size().max(egui::vec2(160.0, 160.0));
        let (viewport, response) = ui.allocate_exact_size(
            egui::vec2(available.x, available.y.min(700.0)),
            egui::Sense::click(),
        );
        let fit = (viewport.width() / (width as f32 * 32.0))
            .min(viewport.height() / (height as f32 * 32.0))
            .max(0.04);
        let map_size = egui::vec2(width as f32 * 32.0 * fit, height as f32 * 32.0 * fit);
        let map_rect = egui::Rect::from_min_size(viewport.min, map_size);
        if response.clicked()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            self.world_map_x = (((pointer.x - map_rect.left()) / (32.0 * fit)).floor() as i32)
                .clamp(0, width as i32 - 1);
            self.world_map_y = (((pointer.y - map_rect.top()) / (32.0 * fit)).floor() as i32)
                .clamp(0, height as i32 - 1);
        }
        let painter = ui.painter().with_clip_rect(viewport);
        painter.rect_filled(viewport, 0.0, egui::Color32::from_gray(12));
        self.paint_map_preview(
            context,
            &painter,
            viewport,
            map_rect,
            32.0 * fit,
            preview,
            true,
        );
        let selection = egui::Rect::from_min_size(
            map_rect.min
                + egui::vec2(
                    self.world_map_x as f32 * 32.0 * fit,
                    self.world_map_y as f32 * 32.0 * fit,
                ),
            egui::vec2(32.0 * fit, 32.0 * fit),
        );
        painter.rect_stroke(
            selection,
            0.0,
            egui::Stroke::new(2.0_f32, egui::Color32::YELLOW),
            egui::StrokeKind::Inside,
        );
        ui.label("Select any tile in the rendered map or world canvas to set the teleport target.");
    }

    #[cfg(feature = "game-api")]
    pub(super) fn paint_map_preview(
        &mut self,
        context: &egui::Context,
        painter: &egui::Painter,
        viewport: egui::Rect,
        map_rect: egui::Rect,
        tile_size: f32,
        preview: &serde_json::Value,
        detail: bool,
    ) {
        let Some(map_id) = preview["map_id"].as_i64() else {
            return;
        };
        let max_tile_pixels = if detail { 32 } else { 8 };
        let required_pixels = (tile_size * context.pixels_per_point()).ceil().max(1.0) as u32;
        let requested_pixels = required_pixels
            .min(max_tile_pixels)
            .next_power_of_two()
            .min(max_tile_pixels);
        let existing = if detail {
            self.world_map_detail_textures.get(&map_id)
        } else {
            self.world_map_map_textures.get(&map_id)
        };
        let texture = existing
            .filter(|(resolution, _)| *resolution >= requested_pixels)
            .map(|(_, texture)| texture.clone())
            .or_else(|| {
                self.compose_map_texture(context, map_id, preview, requested_pixels, detail)
            });
        let Some(texture) = texture else {
            return;
        };
        painter.image(
            texture.id(),
            map_rect,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        let _ = (viewport, tile_size);
    }

    #[cfg(feature = "game-api")]
    pub(super) fn map_asset_image(
        &mut self,
        category: &str,
        name: &str,
    ) -> Option<std::sync::Arc<image::RgbaImage>> {
        if name.is_empty() || std::path::Path::new(name).file_name()?.to_str()? != name {
            return None;
        }
        let key = format!("{category}/{name}");
        if let Some(image) = self.world_map_asset_images.get(&key) {
            return Some(image.clone());
        }
        let rgba = self.backend.asset_image(category, name)?;
        self.world_map_asset_images.insert(key, rgba.clone());
        Some(rgba)
    }

    #[cfg(feature = "game-api")]
    pub(super) fn map_asset_image_scaled(
        &mut self,
        category: &str,
        name: &str,
        tile_pixels: u32,
    ) -> Option<std::sync::Arc<image::RgbaImage>> {
        if tile_pixels == 0
            || name.is_empty()
            || std::path::Path::new(name).file_name()?.to_str()? != name
        {
            return None;
        }
        let key = format!("{category}/{name}@{tile_pixels}");
        if let Some(image) = self.world_map_scaled_images.get(&key) {
            return Some(image.clone());
        }
        let source = self.map_asset_image(category, name)?;
        let width = ((source.width() as u64 * tile_pixels as u64) / 32).max(1) as u32;
        let height = ((source.height() as u64 * tile_pixels as u64) / 32).max(1) as u32;
        let scaled = if tile_pixels == 32 {
            source
        } else {
            std::sync::Arc::new(image::imageops::resize(
                source.as_ref(),
                width,
                height,
                image::imageops::FilterType::Nearest,
            ))
        };
        self.world_map_scaled_images.insert(key, scaled.clone());
        Some(scaled)
    }

    #[cfg(feature = "game-api")]
    pub(super) fn compose_map_texture(
        &mut self,
        context: &egui::Context,
        map_id: i64,
        preview: &serde_json::Value,
        tile_pixels: u32,
        detail: bool,
    ) -> Option<egui::TextureHandle> {
        let width = preview["width"].as_u64()? as u32;
        let height = preview["height"].as_u64()? as u32;
        let max_texture_side = context
            .input(|input| input.max_texture_side)
            .min(u32::MAX as usize) as u32;
        if width == 0
            || height == 0
            || width > max_texture_side
            || height > max_texture_side
            || tile_pixels == 0
        {
            return None;
        }
        // Keep both the overview and the full-resolution selected-map preview
        // under the current egui/wgpu per-side texture limit, including unusually large maps.
        let mut tile_pixels = tile_pixels
            .min((max_texture_side / width).min(max_texture_side / height))
            .max(1);
        if tile_pixels > 1 && tile_pixels % 2 != 0 {
            tile_pixels -= 1;
        }
        let mut canvas = image::RgbaImage::new(width * tile_pixels, height * tile_pixels);
        let tileset = self.map_asset_image_scaled(
            "Tilesets",
            preview["tileset"].as_str().unwrap_or(""),
            tile_pixels,
        );
        let autotiles: Vec<Option<std::sync::Arc<image::RgbaImage>>> = preview["autotiles"]
            .as_array()?
            .iter()
            .map(|name| {
                self.map_asset_image_scaled("Autotiles", name.as_str().unwrap_or(""), tile_pixels)
            })
            .collect();
        let layers = preview["layers"].as_array()?;
        let parts = preview["autotile_parts"].as_array()?;
        for layer in layers {
            let Some(rows) = layer.as_array() else {
                continue;
            };
            for (y, row) in rows.iter().enumerate() {
                let Some(cells) = row.as_array() else {
                    continue;
                };
                for (x, value) in cells.iter().enumerate() {
                    let id = value.as_u64().unwrap_or(0) as usize;
                    if id < 48 || x >= width as usize || y >= height as usize {
                        continue;
                    }
                    if id >= 384 {
                        if let Some(bitmap) = &tileset {
                            let sx = ((id - 384) % 8) as u32 * tile_pixels;
                            let sy = ((id - 384) / 8) as u32 * tile_pixels;
                            if sx + tile_pixels <= bitmap.width()
                                && sy + tile_pixels <= bitmap.height()
                            {
                                overlay_map_region(
                                    &mut canvas,
                                    bitmap,
                                    sx,
                                    sy,
                                    tile_pixels,
                                    tile_pixels,
                                    x as u32 * tile_pixels,
                                    y as u32 * tile_pixels,
                                );
                            }
                        }
                    } else {
                        let autotile_index = id / 48 - 1;
                        let Some(Some(bitmap)) = autotiles.get(autotile_index) else {
                            continue;
                        };
                        if tile_pixels < 2 {
                            continue;
                        }
                        let Some(quadrants) =
                            parts.get(id % 48).and_then(serde_json::Value::as_array)
                        else {
                            continue;
                        };
                        let half = tile_pixels / 2;
                        for (quadrant, part) in quadrants.iter().enumerate() {
                            let part = part.as_u64().unwrap_or(1).saturating_sub(1) as u32;
                            let sx = part % 6 * half;
                            let sy = part / 6 * half;
                            if sx + half > bitmap.width() || sy + half > bitmap.height() {
                                continue;
                            }
                            let dx = (x as u32 * tile_pixels + quadrant as u32 % 2 * half) as i64;
                            let dy = (y as u32 * tile_pixels + quadrant as u32 / 2 * half) as i64;
                            overlay_map_region(
                                &mut canvas,
                                bitmap,
                                sx,
                                sy,
                                half,
                                half,
                                dx as u32,
                                dy as u32,
                            );
                        }
                    }
                }
            }
        }
        let size = [canvas.width() as usize, canvas.height() as usize];
        let color = egui::ColorImage::from_rgba_unmultiplied(size, canvas.as_raw());
        let texture = context.load_texture(
            format!("world-map-{map_id}"),
            color,
            egui::TextureOptions::NEAREST,
        );
        if detail {
            self.world_map_detail_textures
                .insert(map_id, (tile_pixels, texture.clone()));
        } else {
            self.world_map_map_textures
                .insert(map_id, (tile_pixels, texture.clone()));
        }
        Some(texture)
    }
}
