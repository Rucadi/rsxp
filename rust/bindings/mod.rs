// SPDX-License-Identifier: GPL-2.0-or-later
//! Desktop Magnus bindings for Ruby-owned objects and typed engine calls.
//!
//! This is the native boundary between the embedded Ruby VM and the `rsxp`
//! engine on desktop builds. It defines the RGSS-facing classes (`Table`,
//! `Sprite`, `Viewport`, `Plane`, `Tilemap`, `Win32API`), the internal
//! `RSXPRetainedMap` cache, and the `__rsxp_*` global helpers the Ruby shim
//! calls to submit snapshots, dependency changes, bulk scrolls and world/camera
//! updates. Nothing here retains a Ruby object across a frame: native state is
//! plain IDs and numbers, mapped onto the engine's `Retained` structures.
//!
//! Shared data/algorithms stay in the library (`data`, `graphics`).
//! The RGSS Ruby API lives in `rust/rgss.rb`; the serialized callback and its
//! timing live in `runtime::bridge`. Browser Ruby uses that shared RGSS shim
//! without depending on these desktop-only Magnus bindings.
//!
//! Submodules: `retained` (change-driven sync plus the bundled `retained.rb`
//! adapter), `retained_map` (Essentials visible-cell cache), `scroll` (bulk tile
//! scrolling), `sprite` (typed sprite snapshots), `table` (native `Table`),
//! `tilemap` (RGSS1 tile plans), `world` (lazy world-space coordinates) and
//! `win32` (legacy native `Win32API`).
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
///
/// `Table` is bound first because later modules reference it. `retained` (and
/// the `retained.rb` source it bundles) is installed after the sprite/viewport/
/// plane classes whose scalar setters it defines.
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
