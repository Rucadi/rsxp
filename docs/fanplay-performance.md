# FANPLAY performance investigation and EricLostie profiling

> Historical record. Measurements and implementation details below describe the
> recorded workload/revision, not the current support matrix. Names and commands
> have been adapted to Fanplay; timings have not been re-measured. See
> [current architecture](architecture.md) and [development](development.md).

Comparison made on 2026-10-15 against the upstream engine architecture.
Local baseline is the working checkout at the start of this investigation,
including its existing staged changes. This is a source comparison, not a
measured head-to-head race against a compiled C++ engine.

This report describes the initial investigation. The subsequent
[native tilemap implementation and panoramic 3x profile](native-tilemap-performance.md)
add retained scene ordering, native tile atlas/geometry/layer batching and
simplified sprite traversal; the architecture table below records the
earlier baseline.

## Different decisions

| Area | Upstream reference | FANPLAY | Performance implication |
| --- | --- | --- | --- |
| Ruby binding | Native C++ objects and property setters | Ruby resource wrappers, cached snapshots, typed Magnus sprite submission and a serialized command bridge | Scanning all sprites still costs CPU even when no native snapshot is submitted. The typed bridge already avoids most serialization for sprites. |
| Rendering | OpenGL textured quads and specialized shaders | wgpu compute over packed integer pixels | Integer composition preserves software rounding; compute has an ordered dispatch cost per draw. wgpu itself does not require compute: a raster backend is an alternative. |
| Tilemaps | Native tile atlas, ground/priority geometry and adjacent layer batching | Essentials' Ruby TilemapRenderer builds pooled sprites; core Tilemap API remains incomplete | Native map chunks could avoid Ruby tile traversal, state polling and many draw commands together. Simply adding Tilemap will not accelerate a game that continues to use its custom renderer. |
| Scene ordering | Ordered native scene list, reinserted on relevant changes | HashMap traversal, grouping and sorting each rendered frame | Retaining sorted draw lists could save CPU, especially while idle; invalidation must cover z, y, viewport and lifecycle changes. |
| Bitmap authority | GPU textures/FBO operations, lazy CPU surface for pixel reads | CPU bitmap pixels, revisions and GPU uploads | GPU-authoritative blits could remove CPU work and repeated uploads for animated bitmap mutations, at the cost of readback and coherence complexity. |
| Resource reuse | Bounded texture/FBO pool | Persistent atlas slots and coalesced free ranges; CPU copy-on-write clones | Both reuse storage. Pooling alone is unlikely to fix the observed sprite traversal/submission costs. |
| Runtime/platform | MRI, SDL, OpenGL; graphics update releases the GVL | MRI/Magnus, winit, wgpu | Native bindings suit the desktop. Releasing the GVL would help other Ruby threads, but is not evidence that the main game loop becomes faster. |
| Ruby JIT | Configurable YJIT | Enabled by default when supported | Keep JIT settings consistent in benchmarks; changing languages does not remove Ruby game logic costs. |

The comparison reviewed source areas for bindings, tilemap batching, scene ordering, bitmap and texture pooling, graphics scheduling, and JIT setup.

## Measurement method

Use the existing EricLostie/Pokémon Añil save: map 2, 50 events, player at
(21, 28). `tools/benchmark-anil.rb` reads the save and disables saving.
Headless wgpu on NVIDIA GeForce RTX 2080; uncapped, release/LTO build, default
YJIT. Wait 150 updates, measure 1,500, fence GPU work at both ends. Use
`FANPLAY_BENCH_FIXED_CLOCK=1` so uncapped CPU throughput does not alter movement
speed. Alternate implementations for three runs per scene; no concurrent
builds or sampling in timed comparisons. Frame times include Ruby game work
and native rendering, but exclude startup, window presentation and pacing.

Walking input (physical SDL scancodes) is:
`200:79:100,400:80:100,600:82:100,800:81:100,1000:79:100,1200:80:100,1400:82:100`.
The finite frame limit and save protection make runs repeatable without
modifying the original save. Timing is throughput, not user-visible capped FPS.

## Baseline profile

A separate CPU StackProf recording of 3,000 updates captured 1,264 samples
(0.55% missed): native bridge 27.4% self; Sprite#__sync 10.3% self/15.5%
inclusive; Sprite.__sync_all 25.3% inclusive; GC 7.0%. Sampling includes native
calls and is not a GPU timestamp. Percentages for parents and children overlap.
Native profiling over a shorter run showed 1,634 composed frames and only
16 reused frames, 34,205,184 uploaded bytes, and two explicit readbacks.
`graphics_update` accounted for 829 ms including warmup/startup updates;
12,488 typed sprite submissions took 27 ms. This suggests reducing draw
submission and Ruby traversal, rather than tuning scalar conversions first.

## Alternative implemented: concurrent disjoint compute draws

The new compositor groups adjacent sprite draws whose clipped destination
rectangles do not overlap. A dispatch uses its z dimension to select each
command; disjoint writes execute concurrently. Overlaps split batches.
Clears, viewport composition (which reads scratch pixels), and brightness
operations remain barriers. Original draw order is preserved where it matters.
A bounded batch size limits the overlap search. All existing pixel math stays
in the compute shader.

This borrows the upstream batching principle, adapted to this renderer rather
than copying its GL implementation. `FANPLAY_GPU_BATCH=0` disables grouping
for diagnosis. That switch uses the new command-storage shader with one draw
per dispatch; the saved original binary is also compared separately to expose
any effect from changing uniform command data to storage command data.

## Results

Medians of three alternating runs per variant (milliseconds per game update):

| Scene | Original executable | New shader, serial dispatch | Batched dispatch | Reduction vs original |
| --- | ---: | ---: | ---: | ---: |
| Idle map | 1.4418 | 1.4935 | 1.2932 | 10.3% |
| Scripted walking | 1.8257 | 1.8266 | 1.6535 | 9.4% |

Ruby allocations remain roughly 1,315/update idle and 1,492/update walking:
this improvement comes from rendering submission, not less Ruby game work.
Raw runs and environment are in
[`benchmarks/ericlostie-gpu-batching.json`](benchmarks/ericlostie-gpu-batching.json).
Idle framebuffer captures match byte for byte across every variant/run.
The walking trials all end at map 2, (10,26), but their images differ even
between original-executable runs: a fixed timestep alone does not seed random
game effects or fix the clock origin. Do not interpret those images as a
pixel-parity test. For stricter captures the harness now accepts
`FANPLAY_BENCH_SEED=0 FANPLAY_BENCH_CLOCK_START=0` alongside the fixed clock.

Separate sampling reports are saved as
[`before`](benchmarks/ericlostie-before-profile.json) and
[`after`](benchmarks/ericlostie-after-profile.json). The final recording captured
1,382 samples; bridge self samples were 20.7%, sprite traversal/synchronization
28.4% inclusive, and GC 9.3%. The final recording also enables native counters,
so its absolute time should not be compared with the baseline sample recording
or the unprofiled timing table. Ruby traversal is still a meaningful next target.

A second experiment replaced arrays of WeakRef wrappers with direct
ObjectSpace::WeakMap key iteration. Results varied too much to establish a
reliable gain (one pair was slower, two were faster, with substantial drift
between runs). It was not adopted; the current resource/GC lifecycle remains.
The exploratory data is in
[`benchmarks/ericlostie-registry-experiment.json`](benchmarks/ericlostie-registry-experiment.json).
This is not evidence against native dirty-state ownership, which would also
remove the snapshot comparison itself.

## Validation

- Core library tests: 20 passed.
- GPU adapter tests: 2 passed, including batched-vs-serial exact comparison
  with alpha overlap and viewport scratch reuse, and the existing software
  reference comparison for effects, rotation, planes and bitmap revisions.
- CLI tests: 13 passed; optional StackProf integration test remains ignored.
- Release build succeeded.
- Software-only build/tests: 19 passed. Formatting and Ruby harness syntax
  checks passed. Library Clippy passes with only the existing lint categories
  allowed on the command line.
- Strict Clippy encountered six existing findings: collapsible conditionals
  in engine/graphics/GPU free-list code, manual pop_if in the GPU free list,
  and manual is_multiple_of in input. These predate this change; no cleanup
  was mixed into the performance patch.
- After allowing those library categories, all-target Clippy also reports
  existing needless_return in desktop initialization and two too_many_arguments
  findings in presentation. Strict all-target lint is therefore not clean.

Native counter runs over the same 1,650-update idle scene counted 709,765
serial dispatches versus 274,890 batched dispatches (61.3% fewer). Both
composed 1,634 frames and reused 16. A separate walking validation with seed 0
and clock origin 0 produced identical original/batched framebuffer SHA-256:
`b3ce7d5af8ef4a3f734521af58ec9e2846485a82a72b93cf275f09a11da23bce`.

Repeat a seeded workload inside the Nix development shell:

```sh
FANPLAY_BENCH_SAVE="$HOME/.local/share/fanplay/Pokemon Anil/Partida 1.rxdata" \
FANPLAY_BENCH_FIXED_CLOCK=1 FANPLAY_BENCH_CLOCK_START=0 FANPLAY_BENCH_SEED=0 \
target/release/fanplay run --script tools/benchmark-anil.rb \
  --headless --renderer wgpu --uncapped --frames 1650
```

Add the walking input above for movement, `FANPLAY_GPU_BATCH=0` for serial
dispatch, `--output /tmp/frame.ppm` for a final capture, or the documented
`--ruby-profile` options for sampling. Disable sampling for throughput timing.

## Larger alternatives to pursue

1. **Native dirty state and a native tile-chunk renderer.** This targets both
   major costs: Ruby object polling and native draw submission. Start with an
   explicit fast path for this game's custom TilemapRenderer, preserve the
   existing path for plugins and unusual getters, and implement native Table
   change notifications. Offscreen tiles should never become individual
   synchronized sprites. Test scrolling, animated autotiles, priorities and
   map changes before replacing the game path.
2. **A raster wgpu compositor.** Use instanced textured quads and texture
   atlases, retaining compute for blend/effect cases requiring exact integer
   behavior. Raster hardware can batch overlapping sprites as ordered
   primitives. Benchmark before adopting: hardware blending rounding and
   unpremultiplied alpha differ from the current software contract.
3. **Persistent ordered scene data.** Rebuild grouping/order only when the
   relevant state changes, not every Graphics.update. Changes to image pixels
   should upload content without rebuilding geometry. Keep viewport and
   lifecycle invalidation explicit.
4. **GPU bitmap operations with lazy CPU synchronization.** Useful for repeated
   blits into animated bitmaps; maintain independent CPU/GPU revisions and
   read back only for get_pixel, export or CPU-only effects. The observed
   upload count makes this worth investigating, but it is a larger ownership
   change than batching and not a measured improvement here.

These are engineering hypotheses, not claimed multipliers. Battles, menus,
other maps and native windows require separate measurements.
