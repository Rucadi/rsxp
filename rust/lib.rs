// SPDX-License-Identifier: GPL-2.0-or-later
//! Platform-independent core of the standalone Rust engine for RPG Maker RGSS
//! games.
//!
//! This library is the reusable engine, not a frontend: the native desktop
//! binary and the WebAssembly browser host both link it and supply platform
//! services. Ruby integration (the Magnus bindings and `rgss.rb`) lives outside
//! the library.
//!
//! ## Module map
//! * [`engine`] — RGSS runtime state and frame execution (bitmaps, sprites,
//!   viewports, input and the native command dispatcher). Its platform-service
//!   traits and input types are re-exported as [`host`] and [`input`].
//! * [`graphics`] — deterministic CPU software rendering, plus the public
//!   [`tilemap`] helpers; the reference for the optional GPU compositor.
//! * [`gpu`] *(feature `gpu`)* — device-side composition on the GPU; CPU
//!   bitmaps remain authoritative for pixel access.
//! * [`data`] — RGSS formats independent of Ruby and rendering: encrypted
//!   archives ([`archive`]) and the signed 16-bit [`table`] type.
//! * [`media`] — pure-Rust cross-platform animated GIF and Theora (`.ogv`)
//!   decoders.
//! * [`patches`] — the runtime-patch catalog and per-thread enabled/parameter
//!   state, re-exported at the crate root as [`runtime_patch`].
//! * [`runtime_settings`] — thread-local live settings (frame-rate cap,
//!   fast-forward, reset) a host UI can change while the game runs.
//! * [`controls`] — the shared gamepad-to-RGSS-action mapping.
//! * [`workspace_services`] *(feature `ui`)* — the default
//!   [`ui::backend::Backend`] adapter bridging the egui workspace to the engine
//!   and host.
//! * [`api_bridge`] *(feature `game-api`)* — the local game-command queue and
//!   HTTP routing used by the desktop and browser tools.
//! * [`cable_club`] / [`cable_unmod`] *(feature `game-api`, non-WASM)* — the
//!   Cable Club peer-relay servers.
//! * [`ui`] *(feature `ui`)* — the shared egui workspace rendered by both the
//!   native and browser frontends.
//!
//! ## Features
//! `desktop` builds the native frontend and enables `game-api`; `ui` enables the
//! shared egui workspace; `gpu` enables the device compositor without pulling in
//! egui, so native/mobile frontends supply their own presentation; `game-api`
//! enables the command bridge; `api-docs` enables the code-first OpenAPI schema
//! used by the `rsxp-mcp` binary.
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
#[cfg(feature = "portable-media")]
pub mod portable_media;
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
