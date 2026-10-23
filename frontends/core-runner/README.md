# Headless core runner

`rsxp-core-runner` is a small frontend used to exercise a core through the same
`rsxp-core` contract the desktop/browser frontends use. It routes content to an
enabled core, runs it on the dedicated core thread, counts frames, writes an
optional final PPM, and persists save states/battery saves.

```sh
cargo run -p rsxp-core-runner -- --content game.gba --frames 120 --output frame.ppm
```

Features select which cores are linked:

```sh
# default: GBA
cargo run -p rsxp-core-runner -- --content game.gba --frames 60

# include the RGSS adapter (requires MRI)
cargo run -p rsxp-core-runner --features rgss -- --content game/Game.ini --frames 60
```

Useful options:

- `--core ID` bypasses content detection.
- `--frames N` sets the safety frame limit (default 60).
- `--fast-forward` disables frame pacing.
- `--save-state SLOT` requests a state at the first frame boundary.
- `--load-state SLOT` loads a state before the first frame.
- `--save-dir DIR` overrides the default save directory.
- `--output FILE` writes the final frame as a PPM.

This runner is intentionally headless and non-interactive. It is not the
desktop application; it is a stable way to validate new cores while the
core-selectable UI is being built.

## Nintendo DS (Dust)

The default `nds` feature adds the [Dust](https://github.com/kelpsyberry/dust)
core, `.nds` content routing, stacked screens, touch input, stereo audio,
cartridge saves and save states. Dust requires nightly Rust; existing default
GBA/RGSS builds remain on stable Rust.

```sh
cargo run -p rsxp-desktop -- run --game /path/to/game.nds
cargo run -p rsxp-core-runner -- \
  --content /path/to/game.nds --frames 120 --output frame.ppm
```

See [the DS core guide](../../cores/nds/README.md) for system files,
controls, build compatibility, licensing and limitations.
