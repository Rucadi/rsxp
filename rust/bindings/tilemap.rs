// SPDX-License-Identifier: GPL-2.0-or-later
//! Apply engine-owned RGSS1 tile plans in one call. No Ruby values are retained.
//!
//! `__rsxp_apply_rgss_tiles(sprites, plan, color, tone, viewport, clip)` is the
//! batch entry point used by the legacy `Tilemap` adapter. It preflights that
//! every sprite is retained and not frozen, converts `color`/`tone` once, then
//! walks `plan`, a parallel array of `[bitmap, src_x, src_y, src_size, x, y, z,
//! priority]` rows. For each row it writes the tile's `@bitmap`, `@src_rect`,
//! `@x`/`@y`/`@z`, `@visible`, `@__rsxp_rgss_priority`, `@color` and `@tone`
//! ivars, clears the `@__sync_state`/`@__native_state` snapshot caches, calls
//! `RSXP::Retained.rewire` when the bitmap identity changed, and builds a
//! `NativeTileUpdate`. Sprites beyond the plan are hidden. One
//! `engine::with_runtime_mut` call applies the updates; the whole call runs under
//! `runtime::direct_call("tilemap_sync_native", ..)` for profiling. Returns
//! `false` (game fallback) on any unsupported sprite.
use magnus::{function, prelude::*, Error, RArray, RModule, RObject, Ruby, TryConvert, Value};
use rsxp::{
    engine::{self, NativeTileUpdate},
    graphics::{Color, Rect, Tone},
};

fn apply(
    ruby: &Ruby,
    sprites: RArray,
    plan: RArray,
    color: RArray,
    tone: RArray,
    viewport: Value,
    clip: RArray,
) -> Result<bool, Error> {
    crate::runtime::direct_call("tilemap_sync_native", || {
        for index in 0..sprites.len() {
            let tile: RObject = sprites.entry(index as isize)?;
            if !tile.ivar_get::<_, Value>("@__rsxp_retained")?.to_bool() || tile.is_frozen() {
                return Ok(false);
            }
        }
        let native_color = Color::rgba(
            super::sprite::channel(ruby, color.entry(0)?)?,
            super::sprite::channel(ruby, color.entry(1)?)?,
            super::sprite::channel(ruby, color.entry(2)?)?,
            super::sprite::channel(ruby, color.entry(3)?)?,
        );
        let native_tone = Tone {
            red: super::sprite::int(ruby, tone.entry(0)?)? as i32,
            green: super::sprite::int(ruby, tone.entry(1)?)? as i32,
            blue: super::sprite::int(ruby, tone.entry(2)?)? as i32,
            gray: super::sprite::int(ruby, tone.entry(3)?)? as i32,
        };
        let viewport = if viewport.to_bool() {
            RObject::try_convert(viewport)?.ivar_get::<_, u64>("@__id")?
        } else {
            0
        };
        let clip = Rect::new(
            clip.entry(0)?,
            clip.entry(1)?,
            clip.entry(2)?,
            clip.entry(3)?,
        );
        let rsxp: RModule = ruby.class_object().const_get("RSXP")?;
        let retained: RModule = rsxp.const_get("Retained")?;
        let mut updates = Vec::with_capacity(plan.len());
        for index in 0..plan.len() {
            let row: RArray = plan.entry(index as isize)?;
            let tile: RObject = sprites.entry(index as isize)?;
            let bitmap: RObject = row.entry(0)?;
            let bitmap_id: u64 = bitmap.ivar_get("@__id")?;
            let previous: Value = tile.ivar_get("@bitmap")?;
            let changed = RObject::from_value(previous)
                .map(|old| old.ivar_get::<_, u64>("@__id").map(|id| id != bitmap_id))
                .transpose()?
                .unwrap_or(true);
            let sx: i32 = row.entry(1)?;
            let sy: i32 = row.entry(2)?;
            let size: u32 = row.entry(3)?;
            let x: i64 = row.entry(4)?;
            let y: i64 = row.entry(5)?;
            let z: i32 = row.entry(6)?;
            let priority: Value = row.entry(7)?;
            tile.ivar_set("@bitmap", bitmap)?;
            let area: RObject = tile.ivar_get("@src_rect")?;
            area.ivar_set("@x", sx)?;
            area.ivar_set("@y", sy)?;
            area.ivar_set("@width", size)?;
            area.ivar_set("@height", size)?;
            tile.ivar_set("@x", x)?;
            tile.ivar_set("@y", y)?;
            tile.ivar_set("@z", z)?;
            tile.ivar_set("@visible", true)?;
            tile.ivar_set("@__rsxp_rgss_priority", priority)?;
            let owned_color: RObject = tile.ivar_get("@color")?;
            let owned_tone: RObject = tile.ivar_get("@tone")?;
            for (index, field) in ["@red", "@green", "@blue", "@alpha"]
                .into_iter()
                .enumerate()
            {
                owned_color.ivar_set(field, color.entry::<Value>(index as isize)?)?;
            }
            for (index, field) in ["@red", "@green", "@blue", "@gray"].into_iter().enumerate() {
                owned_tone.ivar_set(field, tone.entry::<Value>(index as isize)?)?;
            }
            // Geometry and appearance are now submitted natively. A later
            // notification must compare against a fresh complete snapshot.
            tile.ivar_set("@__sync_state", ())?;
            tile.ivar_set("@__native_state", ())?;
            if changed {
                retained.funcall::<_, _, Value>("rewire", (tile,))?;
            }
            updates.push(NativeTileUpdate {
                id: tile.ivar_get("@__id")?,
                bitmap: bitmap_id,
                src: Rect::new(sx, sy, size, size),
                x: x as f64,
                y: y as f64,
                z,
                visible: true,
                viewport,
                clip,
                color: native_color,
                tone: native_tone,
            });
        }
        let mut hidden = Vec::with_capacity(sprites.len().saturating_sub(plan.len()));
        for index in plan.len()..sprites.len() {
            let tile: RObject = sprites.entry(index as isize)?;
            tile.ivar_set("@visible", false)?;
            tile.ivar_set("@__sync_state", ())?;
            tile.ivar_set("@__native_state", ())?;
            hidden.push(tile.ivar_get::<_, u64>("@__id")?);
        }
        // All Ruby reads, allocations and dependency callbacks finish before
        // borrowing the runtime; finalizers may safely reenter the engine.
        engine::with_runtime_mut(|runtime| runtime.update_native_tiles(&updates, &hidden));
        Ok(true)
    })
}
pub(super) fn bind(ruby: &Ruby) -> Result<(), Error> {
    ruby.define_global_function("__rsxp_apply_rgss_tiles", function!(apply, 6))?;
    Ok(())
}
