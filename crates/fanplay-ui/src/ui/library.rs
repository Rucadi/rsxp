// SPDX-License-Identifier: GPL-2.0-or-later
//! Responsive, keyboard-accessible launcher. No filesystem work occurs while drawing.
//!
//! The library is the product's front door, so it gets the full ArcadeNeon
//! treatment: platform-filtered chips, illustrated cover cards with neon
//! selection glows, a featured "ready to play" banner, and a compact import
//! shelf. Everything here is presentation only; the returned
//! [`SessionActions`] are executed by the platform adapter.
use super::session::{RubyJit, SessionActions, SessionPanel};
use super::theme::{self, ButtonKind, ButtonSize, Tone};

fn title(path: &str) -> &str {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Platform {
    RpgMaker,
    Gba,
    Nds,
}

impl Platform {
    fn of(path: &str) -> Self {
        let extension = path
            .rsplit_once('.')
            .map(|(_, extension)| extension.to_ascii_lowercase());
        match extension.as_deref() {
            Some("gba") => Self::Gba,
            Some("nds") => Self::Nds,
            _ => Self::RpgMaker,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::RpgMaker => "RPG Maker",
            Self::Gba => fanplay_gba::ui::LABEL,
            Self::Nds => fanplay_nds::ui::LABEL,
        }
    }

    fn tone(self) -> Tone {
        match self {
            Self::RpgMaker => Tone::Lime,
            Self::Gba => Tone::Cyan,
            Self::Nds => Tone::Pink,
        }
    }

    fn accent(self) -> egui::Color32 {
        self.tone().color()
    }

    fn cover(self) -> (egui::Color32, egui::Color32) {
        match self {
            Self::RpgMaker => (
                egui::Color32::from_rgb(12, 40, 30),
                egui::Color32::from_rgb(32, 92, 66),
            ),
            Self::Gba => fanplay_gba::ui::cover_colors(),
            Self::Nds => fanplay_nds::ui::cover_colors(),
        }
    }

    fn matches(self, filter: &str) -> bool {
        filter == "All" || filter == self.label()
    }
}

fn format_count(count: usize) -> String {
    match count {
        1 => "1 game".into(),
        _ => format!("{count} games"),
    }
}

impl SessionPanel {
    pub(super) fn draw_library(&mut self, ui: &mut egui::Ui, actions: &mut SessionActions) {
        self.draw_library_heading(ui, actions);
        ui.add_space(theme::MD);
        self.draw_library_filters(ui);
        ui.add_space(theme::LG);

        let query = self.library_query.trim().to_lowercase();
        let mut games: Vec<String> = self
            .recent
            .iter()
            .filter(|path| Platform::of(path).matches(&self.library_platform))
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

        let (columns, tile_width) = theme::grid_cell(
            ui,
            if self.library_compact { 132.0 } else { 168.0 },
            230.0,
            16.0,
        );
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
            self.draw_featured(ui, &path, actions);
            ui.add_space(theme::LG);
        }

        if games.is_empty() {
            if self.recent.is_empty() {
                theme::empty_state(
                    ui,
                    "✦",
                    "Your next adventure starts here",
                    "Add a game folder, ZIP, GBA, or NDS ROM below. Your library stays on this device.",
                );
            } else {
                theme::empty_state(
                    ui,
                    "⌕",
                    "No matching games",
                    "Try a different title, clear the search, or switch platform.",
                );
            }
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(16.0, 20.0);
                for path in &games {
                    ui.allocate_ui_with_layout(
                        egui::vec2(tile_width, tile_width * 1.4 + 44.0),
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
                                egui::Label::new(
                                    egui::RichText::new(title(path))
                                        .font(theme::body_font(14.0))
                                        .color(if selected {
                                            Platform::of(path).accent()
                                        } else {
                                            theme::TEXT
                                        }),
                                )
                                .truncate(),
                            );
                            ui.horizontal(|ui| {
                                theme::tag(
                                    ui,
                                    Platform::of(path).label(),
                                    Platform::of(path).tone(),
                                );
                                ui.label(
                                    egui::RichText::new(if selected {
                                        "SELECTED"
                                    } else {
                                        "READY"
                                    })
                                    .font(theme::mono_font(9.0))
                                    .color(if selected {
                                        Platform::of(path).accent()
                                    } else {
                                        theme::TEXT_FAINT
                                    }),
                                );
                            });
                        },
                    );
                }
            });
            ui.add_space(theme::MD);
            theme::shortcut_hint(
                ui,
                &[
                    ("← → ↑ ↓", "Navigate"),
                    ("Enter", "Play"),
                    ("Double-click", "Launch"),
                ],
            );
        }

        ui.add_space(theme::XL);
        self.draw_import_shelf(ui, actions);
        ui.add_space(theme::XXL);
    }

    fn draw_library_heading(&mut self, ui: &mut egui::Ui, actions: &mut SessionActions) {
        ui.horizontal_wrapped(|ui| {
            ui.label(theme::display("Library", 32.0));
            ui.add_space(theme::SM);
            theme::tag(ui, &format_count(self.recent.len()), Tone::Neutral);
            ui.add_space(theme::SM);
            if theme::button(ui, "+ Add game", ButtonKind::Primary).clicked() {
                actions.browse = true;
            }
        });
    }

    fn draw_library_filters(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new()
            .fill(theme::SURFACE)
            .stroke(egui::Stroke::new(1.0, theme::BORDER))
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(egui::Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.library_query)
                            .hint_text("Search your games…")
                            .desired_width((ui.available_width() * 0.42).clamp(140.0, 360.0)),
                    );
                    if !self.library_query.is_empty()
                        && theme::button_sized(ui, "Clear", ButtonKind::Ghost, ButtonSize::Sm)
                            .clicked()
                    {
                        self.library_query.clear();
                    }
                    ui.separator();
                    for platform in ["All", "RPG Maker", "GBA", "NDS"] {
                        let tone = match platform {
                            "GBA" => Tone::Cyan,
                            "NDS" => Tone::Pink,
                            "RPG Maker" => Tone::Lime,
                            _ => Tone::Pink,
                        };
                        if theme::chip(ui, platform, tone, self.library_platform == platform)
                            .clicked()
                        {
                            self.library_platform = platform.into();
                        }
                    }
                    ui.separator();
                    egui::ComboBox::from_id_salt("library-sort")
                        .selected_text(if self.library_sort_title {
                            "Title A–Z"
                        } else {
                            "Recently opened"
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.library_sort_title,
                                false,
                                "Recently opened",
                            );
                            ui.selectable_value(&mut self.library_sort_title, true, "Title A–Z");
                        });
                    let compact = if self.library_compact {
                        "Compact · on"
                    } else {
                        "Compact"
                    };
                    if theme::chip(ui, compact, Tone::Cyan, self.library_compact).clicked() {
                        self.library_compact = !self.library_compact;
                    }
                });
            });
    }

    fn draw_featured(&mut self, ui: &mut egui::Ui, path: &str, actions: &mut SessionActions) {
        let platform = Platform::of(path);
        let accent = platform.accent();
        egui::Frame::new()
            .fill(theme::SURFACE_RAISED)
            .stroke(egui::Stroke::new(1.0, accent.gamma_multiply(0.55)))
            .corner_radius(egui::CornerRadius::same(12))
            .inner_margin(egui::Margin::same(22))
            .shadow(egui::epaint::Shadow {
                offset: [0, 0],
                blur: 22,
                spread: 0,
                color: accent.gamma_multiply(0.16),
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("READY TO PLAY")
                                .font(theme::mono_font(10.0))
                                .color(accent),
                        );
                        ui.add_space(2.0);
                        ui.label(theme::display(title(path), 28.0));
                        ui.add_space(2.0);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(path)
                                    .font(theme::mono_font(12.0))
                                    .color(theme::TEXT_MUTED),
                            )
                            .truncate(),
                        )
                        .on_hover_text(path);
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            theme::tag(ui, platform.label(), platform.tone());
                            theme::tag(ui, "Local", Tone::Neutral);
                            if self.active {
                                theme::tag(ui, "Replaces current session", Tone::Warning);
                            }
                        });
                    });
                    if theme::button_sized(ui, "Play game", ButtonKind::Primary, ButtonSize::Lg)
                        .clicked()
                    {
                        actions.open = Some(path.to_owned());
                        self.open = false;
                    }
                });
            });
        if self.active {
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new("Starting a game replaces your current session. Save first.")
                    .font(theme::body_font(12.0))
                    .color(theme::WARNING),
            );
        }
    }

    fn draw_import_shelf(&mut self, ui: &mut egui::Ui, actions: &mut SessionActions) {
        theme::section_label(ui, "Add to your library", theme::CYAN);
        ui.add_space(theme::SM);
        egui::Frame::new()
            .fill(theme::SURFACE)
            .stroke(egui::Stroke::new(1.0, theme::BORDER))
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(egui::Margin::same(18))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if theme::button(ui, "+ Add game folder", ButtonKind::Secondary).clicked() {
                        actions.browse = true;
                    }
                    if self.capabilities.zip_import
                        && theme::button(ui, "Open ZIP archive", ButtonKind::Ghost).clicked()
                    {
                        actions.import_zip = true;
                    }
                    if self.capabilities.rom_import
                        && theme::button(ui, "Open GBA / NDS ROM", ButtonKind::Ghost).clicked()
                    {
                        actions.browse_game_file = true;
                    }
                });
                if self.capabilities.zip_import {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(
                            "Choose a folder containing Game.ini and Data, or open a ZIP archive.",
                        )
                        .font(theme::body_font(12.0))
                        .color(theme::TEXT_MUTED),
                    );
                    ui.label(
                        egui::RichText::new("Folders and archives are read locally on this device.")
                            .font(theme::body_font(11.0))
                            .color(theme::TEXT_FAINT),
                    );
                }
                if self.capabilities.rom_import {
                    ui.label(
                        egui::RichText::new("GBA and NDS ROMs are read locally on this device.")
                            .font(theme::body_font(11.0))
                            .color(theme::TEXT_FAINT),
                    );
                }
                if self.capabilities.torrent_import {
                    ui.add_space(theme::SM);
                    theme::divider(ui);
                    theme::section_label(ui, "WebTorrent", theme::PINK);
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.source)
                                .desired_width((ui.available_width() - 170.0).max(140.0))
                                .hint_text("magnet:?xt=urn:btih:…"),
                        );
                        if theme::button_sized(
                            ui,
                            "Download game",
                            ButtonKind::Primary,
                            ButtonSize::Sm,
                        )
                        .clicked()
                        {
                            actions.download = Some(self.source.trim().into());
                        }
                    });
                    ui.label(
                        egui::RichText::new(
                            "Torrents need WebRTC-compatible seeders. Your browser may share pieces while downloading.",
                        )
                        .font(theme::body_font(11.0))
                        .color(theme::TEXT_FAINT),
                    );
                }
                if self.capabilities.paths {
                    ui.add_space(theme::SM);
                    egui::CollapsingHeader::new("Open from a path")
                        .id_salt("library-open-path")
                        .default_open(self.recent.is_empty())
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.source)
                                        .desired_width((ui.available_width() - 170.0).max(140.0))
                                        .hint_text("Paste a game folder, Game.ini, or .gba/.nds path…"),
                                );
                                if theme::button_sized(
                                    ui,
                                    "Open",
                                    ButtonKind::Secondary,
                                    ButtonSize::Sm,
                                )
                                .clicked()
                                {
                                    actions.open = Some(self.source.trim().into());
                                    self.open = false;
                                }
                                if theme::button_sized(
                                    ui,
                                    "Browse…",
                                    ButtonKind::Ghost,
                                    ButtonSize::Sm,
                                )
                                .clicked()
                                {
                                    actions.browse_game_file = true;
                                }
                            });
                        });
                }
                if self.capabilities.ruby_jit {
                    egui::CollapsingHeader::new("Runtime options")
                        .id_salt("library-runtime-options")
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(theme::eyebrow("Ruby JIT"));
                                egui::ComboBox::from_id_salt("ruby-jit")
                                    .selected_text(self.ruby_jit.label())
                                    .show_ui(ui, |ui| {
                                        for mode in
                                            [RubyJit::Yjit, RubyJit::Zjit, RubyJit::Disabled]
                                        {
                                            ui.selectable_value(
                                                &mut self.ruby_jit,
                                                mode,
                                                mode.label(),
                                            );
                                        }
                                    });
                                ui.label(
                                    egui::RichText::new("Applies when starting or restarting a game.")
                                        .font(theme::body_font(11.0))
                                        .color(theme::TEXT_FAINT),
                                );
                            });
                        });
                }
            });
    }
}

/// Stable, typographic placeholder art until a platform supplies real game covers.
fn cover(ui: &mut egui::Ui, path: &str, width: f32, selected: bool) -> egui::Response {
    let height = width * 1.4;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    let platform = Platform::of(path);
    let accent = platform.accent();
    let (base, ridge) = platform.cover();
    let hovered = response.hovered() || response.has_focus();
    let painter = ui.painter_at(rect);

    if selected || hovered {
        theme::glow(
            &painter,
            rect,
            12,
            accent,
            if selected { 1.4 } else { 0.65 },
        );
    }
    painter.rect_filled(rect, 12, base);
    painter.rect_stroke(
        rect,
        egui::CornerRadius::same(12),
        egui::Stroke::new(
            if selected {
                3.0
            } else if hovered {
                1.5
            } else {
                1.0
            },
            if selected || hovered {
                accent
            } else {
                theme::BORDER
            },
        ),
        egui::StrokeKind::Inside,
    );

    // Retro horizon rings, tinted by platform.
    let center = rect.center_top() + egui::vec2(width * 0.18, height * 0.40);
    for index in (1..=4).rev() {
        painter.circle_stroke(
            center,
            width * (0.14 + index as f32 * 0.10),
            egui::Stroke::new(1.0, ridge.gamma_multiply(0.9)),
        );
    }
    painter.circle_filled(center, width * 0.075, accent.gamma_multiply(0.35));
    painter.circle_filled(center, width * 0.025, accent);

    // Scanlines.
    let mut y = rect.top() + 8.0;
    while y < rect.bottom() {
        painter.line_segment(
            [
                egui::pos2(rect.left() + 1.0, y),
                egui::pos2(rect.right() - 1.0, y),
            ],
            egui::Stroke::new(1.0, egui::Color32::from_white_alpha(6)),
        );
        y += 4.0;
    }

    painter.text(
        rect.left_top() + egui::vec2(14.0, 14.0),
        egui::Align2::LEFT_TOP,
        platform.label().to_uppercase(),
        theme::mono_font(10.0),
        accent,
    );
    painter.text(
        rect.right_top() + egui::vec2(-14.0, 14.0),
        egui::Align2::RIGHT_TOP,
        if selected { "SELECTED" } else { "FANPLAY" },
        theme::mono_font(9.0),
        if selected { accent } else { theme::TEXT_FAINT },
    );

    let galley = painter.layout(
        title(path).to_owned(),
        theme::display_font((width * 0.11).clamp(15.0, 24.0)),
        theme::TEXT,
        width - 28.0,
    );
    painter.galley(
        egui::pos2(
            rect.left() + 14.0,
            (rect.bottom() - galley.size().y - 18.0).max(rect.top() + 44.0),
        ),
        galley,
        theme::TEXT,
    );
    if selected {
        let bar = egui::Rect::from_min_size(
            rect.left_bottom() + egui::vec2(14.0, -6.0),
            egui::vec2(width - 28.0, 2.0),
        );
        painter.rect_filled(bar, egui::CornerRadius::same(1), accent);
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
                            theme::apply(ui.ctx());
                            panel.draw_library(ui, &mut actions);
                            assert!(
                                ui.min_rect().width() <= width + 0.5,
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
    fn featured_play_button_returns_the_selected_game() {
        for width in [360.0, 720.0, 1100.0] {
            let context = egui::Context::default();
            let mut panel = SessionPanel::default();
            panel.recent = vec!["/games/Alpha".into()];
            let mut actions = SessionActions::default();
            let mut frame = |events: Vec<egui::Event>| {
                let mut output = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 900.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| actions = panel.draw(ui),
                );
                output.textures_delta.clear();
                output
            };
            frame(vec![]);
            let output = frame(vec![]);
            let mut position = None;
            fn find(shape: &egui::epaint::Shape, position: &mut Option<egui::Pos2>) {
                match shape {
                    egui::epaint::Shape::Text(text) if text.galley.text() == "Play game" => {
                        *position = Some(text.pos + text.galley.size() * 0.5);
                    }
                    egui::epaint::Shape::Vec(shapes) => {
                        for shape in shapes {
                            find(shape, position);
                        }
                    }
                    _ => {}
                }
            }
            for shape in &output.shapes {
                find(&shape.shape, &mut position);
            }
            let position = position.unwrap_or_else(|| panic!("featured play button at {width}"));
            frame(vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
            frame(vec![egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }]);
            assert_eq!(
                actions.open.as_deref(),
                Some("/games/Alpha"),
                "play button at {width}"
            );
        }
    }

    #[test]
    fn game_titles_support_both_platform_path_separators() {
        assert_eq!(title("/games/Alpha/"), "Alpha");
        assert_eq!(title("C:\\Games\\Beta\\"), "Beta");
    }

    #[test]
    fn platform_filter_limits_visible_games() {
        let context = egui::Context::default();
        let mut panel = SessionPanel::default();
        panel.recent = vec![
            "/games/Alpha".into(),
            "/roms/Beta.gba".into(),
            "/roms/Gamma.nds".into(),
        ];
        panel.library_platform = "GBA".into();
        let mut actions = SessionActions::default();
        context
            .run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900.0, 900.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    theme::apply(ui.ctx());
                    panel.draw_library(ui, &mut actions);
                },
            )
            .drop_without_applying_deltas();
        assert_eq!(panel.library_selected.as_deref(), Some("/roms/Beta.gba"));
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
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1000.0, 900.0),
                        )),
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
