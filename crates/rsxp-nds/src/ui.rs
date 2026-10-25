//! DS screen layout and presentation, independent of the emulator.
/// Presentation presets for the two Nintendo DS LCDs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DsLayout {
    #[default]
    Vertical,
    Horizontal,
    TopLarge,
    TouchLarge,
    TopOnly,
    TouchOnly,
}

impl DsLayout {
    pub const fn aspect(self) -> f32 {
        match self {
            Self::Vertical => 256.0 / 384.0,
            Self::Horizontal => 512.0 / 192.0,
            Self::TopLarge | Self::TouchLarge => 768.0 / 384.0,
            Self::TopOnly | Self::TouchOnly => 256.0 / 192.0,
        }
    }
}

impl DsLayout {
    pub fn rects(
        self,
        available: egui::Rect,
        controls_visible: bool,
    ) -> (Option<egui::Rect>, Option<egui::Rect>) {
        let (width, height) = match self {
            DsLayout::Vertical => (256, 384),
            DsLayout::Horizontal => (512, 192),
            DsLayout::TopLarge | DsLayout::TouchLarge => (768, 384),
            DsLayout::TopOnly | DsLayout::TouchOnly => (256, 192),
        };
        let canvas = if self == DsLayout::Horizontal && !controls_visible {
            // Horizontal DS mode is intentionally edge-to-edge vertically:
            // both LCDs fill the window height and meet its top and bottom.
            let scale = available.height() / height as f32;
            let size = egui::vec2(width as f32 * scale, available.height());
            egui::Rect::from_center_size(
                egui::pos2(available.center().x, available.min.y + size.y * 0.5),
                size,
            )
        } else {
            aspect_rect(available, (width, height))
        };
        let scale = (canvas.width() / width as f32).min(canvas.height() / height as f32);
        let at = |x: u32, y: u32, w: u32, h: u32| {
            egui::Rect::from_min_size(
                canvas.min + egui::vec2(x as f32 * scale, y as f32 * scale),
                egui::vec2(w as f32 * scale, h as f32 * scale),
            )
        };
        match self {
            DsLayout::Vertical => (Some(at(0, 0, 256, 192)), Some(at(0, 192, 256, 192))),
            DsLayout::Horizontal => (Some(at(0, 0, 256, 192)), Some(at(256, 0, 256, 192))),
            DsLayout::TopLarge => (Some(at(0, 0, 512, 384)), Some(at(512, 96, 256, 192))),
            DsLayout::TouchLarge => (Some(at(0, 96, 256, 192)), Some(at(256, 0, 512, 384))),
            DsLayout::TopOnly => (Some(at(0, 0, 256, 192)), None),
            DsLayout::TouchOnly => (None, Some(at(0, 0, 256, 192))),
        }
    }

    pub fn paint(
        self,
        ui: &egui::Ui,
        texture: Option<egui::TextureId>,
        controls_visible: bool,
    ) -> (egui::Rect, egui::Rect) {
        let (top, touch) = self.rects(ui.max_rect(), controls_visible);
        for (rect, min_y) in [(top, 0.0), (touch, 0.5)] {
            if let (Some(rect), Some(texture)) = (rect, texture) {
                let uv =
                    egui::Rect::from_min_max(egui::pos2(0.0, min_y), egui::pos2(1.0, min_y + 0.5));
                ui.painter().image(texture, rect, uv, egui::Color32::WHITE);
            }
        }
        let game = match (top, touch) {
            (Some(a), Some(b)) => a.union(b),
            (Some(a), None) | (None, Some(a)) => a,
            _ => egui::Rect::ZERO,
        };
        (game, touch.unwrap_or(egui::Rect::ZERO))
    }
}
fn aspect_rect(available: egui::Rect, logical_size: (u32, u32)) -> egui::Rect {
    let (width, height) = logical_size;
    if width == 0 || height == 0 || available.width() <= 0.0 || available.height() <= 0.0 {
        return available;
    }
    let scale = (available.width() / width as f32).min(available.height() / height as f32);
    let size = egui::vec2(width as f32 * scale, height as f32 * scale);
    egui::Rect::from_center_size(available.center(), size)
}

impl DsLayout {
    pub fn selector(&mut self, ui: &mut egui::Ui) {
        for (layout, label) in [
            (Self::Vertical, "Vertical (natural)"),
            (Self::Horizontal, "Horizontal"),
            (Self::TopLarge, "Top large · touch small"),
            (Self::TouchLarge, "Touch large · top small"),
            (Self::TopOnly, "Top screen only"),
            (Self::TouchOnly, "Touch screen only"),
        ] {
            ui.selectable_value(self, layout, label);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn layouts_preserve_screen_ratio_and_touch_identity() {
        let available =
            egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(1000.0, 700.0));
        for layout in [
            DsLayout::Vertical,
            DsLayout::Horizontal,
            DsLayout::TopLarge,
            DsLayout::TouchLarge,
            DsLayout::TopOnly,
            DsLayout::TouchOnly,
        ] {
            for controls in [true, false] {
                let (top, touch) = layout.rects(available, controls);
                assert_eq!(top.is_none(), layout == DsLayout::TouchOnly);
                assert_eq!(touch.is_none(), layout == DsLayout::TopOnly);
                for rect in [top, touch].into_iter().flatten() {
                    assert!((rect.width() / rect.height() - 4.0 / 3.0).abs() < 0.0001);
                }
            }
        }
        let (top, touch) = DsLayout::Vertical.rects(available, true);
        assert_eq!(top.unwrap().max.y, touch.unwrap().min.y);
        let (top, touch) = DsLayout::Horizontal.rects(available, false);
        assert!((top.unwrap().height() - available.height()).abs() < 0.001);
        assert_eq!(top.unwrap().max.x, touch.unwrap().min.x);
    }
}

/// Platform label shown in the game library.
pub const LABEL: &str = "NDS";

/// Background and highlight colors for platform cover cards.
pub fn cover_colors() -> (egui::Color32, egui::Color32) {
    (
        egui::Color32::from_rgb(38, 14, 44),
        egui::Color32::from_rgb(88, 32, 92),
    )
}
