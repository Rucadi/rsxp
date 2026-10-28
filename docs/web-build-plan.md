# FANPLAY web build

The browser frontend is a thin Rust/WASM host for the existing eframe
application. It reuses the shared workspace, launcher, runtime tools, plugin
UI, core contract, video conversion, input model, save-state model and core
adapters. Browser-specific code is limited to file handles, `localStorage`,
Web Audio, the eframe canvas, worker transport and the cooperative frame loop.

## Supported web application

The release bundle includes the complete shared FANPLAY surface for the
browser-compatible cartridge cores:

- Game Boy / Game Boy Color (`.gb`, `.gbc`)
- Game Boy Advance (`.gba`)
- Nintendo DS (`.nds`), using its existing software renderer with WASM SIMD
- drag-and-drop and browser file-picker loading
- optional DS BIOS/firmware and GBA BIOS dumps, persisted under versioned browser
  storage keys; the embedded GBA BIOS remains the default, while an official
  GBA dump runs its normal reset sequence
- keyboard input and DS touch input
- shared launcher/workspace panels, runtime tools and performance view
- cooperative frame execution so the browser event loop remains responsive
- bounded browser speed controls for cores that support them; GB/GBC stay at game rate
- browser Web Audio output backed by the existing PCM ring
- WGPU-first eframe presentation: the dashboard and live game callback share
  one browser GPU device; the game framebuffer is uploaded directly as raw
  RGBA/BGRA bytes to a nearest-neighbor texture shader, with egui-wgpu's
  WebGL fallback retained for browsers without WebGPU
- cartridge battery saves in browser storage; RGSS writes `Game.rxdata` to
  Ruby's private WASI VFS and exports/imports that file directly
- four persistent save-state slots
- bundled Rhai plugin UI and GBA/NDS extension requests
- `.rhai` plugin import through the browser file picker

The HTML entrypoint contains only a canvas, a small layout style and Trunk's
Rust loader. Application behavior is Rust/WASM; there is no JavaScript
framework, duplicated web UI, REST server or browser-side emulator.

## Target shape

```text
frontends/web
    eframe::WebRunner
        WebApp
            WebSession
                CoreCatalog::route_probe
                BrowserPlatform
                BrowserVideo / BrowserAudio / BrowserEvents
                cooperative core execution
            fanplay_ui::SharedApp
```

## Host boundaries

The web crate does not depend on `frontends/desktop`. Browser services satisfy
the same `Platform`, `VideoSink`, `AudioSink` and `EventSink` contracts used by
native runners. Core code remains synchronous; only browser file reads are
asynchronous, and their bytes are loaded before a core sees them.

The web host calls `Core::run_slice` in a bounded batch per eframe repaint. Each
slice advances at most one frame; GBC, GBA and NDS drain commands, present
video, queue audio, flush saves at their existing intervals and observe
pause/fast-forward/stop controls. Native `Core::run` behavior remains
unchanged.

The web canvas selects `Renderer::Wgpu` when the WGPU eframe feature is
available. `wgpu` chooses WebGPU first and keeps WebGL as its adapter fallback;
both the dashboard and the game paint callback use the same eframe device. The
callback retains one texture and only calls `write_texture` for a new worker
revision, avoiding the previous BGRA/RGBA-to-`Color32` conversion and second
egui texture upload.

RGSS uses a dedicated module worker. The worker instantiates the same compiled
frontend module without starting egui, then owns the RGSS core, Ruby VM and
software renderer. The window never resumes Ruby inside an egui draw pass.
Input and pause/rate controls are messages; audio playback stays on the window
thread, with playback-position snapshots sent back to the worker. Video uses
transferable buffers with one unacknowledged frame and one replaceable latest
frame, preventing a presentation backlog. Script-loading checkpoints yield
without consuming a full game-frame pacing interval. FPS is measured from
completed game frames rather than displaying the requested rate.

The imported immutable game file map is shared between the library entry and
its session. Starting or restarting a game does not deep-copy the whole folder
inside the UI WASM memory (the local Añil folder is about 1.4 GB).

The workspace release profile uses full cross-crate LTO and one codegen unit.
The GBA adapter now tracks `Rucadi/rustboyadvance-ng`'s `master` branch, whose
ARM7TDMI/core crates forbid unsafe code, so the old conservative release
workaround is no longer needed. The resolved commit remains recorded in
`Cargo.lock` until the next dependency update.

The WASM target also enables `target-feature=+simd128`. Dust's DS software
renderer is written with Rust portable SIMD; without this target feature the
browser receives a scalar fallback and DS frame time drops sharply. This keeps
the optimization in the Rust build and adds no JavaScript runtime dependency.

## Persistence

Battery saves, save-state slots and the latest-save backup use browser
`localStorage`, keyed by content hash and save name. The selected ROM library
remains in memory for the current tab; the browser picker or drop zone can
reload it. IndexedDB is intentionally not part of the first host because it
would add a second asynchronous storage abstraction before the runtime needs
it. It is the next step if persistent ROM catalogs or large save data become a
requirement. System BIOS uploads use a versioned key namespace so browser data
written by an older build cannot silently replace the embedded GBA BIOS.

## RGSS boundary

Native RGSS keeps its existing MRI/Magnus bindings. Browser RGSS uses a local
Ruby 4.0 WASI image built with `rbwasm`, following the companion-gem pattern
used by HyperCast. The local `fanplay-rgss-wasm` gem compiles the Rust Magnus
extension as a static library during Ruby's WASI build, packages it into
`/bundle/setup`, and makes it available as `require "fanplay_native"` inside
Ruby. This keeps the same Ruby extension surface across desktop and browser.

The browser image installs the same Magnus bindings as desktop: Audio, Table,
sprite conversion, bulk scrolling, RGSS tile plans, world camera placement,
retained dirty/dependency tracking, retained map/chunk caches and Win32API.
The portable Win32 compatibility layer handles supported calls off Windows.
Table storage, Ruby property updates and native caches live beside Ruby.
Converted rendering updates are batched into the shared `native_updates`
protocol before the next engine query, frame or resource disposal; the outer
Rust/WASM instance owns the actual rendering state. This avoids maintaining an
inactive engine inside the interpreter. Native Audio calls use the same web
bridge and the browser AudioSink decodes streams with HTMLAudioElement.

The frontend respects RGSS1's native 40 Hz default. Catch-up credit is capped
at two frames, and the host stops starting extra frames after 8 ms of game
work per repaint. A single Ruby frame remains cooperative and may exceed that
budget, but a backlog no longer blocks the UI for 16 consecutive game frames.
For RGSS, that work now occurs in the dedicated worker; the UI only drains
messages. Parsed fonts and glyph coverage are cached in bounded per-thread
caches. Text measurement uses font metrics without rasterizing glyphs, and
foreground/outline/shadow drawing reuses the same glyph coverage.
The WASI build does not provide browser threads or networking, so the browser
path is intentionally offline and uses uploaded folders, ZIPs or RGSS
script/archive files.

Build the embedded Ruby image before building the Trunk application:

```bash
nix-shell --run 'tools/build-ruby-wasm.sh'
nix-shell --run 'cd frontends/web && CARGO_TARGET_DIR=../../target/web-release trunk build --release --dist ../../target/trunk-dist'
```

The generated `frontends/web/ruby.wasm` and Ruby-WASM build directory are
ignored artifacts. The build script uses `bundle install`, builds the local
companion gem, then runs `bundle exec rbwasm build --ruby-version 4.0`; it
does not depend on a CDN Ruby image at runtime.

The build passes `--remake` because rbwasm's gem cache does not track the Rust
sources outside the companion gem. This reruns extconf/Cargo and relinks the
interpreter with current bindings instead of silently retaining an older
`Audio.bgm_play` that dispatches into the interpreter's inactive engine.

## Verification

Run all commands from the repository's Nix shell:

```bash
nix-shell --run 'NO_COLOR=false cargo check -p fanplay-web --target wasm32-unknown-unknown'
nix-shell --run 'cargo fmt --all -- --check'
nix-shell --run 'tools/build-ruby-wasm.sh'
nix-shell --run 'cd frontends/web && CARGO_TARGET_DIR=../../target/web-release NO_COLOR=false trunk build --release --dist ../../target/trunk-dist'
nix-shell --run 'cd frontends/web && NO_COLOR=false trunk serve --release=true'
```

The last command serves `frontends/web/index.html`; Trunk discovers the
`fanplay-web` package from that directory. Running Trunk from the workspace
root with the HTML path makes it search for a package root and produces the
`could not find the root package of the target crate` error.

To check the built interpreter's bindings and audio bridge under Node, install
the same ruby.wasm adapter used by the HTML loader into an ignored directory:

```bash
nix-shell --run 'npm install --prefix target/ruby-wasm/check --no-save @ruby/wasm-wasi@2.10.1'
nix-shell --run 'NODE_PATH="$PWD/target/ruby-wasm/check/node_modules" node tools/check-ruby-wasm.cjs'
```

An optional game directory argument (for example `ericlostie`) also executes
Pokémon Añil's own `Game_System#bgm_play_internal2` from `Data/Scripts.rxdata`,
including its five-argument `Audio.bgm_play` call. The check verifies the actual
WASI interpreter and serialized requests; browser decoding/playback still
requires a browser run.

For native-binding rendering parity and synchronization measurements, build
the small headless WASM engine (the same dispatcher and renderer used by the
browser). Use Trunk's installed wasm-bindgen executable if it is not on PATH:

```bash
nix-shell --run 'cargo build -p fanplay-rgss --example web-check --no-default-features --features portable-media,game-api --target wasm32-unknown-unknown --release'
nix-shell --run 'wasm-bindgen target/wasm32-unknown-unknown/release/examples/web_check.wasm --target nodejs --out-dir target/ruby-wasm/check-engine'
nix-shell --run 'NODE_PATH="$PWD/target/ruby-wasm/check/node_modules" node tools/benchmark-ruby-wasm.cjs'
nix-shell --run 'NODE_PATH="$PWD/target/ruby-wasm/check/node_modules" FANPLAY_WEB_FIXTURE=crates/fanplay-rgss/testdata/rgss_web_native.rb node tools/benchmark-ruby-wasm.cjs frontends/web/ruby.wasm native'
```

The benchmark compares the portable and native paths in the same Ruby image,
checks their final framebuffer hashes, and reports static/changing sprite
work plus full software rendering. It excludes browser presentation and UI.
`FANPLAY_WEB_FIXTURE` runs an existing Ruby rendering fixture through both
paths. `FANPLAY_WEB_GAME=ericlostie FANPLAY_WEB_FRAMES=300` instead replays the
local game through the two real WASM instances; pass `native` as the final
argument to select just the native path. This smoke replay exercises assets
and startup but does not validate a complete playthrough.

The native-binding change was measured with 256 sprites and 80 iterations per
workload, using three isolated runs per mode in alternating order. Medians on
the development host (milliseconds per iteration):

| Workload | Portable bindings | Native bindings |
| --- | ---: | ---: |
| Unchanged sprites | 2.667 | 0.048 |
| One quarter of sprites moving | 5.641 | 4.660 |
| Moving sprites plus update/render | 5.666 | 3.911 |

Render-workload Ruby allocations fell from about 3,319 to 58 per iteration.
All runs produced framebuffer SHA-256
`decfcad98aa62616a86768d36dd0d0a6d036e74078927e55d4ed598c52e41d7f`.
These are synthetic throughput measurements; timings vary with host load and
exclude browser presentation, UI and real-game startup. Separate Añil smoke
replays completed 1,000 Fiber resumes with nonblank frames on both paths.

