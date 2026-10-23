// SPDX-License-Identifier: GPL-2.0-or-later
//! Responsive, keyboard-accessible launcher. No filesystem work occurs while drawing.
use super::session::{RubyJit, SessionActions, SessionPanel};

const MINT: egui::Color32 = egui::Color32::from_rgb(127, 231, 200);
const MUTED: egui::Color32 = egui::Color32::from_rgb(151, 174, 183);

fn title(path: &str) -> &str {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
}

impl SessionPanel {
    pub(super) fn draw_library(&mut self, ui: &mut egui::Ui, actions: &mut SessionActions) {
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new("Library").size(30.0).strong());
            ui.label(egui::RichText::new(format!("{} games", self.recent.len())).color(MUTED));
            if ui
                .add_enabled(
                    !self.busy,
                    egui::Button::new("+ Add game").min_size(egui::vec2(110.0, 36.0)),
                )
                .clicked()
            {
                actions.browse = true;
            }
            if self.busy {
                ui.spinner();
                ui.label("Opening game…");
            }
        });
        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.library_query)
                    .hint_text("Search your games…")
                    .desired_width((ui.available_width() * 0.45).clamp(100.0, 340.0)),
            );
            if !self.library_query.is_empty() && ui.button("Clear").clicked() {
                self.library_query.clear();
            }
            egui::ComboBox::from_id_salt("library-sort")
                .selected_text(if self.library_sort_title {
                    "Title A–Z"
                } else {
                    "Recently opened"
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.library_sort_title, false, "Recently opened");
                    ui.selectable_value(&mut self.library_sort_title, true, "Title A–Z");
                });
            ui.toggle_value(&mut self.library_compact, "Compact tiles");
        });
        ui.add_space(16.0);

        let query = self.library_query.trim().to_lowercase();
        let mut games: Vec<String> = self
            .recent
            .iter()
            .filter(|path| title(path).to_lowercase().contains(&query))
            .cloned()
            .collect();
        if self.library_sort_title {
            games.sort_by_cached_key(|path| title(path).to_lowercase());
        }
        if !games
            .iter()
            .any(|p| Some(p) == self.library_selected.as_ref())
        {
            self.library_selected = games.first().cloned();
        }

        let gap = 16.0;
        let preferred = if self.library_compact { 132.0 } else { 180.0 };
        let columns = ((ui.available_width() + gap) / (preferred + gap))
            .floor()
            .max(1.0) as usize;
        let tile_width =
            ((ui.available_width() - gap * (columns - 1) as f32) / columns as f32).min(240.0);
        // Do not steal arrows or Enter from search, menus, or another focused widget.
        if !ui.ctx().egui_wants_keyboard_input() && !games.is_empty() {
            let mut index = games
                .iter()
                .position(|p| Some(p) == self.library_selected.as_ref())
                .unwrap_or(0);
            let previous = index;
            ui.input(|input| {
                if input.key_pressed(egui::Key::ArrowRight) {
                    index = (index + 1).min(games.len() - 1);
                }
                if input.key_pressed(egui::Key::ArrowLeft) {
                    index = index.saturating_sub(1);
                }
                if input.key_pressed(egui::Key::ArrowDown) {
                    index = (index + columns).min(games.len() - 1);
                }
                if input.key_pressed(egui::Key::ArrowUp) {
                    index = index.saturating_sub(columns);
                }
            });
            self.library_selected = Some(games[index].clone());
            if previous != index {
                ui.ctx().data_mut(|d| {
                    d.insert_temp(egui::Id::unique("library-scroll-selection"), true)
                });
            }
            if !self.busy && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                actions.open = self.library_selected.clone();
                self.open = false;
            }
        }

        if let Some(path) = self.library_selected.clone() {
            egui::Frame::new()
                .fill(egui::Color32::from_rgb(24, 45, 50))
                .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(48, 78, 80)))
                .corner_radius(18)
                .inner_margin(20.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        egui::RichText::new("READY TO PLAY")
                            .size(10.0)
                            .strong()
                            .color(MINT),
                    );
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(title(&path)).size(26.0).strong());
                    ui.add(
                        egui::Label::new(egui::RichText::new(&path).small().color(MUTED))
                            .truncate(),
                    )
                    .on_hover_text(&path);
                    ui.add_space(12.0);
                    if ui
                        .add_enabled(
                            !self.busy,
                            egui::Button::new(
                                egui::RichText::new("Play game")
                                    .strong()
                                    .color(egui::Color32::from_rgb(13, 36, 31)),
                            )
                            .fill(MINT)
                            .min_size(egui::vec2(150.0, 42.0)),
                        )
                        .clicked()
                    {
                        actions.open = Some(path);
                        self.open = false;
                    }
                    if self.active {
                        ui.small("Starting a game replaces your current session. Save first.");
                    }
                });
            ui.add_space(20.0);
        }

        if games.is_empty() {
            ui.add_space(24.0);
            ui.label(
                egui::RichText::new(if self.recent.is_empty() {
                    "Your next adventure starts here"
                } else {
                    "No matching games"
                })
                .size(22.0)
                .strong(),
            );
            ui.label(if self.recent.is_empty() {
                "Add a game folder or drop one into this window. Your games will appear here."
            } else {
                "Try a different title or clear your search."
            });
            ui.add_space(24.0);
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(gap, 20.0);
                for path in &games {
                    ui.allocate_ui_with_layout(
                        egui::vec2(tile_width, tile_width * 1.4 + 32.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            let selected = self.library_selected.as_ref() == Some(path);
                            let response = cover(ui, path, tile_width, selected);
                            if response.clicked() {
                                self.library_selected = Some(path.clone());
                            }
                            if response.double_clicked() && !self.busy {
                                actions.open = Some(path.clone());
                                self.open = false;
                            }
                            if selected
                                && ui
                                    .ctx()
                                    .data_mut(|d| {
                                        d.remove_temp::<bool>(egui::Id::unique(
                                            "library-scroll-selection",
                                        ))
                                    })
                                    .unwrap_or(false)
                            {
                                response.scroll_to_me(Some(egui::Align::Center));
                            }
                            response.on_hover_text(format!(
                                "{}\n{}\nSelect to view · Double-click to play",
                                title(path),
                                path
                            ));
                            ui.add(
                                egui::Label::new(egui::RichText::new(title(path)).strong().color(
                                    if selected {
                                        MINT
                                    } else {
                                        egui::Color32::from_rgb(224, 233, 237)
                                    },
                                ))
                                .truncate(),
                            );
                        },
                    );
                }
            });
            ui.add_space(16.0);
            ui.label(
                egui::RichText::new(
                    "Arrow keys  Navigate     Enter  Play     Double-click  Launch",
                )
                .small()
                .color(MUTED),
            );
        }
        ui.add_space(16.0);
        ui.separator();
        egui::CollapsingHeader::new("Open from path, Game.ini, or GBA ROM")
            .default_open(self.recent.is_empty())
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.source)
                            .desired_width((ui.available_width() - 170.0).max(100.0))
                            .hint_text("Paste a game folder, Game.ini, or .gba path…"),
                    );
                    if ui
                        .add_enabled(
                            !self.busy && !self.source.trim().is_empty(),
                            egui::Button::new("Open"),
                        )
                        .clicked()
                    {
                        actions.open = Some(self.source.trim().into());
                        self.open = false;
                    }
                    if ui
                        .add_enabled(!self.busy, egui::Button::new("Browse game file…"))
                        .clicked()
                    {
                        actions.browse_game_file = true;
                    }
                });
            });
        egui::CollapsingHeader::new("Runtime options").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Ruby JIT");
                egui::ComboBox::from_id_salt("ruby-jit")
                    .selected_text(self.ruby_jit.label())
                    .show_ui(ui, |ui| {
                        for mode in [RubyJit::Yjit, RubyJit::Zjit, RubyJit::Disabled] {
                            ui.selectable_value(&mut self.ruby_jit, mode, mode.label());
                        }
                    });
                ui.weak("Applies when starting or restarting a game.");
            });
        });
    }
}

/// Stable, typographic placeholder art until a platform supplies real game covers.
fn cover(ui: &mut egui::Ui, path: &str, width: f32, selected: bool) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, width * 1.4), egui::Sense::click());
    let hash = path
        .bytes()
        .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    let palettes = [
        (38, 77, 88),
        (66, 56, 100),
        (90, 61, 61),
        (37, 83, 73),
        (69, 75, 111),
    ];
    let (r, g, b) = palettes[hash as usize % palettes.len()];
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 12, egui::Color32::from_rgb(r, g, b));
    let center = rect.center_top() + egui::vec2(width * 0.2, width * 0.45);
    for i in (1..=4).rev() {
        painter.circle_stroke(
            center,
            width * (0.12 + i as f32 * 0.10),
            egui::Stroke::new(1.0, egui::Color32::from_white_alpha(18 + i * 5)),
        );
    }
    painter.circle_filled(center, width * 0.14, egui::Color32::from_white_alpha(25));
    painter.text(
        rect.left_top() + egui::vec2(14.0, 14.0),
        egui::Align2::LEFT_TOP,
        "RSXP  /  GAME",
        egui::FontId::proportional(10.0),
        egui::Color32::from_white_alpha(170),
    );
    let galley = painter.layout(
        title(path).to_owned(),
        egui::FontId::proportional((width * 0.12).clamp(15.0, 26.0)),
        egui::Color32::WHITE,
        width - 28.0,
    );
    painter.galley(
        egui::pos2(
            rect.left() + 14.0,
            (rect.bottom() - galley.size().y - 24.0).max(rect.top() + 45.0),
        ),
        galley,
        egui::Color32::WHITE,
    );
    if selected || response.hovered() || response.has_focus() {
        painter.rect_stroke(
            rect.shrink(1.5),
            12,
            egui::Stroke::new(if selected { 3.0 } else { 1.5 }, MINT),
            egui::StrokeKind::Inside,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Button, true, selected, title(path))
    });
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_handles_empty_filtered_and_narrow_layouts() {
        for width in [320.0, 860.0, 1920.0] {
            for query in ["", "missing", "alpha"] {
                let context = egui::Context::default();
                let mut panel = SessionPanel::default();
                panel.recent = vec!["/games/Alpha".into(), "/games/Beta".into()];
                panel.library_query = query.into();
                let mut actions = SessionActions::default();
                context
                    .run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(width, 900.0),
                            )),
                            ..Default::default()
                        },
                        |ui| {
                            super::super::theme::apply(ui.ctx());
                            panel.draw_library(ui, &mut actions);
                            assert!(
                                ui.min_rect().width() <= width,
                                "library overflow at {width}"
                            );
                        },
                    )
                    .drop_without_applying_deltas();
                assert_eq!(
                    panel.library_selected.as_deref(),
                    if query == "missing" {
                        None
                    } else {
                        Some("/games/Alpha")
                    }
                );
                assert!(actions.open.is_none());
            }
        }
    }

    #[test]
    fn game_titles_support_both_platform_path_separators() {
        assert_eq!(title("/games/Alpha/"), "Alpha");
        assert_eq!(title("C:\\Games\\Beta\\"), "Beta");
    }

    #[test]
    fn keyboard_launch_respects_selection_and_busy_state() {
        for busy in [false, true] {
            let context = egui::Context::default();
            let mut panel = SessionPanel::default();
            panel.recent = vec!["/games/Alpha".into(), "/games/Beta".into()];
            panel.busy = busy;
            let mut actions = SessionActions::default();
            context
                .run_ui(
                    egui::RawInput {
                        events: [egui::Key::ArrowRight, egui::Key::Enter]
                            .into_iter()
                            .map(|key| egui::Event::Key {
                                key,
                                physical_key: None,
                                pressed: true,
                                repeat: false,
                                modifiers: egui::Modifiers::NONE,
                            })
                            .collect(),
                        ..Default::default()
                    },
                    |ui| panel.draw_library(ui, &mut actions),
                )
                .drop_without_applying_deltas();
            assert_eq!(panel.library_selected.as_deref(), Some("/games/Beta"));
            assert_eq!(
                actions.open.as_deref(),
                if busy { None } else { Some("/games/Beta") }
            );
        }
    }
}
