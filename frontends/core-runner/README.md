# Headless core runner

`fanplay-core-runner` is a small frontend used to exercise a core through the same
`fanplay-core` contract the desktop frontend uses. It routes content to an
enabled core, runs it on the dedicated core thread, counts frames, writes an
optional final PPM, and persists save states/battery saves.

```sh
cargo run -p fanplay-core-runner -- --content game.gba --frames 120 --output frame.ppm
```

Features select which cores are linked:

```sh
# default: GB/GBC, GBA and DS
cargo run -p fanplay-core-runner -- --content game.gba --frames 60

# include the RGSS adapter (requires MRI)
cargo run -p fanplay-core-runner --features rgss -- --content game/Game.ini --frames 60
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
graphical application evolves.

## Nintendo DS (Dust)

The default `gbc` feature adds the portable Rust Game Boy / Game Boy Color core
from [`mvdnes/rboy`](https://github.com/mvdnes/rboy). The default `nds` feature adds the [Dust](https://github.com/kelpsyberry/dust)
core, `.nds` content routing, stacked screens, touch input, stereo audio,
cartridge saves and save states. Dust requires unstable Rust features. Disable default features and enable
only `gba` to omit Dust from a GBA-only runner.

```sh
cargo run -p fanplay-desktop -- run --game /path/to/game.nds
cargo run -p fanplay-core-runner -- \
  --content /path/to/game.nds --frames 120 --output frame.ppm
```

See [the DS core guide](../../cores/nds/README.md) for system files,
controls, build compatibility, licensing and limitations.
