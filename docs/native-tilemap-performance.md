# Native tile atlas and retained world-map rendering

> Historical record. Measurements and implementation details below describe the
> recorded workload/revision, not the current support matrix. Names and commands
> have been adapted to Fanplay; timings have not been re-measured. See
> [current architecture](architecture.md) and [development](development.md).

The desktop wgpu renderer now has a dedicated tile path for Essentials'
`TilemapRenderer::TileSprite`. This replaces individual tile dispatches with a
native cell atlas and retained ground/priority quad geometry. It also retains
scene ordering, batch schedules and GPU bindings, and avoids resolving every
weak reference twice per update.

The reference is fanplay commit
`37a04d1`.
Its `tilemap.cpp`
retains an atlas, ground vertices and priority layers and batches adjacent
layers. Its sprite setters update native state when values change. We apply
those ideas to this project's compute renderer and Essentials tile resources;
this is not a copy of its OpenGL rendering implementation.

## Native ownership and draw ordering

`crates/fanplay-rgss/src/graphics/tilemap.rs` owns the atlas and geometry:

- Cells are keyed by source bitmap ID and source rectangle. Multiple tile
  instances share one packed GPU atlas cell. Autotile frame selection refers
  to another cached cell, rather than uploading a whole expanded autotile or
  tall tileset. Source bitmap revisions invalidate the corresponding cells.
- Geometry contains compact 96-byte tile quads grouped into ground and
  priority z layers. Flat spatial bins cover only each run's occupied area
  and index only the quads that can cover a pixel. The tile shader reads
  compact fields directly and uses integer blending for unmodified cells,
  avoiding a full sprite-state copy per pixel.
- Adjacent tile layers share one compute draw. An intervening character,
  ordinary sprite, plane or viewport operation ends the run. A shader
  invocation composites all matching quads for one pixel in the original
  order, so overlapping transparent ground tiles do not race.
- Quad topology and bins persist until their extent, clipped bounds, priority
  or membership changes. Animation or pixel mutations update cell/quad data
  without rebuilding unchanged spatial bins. A run's indices are relocated
  when it moves within the frame's geometry buffer.
- Packed integer alpha/effect math is shared with ordinary sprites. Rotation,
  patterns and unusually large plugin tiles retain the ordinary sprite path.

The adapter registers Essentials tile resources as they are constructed.
Tile selection, connections between maps, bridge/reflection rules and plugin
callbacks still execute in the game's Ruby renderer. Their resulting native
quad ordering is preserved. This is the native atlas/geometry rendering path
for this game; it does not claim a complete implementation of the standard
RGSS Tilemap API or move all game map logic into Rust. Software/browser
composition keeps the existing sprite path.

`FANPLAY_NATIVE_TILEMAP=0` disables the adapter for A/B comparisons.
`FANPLAY_SCENE_CACHE=0` rebuilds native scene ordering each frame.

## Profile-driven synchronization improvement

One CPU profile of the walking world map at panoramic 3x recorded 3532 samples.
`Sprite.__sync_all` accounted for 51.4% inclusive and `Sprite#__sync` for
24.8% self: polling thousands of tile sprites remained a larger Ruby cost than
tile selection itself (20.9% inclusive). The profile was taken after the native
atlas/geometry implementation, before change-driven tile synchronization.

The retained implementation removes redundant weak-reference liveness lookups
and a second disposal predicate in `Sprite.__sync_all`. Native disposal remains
immediate; a custom sprite that disposes itself during synchronization has its
weak registry entry pruned on the following update.

An additional setter-driven dirty-state shortcut was implemented, measured and
**discarded**. Even after eliminating dynamic setter allocations, its guards
and setter interception increased walking time from 9.6145 to 10.6574 ms per
update in the focused comparison. Both captures were pixel-identical. The raw
[experiment](benchmarks/ericlostie-tile-dirty-experiment.json) is retained to
avoid presenting a shortcut-only comparison as an overall improvement.
Ordinary and tile sprites retain virtual-getter synchronization. Removing all
Ruby polling needs a more complete native property binding and mutable-value
notification design; this experiment is not shipped.

## Required workload: world map, panoramic 3x

Use the existing EricLostie/Pokémon Añil save (map 2, 50 events, starting at
21,28) and the benchmark harness. The harness sets the patch value, verifies
that panoramic mode is active at the requested zoom before measurement, and
prints the actual render dimensions. At 3x they must be **1536 x 1152**.
Saving is disabled by the harness.

```sh
FANPLAY_BENCH_SAVE="$HOME/.local/share/fanplay/Pokemon Anil/Partida 1.rxdata" \
FANPLAY_BENCH_ZOOM=3 FANPLAY_BENCH_FIXED_CLOCK=1 \
FANPLAY_BENCH_CLOCK_START=0 FANPLAY_BENCH_SEED=0 \
target/release/fanplay run --script tools/benchmark-anil.rb \
  --headless --renderer wgpu --uncapped --frames 1650
```

Use 150 warmup updates, then measure 1500 with GPU fences at both ends.
Alternate baseline and candidate runs without builds or sampling in parallel.
For walking add:

```sh
FANPLAY_INPUT=200:79:100,400:80:100,600:82:100,800:81:100,1000:79:100,1200:80:100,1400:82:100
```

`--output /tmp/map.ppm` captures the last frame. Compare hashes as well as the
printed final map/position. Throughput measurements exclude startup, native
window presentation and the RGSS pacing cap. The game runs faster in uncapped
wall-clock time; these measurements are not capped-play FPS promises.

## Regression coverage

- Atlas cell sharing and invalidation after bitmap writes.
- Ground/priority grouping, overlapping alpha, and characters between layers.
- Exact native-vs-sprite GPU output, including scrolling, animated source
  cells, mirror, scaled tiles and tint/tone effects.
- Retained geometry index relocation and animation without topology changes.
- Essentials adapter integration, priority changes for bridges/reflections,
  reparenting, disposal, resolution changes and plugin-effect fallback.
- Existing software/GPU parity, typed sprite getter overrides and GC tests.
- Scene ordering cache and GPU bindings across atlas/buffer growth and resize.

GPU tests require an adapter:

```sh
cargo test -p fanplay-rgss --lib gpu::tests -- --ignored --nocapture
cargo test -p fanplay-desktop --test cli native_tiles_match -- --ignored --nocapture
cargo test -p fanplay-desktop --test cli retained_scene_matches -- --ignored --nocapture
```
