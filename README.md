# RSXP

**RSXP is highly experimental and currently in discovery mode.** Its design,
architecture, features and compatibility are still being explored and may
change substantially. Expect incomplete behavior and breaking changes; do not
rely on it for production use or on compatibility with any particular game.

RSXP is being reorganized as a multi-core emulator shell. The first core runs
RPG Maker RGSS game scripts on a Rust engine; a Game Boy Advance core built on
RustBoyAdvance-NG uses the same contract, and GB/GBC and DS cores can follow
without linking MRI, RPG Maker data code or RGSS panels. Desktop RGSS execution
embeds MRI Ruby through Magnus; the browser runs Ruby and the Rust engine in
separate WebAssembly modules. The shared engine owns graphics resources, input
state, archive decoding and frame execution. Game logic remains Ruby. See
[multi-core architecture](docs/multi-core-architecture.md).

Compatibility is incomplete. Selecting RGSS 1, 2 or 3 loads the corresponding
RPG data definitions and compatibility behavior; it does not guarantee that a
game or its plugins will run. See [runtime behavior and compatibility](docs/rust-port.md).

## Build and launch

The workspace requires Rust 1.95 or newer. The desktop package default
features are `gpu` and its own frontend; the engine enables GPU composition. Desktop builds need shared MRI Ruby and development headers,
a C compiler, libclang and the platform audio/window dependencies. Ruby and its
standard libraries, including Zlib, must be available. The Nix development shell
provides the native build tools and dependencies.

```sh
nix develop path:.
cargo build --release -p rsxp-desktop
cargo run --release -p rsxp-desktop --bin rsxp
```

With default features, no arguments opens a persistent game dashboard. Load a
game directory or Game.ini there, or select a game at launch:

```sh
cargo run --release -p rsxp-desktop --bin rsxp -- run --game /path/to/game
cargo run --release -p rsxp-desktop --bin rsxp -- run --script /path/to/scene.rb --rgss 3
cargo run --release -p rsxp-desktop --bin rsxp -- demo
```

Interactive `run` starts a dashboard and an isolated game process. Headless
execution runs the Ruby runtime directly and uses silent audio:

```sh
cargo run --release -p rsxp-desktop --bin rsxp -- run --script crates/rsxp-rgss/testdata/rgss_demo.rb \
  --headless --frames 60 --output /tmp/rgss.ppm
```

`run` requires exactly one of `--game DIRECTORY` and `--script FILE`. `--frames`
limits Graphics updates, and `--output` writes the final framebuffer as PPM.
Without a frame limit, a headless Ruby script runs until it exits or fails.
A headless `demo` defaults to one frame.

`--renderer auto|wgpu|software` selects game composition. Interactive `auto`
uses the GPU path; headless `auto` uses software. Software composition still
uses wgpu to display an interactive frame. GPU composition can fall back to
software for specific scene effects. Set frame rate and uncapped execution in
the workspace, revealed with F1. Gameplay starts with the UI hidden. F2 toggles
Tools, F3 output, F4 desktop fullscreen, F10 performance, and F11 uncapped
execution. Entering fullscreen hides the UI; F1 can reveal it again.
Press F1 and open **Controls** to remap controller buttons and left-stick
movement to RGSS actions, adjust the stick dead zone, or restore defaults.
D-pad/left stick move; South confirms and East cancels. Multiple controllers
share the same mapping and can be used alongside the keyboard. Mappings are
saved in `$XDG_CONFIG_HOME/rsxp/controls` (or `~/.config/rsxp/controls`) on
Linux/macOS, `%APPDATA%/rsxp/controls` on Windows, and browser local storage.
Browser support requires a controller
with a standard Gamepad API mapping; press a controller button to expose it.

The parser does not accept `--fps`, `--fast-forward` or `--uncapped`.

### Running a GBA core

The GBA core can be exercised without the desktop UI through the generic
headless runner:

```sh
cargo run -p rsxp-core-runner -- --content /path/to/game.gba --frames 120 \
  --output /tmp/gba.ppm
```

The backend is
[RustBoyAdvance-NG](https://github.com/michelhe/rustboyadvance-ng) (MIT).
See [cores/gba/README.md](cores/gba/README.md) for BIOS resolution, save
locations and current limitations. The runner links only the GBA core by
default; add `--features rgss` to include the RGSS adapter. A cartridge-only
desktop binary builds without MRI or the RGSS engine:

```sh
cargo run -p rsxp-desktop --no-default-features --features gba -- \
  run --game /path/to/game.gba --headless --frames 120 --output /tmp/gba.ppm
```

Games start directly at the cartridge without the BIOS startup image. The
embedded open-source BIOS remains available for BIOS services during gameplay.
Development builds optimize the GBA CPU cores (`opt-level = 3`) even in the dev
profile; use `--release` for the highest throughput.

The repository also supplies `flake.nix`, `default.nix` and `shell.nix`. The
flake declares x86-64 and AArch64 Linux. Its package source filter currently
omits `crates/rsxp-rgss/api/`, although the default desktop build includes files from that
directory; use the development-shell Cargo workflow above until packaging is
updated. Platform declarations alone are not evidence of successful builds.

## How the project fits together

The experimental [mobile app](mobile/README.md) shares one touch interface between
iOS and Android, with a native Rust core and native MRI Ruby for game execution.
The iOS Simulator prototype runs; Android still requires a native MRI build.
Start with
`python3 mobile/build.py ios --run` inside `nix-shell` after completing Xcode's
first-launch setup. The mobile README covers the native host preview and current
compatibility limits.

- [Architecture](docs/architecture.md): startup flowcharts, frame sequence,
  desktop rendering paths, browser scheduling, ownership and a source map.
- [Runtime and compatibility](docs/rust-port.md): startup, RGSS behavior,
  supported API areas, limitations and debugging controls.
- [Game workspace](docs/dashboard.md): game library, session controls, saves
  and desktop transport.
- [Browser frontend](platforms/web/README.md): build, assets, cooperative Ruby execution
  and browser limitations.
- [Live game API](crates/rsxp-rgss/api/README.md): HTTP inspection/control and the MCP executable.
- [Multi-core architecture](docs/multi-core-architecture.md): core contract,
  content routing, performance rules and the path for GBA/GB/DS cores.
- [Core conformance](docs/core-conformance.md): feature matrix, replay checks
  and measurement commands.

The performance documents describe particular workloads and measurements,
not expected FPS for every game: [renderer](docs/rsxp-performance.md),
[native tilemaps](docs/native-tilemap-performance.md),
[session transport](docs/session-performance.md),
[browser](docs/web-performance.md).

## Development checks

```sh
cargo test -p rsxp --lib --no-default-features
cargo test -p rsxp --lib
cargo test -p rsxp-desktop --test cli
cargo test -p rsxp-core -p rsxp-frontend
cargo test -p rsxp-core-gba -p rsxp-cores
cargo check -p rsxp-desktop --no-default-features --features gba
nix-shell --run 'cargo test -p rsxp-core-rgss --test headless'
cargo fmt --check
cargo clippy --all-targets -- -D warnings
npm --prefix platforms/web test
```

The first command exercises the portable CPU core on the host; it does not
cross-compile WebAssembly. The RGSS adapter test needs MRI and therefore runs
inside the development shell. GPU-dependent checks are ignored by default and
require an adapter when explicitly enabled. Some game-specific integration
checks are also ignored. Build the browser with `cargo xtask web`.

Archive commands read encrypted game archives without executing Ruby:

```sh
cargo run --release -p rsxp-desktop --bin rsxp -- archive-list Game.rgssad
cargo run --release -p rsxp-desktop --bin rsxp -- archive-cat Game.rgss3a Data/Scripts.rvdata2 > /tmp/Scripts.rvdata2
```

The project is licensed GPL-2.0-or-later; see [COPYING](COPYING).

## Automated builds

The repository's `Build` GitHub Actions workflow runs on pushes, pull requests,
and manual dispatch. It produces desktop `rsxp` and self-contained `rsxp-web`
archives for Linux x86-64, Windows x86-64, and macOS ARM64, plus `rsxp.html`, a
single self-contained static page ready to host. Download them from the workflow
run's Artifacts section; pushes to the default branch also publish `rsxp.html` to
GitHub Pages.
The desktop builds bundle MRI Ruby 3.4 and their native libraries: Linux ships
a self-contained AppImage built against Debian 11's glibc (2.31) so it runs on
Ubuntu 20.04+, Debian 11+, and every newer distribution, Windows a portable
`.zip` (executable plus DLLs, no installer), and macOS a drag-to-Applications
`.dmg`. The Linux AppImage is intentionally produced in a plain Debian 11
container rather than with Nix (see `packaging/appimage/Dockerfile` and
`tools/build-appimage.sh`); Nix still builds and tests the desktop package.
macOS builds are only ad-hoc signed, so Gatekeeper requires a right-click → Open
(or clearing the quarantine attribute) on first launch. The web executables
include the browser runtime and require no Ruby installation. Games are not
included in any CI artifact.

To build the AppImage locally:

```sh
docker build -t rsxp-appimage packaging/appimage
docker run --rm --user "$(id -u):$(id -g)" --env HOME=/tmp \
  --volume "$PWD":/src --workdir /src \
  rsxp-appimage bash tools/build-appimage.sh /src/artifacts
```

## Nintendo DS (Dust)

The default desktop build includes the [Dust](https://github.com/kelpsyberry/dust)
Nintendo DS core, `.nds` content routing, stacked screens, touch input, stereo
audio, cartridge saves and save states. Dust requires nightly Rust features;
the Nix shell enables them for its bundled compiler.

```sh
cargo run -p rsxp-desktop -- run --game /path/to/game.nds
cargo run -p rsxp-core-runner -- \
  --content /path/to/game.nds --frames 120 --output frame.ppm
```

See [the DS core guide](cores/nds/README.md) for system files,
controls, build compatibility, licensing and limitations.
