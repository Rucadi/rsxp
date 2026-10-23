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
