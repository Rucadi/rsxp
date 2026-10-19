# RSXP mobile prototype

One shared touch interface, with the existing Rust engine compiled natively for
each platform. Both the Rust engine and MRI Ruby interpreter are native machine code. No WASM
or WASI interpreter is packaged or used for mobile gameplay. The WebView only
draws the shared library and touch controls; game pixels appear in a native view.

The built-in demo runs RGSS/Ruby, renders through the native Rust software
renderer, and responds to the D-pad and A button. Import a game folder from
Files on iOS or the system document picker on Android, then tap its library card.
Import copies the folder into the application's private library; it does not
modify the source. Choose the folder containing Game.ini and Data, not its parent.
Both compiled `Data/Scripts.*` bundles and MKXP-style extracted `Data/Scripts`
directories of `.rb` files are supported.

## Develop on an Apple Silicon Mac

Install Xcode, open it once to accept Apple's license and finish installing its
components. If no iOS Simulator runtime is installed, `--run` downloads Apple's
ARM64 runtime and creates an iPhone Simulator automatically (a large first-time download).
Then, from the repository root:

```sh
nix-shell
python3 mobile/build.py ios --run
```

This prepares the pinned Ruby runtime and shared UI assets, builds the Rust core
for `aarch64-apple-ios-sim`, builds the app, and installs/launches it in an available
iPhone Simulator. The script uses `/Applications/Xcode.app` without changing the
machine's selected developer directory. Set `DEVELOPER_DIR` to use another Xcode.
First builds require network access. When the current compiler lacks mobile target libraries, the helper provisions
a minimal Rust toolchain under `mobile/build/rustup/`. Nix supplies `rustup`
automatically if needed; it does not change your global default Rust toolchain.

For a physical iPhone, run `python3 mobile/build.py ios --device`, open
`mobile/ios/RSXP.xcodeproj`, set your signing team, select your device, and run.
Device installation and distribution require Apple signing credentials.

For fast interface/core development without the Simulator:

```sh
python3 mobile/build.py host
python3 mobile/preview.py
```

Open `http://127.0.0.1:8765`. This preview uses the real native Rust dynamic
library through the same C ABI. It is a host preview, not an iOS emulation.
Copy a game folder into `mobile/build/preview-games/` and press Refresh to test
imported games there. Keep the server local; it exposes a development native API.

## Android

Android uses the same Rust core and shared touch interface as iOS. The native
MRI runtime is the Termux-patched Ruby 3.3 build published by
[jekyllex/ruby-android](https://github.com/jekyllex/ruby-android); the build
helper downloads the pinned arm64 archive and stages it into the APK. No WASM
or WASI interpreter is used.

The development environment is in `shell-droid.nix` at the repository root. It
provides Android Studio, the Android SDK 35, NDK r28, CMake 3.22.1, Gradle 8,
JDK 17, the arm64 emulator image and the required Rust tooling. Unfree packages
and Android SDK licences are accepted by the shell expression.

```sh
nix-shell shell-droid.nix
python3 mobile/build.py android
```

The debug APK is written to
`mobile/android/app/build/outputs/apk/debug/app-debug.apk`. The build stages the
Rust core, Ruby shared library and the Ruby standard library; the app copies the
stdlib out of its assets on first launch. The app currently supports arm64
devices and Apple Silicon emulators. See [android.md](android.md) for emulator
setup, installation and game transfer commands.

The import button opens Android's Storage Access Framework folder picker. The
selected folder (the one directly containing `Game.ini` and `Data`) is copied
into the app's private `files/Games` library; the source folder is not modified.

## Where to work

| Location | Responsibility |
| --- | --- |
| `ui/` | Shared library, game launch, pause, controller, and errors |
| `ui/input.js` | Pointer ownership, multitouch, and releasing held inputs |
| `core/` | Native Rust runtime, safe asset access, saves, framebuffer, C ABI |
| `ios/` | WKWebView shell, document picker, native-library linkage |
| `android/` | WebView shell, document-tree import, JNI adapter |
| `build.py` | Asset preparation and platform build commands |
| `demo.rb` | Small built-in RGSS game for end-to-end checks |

Image/font services live in `rust/portable_media.rs`, shared with the browser.
The core uses the existing runtime, archives, patches, graphics, and input modules.
Only the OS shells differ. Touch UI changes need no platform-specific layout work.

Ruby scripts execute in native MRI, linked into the Rust core. Apple builds use
Ruby 4.0.7 from the pinned, SHA-256 verified [CRuby 4.0.701
distribution](https://github.com/xord/cruby/releases/tag/v4.0.701). Android uses
the SHA-256 verified Termux-patched Ruby 3.3.4 bootstrap from
[jekyllex/ruby-android](https://github.com/jekyllex/ruby-android/releases/tag/v0.1.5).
Its headers, interpreter, static extensions, and standard library target
macOS, iOS device, and iOS Simulator separately on Apple, and arm64 Android on
mobile. JIT compilation is disabled. The C adapter keeps
Ruby exception unwinding inside C, then returns errors to Rust. Game fibers
yield after Graphics.update so the shell stays responsive between frames.
The same `rust/rgss.rb` compatibility layer calls the native Rust renderer and
input engine directly, without JSON/base64 pixel transport through JavaScript.
Platform-specific Ruby plugins still require a compatible native mobile port.

All engine requests must run on one thread because the shared engine is
thread-local. Both mobile shells dispatch synchronous prompts on their WebView
UI thread. Only the bundled interface may navigate or invoke that bridge.

## Current limits and verification

This is an experimental development build, not an App Store release. Audio and
the desktop inspector are not implemented. On iOS the core composites scenes with
wgpu on Metal and presents directly to a `CAMetalLayer`, with no RGBA readback;
if Metal is unavailable it falls back to the CPU renderer and UIImageView.
Android currently uses CPU frames through ImageView. Native execution alone does
not establish a particular frame rate. Large games and long play sessions need
device testing.
MRI writes saves directly to native application storage, separately for each game.
Uninstalling the app removes its private games and saves; save export is pending.

The desktop Magnus bindings (retained sprite state, built-in RGSS1 tilemap
batching, Win32API) are not compiled into the mobile core. Mobile uses the shared
`rgss.rb` plus the serialized native-call path instead, so tiles still reach the
GPU native-tile geometry path but per-tile Ruby updates are not batched as they
are on desktop.

The native MRI integration test launches the RGSS demo, renders a frame, sends
Right input, and checks that the rendered frame changes. The shared touch tests
check multitouch ownership and release behavior. Build the host runtime first
to provision MRI before running the tests:

```sh
python3 mobile/build.py host
cargo test --manifest-path mobile/core/Cargo.toml
node --test mobile/tests/input.mjs
cargo check --manifest-path web/engine/Cargo.toml --target wasm32-unknown-unknown
```

For a repeatable iOS Simulator smoke test, launch the app with
`xcrun simctl launch booted org.rsxp.mobile --demo`. This starts the same built-in
Ruby game through the shared UI in Debug or Release builds. Pass
`--game 'Game Folder'` to open an imported library game directly. Normal launches
open the library.
