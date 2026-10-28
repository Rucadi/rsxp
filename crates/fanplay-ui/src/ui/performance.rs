// SPDX-License-Identifier: GPL-2.0-or-later
//! Compact, readable live session metrics with a neon frame-rate trace.
use super::*;

impl Workspace {
    pub(super) fn draw_performance_panel(&mut self, ui: &mut egui::Ui, info: &WorkspaceInfo) {
        egui::Panel::right("workspace-performance-panel")
            .resizable(true)
            .default_size(260.0)
            .size_range(220.0..=420.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::SURFACE)
                    .inner_margin(egui::Margin::symmetric(14, 12)),
            )
            .show(ui, |ui| {
                theme::section_label(ui, "Performance", theme::LIME);
                ui.label(theme::display("Live metrics", 20.0));
                ui.add_space(10.0);
                theme::stat(
                    ui,
                    "FRAMES PER SECOND",
                    &if info.fps > 0.0 {
                        format!("{:.1}", info.fps)
                    } else {
                        "—".into()
                    },
                    theme::LIME,
                );
                ui.add_space(8.0);
                theme::stat(
                    ui,
                    "FRAME INTERVAL",
                    &if info.fps > 0.0 {
                        format!("{:.2} ms", 1000.0 / info.fps)
                    } else {
                        "—".into()
                    },
                    theme::CYAN,
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    theme::stat(ui, "FRAMES RENDERED", &info.frames.to_string(), theme::TEXT);
                });
                ui.add_space(8.0);
                theme::stat(
                    ui,
                    "NATIVE RESOLUTION",
                    &format!("{}×{}", info.logical_size.0, info.logical_size.1),
                    theme::PINK,
                );
                ui.add_space(12.0);
                history(ui, &self.fps_history);
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new(
                        "F10 toggles this panel. The trace keeps the last 120 samples.",
                    )
                    .font(theme::body_font(11.0))
                    .color(theme::TEXT_FAINT),
                );
            });
    }
}

fn history(ui: &mut egui::Ui, history: &std::collections::VecDeque<f32>) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 76.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, egui::CornerRadius::same(8), theme::BG);
    painter.rect_stroke(
        rect,
        egui::CornerRadius::same(8),
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );
    painter.text(
        rect.left_top() + egui::vec2(10.0, 8.0),
        egui::Align2::LEFT_TOP,
        "FPS TRACE",
        theme::mono_font(9.0),
        theme::TEXT_FAINT,
    );
    if history.len() < 2 {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "waiting for frames…",
            theme::body_font(11.0),
            theme::TEXT_FAINT,
        );
        return;
    }
    let max = history.iter().copied().fold(60.0_f32, f32::max).max(1.0);
    let plot = rect.shrink2(egui::vec2(10.0, 18.0));
    for fraction in [0.25, 0.5, 0.75] {
        let y = plot.bottom() - plot.height() * fraction;
        painter.line_segment(
            [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
            egui::Stroke::new(1.0, theme::BORDER.gamma_multiply(0.6)),
        );
    }
    let step = plot.width() / (history.len().max(2) - 1) as f32;
    let mut previous = None;
    for (index, fps) in history.iter().enumerate() {
        let point = egui::pos2(
            plot.left() + step * index as f32,
            plot.bottom() - plot.height() * (fps / max).clamp(0.0, 1.0),
        );
        if let Some(previous) = previous {
            painter.line_segment([previous, point], egui::Stroke::new(1.6, theme::CYAN));
        }
        previous = Some(point);
    }
    if let Some(last) = previous {
        painter.circle_filled(last, 3.0, theme::LIME);
        painter.circle_stroke(
            last,
            5.0,
            egui::Stroke::new(1.0, theme::LIME.gamma_multiply(0.6)),
        );
    }
    painter.text(
        rect.right_top() + egui::vec2(-10.0, 8.0),
        egui::Align2::RIGHT_TOP,
        format!("{:.0} max", max),
        theme::mono_font(9.0),
        theme::TEXT_FAINT,
    );
}
