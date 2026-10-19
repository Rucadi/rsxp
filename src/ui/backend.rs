// SPDX-License-Identifier: GPL-2.0-or-later
//! Platform services the workspace panels call into, injected as a trait object.
//!
//! This module defines [`Backend`], the boundary through which the egui
//! workspace (`super`) reaches the running game and the host platform without
//! knowing whether it is embedded in the desktop runner or the browser build.
//! Panels hold a `Box<dyn Backend>`; the concrete adapter lives elsewhere
//! (`crate::workspace_services::LocalServices` is the default). No VM, window,
//! renderer, or egui types cross this boundary — only plain values (`bool`,
//! `u32`, `f64`, `u16`, `String`, `RgbaImage`, `PathBuf`) and the Cable Club
//! `ServerStatus` snapshot.
//!
//! The methods group into four areas:
//!
//! * frame pacing — [`Backend::fast_forward`] / [`Backend::set_fast_forward`]
//!   toggle uncapped execution; [`Backend::current_rate`] /
//!   [`Backend::set_rate_override`] read and set the target FPS (`0` means no
//!   override is in effect);
//! * lifecycle — [`Backend::request_reset`] restarts the game from the start;
//! * runtime patches — the `patch_*` methods read and write the enabled flag and
//!   optional numeric parameter for a patch id (catalog in `crate::runtime_patch`);
//! * game API and Cable Club — the asynchronous request/response channel, the
//!   external HTTP endpoint, asset lookup, and the local relay, all behind
//!   `#[cfg(feature = "game-api")]`. The Cable Club relay methods are also
//!   unavailable on `wasm32`, so callers gate their panels to match.
pub trait Backend {
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
    /// Port of the external HTTP API, once it has been started.
    #[cfg(feature = "game-api")]
    fn listening_port(&self) -> Option<u16>;
    /// Starts the external HTTP API on `port`.
    #[cfg(feature = "game-api")]
    fn start_api(&self, port: u16) -> Result<(), String>;
    /// Non-blocking fetch of the reply to request `id` (removes it when present).
    #[cfg(feature = "game-api")]
    fn take_response(&self, id: u64) -> Option<Result<String, String>>;
    /// Queues an API operation and returns its request id for later polling.
    #[cfg(feature = "game-api")]
    fn send_request(&self, operation: &str, args: Vec<String>) -> Result<u64, String>;
    /// Loads a game asset image by category and name.
    #[cfg(feature = "game-api")]
    fn asset_image(&self, category: &str, name: &str) -> Option<std::sync::Arc<image::RgbaImage>>;
    /// Snapshot of the Cable Club relay's bound address, clients, and error.
    #[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
    fn cable_status(&self) -> crate::cable_club::ServerStatus;
    /// Stops the Cable Club relay if it is running.
    #[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
    fn stop_cable(&self);
    /// Starts the Cable Club relay bound to `bind`:`port`.
    #[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
    fn start_cable(&self, bind: &str, port: u16) -> Result<(), String>;
    /// Persists `host`:`port` for the game's Cable Club client; returns the path written.
    #[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
    fn save_endpoint(&self, host: &str, port: u16) -> Result<std::path::PathBuf, String>;
}
