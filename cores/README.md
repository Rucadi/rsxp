# Emulator cores

Each directory here is an emulator core, not an application. A core implements
[`rsxp_core::Core`](../crates/rsxp-core/src/lib.rs) and is selected by content
through the frontend catalog.

```text
cores/
  rgss/
    core/     # RGSS adapter implementing the core contract
    ruby/     # embedded MRI and Magnus bindings
  gba/
    core/     # GBA adapter using RustBoyAdvance-NG (MIT)
  gb/         # future
  nds/        # DS adapter and Dust vendor dependencies
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

## Nintendo DS (Dust)

The default `nds` feature adds the [Dust](https://github.com/kelpsyberry/dust)
core, `.nds` content routing, stacked screens, touch input, stereo audio,
cartridge saves and save states. The desktop and runner include DS by default.
Dust requires nightly Rust features; the Nix shell enables those features for
its bundled compiler.

```sh
cargo run -p rsxp-desktop -- run --game /path/to/game.nds
cargo run -p rsxp-core-runner -- \
  --content /path/to/game.nds --frames 120 --output frame.ppm
```

See [the DS core guide](nds/README.md) for system files,
controls, build compatibility, licensing and limitations.

Platform-specific RSXP interface, GUI and connectivity code belongs in
`crates/rsxp-gba` or `crates/rsxp-nds`, separate from these emulator cores.
