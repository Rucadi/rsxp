// SPDX-License-Identifier: GPL-2.0-or-later
pub(super) fn apply(context: &egui::Context) {
    let id = egui::Id::new("rsxp-theme-v1");
    if context.data_mut(|d| d.get_temp::<bool>(id).unwrap_or(false)) {
        return;
    }
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = egui::Color32::from_rgb(24, 28, 35);
    visuals.window_fill = egui::Color32::from_rgb(30, 35, 43);
    visuals.selection.bg_fill = egui::Color32::from_rgb(39, 99, 88);
    visuals.widgets.active.bg_fill = egui::Color32::from_rgb(47, 112, 99);
    visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(49, 59, 70);
    context.set_visuals(visuals);
    context.style_mut(|style| {
        style.interaction.selectable_labels = true;
        style.spacing.item_spacing = egui::vec2(9.0, 7.0);
        style.spacing.button_padding = egui::vec2(10.0, 6.0);
        style.spacing.window_margin = egui::Margin::same(16);
    });
    context.data_mut(|d| d.insert_temp(id, true));
}
