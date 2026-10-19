# RSXP

RSXP runs RPG Maker RGSS game scripts on a Rust engine. Desktop execution embeds
MRI Ruby through Magnus; the browser runs Ruby and the Rust engine in separate
WebAssembly modules. The shared engine owns graphics resources, input state,
archive decoding and frame execution. Game logic remains Ruby.

Compatibility is incomplete. Selecting RGSS 1, 2 or 3 loads the corresponding
RPG data definitions and compatibility behavior; it does not guarantee that a
game or its plugins will run. See [runtime behavior and compatibility](docs/rust-port.md).

## Build and launch

The root package requires Rust 1.95 or newer. Default Cargo features are
`desktop` and `gpu`. Desktop builds need shared MRI Ruby and development headers,
a C compiler, libclang and the platform audio/window dependencies. Ruby and its
standard libraries, including Zlib, must be available. The Nix development shell
provides the native build tools and dependencies.

```sh
nix develop path:.
cargo build --release
cargo run --release --bin rsxp
```

With default features, no arguments opens a persistent game dashboard. Load a
game directory or Game.ini there, or select a game at launch:

```sh
cargo run --release --bin rsxp -- run --game /path/to/game
cargo run --release --bin rsxp -- run --script /path/to/scene.rb --rgss 3
cargo run --release --bin rsxp -- demo
```

Interactive `run` starts a dashboard and an isolated game process. Headless
execution runs the Ruby runtime directly and uses silent audio:

```sh
cargo run --release --bin rsxp -- run --script rust/testdata/rgss_demo.rb \
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

The repository also supplies `flake.nix`, `default.nix` and `shell.nix`. The
flake declares x86-64 and AArch64 Linux. Its package source filter currently
omits `api/`, although the default desktop build includes files from that
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
- [Browser frontend](web/README.md): build, assets, cooperative Ruby execution
  and browser limitations.
- [Live game API](api/README.md): HTTP inspection/control and the MCP executable.

The performance documents describe particular workloads and measurements,
not expected FPS for every game: [renderer](docs/rsxp-performance.md),
[native tilemaps](docs/native-tilemap-performance.md),
[session transport](docs/session-performance.md),
[browser](docs/web-performance.md), and [Ruby profiling](docs/ruby-profiling.md).

## Development checks

```sh
cargo test --lib --no-default-features
cargo test --lib
cargo test --test cli
cargo fmt --check
cargo clippy --all-targets -- -D warnings
npm --prefix web test
```

The first command exercises the portable CPU core on the host; it does not
cross-compile WebAssembly. GPU-dependent checks are ignored by default and
require an adapter when explicitly enabled. Some game-specific integration
checks are also ignored. Build the browser with `cargo xtask web`.

Archive commands read encrypted game archives without executing Ruby:

```sh
cargo run --release --bin rsxp -- archive-list Game.rgssad
cargo run --release --bin rsxp -- archive-cat Game.rgss3a Data/Scripts.rvdata2 > /tmp/Scripts.rvdata2
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
