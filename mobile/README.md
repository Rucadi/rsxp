# Native RSXP mobile frontend

`rsxp-mobile-desktop` is the Linux test target for the same native frontend used
by Android and iOS. It uses winit for the event loop, egui for the library/tools
and touch controls, and wgpu for composition and presentation. There is no
WebView, HTML, JavaScript, or pixel IPC in the frontend.

The shared MRI bindings include native Table storage, retained sprite state,
world/map helpers, and RGSS1 tilemap batching. Ruby runs in a fiber on the event
loop thread and yields at `Graphics.update` and during script loading. The
native event loop paces updates and releases held inputs on pause/focus loss.
Kira/CPAL audio uses the same implementation as desktop.

The compositor is the existing `rsxp::gpu::GpuRenderer`. Its output buffer is
bound directly by the shared native presenter; ordinary frames do not read
pixels back to the CPU. The engine retains its existing software compatibility
fallback for unsupported effects. Explicit captures and integration-test frame
requests still perform readbacks.

## Linux first

From the repository root:

```sh
nix-shell
cargo run --manifest-path mobile/Cargo.toml --bin rsxp-mobile-desktop --release
cargo run --manifest-path mobile/Cargo.toml --bin rsxp-mobile-desktop --release -- --demo
cargo run --manifest-path mobile/Cargo.toml --bin rsxp-mobile-desktop --release -- --game /path/to/game
```

Or run `cargo run --release -- --demo` inside `mobile/` from that shell.
`--game` accepts a directory (opened in place, without copying) or an imported
library name. The library's Import button copies a folder into private storage.
The shell supplies Ruby, libclang, Vulkan/Wayland/X11, and audio dependencies.
GPU initialization errors are reported rather than silently selecting CPU-only
rendering. The selected adapter is printed at startup.

Use arrows and Z/Enter (A), X/Escape (B), Shift, or the on-screen controls.
Multiple touches can hold directions and action buttons together. F1 opens
shared game tools; F11 or the toolbar enables uncapped updates. Pause and Library
release held buttons. Debug builds optimize the shared renderer and Theora, but
use `--release` for performance comparisons.

## Verification

```sh
# No display server or GPU required for the default tests.
RSXP_SILENT=1 cargo test --manifest-path mobile/Cargo.toml

# Also verify GPU composition, input, restart, and pixel parity with software.
RSXP_SILENT=1 RSXP_TEST_GPU=1 cargo test --manifest-path mobile/Cargo.toml --test native_mri

# Real native frontend, automatically close after 120 completed game frames.
cargo run --manifest-path mobile/Cargo.toml -- --demo --frames 120

# Frame throughput without window/presentation overhead (not an on-screen FPS benchmark).
RSXP_SILENT=1 cargo run --release --manifest-path mobile/Cargo.toml -- --headless --demo --frames 300
# Software reference for comparison:
RSXP_SILENT=1 cargo run --release --manifest-path mobile/Cargo.toml -- --headless --demo --frames 300 --renderer software
```

The MRI integration test runs on the process main thread, checks that native
Table/Sprite/retained bindings are installed, and checks input changes pixels.
The optional GPU pass checks native GPU frames before taking explicit captures.
Rust input tests cover shared ownership, sliding touches, cancellation and
focus-loss release.

## Android and iOS

The native frontend uses winit's application lifecycle on all targets. Android
exports `android_main(AndroidApp)` through `android-native-activity`; iOS exports
`start_app` for cargo-mobile2's generated native entry point. Suspend drops the
window surface; resume creates a new compatible surface while retaining the
game/device. This path needs real device validation; a Linux run does not prove
Android/iOS readiness.

[cargo-mobile2](https://github.com/tauri-apps/cargo-mobile2) remains the platform
build/run tool. `mobile.toml` now selects its native `winit` template, not Wry.
The Cargo package and app name are `rsxp-mobile`; the generated Android manifest
loads `librsxp_mobile.so`. The installed app identifier remains `org.rsxp.mobile`.
Regenerate existing Wry-based `gen/android` and `gen/apple` projects before using
this frontend. Back up local signing/project customizations first. Do not reuse
the old WryActivity project or copy its generated source over this native app.

```sh
cd mobile
cargo mobile init --skip-targets-install
cargo android run
# On macOS with Xcode:
cargo apple run
```

Android needs the SDK/NDK, JDK and Rust target (`android-shell.nix` supplies the
tooling). iOS needs Xcode, its command-line tools, xcodegen and a signing team
for devices. Both need a target-compatible native MRI 4.0 distribution and
**target Ruby headers/configuration for Magnus/rb-sys**, not the host's Ruby ABI.
Set `RSXP_RUBY_PREFIX` for Android's native Ruby prefix. `build.rs` retains the
pinned Apple Ruby provisioner and asset staging.

Cross-build environments must configure rb-sys **before Cargo builds its
dependencies**: `RBCONFIG_CROSS_COMPILING=yes`, the target's `RBCONFIG_*` values
(including version, architecture, header and library directories), static Ruby
link settings, and bindgen's target/sysroot flags. A parent build script cannot
configure a dependency that has already built. The old WebView build's Ruby
prefix alone is therefore insufficient. Cross-build/device validation is still
pending; do not use host Ruby headers to get past that step.

On devices, put game folders in the app's `Games` directory and press Refresh.
iOS exposes Documents through Files. Android can provision private storage with
`adb run-as` for development. A native document-tree picker and save export are
follow-up platform work. Arbitrary third-party native Ruby plugins still need
ports for each target. MRI is reused between games, so games that patch global
classes may still require restarting the app for isolation.

## Source layout

- `src/app.rs`: native lifecycle, library, egui tools, input and frame scheduling.
- `src/input.rs`: touch/keyboard ownership and regression tests.
- `src/engine.rs`: game setup, filesystem import, native host and test commands.
- `src/platform.rs`: storage paths and Android runtime asset extraction.
- `../src/desktop/gpu_present.rs`: shared wgpu presenter and egui rendering.
- `../src/desktop/audio.rs`: shared Kira/CPAL audio backend.
- `../ruby_rgss_core/src/bindings/`: shared optimized native RGSS bindings.
- `../ruby_rgss_core/src/mobile.rs`, `mobile_bridge.c`: fiber lifecycle and Ruby
  exception boundary; bindings install before the bundled RGSS source runs.
- `build.rs`: native MRI provisioning, linking and asset staging.

`mobile/` is its own Cargo workspace, including its own development/release
profiles. Keep its dependency versions aligned with the normal desktop target.
