// SPDX-License-Identifier: GPL-2.0-or-later
//! Deterministic software rendering shared by desktop, browser and headless frontends.
//!
//! Types and bitmap ownership are separate from rasterization and pixel effects.
//! Public re-exports keep callers independent of the internal source layout.
mod bitmap;
mod effects;
mod raster;
mod scene;
#[cfg(test)]
mod tests;
pub mod tilemap;
mod types;

pub use bitmap::{Bitmap, InvalidBitmap};
pub use scene::{Scene, Sprite};
pub use types::{Color, Rect, SpriteFx, SpriteTransform, Tone};
