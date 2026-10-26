# Change-driven graphics, retained map chunks and a native camera

> Historical record. Measurements and implementation details below describe the
> recorded workload/revision, not the current support matrix. Names and commands
> have been adapted to Fanplay; timings have not been re-measured. See
> [current architecture](architecture.md) and [development](development.md).

Desktop execution enables change-driven graphics synchronization and a native
visible-cell cache for the supported Essentials `TilemapRenderer`. Game scripts,
script bundles, plugins and save formats do not need modification. The game
still owns its Ruby scene loop; this change does not introduce a Rust scheduler.

## Ownership

The engine's existing `SpriteState` and retained scene remain the authoritative
resolved rendering state. Ruby objects retain their public properties and value
objects. Frequent scalar setters execute directly in Rust; complex setters keep
their existing Ruby validation/copy behavior. Desktop observers notify a native
dirty-resource registry when those
properties change; only dirty objects resolve and submit a new snapshot.

`cores/rgss/ruby/src/bindings/retained.rs` stores IDs, dirty sets, and resource dependencies. It
stores no Ruby `Value`s or strong Ruby object references. A mutex shares change
notifications across Ruby threads. Ruby conversions and property access happen
outside its lock and outside engine borrows. `retained.rb` supplies the API
observers, weak resource registry, compatibility selection, and game adapter.

Nested `Rect`, `Color` and `Tone` mutations invalidate their dependents. Copying
one of these values keeps independent state and a distinct dependency token;
Marshal payloads are unchanged. Viewport changes and bitmap/viewport disposal
invalidate dependent sprites. Flash updates invalidate their appearance only
when the game's existing `update` method advances the flash.

Ordinary base graphics objects and compatible Essentials tile sprites can skip
per-frame polling. Other subclasses, overridden graphics accessors, custom sync
methods, and mutable/coercible scalar inputs retain getter-based synchronization.
Late method changes trigger compatibility reevaluation. Alias-based plugin
wrappers retain their captured original implementations. An explicit failed
synchronization is retried if the game rescues the error.

This is retained native render state with change notifications, **not** a full
replacement of Ruby property/value storage with Magnus native objects. Changed
objects still use the existing validated snapshot conversion. Arbitrary direct
instance-variable writes inside additional game methods are not observable by
setter notifications; such integrations should use the compatibility switch.
Explicit `instance_variable_set` on a graphics property is tracked and switches
the resource to polling.

## Retained maps

`crates/fanplay-rgss/src/engine/retained_map.rs` selects visible cells using native `Table` storage.
The desktop binding supplies map/table identity, table revision, dimensions and
camera tile offsets. An unchanged input returns without traversing the grid.
Changed inputs resolve the grid in Rust and return only changed cells to Ruby.
At tile boundaries the cache rotates with the game's reusable sprite pool, so
existing world cells are retained and only newly exposed rows/columns refresh.
Later connected maps win overlapping cells, matching the existing renderer.
Uncovered cells are explicitly cleared.

`Table` writes increment a native revision only when a valid cell changes.
Resize/reinitialization/copy invalidates revisions; replacing a table invalidates
its identity. Revisions do not enter Table's Marshal payload.

The adapter calls the game's existing `refresh_tile`, source-rectangle and
priority helpers, preserving terrain/bridge/reflection and resource behavior.
It retains animated-tile membership rather than finding animated tiles through
a full Ruby map traversal every frame.

`crates/fanplay-rgss/src/engine/map_chunks.rs` caches 16×16×3 tile-ID chunks in world coordinates.
It prefetches a 16-tile margin around the viewport, including nearby connected
maps. Hidden chunks remain cached and are reused when revisited. The default
budget is 128 chunks (192 KiB of tile-ID payload, plus bookkeeping); least
recently used inactive chunks are evicted first. An active viewport/prefetch
working set larger than the budget is preserved to avoid churn. Small maps can
therefore be warmed entirely, while large maps do not allocate a sprite or
texture for every hidden tile. A Table revision validates cached chunk contents;
only chunks whose cells changed replace their data. Hidden stale chunks are
validated before reuse.

`crates/fanplay-rgss/src/engine/world_map.rs` retains world placements for compatible tile sprites
and applies one shared camera. Ordinary pixel scrolling updates this camera
without walking the Ruby sprite pool or rewriting every tile's x/y/z. Tile
boundaries bind only newly exposed cells. Ruby's public x/y/z getters resolve
screen coordinates lazily, preserving normal script access. External coordinate
or priority/bridge/reflection changes detach the tile, materialize its current
coordinates, and select the conservative scrolling path until a full refresh.
Disposal and compatibility fallback release the camera bindings.

Bridge/reflection/priority ordering is preserved. Unsupported tile properties
and overridden helpers use the previous native bulk-scroll or Ruby path. The
GPU still encodes/projects visible draw geometry each frame; this is not a cache
of complete GPU command buffers. Existing atlas, geometry and scene caches are
reused. Eliminating Ruby tile objects entirely remains future work.

GPU sprite/plane culling happens before pending native tiles are flushed.
Off-screen, fully transparent and zero-sized objects do not split tile runs;
visible objects remain ordering barriers. This reduces unnecessary tile draws
without merging across characters or other draws that affect pixels.

Replacing `TilemapRenderer#update`, using an unsupported renderer layout, or
overriding native Table reads keeps the original renderer path. Explicit map
`refresh` remains the invalidation contract for tileset/terrain/priority metadata
changes, as it was in the original game. Per-frame plugin side effects that rely
on every tile helper being invoked should use the map compatibility switch.

## Comparison switches

- `FANPLAY_NATIVE_STATE=0`: restore full Ruby graphics-object polling.
- `FANPLAY_RETAINED_MAP=0`: restore Essentials' original map update.
- `FANPLAY_RETAINED_TILEMAP=0`: disable the separate desktop RGSS1 tilemap cache.
- `FANPLAY_NATIVE_TILEMAP_BATCH=0`: keep that cache but apply full refreshes with Ruby setters.
- `FANPLAY_WORLD_CAMERA=0`: disable shared-camera scrolling.
- `FANPLAY_MAP_CHUNKS=0`: disable chunk prefetch/retention.
- `FANPLAY_NATIVE_SCROLL=0`: keep retained maps but use Ruby scrolling helpers.
- `FANPLAY_NATIVE_TILEMAP=0`: independently disable the existing tile atlas adapter.
- `FANPLAY_SPRITE_SYNC=ruby`: use the existing serialized binding, which also
  disables the new change-driven state path.

The browser keeps its existing graphics synchronization. The visible-cell
algorithm is portable, but its new Ruby binding is desktop-only.

## Verification

`cargo test -p fanplay-desktop --test cli retained_native_state_matches_legacy_frames`
compares a repository-owned RGSS fixture across eight combinations of state,
map, native scrolling, world camera and chunk caching. Other hermetic CLI tests
cover the legacy retained tilemap, sprite lifecycle, native tiles and rendering
paths.

`cargo test -p fanplay-rgss --lib --no-default-features` exercises the portable map cache without
Ruby or GPU dependencies. Existing sprite/lifecycle and native-tile integration
tests continue to cover the original synchronization and rendering paths.

The portable cache also checks that diagonal/reverse scrolling refreshes only
exposed strips. The GPU regression
`gpu::tests::culled_sprites_and_planes_do_not_split_native_tile_draws` verifies
that invisible draws between tiles preserve a single native tile batch and
identical output pixels.

The portable tests cover chunk prefetch reuse, hidden-chunk retention, targeted
rebuilds after edits, budget eviction and camera projection/priority rules.

A three-run alternating release benchmark of the saved overworld at panoramic
zoom 3 (1536×1152, scripted walking, RTX 2080, YJIT) reduced median measured
frame time from 1.7650 to 1.4117 ms (20.0%). All six output hashes and final
positions matched. Nine chunks were built, with no evictions. Allocations rose
slightly, from 842 to 862 per frame; no measured GC cycles occurred. This is a
short headless workload, not an interactive FPS guarantee. Raw measurements are
in `docs/benchmarks/ericlostie-world-chunks.json`.

Older Essentials games such as Pokémon Z use the RGSS1 Tilemap path rather than
`TilemapRenderer`. Its retained layout, native scroll batch, immediate-float
tracking fix and filesystem-index optimization are documented separately in
`docs/pokemonz-performance.md`.
