# Shared frontend and platform adapters

Desktop and browser now use the shared `rsxp_ui::ui::Workspace` and `SessionPanel`
inside the shared `rsxp-ui::SharedApp`. Eframe owns both windows/canvas and UI
input; browser HTML contains the canvas, file inputs, and a boot-error message.

`SessionCapabilities` describes available services, not an operating system.
`SessionActions` carries user intents out of the UI. Drawing never opens files,
starts downloads, or boots a VM. An Android or iOS host can enable ZIP/document
import while leaving raw paths, directory backups, torrents, and Ruby JIT disabled.
Do not infer any of these capabilities from `not(wasm32)`.

## Current adapters

- Desktop: `frontends/desktop/src/dashboard.rs` executes actions with native
  dialogs, filesystem discovery, recent-game storage, save-copy workers, and the
  existing child-process session protocol.
- Browser: `platforms/web/browser/launcher-controller.js` executes import and launch
  actions. Folder, ZIP, torrent, served, and embedded content all become the same
  `createLocalGame` interface. The browser library contains the currently prepared
  source; it does not pretend that browser permissions persist across reloads.
- `platforms/web/browser/player-launcher.js` starts `eframe::WebRunner` before choosing
  a game. The same WASM instance and eframe canvas are reused by `app.js` through the
  shared `SharedApp`; the browser runner only feeds it game frames and host actions.
- Browser filesystem adaptation remains in `local-game.js`, `game-import.js`,
  `app.js`, and `platforms/web/engine/src/files.rs`. Virtual files, archive handling, Ruby
  preload, save persistence, and synchronous reads after async preparation remain
  host responsibilities. A UI/window framework cannot replace these services.

## Eframe migration status

`crates/rsxp-ui` defines `SharedApp`, an eframe `App` that owns the shared
`Workspace`, takes game frames as egui textures, and exposes workspace actions.
The workspace layout accepts eframe's borrowed root `Ui`. Desktop starts it with
`run_native`; browser starts it with `WebRunner`.

The browser now uses eframe's WebRunner and uploads CPU game frames as egui
textures through `SharedApp`. The desktop launcher now uses `run_native`; it
temporarily asks its child renderer for CPU frames while GPU paint-callback
support is moved into eframe. Verify color space, DPI/game pointer mapping,
texture lifetime, surface recovery, and WebGPU/WebGL compatibility. Preserve
browser user activation for file pickers and audio startup.

## Android, then iOS

[eframe 0.37](https://docs.rs/crate/eframe/0.37.0) documents desktop, Android, and
WASM targets. Android supplies its activity through
[`NativeOptions::android_app`](https://docs.rs/eframe/0.37.0/eframe/struct.NativeOptions.html).
That provides a window integration, not the application's storage/runtime layer.
The Android adapter still needs document-provider URI access, persistent grants,
private save storage, lifecycle suspension/resume, audio, and a runtime launch
strategy that does not assume the desktop child-process executable exists.

Keep imported content identified by a host-owned handle with a display name;
never treat Android content URIs or browser handles as `std::fs` paths. Before
expanding the library to persistent mobile entries, split the current desktop
path/display string into these fields. Async acquisition and persistence should
complete through events; widgets must not block on filesystem or network work.

Treat iOS as a subsequent runner/integration project: verify upstream runner
support before selecting it, then supply document access, sandbox persistence,
background lifecycle, touch/IME, and an appropriate runtime configuration. Do not
make shared UI depend on Android activity types, POSIX paths, process spawning,
or JIT availability. This change tests capability combinations but does not claim
Android/iOS build or device validation.
