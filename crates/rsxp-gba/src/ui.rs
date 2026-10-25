//! GBA library presentation.

/// Platform label shown in the game library.
pub const LABEL: &str = "GBA";

/// Background and highlight colors for platform cover cards.
pub fn cover_colors() -> (egui::Color32, egui::Color32) {
    (
        egui::Color32::from_rgb(8, 35, 46),
        egui::Color32::from_rgb(20, 78, 96),
    )
}
