# Multi-core architecture

This document is the target architecture for RSXP as a multi-emulator system.
RGSS is the first core, not the application. GBA, GB/GBC and DS cores must be
able to run without linking MRI, egui, RPG Maker data code or RGSS panels.

The migration is intentionally incremental: the existing RGSS application must
keep working at every step.

## Goals

- Make an emulator core a self-contained, independently buildable unit.
- Make the frontend independent of any one core.
- Keep the emulation hot path monomorphic, allocation-free per frame, and free
  of locks, blocking I/O, UI work and process round trips.
- Preserve the existing RGSS GPU presentation path for desktop sessions.
- Let cores declare capabilities and options without teaching the frontend
  about RGSS, Pokémon Essentials or any other game family.
- Make save RAM, save states, screenshots, input remapping, fast-forward and
  audio the frontend contract rather than RGSS-specific plumbing.

## Non-goals

- Rewriting the RGSS renderer while moving crate boundaries.
- Sharing one Ruby VM between multiple games or cores.
- Running different cores in one process in the first migration stage.
- A plugin ABI before the in-tree cores prove the contract.

## Target layout

```text
Cargo.toml                     # virtual workspace
crates/
  rsxp-core/                   # core contract: no RGSS, MRI, wgpu or egui
  rsxp-frontend/               # core registry, content routing, session driver
  rsxp-cores/                  # build-time registry (feature-selected cores)
  rsxp-rgss/                   # RGSS engine + Ruby shim + RGSS UI (for now)
    src/                       # engine, graphics, gpu, data, media, Ruby shim
    api/                       # game bridge / MCP source
    testdata/
  rsxp-rgss-ui/                # (later) RGSS-specific egui panels and API bridge
cores/
  rgss/
    core/                      # RGSS adapter implementing rsxp-core
    ruby/                      # embedded MRI and Magnus bindings
  gba/
    core/                      # GBA adapter (RustBoyAdvance-NG backend)
  gb/                          # future
  nds/core/                    # Nintendo DS adapter (Dust; optional nightly feature)
frontends/
  core-runner/                 # headless generic runner for core validation
  desktop/                     # winit + egui + Kira desktop frontend
  web/                         # wasm frontend (separate workspace today)
  mobile/                      # iOS/Android host (separate workspace today)
```

The dependency direction is always:

```text
frontend  ---->  rsxp-core  <----  core
   |                                  |
   +---- optional core-specific UI ---+
```

A core never depends on a frontend, and `rsxp-core` never depends on a core.
`rsxp-frontend` may depend on `rsxp-core`; it must not depend on
`rsxp-rgss` unless a binary explicitly selects the RGSS core.

## Core contract

The contract lives in `crates/rsxp-core`.

```rust
pub trait Core: 'static {
    fn info(&self) -> &CoreInfo;
    fn load(&mut self, content: Content, context: CoreContext) -> CoreResult<()>;
    fn run(&mut self) -> CoreResult<ExitStatus>;
}
```

Cores are constructed on the thread that will run them. This keeps MRI,
thread-affine GPU state and any future JIT caches from crossing a thread
boundary. The frontend keeps a control handle and the core factory, not a live
core object.

`Core::run` owns the emulation loop. The frontend calls it **once per
session**, not once per frame. This is the most important performance decision:

- The core keeps its CPU state, decoded ROM data, JIT caches and renderer
  caches in one place.
- The compiler can inline and optimize the whole core loop.
- No Rust trait object, IPC message or command queue sits in the per-frame
  path.
- A core that needs a fixed timestep (GBA/GB/DS) can implement its own
  scheduler without the frontend imposing one.
- RGSS can keep driving MRI until `Graphics.update` returns, exactly as it does
  now, and publish each completed frame to the video sink.

`CoreContext` contains the only frontend services a core can use:

- `Platform`: file reads/writes, save directory, monotonic clock, sleep and
  logging.
- `VideoSink`: borrow a completed frame and present it synchronously.
- `AudioSink`: stream commands for pre-encoded music/effects (RGSS) or raw PCM
  (GBA/GB/DS).
- `EventSink`: low-frequency lifecycle events, window requests, logs, save
  state blobs and errors.
- `CoreControl`: atomics for stop/pause/fast-forward, lock-free button/axis
  state, and a bounded command queue for cold operations (reset, save state,
  option changes).

### Hot-path rules

1. **No per-frame allocation in the core contract.** `VideoFrame` borrows the
   core's pixels. `AudioSink::queue_pcm` borrows the core's sample buffer.
   `CoreControl::take_commands` reuses a caller-owned `Vec`.
2. **No per-frame locking.** Buttons, axes, pause, stop and fast-forward are
   atomics. The command queue is a cold path and is drained once per frame (or
   less).
3. **No per-frame virtual calls beyond the sink boundary.** One video submit
   and one or two audio submits per frame are acceptable and measurable. Per
   pixel, per sample, per sprite or per scanline callbacks are forbidden.
4. **Pixel formats are explicit.** A core publishes RGBA8, BGRA8, RGB565 or
   indexed frames with a stride. The frontend converts only when its
   presentation backend requires it.
5. **No implicit copies.** In-process frontends upload directly from
   `VideoFrame`. A process-isolated runner writes the borrowed bytes straight
   to the transport. The old path must not gain an intermediate framebuffer
   allocation.
6. **Accelerated presentation is an extension, not a CPU fallback.** RGSS can
   submit its encoded GPU packet through `VideoSink::present_packet`; a
   frontend that understands `rgss-gpu-v1` replays it. Other frontends fall
   back only if the core explicitly supports a CPU frame.
7. **Core crates choose their own optimization profile.** Hot cores are
   compiled with `opt-level = 3`, LTO and one codegen unit in release. Debug
   dependencies are opt-in per core, never workspace-wide.
8. **No UI or filesystem work in `run_frame`-equivalent code.** Asset loading
   happens in `load`; audio/file writes happen on cold paths or frontend
   threads.

### Why `run` instead of `step`

A conventional `step(&mut self) -> Frame` API is easier to embed, but it
forces the frontend to own pacing, input polling and the loop. That is wrong
for performance-sensitive cores and for RGSS, whose game loop is Ruby code.
With `run`, the core controls its timing and can batch or skip work exactly as
its hardware/software model requires. The frontend controls it through
`CoreControl` without entering the hot loop.

The contract can later add an optional `step` capability for debugging,
frame-advance and deterministic tests. It is not the primary interface.

## Content routing

A core registers:

- a stable id (`rgss`, `gba`, `gb`, `nds`),
- a display name and version,
- accepted extensions and/or a content probe,
- capabilities (`SAVE_STATES`, `SAVE_RAM`, `CHEATS`, `REWIND`, `RUMBLE`,
  `MULTI_SCREEN`, `ACCELERATED_PRESENTATION`, ...),
- supported pixel formats and audio modes.

The frontend probes content once at load time and chooses the highest-priority
matching core. Content routing never lives in an `if rgss` branch in the
frontend. A directory with `Game.ini`/RGSS archives routes to `rgss`; `.gba`
routes to `gba`; `.gb`/`.gbc` to `gb`; `.nds` to `nds`.

For emulators without a stable file magic (RGSS game directories), the probe
may inspect filenames and small header reads. Probing is a load-time cost only.

## Frontend responsibilities

The generic frontend owns:

- window, input devices and controller mapping,
- audio device and mixer,
- presentation and scaling,
- content library and core selection,
- session lifecycle, crash recovery and process supervision,
- save RAM, save states and screenshots,
- per-core option storage,
- generic UI: game list, core options, save states, input, performance.

The RGSS-specific frontend plugin owns:

- the game workspace (inspector, map, party, Pokédex, patches, Cable Club),
- the Ruby JIT selector,
- RGSS archive commands and game discovery,
- the RGSS GPU presentation packet decoder.

This split lets a GBA-only build exclude the RGSS UI and its dependencies.

## Session model

Two execution modes share the same core contract:

1. **In-process** (development, mobile, embedded): the frontend runs
   `Core::run` on a dedicated core thread and presents frames from the sink.
   This is the lowest-latency mode and the target for GBA/GB/DS.
2. **Isolated runner** (desktop): the existing dashboard launches a child
   process that loads exactly one core. The transport is generic: input
   commands in, video/audio/events out. RGSS may attach its accelerated
   presentation packet as a core extension. Isolation is retained for Ruby
   crashes, resets and working-directory changes.

Both modes use the same `rsxp-core` types. The isolated mode is a transport
adapter, not a second core API.

## Save data

The frontend, not the core, decides where user data lives. Cores expose:

- battery/save RAM as a byte blob (GBA/GB/DS) or game files (RGSS),
- save states as opaque versioned blobs with a core id and format version,
- optional thumbnail pixels for the save-state browser.

A core must reject a state blob from another core id or format version. State
loading must never silently reset a running game.

## Migration phases

### Phase 0 — plan and boundaries (this change)

- Add `rsxp-core` with the contract above.
- Add `rsxp-frontend` with registry, content routing and a core thread driver.
- Move the repository into `crates/`, `cores/` and `frontends/` so new cores do
  not start inside the RGSS application.
- Document the contract and hot-path rules.
- No RGSS behavior changes.

### Phase 1 — RGSS adapter

- Move game discovery and runner setup from `frontends/desktop` into
  `cores/rgss`.
- Implement `Core` for RGSS using the existing `Runtime`, MRI bindings and
  `Host` implementation.
- Keep the existing GUI path working; route the isolated runner through the
  generic runner first.
- The RGSS sink adapts `Host` audio/present calls to `AudioSink`/`VideoSink`.

### Phase 2 — generic frontend

- Move content library, session supervision, input mapping, audio device and
  save-state storage into `rsxp-frontend`.
- Keep RGSS panels in `rsxp-rgss-ui`, loaded only when the active core id is
  `rgss`.
- Add the core selector to the desktop library UI.

## Implemented cores

| Core | Backend | Status |
| --- | --- | --- |
| `rgss` | Embedded MRI + Rust RGSS engine | Adapter available and headless-tested; desktop runner migration in progress |
| `nds` | [Dust](https://github.com/kelpsyberry/dust) (GPL-3.0) | Optional `nds` feature; software 2D/3D, stacked screens and touch |
| `gba` | [RustBoyAdvance-NG](https://github.com/michelhe/rustboyadvance-ng) (MIT) | Adapter available and headless-tested |

The GBA adapter embeds the open-source Normmatt/VBA-M BIOS replacement from
[ReGBA](https://github.com/Nebuleon/ReGBA/blob/master/bios/gba_bios.bin), so no
external BIOS is required. `RSXP_GBA_BIOS` or a `gba_bios.bin`/`bios.bin`
beside the ROM or in the save directory can override it. The adapter runs the
BIOS boot without presenting frames, which skips the startup image while
preserving BIOS initialization and SWI services.

GBA battery saves are written through the backend to `<rom-stem>.sav` in the
frontend save directory. Save states are emitted as opaque blobs through
`CoreEvent::SaveState`.

### Phase 3 — first non-RGSS core

- Added `cores/gba` using RustBoyAdvance-NG.
- Validate the performance rules with a frame-time trace and allocation checks.
- Add a headless core test harness that asserts deterministic frame hashes,
  audio sample counts and save-state round trips.

### Phase 4 — loadable cores (optional)

- Add a stable C ABI (`rsxp_core_v1`) and a manifest for out-of-tree dylibs.
- Keep in-tree cores statically linked for performance and simplicity.
- A loadable core is a distribution feature, not a requirement for the
  architecture.

## Performance acceptance criteria

Each core must have:

- a headless benchmark reporting frame time, audio underruns and allocations
  per frame,
- a release profile with LTO and a single codegen unit,
- a frame-hash test that is stable across runs,
- a save-state round-trip test,
- no `Vec`/`String` allocation in the steady-state frame loop,
- no mutex acquisition in the steady-state frame loop,
- no more than one video sink call per frame.

The RGSS desktop path must not regress relative to the existing
`docs/session-performance.md` and `docs/rsxp-performance.md` baselines.
