# Development and validation

## Toolchain

`nix develop path:.` loads the dependencies in [`nix/shell.nix`](../nix/shell.nix).
It supplies Rust, Cargo, Clang/bindgen support, a C compiler, Node, linker tools and
native audio/window/WebKit libraries. Dust requires unstable Rust features;
the shell sets `RUSTC_BOOTSTRAP=1`. A bare Rust 1.95 installation is not enough
for every default dependency even though many packages declare that minimum.

The root Cargo workspace defaults to `fanplay-desktop`. There is no separate
mobile workspace in this checkout.

## Commands

```sh
# Default desktop: GPU + RGSS + GB/GBC + GBA + DS
cargo build --release -p fanplay-desktop
cargo run -p fanplay-desktop --bin fanplay -- --help

# GBA-only headless desktop; does not link the RGSS runtime or SabiRuby
cargo check -p fanplay-desktop --no-default-features --features gba
cargo run -p fanplay-core-runner --no-default-features --features gba -- \
  --content /path/to/game.gba --frames 60

# Contract, emulators and shared interface
cargo test -p fanplay-core -p fanplay-frontend -p fanplay-cores
cargo test -p fanplay-core-gbc -p fanplay-core-gba -p fanplay-core-nds
cargo test -p fanplay-gba -p fanplay-nds -p fanplay-ui

# RGSS engine and embedded SabiRuby runtime
cargo test -p fanplay-rgss-ruby --lib
cargo test -p fanplay-rgss --lib --no-default-features
cargo test -p fanplay-core-rgss --test headless
FANPLAY_API=0 cargo test -p fanplay-desktop --test cli -- --test-threads=1

# Services and plugins; some tests bind loopback sockets
cargo test -p fanplay-control -p fanplay-api -p fanplay-provision
cargo test -p fanplay-extension-api -p fanplay-extensions -p fanplay-extension-ui

# Generate the static control catalog
cargo api-openapi > crates/fanplay-mcp/openapi.json

```

Network and relay tests need permission to bind loopback sockets. Tests needing
a GPU adapter or local game trees may be ignored or feature-gated; inspect the
reported skipped tests.

`cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` are useful
repository-wide checks; they can also expose pre-existing formatting or lint
work. Do not substitute formatting success for runtime tests.

## Packaging and CI

[`build.yml`](../.github/workflows/build.yml) is authoritative. It configures
Linux Nix builds and AppImage packaging, Windows Nix cross-builds, and direct
Windows/macOS desktop builds. Inspect
the workflow's conditions for artifact publication; configuration is
not proof that the latest run succeeded. Desktop builds embed SabiRuby and its
bundled C source compiler; packaging does not copy CRuby libraries or gems.

Nix attributes include `fanplay`, `appimage` and `windows`. The external Cachix
cache is selected by the `CACHIX_CACHE` repository variable, falling back to
the existing `rsxp` cache. A source rename does not create a new hosted cache.
Games are supplied separately from the player.

## Performance

Release LTO is disabled because of an upstream ARM interpreter miscompilation.
Development profiles optimize interpreter dependencies. Keep these settings
when renaming packages. Measure with fixed content, renderer, toolchain and
input; historical benchmark notes are not current performance guarantees.
