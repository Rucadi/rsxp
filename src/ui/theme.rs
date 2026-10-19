// SPDX-License-Identifier: GPL-2.0-or-later
//! Shared egui theme for the workspace.
//!
//! `apply` installs the dark visual style — panel and window fills, the green
//! selection/active accents, tighter item spacing, button padding, and window
//! margins — and enables selectable labels. It is idempotent per
//! `egui::Context`: a `"rsxp-theme-v1"` temp flag in the context's data cache
//! makes later frames a no-op, so callers may invoke it unconditionally each
//! frame (`Workspace::draw` does).
pub(super) fn apply(context: &egui::Context) {
    let id = egui::Id::unique("rsxp-theme-v1");
    if context.data_mut(|d| d.get_temp::<bool>(id).unwrap_or(false)) {
        return;
    }
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = egui::Color32::from_rgb(25, 22, 33);
    visuals.window_fill = egui::Color32::from_rgb(34, 29, 43);
    visuals.extreme_bg_color = egui::Color32::from_rgb(18, 16, 25);
    visuals.faint_bg_color = egui::Color32::from_rgb(39, 34, 49);
    visuals.selection.bg_fill = egui::Color32::from_rgb(115, 82, 143);
    visuals.widgets.active.bg_fill = egui::Color32::from_rgb(128, 91, 155);
    visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(65, 54, 78);
    visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(47, 40, 57);
    visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(30, 26, 39);
    visuals.widgets.active.corner_radius = egui::CornerRadius::same(10);
    visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(10);
    visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(10);
    visuals.widgets.noninteractive.corner_radius = egui::CornerRadius::same(10);
    visuals.window_corner_radius = egui::CornerRadius::same(16);
    visuals.menu_corner_radius = egui::CornerRadius::same(12);
    context.set_visuals(visuals);
    context.global_style_mut(|style| {
        style.interaction.selectable_labels = true;
        style.spacing.item_spacing = egui::vec2(9.0, 7.0);
        style.spacing.button_padding = egui::vec2(10.0, 6.0);
        style.spacing.window_margin = egui::Margin::same(16);
    });
    context.data_mut(|d| d.insert_temp(id, true));
}

#[cfg(test)]
mod tests {
    #[test]
    fn workspace_fonts_cover_labels_and_accents() {
        let context = egui::Context::default();
        context
            .run_ui(egui::RawInput::default(), |ui| {
                super::apply(ui.ctx());
                ui.fonts_mut(|fonts| {
                    for font in [
                        egui::FontId::proportional(14.0),
                        egui::FontId::monospace(14.0),
                    ] {
                        assert!(fonts.has_glyphs(&font, "rsxp Pokédex Poción Conexión × · −"));
                    }
                });
            })
            .drop_without_applying_deltas();
    }
}
