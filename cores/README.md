# Emulator cores

Each directory here is an emulator core, not an application. A core implements
[`rsxp_core::Core`](../crates/rsxp-core/src/lib.rs) and is selected by content
through the frontend catalog.

```text
cores/
  rgss/
    core/     # RGSS adapter implementing the core contract
    ruby/     # embedded MRI and Magnus bindings
  gba/        # future
  gb/         # future
  nds/        # future
```

## Adding a core

1. Create `cores/<id>/core` with a `Cargo.toml` that depends on
   `crates/rsxp-core` but not on a frontend, egui, wgpu or RGSS.
2. Implement `Core::load` and `Core::run`. Run the emulation loop inside
   `run`; do not ask the frontend to call a step function per frame.
3. Publish frames with `VideoSink::present` using a borrowed buffer. Publish
   raw audio with `AudioSink::queue_pcm`.
4. Read input from `CoreControl::input()`. Handle save-state/reset/options
   commands from `CoreControl::take_commands` at frame boundaries.
5. Register a `CoreRegistration` in the frontend binary. Use `detect` to claim
   content by extension, header magic or directory layout.
6. Add a headless test that loads a ROM/script, runs a bounded number of
   frames, verifies a frame hash and round-trips a save state.

## Performance checklist

- No allocation in the steady-state frame loop.
- No mutex in the steady-state frame loop.
- No per-pixel, per-sample or per-scanline callbacks.
- One video sink call per completed frame.
- Release profile with LTO and one codegen unit.
- Benchmark against the same core on the same host; record it in `docs/`.

See [multi-core architecture](../docs/multi-core-architecture.md) for the full
contract and migration plan.
