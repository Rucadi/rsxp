// SPDX-License-Identifier: GPL-2.0-or-later
//! `FANPLAYRetainedMap`: the Ruby-visible handle to the engine's retained
//! Essentials visible-cell cache (`fanplay_rgss::engine::retained_map::RetainedMap`).
//!
//! The class is `#[magnus::wrap]`ped over `RefCell<RetainedMap>` and exposes
//! three methods to `retained.rb`:
//!
//! * `update(width, height, maps, force)` — `maps` is an array of per-connected-
//!   map descriptors `[identity, Table, x, y, width, height, data_identity]`,
//!   where `identity`/`data_identity` are Ruby `object_id`s of the map object and
//!   its `Table`. Each `Table` is borrowed, wrapped as a `MapView` carrying the
//!   table's `revision`, and diffed by `RetainedMap::update`. The returned Ruby
//!   array holds one `[x, y, layer, map_index_or_nil, tile]` entry per changed
//!   cell; `nil` marks a now-uncovered cell.
//! * `shift(x, y)` — rotates retained cells to follow Essentials' reused sprite
//!   columns/rows, so the next `update` reports only newly exposed strips.
//! * `chunk_stats` — `[builds, hits, checks, evictions, resident]` counters.
//!
//! The chunk cache is enabled unless `FANPLAY_MAP_CHUNKS=0`. Ruby calls and engine
//! borrows happen outside the `RefCell` borrows; an uninitialized `Table` raises
//! `RuntimeError` and engine limits raise `ArgumentError`.
use super::table::RubyTable;
use fanplay_rgss::engine::retained_map::{MapView, RetainedMap};
use magnus::{method, prelude::*, Error, RArray, Ruby};
use std::cell::RefCell;

#[magnus::wrap(class = "FANPLAYRetainedMap", free_immediately, size)]
struct RubyRetainedMap(RefCell<RetainedMap>);
impl Default for RubyRetainedMap {
    fn default() -> Self {
        let mut map = RetainedMap::default();
        map.set_chunks_enabled(std::env::var("FANPLAY_MAP_CHUNKS").as_deref() != Ok("0"));
        Self(RefCell::new(map))
    }
}
impl RubyRetainedMap {
    fn stats(&self) -> (u64, u64, u64, u64, usize) {
        let stats = self.0.borrow().chunk_stats();
        (
            stats.builds,
            stats.hits,
            stats.checks,
            stats.evictions,
            stats.resident,
        )
    }
    fn shift(&self, x: usize, y: usize) {
        self.0.borrow_mut().shift(x, y);
    }
    fn update(
        ruby: &Ruby,
        this: &Self,
        width: usize,
        height: usize,
        maps: RArray,
        force: bool,
    ) -> Result<RArray, Error> {
        let mut inputs = Vec::new();
        for index in 0..maps.len() {
            let map: RArray = maps.entry(index as isize)?;
            inputs.push((
                map.entry::<u64>(0)?,
                map.entry::<&RubyTable>(1)?,
                map.entry::<i64>(2)?,
                map.entry::<i64>(3)?,
                map.entry::<usize>(4)?,
                map.entry::<usize>(5)?,
                map.entry::<u64>(6)?,
            ));
        }
        // No Ruby API calls or engine borrows while tables/cache are borrowed.
        let changes = {
            let tables: Vec<_> = inputs.iter().map(|input| input.1 .0.borrow()).collect();
            let mut views = Vec::new();
            for (input, table) in inputs.iter().zip(&tables) {
                let data = table.as_ref().ok_or_else(|| {
                    Error::new(ruby.exception_runtime_error(), "uninitialized map Table")
                })?;
                views.push(MapView {
                    identity: input.0,
                    data_identity: input.6,
                    revision: input.1.revision(),
                    data,
                    x: input.2,
                    y: input.3,
                    width: input.4,
                    height: input.5,
                });
            }
            this.0.borrow_mut().update(width, height, &views, force)
        }
        .map_err(|error| Error::new(ruby.exception_arg_error(), error))?;
        let result = ruby.ary_new_capa(changes.len());
        for change in changes {
            let cell = change.cell;
            result.push((
                change.x,
                change.y,
                change.layer,
                cell.map(|cell| cell.map),
                cell.map_or(0, |cell| cell.tile),
            ))?;
        }
        Ok(result)
    }
}
pub(super) fn bind(ruby: &Ruby) -> Result<(), Error> {
    let class = ruby.define_class("FANPLAYRetainedMap", ruby.class_object())?;
    class.define_alloc_func::<RubyRetainedMap>();
    class.define_method("chunk_stats", method!(RubyRetainedMap::stats, 0))?;
    class.define_method("shift", method!(RubyRetainedMap::shift, 2))?;
    class.define_method("update", method!(RubyRetainedMap::update, 4))?;
    Ok(())
}
