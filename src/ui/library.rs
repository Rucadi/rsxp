// SPDX-License-Identifier: GPL-2.0-or-later
//! Visual, responsive game shelf for the desktop workspace.
use super::session::{SessionActions, SessionPanel};

const MINT: egui::Color32 = egui::Color32::from_rgb(224, 186, 255);

impl SessionPanel {
    pub(super) fn draw_library(&mut self, ui: &mut egui::Ui, actions: &mut SessionActions) {
        let width = ui.available_width();
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            paint_sparkle(ui, egui::vec2(10.0, 10.0), MINT, 4.0);
            ui.label(egui::RichText::new("YOUR LITTLE GAME CORNER").size(11.0).strong().color(MINT));
        });
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Where shall we wander?").size(28.0).strong());
        ui.label(egui::RichText::new("A cozy shelf for all your next adventures.").color(egui::Color32::from_rgb(194, 183, 205)));
        ui.add_space(18.0);

        egui::Frame::new()
            .fill(egui::Color32::from_rgb(45, 36, 57))
            .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(91, 70, 112)))
            .corner_radius(egui::CornerRadius::same(20))
            .inner_margin(egui::Margin::same(18))
            .show(ui, |ui| {
                ui.set_width((width - 40.0).max(240.0));
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("A NEW FRIEND FOR THE SHELF").size(11.0).strong().color(MINT));
                        ui.add_space(3.0);
                        ui.label(egui::RichText::new("Bring a new world along").size(18.0).strong());
                        ui.label(egui::RichText::new("Pick its folder, or drop it right here.").small().color(egui::Color32::from_rgb(196, 184, 208)));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add_enabled(!self.busy, egui::Button::new("Game folder").min_size(egui::vec2(128.0, 40.0))).clicked() { actions.browse = true; }
                        if ui.add_enabled(!self.busy, egui::Button::new("Game.ini...")).clicked() { actions.browse_game_file = true; }
                    });
                });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let field_width = (ui.available_width() - 82.0).max(100.0);
                    ui.add(egui::TextEdit::singleline(&mut self.source).desired_width(field_width).hint_text("Or paste a game folder path..."));
                    if ui.add_enabled(!self.busy && !self.source.trim().is_empty(), egui::Button::new("Open").min_size(egui::vec2(68.0, 32.0))).clicked() {
                        actions.open = Some(self.source.trim().into());
                        self.open = false;
                    }
                });
                if self.active { ui.add_space(8.0); ui.weak("Starting another game replaces the current session. Save first."); }
            });

        ui.add_space(24.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(if self.recent.is_empty() { "Your shelf" } else { "Back to your stories" }).size(21.0).strong());
            ui.add_space(8.0);
            ui.label(egui::RichText::new(format!("{} GAMES", self.recent.len())).small().strong().color(egui::Color32::from_gray(150)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if !self.recent.is_empty() && ui.add_enabled(!self.busy, egui::Button::new("Add game")).clicked() { actions.browse = true; }
            });
        });
        ui.add_space(12.0);

        if self.recent.is_empty() {
            egui::Frame::new().fill(egui::Color32::from_rgb(36, 30, 46)).corner_radius(egui::CornerRadius::same(18)).inner_margin(20.0).show(ui, |ui| {
                ui.set_width((width - 44.0).max(220.0));
                ui.label(egui::RichText::new("A little space for big adventures").size(17.0).strong());
                ui.add_space(4.0);
                ui.weak("Games you open will appear here, ready for next time.");
            });
            return;
        }

        let tile_width = 224.0_f32.min((width - 24.0).max(180.0));
        ui.horizontal_wrapped(|ui| {
            for (index, path) in self.recent.iter().enumerate() {
                let title = path.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or(path);
                ui.allocate_ui_with_layout(egui::vec2(tile_width, 190.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    let (cover, _) = ui.allocate_exact_size(egui::vec2(tile_width, 112.0), egui::Sense::hover());
                    let palettes = [
                        (egui::Color32::from_rgb(112, 91, 145), egui::Color32::from_rgb(210, 157, 180), egui::Color32::from_rgb(86, 108, 119)),
                        (egui::Color32::from_rgb(110, 157, 174), egui::Color32::from_rgb(231, 188, 138), egui::Color32::from_rgb(98, 143, 131)),
                        (egui::Color32::from_rgb(220, 154, 126), egui::Color32::from_rgb(246, 204, 146), egui::Color32::from_rgb(124, 105, 145)),
                        (egui::Color32::from_rgb(112, 150, 124), egui::Color32::from_rgb(232, 188, 153), egui::Color32::from_rgb(76, 107, 105)),
                    ];
                    let (sky, glow, hill) = palettes[index % palettes.len()];
                    let painter = ui.painter();
                    painter.rect_filled(cover, egui::CornerRadius::same(15), sky);
                    painter.circle_filled(egui::pos2(cover.left() + cover.width() * 0.73, cover.top() + 34.0), 19.0, glow);
                    painter.add(egui::Shape::convex_polygon(vec![egui::pos2(cover.left(),cover.bottom()-24.0),egui::pos2(cover.left()+cover.width()*0.20,cover.top()+65.0),egui::pos2(cover.left()+cover.width()*0.42,cover.bottom()-25.0),egui::pos2(cover.left()+cover.width()*0.67,cover.top()+59.0),egui::pos2(cover.right(),cover.bottom()-21.0),egui::pos2(cover.right(),cover.bottom()),egui::pos2(cover.left(),cover.bottom())], hill, egui::Stroke::NONE));
                    let face = egui::pos2(cover.left() + cover.width() * 0.30, cover.top() + 49.0);
                    painter.circle_filled(face, 16.0, egui::Color32::from_rgb(255, 226, 211));
                    painter.circle_filled(face + egui::vec2(-5.0,-1.0), 1.5, egui::Color32::from_rgb(63,48,65));
                    painter.circle_filled(face + egui::vec2(5.0,-1.0), 1.5, egui::Color32::from_rgb(63,48,65));
                    painter.circle_filled(face + egui::vec2(-8.0,3.0), 2.5, egui::Color32::from_rgb(239,139,151));
                    painter.circle_filled(face + egui::vec2(8.0,3.0), 2.5, egui::Color32::from_rgb(239,139,151));
                    for (dx, dy) in [(0.12,0.24),(0.54,0.18),(0.88,0.48)] {
                        let c = egui::pos2(cover.left()+cover.width()*dx, cover.top()+cover.height()*dy);
                        painter.line_segment([c-egui::vec2(0.0,4.0),c+egui::vec2(0.0,4.0)],egui::Stroke::new(1.5,egui::Color32::from_white_alpha(210)));
                        painter.line_segment([c-egui::vec2(4.0,0.0),c+egui::vec2(4.0,0.0)],egui::Stroke::new(1.5,egui::Color32::from_white_alpha(210)));
                    }
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new(title).size(15.0).strong());
                    ui.add(egui::Label::new(egui::RichText::new(path).small().color(egui::Color32::from_gray(145))).truncate()).on_hover_text(path);
                    ui.add_space(4.0);
                    if ui.add_enabled(!self.busy, egui::Button::new("Play").min_size(egui::vec2(90.0, 30.0))).clicked() {
                        actions.open = Some(path.clone());
                        self.open = false;
                    }
                });
            }
        });
    }
}

fn paint_sparkle(ui: &mut egui::Ui, size: egui::Vec2, color: egui::Color32, radius: f32) {
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let center = rect.center();
    let painter = ui.painter();
    painter.line_segment([center - egui::vec2(0.0, radius), center + egui::vec2(0.0, radius)], egui::Stroke::new(1.5, color));
    painter.line_segment([center - egui::vec2(radius, 0.0), center + egui::vec2(radius, 0.0)], egui::Stroke::new(1.5, color));
    painter.circle_filled(center, 1.4, color);
}
