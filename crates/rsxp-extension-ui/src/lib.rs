// SPDX-License-Identifier: GPL-2.0-or-later
//! Renders the plugin API's declarative UI without executing scripts during
//! layout. The host supplies cached descriptions and receives plain events.

use egui::{Color32, RichText, ScrollArea, Ui};
use rsxp_extension_api::{UiDescription, UiEvent, UiNode, UiOption};
use serde_json::Value;

#[derive(Default)]
pub struct ExtensionUi {
    descriptions: Vec<UiDescription>,
    events: Vec<UiEvent>,
}

impl ExtensionUi {
    pub fn set_descriptions(&mut self, descriptions: impl IntoIterator<Item = UiDescription>) {
        self.descriptions = descriptions.into_iter().collect();
    }

    pub fn clear(&mut self) {
        self.descriptions.clear();
        self.events.clear();
    }

    pub fn take_events(&mut self) -> Vec<UiEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        ui.heading("Plugins");
        ui.weak("Runtime patches and game tools supplied by Rhai plugins.");
        ui.horizontal_wrapped(|ui| {
            if ui.button("Paste plugin text…").clicked() {
                self.events.push(UiEvent {
                    panel_id: "__host".into(),
                    widget_id: "paste-plugin".into(),
                    value: Value::Bool(true),
                });
            }
            #[cfg(not(target_arch = "wasm32"))]
            if ui.button("Import plugin…").clicked() {
                self.events.push(UiEvent {
                    panel_id: "__host".into(),
                    widget_id: "import-plugin".into(),
                    value: Value::Bool(true),
                });
            }
        });
        if self.descriptions.is_empty() {
            ui.weak("No plugins loaded.");
            return;
        }
        for description in &self.descriptions {
            ui.push_id(&description.panel_id, |ui| {
                ui.collapsing(&description.title, |ui| {
                    if let Some(error) = &description.error {
                        ui.colored_label(Color32::RED, error);
                    }
                    for node in &description.nodes {
                        draw_node(ui, &description.panel_id, node, &mut self.events);
                    }
                });
            });
        }
    }
}

fn draw_node(ui: &mut Ui, panel_id: &str, node: &UiNode, events: &mut Vec<UiEvent>) {
    match node {
        UiNode::Label { id, text } => {
            ui.push_id(id, |ui| {
                ui.label(text);
            });
        }
        UiNode::Button { id, label } => {
            if ui.button(label).clicked() {
                events.push(UiEvent {
                    panel_id: panel_id.into(),
                    widget_id: id.clone(),
                    value: Value::Bool(true),
                });
            }
        }
        UiNode::Checkbox { id, label, value } => {
            let mut value = *value;
            if ui.checkbox(&mut value, label).changed() {
                events.push(UiEvent {
                    panel_id: panel_id.into(),
                    widget_id: id.clone(),
                    value: Value::Bool(value),
                });
            }
        }
        UiNode::TextInput { id, label, value } => {
            let mut value = value.clone();
            ui.horizontal(|ui| {
                ui.label(label);
                if ui.text_edit_singleline(&mut value).changed() {
                    events.push(UiEvent {
                        panel_id: panel_id.into(),
                        widget_id: id.clone(),
                        value: Value::String(value.clone()),
                    });
                }
            });
        }
        UiNode::Select {
            id,
            label,
            value,
            options,
        } => {
            let mut selected = value.clone();
            egui::ComboBox::from_id_salt(id)
                .selected_text(option_label(&selected, options))
                .show_ui(ui, |ui| {
                    for option in options {
                        if ui
                            .selectable_value(&mut selected, option.value.clone(), &option.label)
                            .changed()
                        {
                            events.push(UiEvent {
                                panel_id: panel_id.into(),
                                widget_id: id.clone(),
                                value: Value::String(selected.clone()),
                            });
                        }
                    }
                });
            ui.label(RichText::new(label).weak());
        }
        UiNode::Slider {
            id,
            label,
            value,
            min,
            max,
            step,
            suffix,
        } => {
            let mut value = *value;
            let slider = egui::Slider::new(&mut value, *min..=*max)
                .step_by(*step)
                .text(label)
                .suffix(suffix.as_str());
            if ui.add(slider).changed() {
                events.push(UiEvent {
                    panel_id: panel_id.into(),
                    widget_id: id.clone(),
                    value: Value::from(value),
                });
            }
        }
        UiNode::Group {
            id,
            title,
            children,
        } => {
            ui.push_id(id, |ui| {
                ui.group(|ui| {
                    ui.strong(title);
                    for child in children {
                        draw_node(ui, panel_id, child, events);
                    }
                });
            });
        }
        UiNode::Row { id, children } => {
            ui.push_id(id, |ui| {
                ui.horizontal(|ui| {
                    for child in children {
                        draw_node(ui, panel_id, child, events);
                    }
                });
            });
        }
        UiNode::Scroll { id, children } => {
            ui.push_id(id, |ui| {
                ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                    for child in children {
                        draw_node(ui, panel_id, child, events);
                    }
                });
            });
        }
        UiNode::Loading { id, label } => {
            ui.push_id(id, |ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(label);
                });
            });
        }
        UiNode::Table { id, columns, rows } => {
            ui.push_id(id, |ui| {
                egui::Grid::new("table").striped(true).show(ui, |ui| {
                    for column in columns {
                        ui.strong(column);
                    }
                    ui.end_row();
                    for row in rows {
                        for cell in row {
                            ui.label(cell);
                        }
                        ui.end_row();
                    }
                });
            });
        }
    }
}

fn option_label(value: &str, options: &[UiOption]) -> String {
    options
        .iter()
        .find(|option| option.value == value)
        .map_or_else(|| value.to_owned(), |option| option.label.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_queue_is_owned_by_renderer_and_can_be_drained() {
        let mut ui = ExtensionUi::default();
        ui.events.push(UiEvent {
            panel_id: "example.main".into(),
            widget_id: "button".into(),
            value: Value::Bool(true),
        });
        assert_eq!(ui.take_events().len(), 1);
        assert!(ui.take_events().is_empty());
    }
}
