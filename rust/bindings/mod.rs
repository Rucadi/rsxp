// SPDX-License-Identifier: GPL-2.0-or-later
//! Desktop Magnus bindings for Ruby-owned objects and typed engine calls.
//!
//! Shared data/algorithms stay in the library (`data`, `graphics`).
//! The RGSS Ruby API lives in `rust/rgss.rb`; the serialized callback and its
//! timing live in `runtime::bridge`. Browser Ruby uses that shared RGSS shim
//! without depending on these desktop-only Magnus bindings.
use magnus::{Error, Ruby};

mod retained;
mod retained_map;
mod scroll;
mod sprite;
mod table;
mod tilemap;
mod win32;
mod world;

/// Install native objects before evaluating the Ruby RGSS compatibility shim.
pub(crate) fn bind(ruby: &Ruby) -> Result<(), Error> {
    table::bind(ruby)?;
    sprite::bind(ruby)?;
    scroll::bind(ruby)?;
    tilemap::bind(ruby)?;
    world::bind(ruby)?;
    retained::bind(ruby)?;
    retained_map::bind(ruby)?;
    win32::bind(ruby)
}
