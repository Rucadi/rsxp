# Desktop Ruby profiling

## Native CPU profiling with perf

`bootstrap.toml` configures builds of the Rust compiler itself. This engine uses
Cargo: `[profile.release]` enables `debug = 1` and disables stripping while
keeping release optimizations. Nix packaging also preserves debug information.
The engine has no custom global allocator, so no allocator setting is needed.

Build and record from the repository root inside `nix-shell` or `nix develop`.
The Linux `perf` tool must also be installed on the host:

```sh
cargo build --release
perf record -F 199 --call-graph dwarf,16384 -- target/release/rsxp run --game ericlostie --renderer wgpu
perf report -i /tmp/rsxp-perf.data
```

Play the scene to investigate, then close the game to finish recording. DWARF
stack unwinding avoids requiring frame pointers in this build. Use the direct
binary so the recording does not include Cargo compilation. Keep that binary
available until the report has been analyzed; rebuilding can invalidate symbol
resolution for an older recording.

`perf` samples native CPU execution, including Rust and MRI. GPU execution time
requires GPU-specific profiling, and Ruby method attribution is better served
by the StackProf workflow below.

The desktop runtime can sample MRI stacks with
[StackProf](https://github.com/tmm1/stackprof), independently of the F10 frame
timings and `RSXP_PROFILE` native-command counters. The Nix package and
development shell provide a pinned StackProf 0.2.28 built for the same Ruby ABI.
Outside Nix, install that gem for the Ruby interpreter used to build the engine.
Sampling is opt-in; ordinary gameplay does not load the profiler extension.

## Record gameplay

Run an optimized build. Choose a delay long enough to load your save, then play
the scene you want to investigate:

```sh
nix run path:. -- run --game ericlostie --renderer wgpu \
  --ruby-profile profiles/overworld.dump \
  --ruby-profile-start-frame 1800 --ruby-profile-frames 1800

# Or, inside nix develop/nix-shell:
cargo run --release -- run --game ericlostie --renderer wgpu \
  --ruby-profile profiles/game.dump
```

`--ruby-profile-start-frame N` skips N completed Graphics updates, counting
independently of `Graphics.frame_count` (which games may reset). The default 0
includes boot and script loading. `--ruby-profile-frames N` saves after N
recorded updates while gameplay continues. Without this limit, closing the
game saves the profile. Early exit and Ruby exceptions also flush an active
recording. Exiting before the requested start prints a warning instead.

Paths resolve against the launch directory, and missing parent directories are
created. The requested file and its sidecars are overwritten when saved:

| File | Contents |
| --- | --- |
| `game.dump` | Standard StackProf Marshal data with raw stacks for further analysis |
| `game.dump.txt` | Top methods by self samples, inclusive samples, and source locations |
| `game.dump.json` | Method rankings, sample/miss/GC counts, allocation and GC-cycle counts, recording window, elapsed wall time and YJIT status |

The engine prints when recording starts and when the files have been saved.
Record a longer window if the report warns about fewer than 1,000 samples.

CPU sampling is the default. `--ruby-profile-mode wall` also captures waiting
and pacing, which helps distinguish CPU work from stalls. The interval defaults
to 1,000 microseconds and can be set with `--ruby-profile-interval US`
(100–1,000,000). Smaller intervals increase measurement overhead.

## Read the results

Inside the development shell:

```sh
stackprof profiles/game.dump --text --limit 30
stackprof profiles/game.dump --method 'TilemapRenderer#update'
stackprof profiles/game.dump --stackcollapse > profiles/game.folded
```

Self samples identify the method executing when sampled. Total/inclusive samples
also include its callees; do not add parent and child percentages together.
GC appears as separate marking/sweeping entries. Native entry points appear as
`<cfunc>`: their samples include Rust/C work and any Ruby getters they invoke.
Virtual source names such as `TilemapRenderer` refer to compressed game-script
section names; `eval` refers to the embedded `rust/rgss.rb` shim.

Compare the sampling report with the F10 Ruby/bridge/native/render/presentation
timings, or enable `RSXP_PROFILE=1` for native operation totals. Typed sprite
calls are included as `sprite_sync_native`; their entry-point time includes
Ruby getter conversion and is attributed to the native API category.

Sampling identifies candidates; validate speedups with profiling disabled and
the same scene, inputs, renderer, pacing, warmup and frame count. Measure walking,
menus and battles separately. Keep YJIT settings consistent across runs.

## Measured desktop optimizations

A CPU profile of the existing save's map 2 (50 events) identified
`Sprite#__sync` at 41.8% inclusive samples, including 20.8% self samples. GC was
17.7%. This synchronization code rebuilt arrays, converted properties into
strings, and serialized a full snapshot every frame before submitting changed
values to the engine.

Desktop sprite synchronization now uses a typed Magnus entry point in
`rust/bindings/sprite.rs`. Ruby getters, including subclass overrides, are still
called on the Ruby side, allowing YJIT to inline ordinary accessors. Their values
are passed directly on the Ruby argument stack, without a snapshot Array or
individual Rust-to-Ruby calls for each sprite field. The converted snapshot goes
directly into the Rust registry, removing
the argument arrays, serialization, parsing and cached Ruby snapshot array.
Ruby conversions finish before borrowing the runtime, so GC finalizers can
safely reenter the command bridge. The serialized implementation remains
available with `RSXP_SPRITE_SYNC=ruby` for A/B tests and troubleshooting.

The reusable snapshot cache also runs on the serialized browser path. Unchanged
sprites skip argument-array allocation, numeric conversion and serialization;
changed sprites queue a single command using the resolved snapshot. Set
`RSXP_SPRITE_CACHE=0` to compare against the legacy Ruby synchronization
path. The historical measurements below predate this shared cache.

The follow-up optimization also restores RGSS value comparison (`==`, `===`,
`eql?`) for Color, Tone and Rect, matching the existing C++ bindings. In the map
renderer, comparisons against cloned Color/Tone values previously always failed,
causing every pooled tile sprite to receive duplicate tint updates each frame.
Component comparisons let that existing game code skip unchanged values while
still noticing fractional channel changes and in-place mutations. Color/Tone
setters now use direct instance-variable writes and `clamp`, removing dynamically
constructed variable names, reflective assignment and temporary clamp arrays.

Three alternating runs of the original Ruby path, initial typed port, and final
optimized path measured the same idle saved-game scene after 150 warmup updates
and 1,500 measured updates per run. The desktop runtime ran headless and
uncapped, with saving disabled. Sampling and concurrent builds were disabled;
GPU work was fenced before and after timing. Medians on the RTX 2080 host were:

| Metric | Original | Initial typed port | Optimized |
| --- | ---: | ---: | ---: |
| Wall time/update | 6.159 ms | 4.905 ms | 2.279 ms |
| Process CPU time/update | 6.148 ms | 4.880 ms | 2.273 ms |
| Ruby allocations/update | 16,938 | 7,535 | 2,797 |

The combined changes reduce update time by about 63% (2.7× uncapped throughput)
and allocations by 83% for this workload. They reduce update time by a further
54% compared with the initial typed port. Results do not imply the same gain in
every scene or higher FPS during capped play. Full measurements and methodology
are in `docs/benchmarks/ruby-sync.json`.

A final 2,800-update CPU recording captured 1,521 samples with 0.13% missed.
The redundant Color/Tone setter loops disappeared from the top methods;
TilemapRenderer#update fell from 21.9% inclusive in the initial typed-port
recording to 6.4%. The remaining typed sprite entry point is 37.7% inclusive and
the serialized native bridge is 18.7%; these include native engine work.
Profiles and reports from this checkout are under ignored `profiles/`.
A battle or another map may identify different hotspots.


## Repeat the saved-game benchmark

`tools/benchmark-anil.rb` loads a supplied Añil save directly into its scene,
disables game saving, waits for warmup, fences GPU work, and prints measured wall
and process CPU time, allocations and GC cycles. Run it from the repository root
inside the development shell:

```sh
RSXP_BENCH_SAVE="$HOME/.local/share/rsxp/Pokemon Anil/Partida 1.rxdata" \
  target/release/rsxp run --script tools/benchmark-anil.rb \
  --headless --renderer wgpu --uncapped --frames 1650
```

The default warmup is 150 updates (`RSXP_BENCH_WARMUP` overrides it). The game
defaults to this checkout's `ericlostie` directory (`RSXP_BENCH_GAME` overrides
it). The harness is specific to this game's Essentials startup and compressed
script archive. It does not include menu navigation or startup in the timed
window. Keep other CPU/GPU workloads stopped while comparing runs. Add the
sampling flags above to locate current hotspots, then rerun without sampling to
measure throughput. Use `RSXP_INPUT` scripted inputs for reproducible movement
and check the printed initial/final map and position.

## Regression checks

```sh
cargo test --test cli typed_sprite_sync_matches_ruby
cargo test --test cli ruby_sampler -- --ignored --nocapture
```

The first test compares rendered pixels through both sprite paths, including
effects, mutable value objects, overridden getters, coercions, and GC/finalizer
reentry. The second needs StackProf and checks delayed/bounded windows,
`Graphics.frame_count` resets, report files and flushing after a Ruby exception.
