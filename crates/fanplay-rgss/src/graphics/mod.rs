// SPDX-License-Identifier: GPL-2.0-or-later
//! Deterministic CPU software rendering shared by the desktop and headless
//! frontends.
//!
//! All operations run on caller-owned [`Bitmap`] pixel buffers, so output is
//! byte-identical on every platform and this module doubles as the reference for
//! the optional GPU compositor.
//!
//! Source layout (all modules are private except the public `tilemap`):
//! - `bitmap`: [`Bitmap`]/[`InvalidBitmap`] — copy-on-write pixel storage,
//!   revision tracking, RGBA import/export, GIF animation and KGL2 shims.
//! - `effects`: in-place pixel math (brightness, hue, viewport tone) plus the
//!   per-pixel sprite shader pipeline.
//! - `raster`: blitting, nearest-neighbor sampling and sprite/plane
//!   rasterization built on the effect helpers.
//! - `scene`: a small owned sprite scene for the standalone demo; RGSS runtime
//!   registries and viewport ordering live in `crate::engine`.
//! - `tilemap`: tile cell deduplication and ground/priority quad geometry
//!   consumed by the GPU compositor.
//! - `types`: shared [`Color`], [`Rect`], [`Tone`], [`SpriteFx`] and
//!   [`SpriteTransform`] values.
//!
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
