// SPDX-License-Identifier: GPL-2.0-or-later
//! The RGSS-only embedded MRI runtime and its native engine bindings.
//!
//! The desktop build initializes MRI through Magnus and runs the game to
//! completion. The mobile build (`mobile` feature) keeps MRI alive and exposes
//! a fiber-pumped lifecycle for the cargo-mobile2 shell.

#[cfg(feature = "native-bindings")]
mod bindings;
#[cfg(feature = "desktop")]
mod bridge;
#[cfg(feature = "desktop")]
mod desktop;
#[cfg(feature = "mobile")]
pub mod mobile;

#[cfg(feature = "desktop")]
pub use desktop::RubyCore;
