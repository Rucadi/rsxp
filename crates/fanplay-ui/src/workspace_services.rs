// SPDX-License-Identifier: GPL-2.0-or-later
//! Default [`crate::ui::backend::Backend`] adapter for the desktop presentation.
//!
//! [`LocalServices`] is what the egui workspace talks to. Runtime settings and
//! patches delegate straight to the thread-local state in `fanplay_rgss::runtime_settings`
//! / `fanplay_rgss::runtime_patch`.
//!
//! Behind `game-api` it exposes the async command bridge (`send_request` /
//! `take_response`), the external HTTP port, and asset lookup. `asset_image`
//! rejects any category or name that is not a single normal path component
//! (blocking `..` traversal) and uses the bounded background decoder in the
//! private `assets` submodule.
#[cfg(feature = "game-api")]
mod assets;

#[derive(Default)]
pub struct LocalServices {
    pub game_root: Option<std::path::PathBuf>,
    #[cfg(feature = "game-api")]
    assets: std::sync::OnceLock<assets::Assets>,
}
impl LocalServices {
    pub fn new(game_root: Option<std::path::PathBuf>) -> Self {
        Self {
            game_root,
            ..Default::default()
        }
    }
}
impl crate::ui::backend::Backend for LocalServices {
    fn keyboard_binding(&self, action: i32) -> i32 {
        crate::controls::keyboard_binding(action)
    }
    fn set_keyboard_binding(&self, action: i32, physical: i32) {
        crate::controls::set_keyboard_binding(action, physical)
    }
    fn fast_forward(&self) -> bool {
        fanplay_rgss::runtime_settings::fast_forward()
    }
    fn set_fast_forward(&self, v: bool) {
        fanplay_rgss::runtime_settings::set_fast_forward(v)
    }
    fn current_rate(&self) -> u32 {
        fanplay_rgss::runtime_settings::current_rate()
    }
    fn set_rate_override(&self, v: u32) {
        if v == 0 {
            fanplay_rgss::runtime_settings::clear_rate_override();
        } else {
            fanplay_rgss::runtime_settings::set_rate_override(v);
        }
    }
    fn request_reset(&self) {
        fanplay_rgss::runtime_settings::request_reset()
    }
    fn patch_enabled(&self, id: &str) -> bool {
        fanplay_rgss::runtime_patch::is_enabled(id)
    }
    fn set_patch_enabled(&self, id: &str, v: bool) -> bool {
        fanplay_rgss::runtime_patch::set_enabled(id, v)
    }
    fn patch_value(&self, id: &str) -> Option<f64> {
        fanplay_rgss::runtime_patch::value(id)
    }
    fn set_patch_value(&self, id: &str, v: f64) -> bool {
        fanplay_rgss::runtime_patch::set_value(id, v)
    }
    #[cfg(feature = "game-api")]
    fn asset_image(&self, category: &str, name: &str) -> Option<std::sync::Arc<image::RgbaImage>> {
        use std::path::{Component, Path};
        if name.is_empty()
            || Path::new(name).components().count() != 1
            || !Path::new(name)
                .components()
                .all(|c| matches!(c, Component::Normal(_)))
            || !Path::new(category)
                .components()
                .all(|c| matches!(c, Component::Normal(_)))
        {
            return None;
        }
        self.assets
            .get_or_init(|| assets::Assets::new(self.game_root.clone()))
            .image(category, name)
    }
    #[cfg(feature = "game-api")]
    fn handle_plugin_operation(
        &self,
        operation: &str,
        arguments: &serde_json::Value,
    ) -> Option<Result<serde_json::Value, String>> {
        fanplay_rgss::cable_club::handle_plugin_operation(
            operation,
            arguments,
            self.game_root.as_deref(),
        )
    }
}
