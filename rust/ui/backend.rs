// SPDX-License-Identifier: GPL-2.0-or-later
//! Platform services injected into panels. No VM or window types cross here.
pub trait Backend {
    fn fast_forward(&self) -> bool;
    fn set_fast_forward(&self, enabled: bool);
    fn current_rate(&self) -> u32;
    fn set_rate_override(&self, rate: u32);
    fn request_reset(&self);
    fn patch_enabled(&self, id: &str) -> bool;
    fn set_patch_enabled(&self, id: &str, enabled: bool) -> bool;
    fn patch_value(&self, id: &str) -> Option<f64>;
    fn set_patch_value(&self, id: &str, value: f64) -> bool;
    #[cfg(feature = "game-api")]
    fn listening_port(&self) -> Option<u16>;
    #[cfg(feature = "game-api")]
    fn start_api(&self, port: u16) -> Result<(), String>;
    #[cfg(feature = "game-api")]
    fn take_response(&self, id: u64) -> Option<Result<String, String>>;
    #[cfg(feature = "game-api")]
    fn send_request(&self, operation: &str, args: Vec<String>) -> Result<u64, String>;
    #[cfg(feature = "game-api")]
    fn asset_image(&self, category: &str, name: &str) -> Option<std::sync::Arc<image::RgbaImage>>;
    #[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
    fn cable_status(&self) -> crate::cable_club::ServerStatus;
    #[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
    fn stop_cable(&self);
    #[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
    fn start_cable(&self, bind: &str, port: u16) -> Result<(), String>;
    #[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
    fn save_endpoint(&self, host: &str, port: u16) -> Result<std::path::PathBuf, String>;
}
