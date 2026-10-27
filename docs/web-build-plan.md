# FANPLAY web build

The browser frontend is a thin Rust/WASM host for the existing eframe
application. It reuses the shared workspace, launcher, runtime tools, plugin
UI, core contract, video conversion, input model, save-state model and core
adapters. Browser-specific code is limited to file handles, `localStorage`,
Web Audio, the eframe canvas and the cooperative frame loop.

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
- battery saves in browser storage, save import/export and latest-save backup
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

RGSS is not included in the WASM target yet. Its core adapter depends on
`fanplay-rgss-ruby`, which embeds MRI Ruby through `magnus`; that is a native
Ruby runtime and cannot be enabled for `wasm32-unknown-unknown` by changing
eframe or Cargo feature flags. Supporting RGSS in the browser requires a
browser-compatible Ruby runtime/binding layer or a deliberate Rust replacement
for the embedded Ruby execution path. The web build keeps RGSS disabled rather
than shipping a misleading runtime that cannot load games.

## Verification

Run all commands from the repository's Nix shell:

```bash
nix-shell --run 'NO_COLOR=false cargo check -p fanplay-web --target wasm32-unknown-unknown'
nix-shell --run 'cargo fmt --all -- --check'
nix-shell --run 'cd frontends/web && NO_COLOR=false trunk build --release --dist ../../target/trunk-dist'
nix-shell --run 'cd frontends/web && NO_COLOR=false trunk serve --release=true'
```

The last command serves `frontends/web/index.html`; Trunk discovers the
`fanplay-web` package from that directory. Running Trunk from the workspace
root with the HTML path makes it search for a package root and produces the
`could not find the root package of the target crate` error.

