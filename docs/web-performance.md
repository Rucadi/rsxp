# Web synchronization and software rendering

The browser runs Ruby and the Rust engine in separate WebAssembly modules.
Reducing Ruby allocations and work before the bridge matters even when unchanged
sprites already avoid native calls. The changes here retain Magnus on desktop
and improve code shared with the browser.

## Changes

- The serialized sprite path now uses the same reusable 41-field snapshot cache
  as desktop synchronization. Unchanged sprites avoid rebuilding argument arrays,
  numeric conversion and command serialization. Changed sprites serialize once
  into the existing synchronization batch without a second cached argument array.
- Virtual getters still run, and the snapshot reads mutable Rect/Color/Tone
  components. Strings used for numeric coercion are copied into the snapshot;
  arbitrary coercible objects force a fresh submission. The serialized path
  continues checking changed geometry while sprites are hidden.
- Unscaled, unmirrored CPU blits use clipped contiguous row slices. They avoid
  the source-column allocation, coordinate division and per-pixel source bounds
  checks of the general scaled path. Integer alpha composition and rounding are
  shared between the paths.

The WeakRef resource registry remains unchanged. Direct weak-map experiments
did not pass the actual Ruby 3.3 WASM collection test and were excluded from the
shipped changes.

## Measured result

`tools/benchmark-web.mjs` runs the actual Ruby WASM -> JavaScript -> Rust WASM
pipeline under Node. It creates 256 sprites with an 8x8 partially transparent
bitmap inside a tinted 160x120 viewport. After 10 warmup updates, each measured
mode runs 80 iterations. Changing modes move one quarter of the sprites each
iteration. The render mode also runs the complete Graphics update.

Three runs per variant alternated before/after order. Compilation and the game
replay had finished before measurement. Medians on the development host:

| Workload | Before | After | Time reduction |
| --- | ---: | ---: | ---: |
| Unchanged sprite synchronization | 4.966 ms | 1.247 ms | 74.9% |
| Changing sprite synchronization | 6.742 ms | 3.657 ms | 45.8% |
| Changing sprites + update/render | 7.934 ms | 4.396 ms | 44.6% |

Ruby allocations in the update/render workload fell from 8,398 to 3,150 per
iteration (62.5%). The unchanged synchronization loop went from 5,376 allocations
to approximately 0.44 per iteration, including the measurement harness overhead.
Bridge-call counts remained the same: the improvement comes from less work
around those calls, rather than a different frame count or skipped rendering.

All six runs produced the identical framebuffer SHA-256:
`6d97b7835b4828004a680394449ed01c8cd1fb99ade8cf602bd7580d27bb186a`.
Raw runs and source/binary fingerprints are in
[web-sync.json](benchmarks/web-sync.json).

These are synthetic CPU-throughput measurements, not a claim of 44.6% faster
gameplay or a particular browser FPS. Node supplies a real WebAssembly runtime,
but the benchmark excludes DOM/canvas presentation, assets, startup, input
events and game-specific scripts. Startup and scene transitions need separate
measurements.

## Repeat and validate

```sh
# Inside the development shell, from the repository root:
cargo xtask web
npm --prefix web run benchmark

# Compare the legacy Ruby synchronization path using the same engine binary:
RSXP_SPRITE_CACHE=0 npm --prefix web run benchmark

# Compare saved before/after artifacts, including CPU renderer changes:
node tools/benchmark-web.mjs /path/to/before/pkg /path/to/before/rgss.rb

cargo test -p rsxp --lib --no-default-features
cargo test -p rsxp-desktop --test cli
RSXP_WEB_FIXTURE=rust/testdata/rgss_sync_lifecycle.rb node tools/benchmark-web.mjs
npm --prefix web test
```

The lifecycle fixture compares native, cached serialized and legacy serialized
paths on desktop. It also runs in Ruby WASM, checking unreachable resources,
disposal, overridden getters, mutable coercions, visibility and viewport changes.
A CPU parity test compares clipped contiguous blits with equivalent scaled
sampling across opacity values, transparent destinations and out-of-bounds
source areas. Existing GPU parity tests remain applicable.

A real-game browser-pipeline smoke run completed 4,000 Fiber resumes / 2,948
Graphics updates and produced a nonblank frame. It was still in the intro, so
this does not validate the reported stack overflow near the starter event.
That issue requires its Ruby backtrace and a reproduction of the same route.
The end-to-end runner now exits nonzero for Ruby failures instead of labeling
a nonblank last frame as a successful run after an exception.
