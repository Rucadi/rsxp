// SPDX-License-Identifier: GPL-2.0-or-later
//! Default [`crate::ui::backend::Backend`] adapter for the desktop and
//! cooperative browser presentation.
//!
//! [`LocalServices`] is what the egui workspace talks to. Runtime settings and
//! patches delegate straight to the thread-local state in `crate::runtime_settings`
//! / `crate::runtime_patch`; the desktop settings are additionally relayed to
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
//! The Cable Club methods (non-WASM) delegate to `crate::cable_club`, and
//! `save_endpoint` writes the game's `Data/serverinfo.ini` under `game_root` (or
//! the current directory).
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
    fn fast_forward(&self) -> bool {
        crate::runtime_settings::fast_forward()
    }
    fn set_fast_forward(&self, v: bool) {
        crate::runtime_settings::set_fast_forward(v)
    }
    fn current_rate(&self) -> u32 {
        crate::runtime_settings::current_rate()
    }
    fn set_rate_override(&self, v: u32) {
        crate::runtime_settings::set_rate_override(v)
    }
    fn request_reset(&self) {
        crate::runtime_settings::request_reset()
    }
    fn patch_enabled(&self, id: &str) -> bool {
        crate::runtime_patch::is_enabled(id)
    }
    fn set_patch_enabled(&self, id: &str, v: bool) -> bool {
        crate::runtime_patch::set_enabled(id, v)
    }
    fn patch_value(&self, id: &str) -> Option<f64> {
        crate::runtime_patch::value(id)
    }
    fn set_patch_value(&self, id: &str, v: f64) -> bool {
        crate::runtime_patch::set_value(id, v)
    }
    #[cfg(feature = "game-api")]
    fn listening_port(&self) -> Option<u16> {
        crate::api_bridge::listening_port()
    }
    #[cfg(feature = "game-api")]
    fn start_api(&self, port: u16) -> Result<(), String> {
        crate::api_bridge::start(port)
    }
    #[cfg(feature = "game-api")]
    fn take_response(&self, id: u64) -> Option<Result<String, String>> {
        crate::api_bridge::take_ui_response(id)
    }
    #[cfg(feature = "game-api")]
    fn send_request(&self, op: &str, args: Vec<String>) -> Result<u64, String> {
        crate::api_bridge::send_from_ui(op, args)
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
            None
        }
    }
    #[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
    fn cable_status(&self) -> crate::cable_club::ServerStatus {
        crate::cable_club::status()
    }
    #[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
    fn stop_cable(&self) {
        crate::cable_club::stop()
    }
    #[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
    fn start_cable(&self, bind: &str, port: u16) -> Result<(), String> {
        crate::cable_club::start(bind, port)
    }
    #[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
    fn save_endpoint(&self, host: &str, port: u16) -> Result<std::path::PathBuf, String> {
        let root = self
            .game_root
            .clone()
            .or_else(|| std::env::current_dir().ok())
            .ok_or("No game directory")?;
        crate::cable_club::save_client_endpoint_at(&root, host, port)
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
