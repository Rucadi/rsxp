// SPDX-License-Identifier: GPL-2.0-or-later
//! Runtime output panel.
//!
//! `draw_logs` renders the bottom, resizable `workspace-output-panel` over
//! `Workspace::logs`, one monospace line each, auto-scrolled to the newest
//! entry, with copy, clear, follow, and filter controls. Crash lines use the
//! neon error colour so the reason a session stopped is visible immediately.
//! It is drawn for the `panels_output` toggle and, when a session has stopped,
//! even with controls hidden so crash output stays visible.
use super::*;

impl Workspace {
    pub(super) fn draw_logs(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("workspace-output-panel")
            .resizable(true)
            .default_size(180.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::SURFACE)
                    .inner_margin(egui::Margin::symmetric(14, 8)),
            )
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    theme::section_label(ui, "Activity", theme::PINK);
                    theme::tag(
                        ui,
                        &format!("{} lines", self.logs.len()),
                        theme::Tone::Neutral,
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut self.logs_filter)
                            .hint_text("Filter output…")
                            .desired_width((ui.available_width() * 0.32).clamp(120.0, 260.0)),
                    );
                    if !self.logs_filter.is_empty() {
                        if theme::button_sized(
                            ui,
                            "Clear filter",
                            theme::ButtonKind::Ghost,
                            theme::ButtonSize::Sm,
                        )
                        .clicked()
                        {
                            self.logs_filter.clear();
                        }
                    }
                    if theme::chip(ui, "Follow", theme::Tone::Cyan, self.logs_follow).clicked() {
                        self.logs_follow = !self.logs_follow;
                    }
                    if theme::button_sized(
                        ui,
                        "Copy",
                        theme::ButtonKind::Ghost,
                        theme::ButtonSize::Sm,
                    )
                    .clicked()
                    {
                        ui.ctx().copy_text(self.logs.join("\n"));
                    }
                    if theme::button_sized(
                        ui,
                        "Clear",
                        theme::ButtonKind::Ghost,
                        theme::ButtonSize::Sm,
                    )
                    .clicked()
                    {
                        self.logs.clear();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(theme::eyebrow("F3 to hide"));
                    });
                });
                theme::divider(ui);
                egui::ScrollArea::vertical()
                    .id_salt("runtime-output-scroll")
                    .stick_to_bottom(self.logs_follow)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let filter = self.logs_filter.trim().to_lowercase();
                        let mut visible = 0usize;
                        for line in &self.logs {
                            if !filter.is_empty() && !line.to_lowercase().contains(&filter) {
                                continue;
                            }
                            visible += 1;
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(line)
                                        .font(theme::mono_font(12.5))
                                        .color(line_color(line)),
                                )
                                .selectable(true),
                            );
                        }
                        if visible == 0 {
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new(if self.logs.is_empty() {
                                    "No runtime messages yet. Launch a game to see its output here."
                                } else {
                                    "No output lines match this filter."
                                })
                                .font(theme::body_font(12.0))
                                .color(theme::TEXT_FAINT),
                            );
                        }
                    });
            });
    }
}

fn line_color(line: &str) -> egui::Color32 {
    let lower = line.to_ascii_lowercase();
    if lower.contains("error")
        || lower.contains("panic")
        || lower.contains("exception")
        || lower.contains("traceback")
        || lower.contains("failed")
        || lower.contains("crash")
    {
        theme::PINK
    } else if lower.contains("warn") {
        theme::WARNING
    } else if lower.contains("loaded") || lower.contains("ready") || lower.contains("started") {
        theme::LIME.gamma_multiply(0.9)
    } else {
        theme::TEXT_MUTED
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn crash_lines_are_highlighted() {
        assert_eq!(super::line_color("RuntimeError: boom"), super::theme::PINK);
        assert_eq!(
            super::line_color("warning: texture missing"),
            super::theme::WARNING
        );
    }
}
