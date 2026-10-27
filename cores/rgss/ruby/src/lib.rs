// SPDX-License-Identifier: GPL-2.0-or-later
//! RGSS runtime: SabiRuby on desktop, ruby.wasm in the browser.
#[cfg(not(target_arch = "wasm32"))]
mod desktop;
#[cfg(not(target_arch = "wasm32"))]
pub use desktop::RubyCore;
#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(target_arch = "wasm32")]
pub use web::RubyCore;
