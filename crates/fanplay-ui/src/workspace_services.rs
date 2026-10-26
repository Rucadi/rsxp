// SPDX-License-Identifier: GPL-2.0-or-later
//! Default [`crate::ui::backend::Backend`] adapter for the desktop and
//! cooperative browser presentation.
//!
//! [`LocalServices`] is what the egui workspace talks to. Runtime settings and
//! patches delegate straight to the thread-local state in `fanplay_rgss::runtime_settings`
//! / `fanplay_rgss::runtime_patch`; the desktop settings are additionally relayed to
//! the runner by its session transport.
//!
//! Behind `game-api` it exposes the async command bridge (`send_request` /
//! `take_response`), the external HTTP port, and asset lookup. `asset_image`
//! rejects any category or name that is not a single normal path component
//! (blocking `..` traversal). When an `image_reader` is injected (the browser
//! path) it decodes `Graphics/<category>/<name>` (with `.png`/`.bmp` fallbacks)
//! into a 256-entry cache; otherwise it uses the bounded background decoder in
//! the private `assets` submodule.
//!
//! Plugin webview assets are resolved from the active game root (or the
//! browser-provided asset reader).
#[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
mod assets;

#[derive(Default)]
pub struct LocalServices {
    #[cfg(feature = "game-api")]
    pub image_reader: Option<fn(&str) -> Option<Vec<u8>>>,
    #[cfg(feature = "game-api")]
    image_cache: std::cell::RefCell<
        std::collections::HashMap<String, Option<std::sync::Arc<image::RgbaImage>>>,
    >,
    pub game_root: Option<std::path::PathBuf>,
    #[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
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
        fanplay_rgss::runtime_settings::set_rate_override(v)
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
        if let Some(reader) = self.image_reader {
            let key = format!("Graphics/{category}/{name}");
            if let Some(image) = self.image_cache.borrow().get(&key) {
                return image.clone();
            }
            let decoded = [key.clone(), format!("{key}.png"), format!("{key}.bmp")]
                .iter()
                .find_map(|path| {
                    reader(path).and_then(|bytes| image::load_from_memory(&bytes).ok())
                })
                .map(|image| std::sync::Arc::new(image.to_rgba8()));
            let mut cache = self.image_cache.borrow_mut();
            if cache.len() >= 256 {
                cache.clear();
            }
            cache.insert(key, decoded.clone());
            return decoded;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.assets
                .get_or_init(|| assets::Assets::new(self.game_root.clone()))
                .image(category, name)
        }
        #[cfg(target_arch = "wasm32")]
        {
            let key = format!("Graphics/{category}/{name}");
            let bytes = fanplay_rgss::engine::with_runtime(|runtime| {
                [key.clone(), format!("{key}.png"), format!("{key}.bmp")]
                    .into_iter()
                    .find_map(|path| runtime.asset(&path).ok())
            })
            .flatten()?;
            image::load_from_memory(&bytes)
                .ok()
                .map(|image| std::sync::Arc::new(image.to_rgba8()))
        }
    }
    #[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
    fn handle_plugin_operation(
        &self,
        operation: &str,
        arguments: &serde_json::Value,
    ) -> Option<Result<serde_json::Value, String>> {
        fanplay_rgss::cable_club::handle_plugin_operation(operation, arguments, self.game_root.as_deref())
    }
}

#[cfg(all(test, feature = "game-api"))]
mod tests {
    use super::*;
    use crate::ui::backend::Backend;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn injected_assets_use_extensions_cache_and_reject_traversal() {
        static READS: AtomicUsize = AtomicUsize::new(0);
        fn read(path: &str) -> Option<Vec<u8>> {
            READS.fetch_add(1, Ordering::Relaxed);
            if path != "Graphics/Tilesets/test.png" {
                return None;
            }
            let image = image::RgbaImage::from_pixel(2, 3, image::Rgba([12, 34, 56, 255]));
            let mut bytes = std::io::Cursor::new(Vec::new());
            image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
            Some(bytes.into_inner())
        }
        let services = LocalServices {
            image_reader: Some(read),
            ..Default::default()
        };
        let first = services.asset_image("Tilesets", "test").unwrap();
        assert_eq!(first.dimensions(), (2, 3));
        let second = services.asset_image("Tilesets", "test").unwrap();
        assert!(std::sync::Arc::ptr_eq(&first, &second));
        assert!(services.asset_image("../Tilesets", "test").is_none());
        assert!(services.asset_image("Tilesets", "../test").is_none());
        assert_eq!(READS.load(Ordering::Relaxed), 2);
    }
}
