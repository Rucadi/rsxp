// SPDX-License-Identifier: GPL-2.0-or-later
//! Platform services the workspace panels call into, injected as a trait object.
//!
//! This module defines [`Backend`], the boundary through which the egui
//! workspace (`super`) reaches the running game and the host platform without
//! knowing whether it is embedded in the desktop runner or the browser build.
//! Panels hold a `Box<dyn Backend>`; the concrete adapter lives elsewhere
//! (`crate::workspace_services::LocalServices` is the default). No VM, window,
//! renderer, or egui types cross this boundary — only plain values (`bool`,
//! `u32`, `f64`, `String`, and `RgbaImage`).
//!
//! The methods group into four areas:
//!
//! * frame pacing — [`Backend::fast_forward`] / [`Backend::set_fast_forward`]
//!   toggle uncapped execution; [`Backend::current_rate`] /
//!   [`Backend::set_rate_override`] read and set the target FPS (`0` means no
//!   override is in effect);
//! * lifecycle — [`Backend::request_reset`] restarts the game from the start;
//! * runtime patches — the `patch_*` methods read and write the enabled flag and
//!   optional numeric parameter for a patch id (catalog in `fanplay_rgss::runtime_patch`);
//! * plugin assets — the game image lookup used by plugin webviews, behind
//!   `#[cfg(feature = "game-api")]`.
pub trait Backend {
    /// Returns the physical scancode assigned to an RGSS action.
    fn keyboard_binding(&self, action: i32) -> i32 {
        action
    }
    /// Updates a keyboard binding for the active game session.
    fn set_keyboard_binding(&self, action: i32, physical: i32) {
        let _ = (action, physical);
    }
    /// Whether uncapped frame pacing (fast-forward) is currently on.
    fn fast_forward(&self) -> bool;
    /// Enables or disables uncapped frame pacing.
    fn set_fast_forward(&self, enabled: bool);
    /// Current target frame rate, or `0` when no override is in effect.
    fn current_rate(&self) -> u32;
    /// Sets a target frame rate; `0` restores the game's own pacing.
    fn set_rate_override(&self, rate: u32);
    /// Asks the runner to restart the game from the beginning.
    fn request_reset(&self);
    /// Whether runtime patch `id` is enabled.
    fn patch_enabled(&self, id: &str) -> bool;
    /// Enables/disables patch `id`; returns whether the change was accepted.
    fn set_patch_enabled(&self, id: &str, enabled: bool) -> bool;
    /// Current numeric parameter for patch `id`, if it exposes one.
    fn patch_value(&self, id: &str) -> Option<f64>;
    /// Sets patch `id`'s numeric parameter; returns whether it was accepted.
    fn set_patch_value(&self, id: &str, value: f64) -> bool;
    /// Loads a game asset image by category and name.
    #[cfg(feature = "game-api")]
    fn asset_image(&self, category: &str, name: &str) -> Option<std::sync::Arc<image::RgbaImage>>;
    /// Gives a native host service an opportunity to answer a plugin-owned
    /// operation. `None` means the operation belongs to the game bridge or a
    /// different host adapter.
    #[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
    fn handle_plugin_operation(
        &self,
        operation: &str,
        arguments: &serde_json::Value,
    ) -> Option<Result<serde_json::Value, String>>;
}
