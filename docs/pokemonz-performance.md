# Pokémon Z renderer performance

> Historical record. Measurements and implementation details below describe the
> recorded workload/revision, not the current support matrix. Names and commands
> have been adapted to Fanplay; timings have not been re-measured. See
> [current architecture](architecture.md) and [development](development.md).

Pokémon Z uses an older Essentials renderer than Ericlostie. At normal zoom,
its `CustomTilemap` paints cached bitmap layers. The panoramic patch selects
`SynchronizedTilemap`, which uses the engine's RGSS1 `Tilemap` path. Optimizations
to the newer `TilemapRenderer` alone therefore did not address this game.

## Identified costs

- Missing or differently cased asset-name probes repeatedly walked filesystem
  directories. Initial CPU samples at normal zoom attributed about 59% of self
  samples to `Dir.children` and `String#casecmp?`, plus substantial garbage
  collection from temporary filename strings.
- RGSS1 `Tilemap#__sync` traversed visible cells and rewrote tile sprite
  properties on every frame. At zoom 3, roughly 4,000 tile pieces were active.
- The legacy screen-resizer assigns floating-point viewport offsets repeatedly.
  Native scalar tracking used Magnus `RFloat`, which excludes immediate Ruby
  floats. Values such as `0.0` were treated as unsupported mutable inputs and
  as changes, invalidating all dependent tile sprites every frame.
- Compatibility polling also touched resources unconditionally. A polled
  viewport could therefore invalidate every tile even without a state change.

These are CPU costs; the initial zoom-3 native profile spent about 0.4 seconds
in `graphics_update` across 240 updates, versus roughly 9 seconds in the
measured game loop. The RTX 2080 was not the main limit in this workload.

## Changes

Filesystem preflight now indexes directory names using Unicode case folding.
Directory modification time, change time and inode invalidate the index, so
negative probes notice new files, renames and deletions. At most 256 directory
indexes are retained. Absolute paths keep relative lookups independent of cwd.

Desktop RGSS1 tilemaps retain their visible layout. Table identity/revision,
priority revisions, viewport dimensions, tile-edge coverage,
bitmap replacement/disposal, visibility, color and tone invalidate it. Pixel
scrolling within the same coverage uses one native batch to update geometry and
Ruby-visible coordinates, including RGSS1's pixel-dependent priority z. Hidden
pool entries keep their previous coordinates. The tile helper's original arity
is preserved.

Engine-owned tile sprites capture the engine Sprite API before game plugins
install public Sprite screen-resizer wrappers. Ordinary game sprites retain
those wrappers. Non-unit legacy resize factors and viewport-free border offsets
use ordinary sprites; fractional camera offsets, custom Table readers and
custom tile helpers keep the conservative layout path. Pool ownership switches
are handled by disposing/recreating the pool.

Native scalar tracking now recognizes both immediate and heap Ruby floats.
Equivalent numeric assignments keep their Ruby values without marking resources
changed. Compatibility viewport polling compares copied property values before
notifying dependents; ordinary setter, nested mutation, flash and disposal
notifications remain active.

`FANPLAY_RETAINED_TILEMAP=0` disables legacy tilemap layout retention and internal
tile sprites for comparisons. The existing native-state switch still works.
Game scripts and save formats are unchanged.

## Reproduction and verification

`tools/benchmark-pokemonz.rb` loads an existing save through the real Continue
loader into a temporary save directory and runs the restored overworld. It
reports frame time, 95th percentile, worst frame, allocations, player positions,
renderer classes and retained-state counters. Set `FANPLAY_BENCH_ZOOM=3`,
`FANPLAY_YJIT=1`, `FANPLAY_INPUT=100:79:40,170:80:40` and run headless with wgpu and
`--frames 240`. The default warmup is 60 updates; the remaining 180 are measured.
`FANPLAY_POKEMONZ_ROOT` and `FANPLAY_POKEMONZ_SAVE` select alternate local assets/save.

The CLI regressions compare per-frame coordinates and pixels against rebuilt
legacy tilemaps across scrolling, tile boundaries, layered autotiles, animation,
Table edits/resizes, visibility, bitmap mutations/disposal and polled viewport
effects. They also check that repeated immediate floats do not dirty child
sprites and that changed floats still synchronize. Directory-index tests cover
creation, rename, deletion, direct reads and cwd changes. An ignored GPU test
compares actual retained GPU output with rebuilt software frames.

## Measured result

Three alternating before/after release runs on the same save and scripted walk
with YJIT and the RTX 2080 produced these median frame averages:

| View | Before | After | Speedup |
| --- | ---: | ---: | ---: |
| Normal, 512×384 | 13.43 ms | 2.02 ms | 6.6× |
| Panoramic 3, 1536×1152 | 47.97 ms | 5.78 ms | 8.3× |

All twelve final images matched exactly within their zoom level, and every run
visited the same five player positions. Zoom-3 sprite synchronizations fell from
943,099 to 57,140 across the full run. Allocations fell from about 74,112 to 3,799
per measured frame; normal zoom fell from about 61,456 to 1,942.

Full layout refreshes on animation steps and viewport-coverage changes remain.
Median zoom-3 p95 improved from 56.5 to 25.3 ms; worst after frames were about
52 ms. Those spikes are a remaining limit, rather than a claim of uniformly
5.8 ms frames. These are short, headless uncapped results, not an interactive
FPS guarantee for every map. Raw runs and initial CPU profile summaries are in
`docs/benchmarks/pokemonz-retained-performance.json`.

## Scrolling artifacts and refresh latency — 2026-10-18

A native bulk scroll updated Ruby coordinates and engine geometry but left
`Sprite#@__sync_state` at the last full submission. Returning to that position
while refreshing could make the comparison incorrectly skip a native update,
leaving a tile a few pixels out of place. Bulk scrolling now updates the cached
x/y/z slots too, and clears the older serialized snapshot cache. The regression
reproduces the failure with a scroll reversal and an unrelated priority-table
revision; it failed before the fix and now matches the rebuilt reference.

Animation frames no longer invalidate the entire visible layout. Full refreshes
record the animated tile pieces, frame strides and source offsets; animation
steps update only those source rectangles. Static tiles keep their geometry.

Tile-boundary refreshes still generate their plan through the existing helpers,
but compatible engine-owned sprites apply bitmap, rectangle, position, priority,
color, tone and visibility in one native call. Bitmap changes rewire resource
dependencies. All Ruby work finishes before borrowing the runtime; only numeric
render updates cross that boundary. Native updates clear stale full snapshots.
Class-level value overrides, incompatible/frozen sprites and custom tile helpers
retain the setter path. `FANPLAY_NATIVE_TILEMAP_BATCH=0` disables only native full
refresh application for comparisons.

The final three alternating release runs at zoom 3 measured these medians
against the previous release under the same current machine conditions:

| Frame metric | Previous release | Fixed release |
| --- | ---: | ---: |
| Average | 6.22 ms | 4.10 ms |
| 95th percentile | 25.99 ms | 7.60 ms |
| Worst frame per run | 57.32 ms | 27.65 ms |

Full layout refreshes fell from 19 to 5 across each run; 84 animated pieces were
updated separately. All final images and player paths matched. A separate
101-frame trace of actual zoom-3 walking matched uncached pixels and positions
on every frame, including reversals. GPU reference tests additionally capture
the reversal bug and an animation transition. Tests compare native batching,
Ruby setters and uncached rendering, with GC compaction enabled separately.

The remaining boundary/coverage rebuilds can still cause isolated slower frames;
this change reduces their cost rather than promising flat frame times on every
map. Raw measurements are in `docs/benchmarks/pokemonz-scroll-artifacts.json`.
Set `FANPLAY_BENCH_PIXEL_TRACE=1` on the benchmark harness to print per-frame pixel
hashes. That mode performs CPU snapshots and must not be used for timing claims.
