# Dashboard transport performance

The original dashboard implementation used `GpuRenderer::readback()` for every
native game frame. That API is deliberately synchronous: it allocates a staging
buffer, copies the framebuffer, maps it, and waits for GPU completion. TCP then
carried all pixels to another device for upload. At panoramic 3× in Añil this
meant transferring 1536 × 1152 × 4 = 7,077,888 bytes per update, plus repeated
allocation/copying and a GPU synchronization point.

## Corrected path

The isolated runner records compositor commands, native tile geometry, and only
changed atlas contents. The dashboard submits these on the device that owns its
surface. This preserves draw ordering, source revisions, tile batching, and
retained-frame reuse without normal-frame readback or framebuffer upload.

Ordered asset updates accumulate when a view is skipped; only its obsolete draw
list is discarded. Pending uploads have a bounded backlog. CPU copies for
software fallback use bulk writes and owned bitmap storage, and an unchanged
bitmap is not uploaded again for each egui repaint.

A separate bounded control reader removes the remaining per-frame request/reply
wait. The game waits for its initial control snapshot only, then consumes
available updates without blocking on dashboard decoding or scheduling. Held
keys persist; edges, text, scrolling, and inspector requests are consumed once.
Pause, disconnect, and process isolation retain their existing behavior.

Screenshots, freeze, and snap still need CPU pixels. They materialize the
recorded scene on the runner's GPU on demand; this does not restore readback to
ordinary presentation.

## Reproducing a comparison

`cargo run --release --example session-bench -- /path/to/rsxp` runs a finite,
uncapped scene with identical GPU work. The consumer substitutes for the window
using the same compositor, fences the GPU around measurement, excludes startup
and 60 warmup frames, and reports time/update and bytes/update. An optional script
and frame limit select a real-game workload:

```sh
cargo run --release --example session-bench -- \
  /path/to/rsxp /path/to/benchmark-script.rb 650
```

For Añil, use `tools/benchmark-anil.rb` with saving disabled, fixed simulation
clock and RNG seed, 150 game warmup updates, and 500 measured updates. A temporary
wrapper can transfer the loaded save to map 2 at (21,28) before entering the
scene. Verify map 2, 50 events, and 1536 × 1152 output at panoramic 3× in the log.
Use a fixed temporary copy of the save, not a save being changed by a running
game. Alternate the committed/pre-dashboard binary and candidate without builds
or profiling in parallel. Report both Ruby wall/CPU timings and consumer GPU
completion time; this controlled workload is not a promise about capped play or
monitor presentation FPS.

## Coverage

- Packet round trips, malformed/truncated data, and merged ordered uploads.
- Exact pixel parity with local GPU rendering through changing assets, atlas
  growth/reuse, frame resizing, native tiles, viewports, tint, and brightness.
- Explicit screenshot parity after skipped intermediate views.
- Native software and GPU runner lifecycle, input, pause/resume, inspector, and
  reopening; ordinary GPU presentation reports zero explicit readbacks.
- A delayed/silent control peer cannot stall the next game poll.
