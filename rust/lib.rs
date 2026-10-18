// SPDX-License-Identifier: GPL-2.0-or-later
//! Platform-independent components of the standalone Rust engine.
//!
//! [`engine`] owns RGSS state and frame execution; [`host`] defines the platform
//! services supplied by desktop and browser frontends. [`graphics`] provides
//! software pixel operations and the optional `gpu` module provides composition
//! on the device. Ruby and frontend integration live outside this library.
#[cfg(feature = "game-api")]
pub mod api_bridge;
#[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
pub mod cable_club;
#[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
pub mod cable_unmod;
pub mod controls;
pub mod data;
pub mod engine;
#[cfg(feature = "gpu")]
pub mod gpu;
pub mod graphics;
pub mod media;
pub mod patches;
pub mod runtime_settings;
/// Shared egui workspace used by both the native and browser frontends.
#[cfg(feature = "ui")]
pub mod ui;

// Preserve existing public paths while grouping implementations by ownership.
pub use data::{archive, table};
pub use engine::{host, input};
pub use graphics::tilemap;
pub use patches as runtime_patch;

#[cfg(feature = "ui")]
pub mod workspace_services;
