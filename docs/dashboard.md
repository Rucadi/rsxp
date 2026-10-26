# Desktop workspace

The desktop executable is `fanplay`, built by `fanplay-desktop`. With default
features, launching without arguments opens the game library. Interactive
`run --game PATH` opens the dashboard and launches a child game process;
`--headless` executes without the dashboard. The GUI currently requires both
`gpu` and `rgss` features even for cartridge sessions.

## Library and settings

`frontends/desktop/src/launcher.rs` owns the native library, repositories and
storage page. `launcher_settings.rs` edits typed launch settings. The shared
workspace and library/session widgets live in `crates/fanplay-ui/src/ui`.
The native and browser hosts share widgets but expose different import and
filesystem capabilities; they are not identical applications.

The library can launch RPG Maker folders/Game.ini and GB/GBC, GBA or DS ROMs. Provisioned
catalog entries can be installed after download/hash validation. Selection is
not a compatibility certification. See [repositories and storage](software-provision.md).

Gameplay starts without the workspace chrome. F1 shows/hides it; F2 toggles
tools, F3 output, F4 fullscreen, F10 performance and F11 uncapped execution.
Fullscreen entry hides controls; F1 can show them again. The toolbar exposes
frame pacing, reset, view placement and controller configuration.

For DS sessions, `fanplay-nds` owns vertical/horizontal, large-top/large-touch,
and single-screen layouts. It computes the rendered screen rectangles and
lower-screen touch rectangle. These are frontend layouts; the DS core still
produces a stacked 256×384 frame. GBA uses aspect-preserving presentation;
`fanplay-gba` owns its library presentation metadata.

## Session and tools

`dashboard.rs` supervises the child process and forwards frames, input, logs
and requests through `session.rs`. `cartridge.rs` adapts generic cores;
`runtime` and `host.rs` implement the direct RGSS path. `gpu_present.rs` and
`gpu_callback.rs` handle native presentation. The dashboard owns the launcher
HTTP listener while a session is running.

Game-specific tools are Rhai extensions. On native hosts, HTML tools use
webviews; browser hosts use their own JS adapter. The shared runtime-service
adapter is in `fanplay-ui/src/workspace_services.rs`, and RGSS data/mutations
remain owned by the loaded extensions. Tool availability depends on the loaded
game and extension declarations.

## Saves and appearance

Desktop saves/settings/backups live under the selected portable home. RGSS
save files and emulator state snapshots are different formats. Close/flush
behavior matters: copying a live directory is not equivalent to taking a
coordinated backup. Consult [storage](software-provision.md) and [core state](multi-core-architecture.md).

The implemented theme definitions are in `fanplay-ui/src/ui/theme.rs`:
Adventure Journal (default), Arcade Neon, Moonlit Grove and Field Guide.
Native custom definitions are loaded from `config/themes.json`. The design
briefs describe visual intent; they are not a feature checklist.
