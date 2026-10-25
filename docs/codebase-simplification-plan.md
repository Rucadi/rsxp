# Codebase simplification plan

Reviewed 2026-10-24 against the current working tree, including existing staged changes. This is a source review and implementation plan, not an implemented refactor or a measured performance result.

## Recommendation

Reduce the number of owners of each behavior before reducing files or crates. The strongest opportunities are RGSS startup duplication, generic UI depending on RGSS, repeated media conversions, and desktop session responsibilities mixed into the dashboard. Keep the core contract, platform adapters, process isolation, and specialized renderers.

Use small changes that remove an old implementation once its replacement is verified. A move that merely creates more modules is organizational cleanup; it is not sufficient evidence of simplification. Do not target an arbitrary line-count reduction.

## Findings

| Priority | Evidence in the current tree | Proposed simplification | Risk |
| --- | --- | --- | --- |
| 1 | `frontends/desktop/src/runtime/game.rs` and `cores/rgss/core/src/game.rs` both resolve RGSS versions, scripts, archives and startup environment. Desktop `runtime/mod.rs` and the RGSS adapter also repeat engine, patches and Ruby lifecycle setup. | One RGSS launch configuration and preparation implementation, then one lifecycle implementation once feature parity exists. | High if switched wholesale: desktop supports explicit version overrides, captures, scripted input, reset and path compatibility that the adapter does not reproduce identically. |
| 1 | `crates/rsxp-ui/Cargo.toml` unconditionally depends on `rsxp` with UI, game API and portable media. `SharedApp` directly owns `rsxp_ui::ui::Workspace` and accepts `rsxp::graphics::Bitmap`. Desktop depends on this crate unconditionally. | Put the generic shell and frame presentation in `rsxp-ui`; make RGSS panels an optional composition layer. | Medium; moving types across this dependency edge can introduce cycles. |
| 1 | Desktop `cartridge.rs::convert` and web `cartridge.rs::convert_frame` separately implement the same six pixel formats. Desktop reuses `Vec<u32>`; web allocates RGBA bytes and then constructs a Bitmap. | Shared validated conversion routines with caller-owned output storage and destination-specific output. | Medium; naïve unification could add a conversion or allocation to desktop. |
| 2 | `frontends/desktop/src/dashboard.rs` combines child ownership, TCP/stdio setup, authentication, worker threads, packet decoding, pending GPU updates, input and eframe UI. | Extract concrete session supervision and packet handling behind the existing transport boundary. Keep dashboard responsible for UI orchestration. | Medium; shutdown, ordered GPU uploads and backpressure must remain unchanged. |
| 2 | Desktop runtime routes through the catalog, but web `cartridge.rs::registration` selects cartridge registrations locally. Native catalog routing reads filesystem headers itself. | Expose selection from an already constructed probe; let filesystem and browser loaders feed the same registration selection policy. | Low to medium; browser builds must not acquire native runtime dependencies. |
| 2 | `docs/architecture.md` says the dashboard does not consume GPU transport, while `dashboard.rs` decodes GPU packets and installs `GameCallback`. The older improvement plan mixes completed findings with remaining targets. | Make current architecture documentation authoritative and label historical plans as historical. | Low; necessary before deciding that a path is obsolete. |

The unconditional dependency through `rsxp-ui` means the current manifest graph still includes the `rsxp` engine package in a GBA-only desktop build. This does not imply that MRI is linked or that every RGSS module is compiled. The documented claim of no RGSS engine dependency needs correction or completion.

## Intended ownership

- `rsxp-core`: small contracts and data validation—content probes, capabilities, controls, borrowed video/audio, events and state envelopes. No UI or platform policy.
- `rsxp-frontend`: shared application policy and utilities—routing, storage identity, persistence, native worker lifecycle and reusable media conversion. Keep native filesystem/thread services separable from portable utilities.
- `rsxp-cores`: feature-selected static registration. Keep this small crate; it prevents backend dependencies from leaking into the contract.
- `cores/*`: machine execution and core-specific behavior. RGSS owns its launch preparation, Ruby lifecycle and compatibility rules.
- `rsxp-ui`: generic workspace shell, library/session controls and presentation interface. RGSS inspector, party, Pokédex, patches and other game tooling compose into this shell through an optional adapter.
- Desktop/web hosts: device APIs, process or browser scheduling, storage implementation and transport. Their scheduling and audio implementations need not be identical.

Start with modules and concrete types. Introduce a trait only at an actual platform or optional-component boundary. Do not create a general plugin framework or a new crate for each extracted responsibility.

## Ordered implementation

### 0. Establish the baseline and reconcile documentation

Record the exact revision plus working-tree diff used as the baseline. Existing staged work must be preserved. Correct the GPU-path and dependency descriptions before classifying code as dead.

Use representative workloads: RGSS software and accelerated desktop sessions, GBA and DS headless plus desktop transport, and browser RGSS plus cartridges. Include retained scenes with changing assets, resize, fallback, capture and pause/reset. Record platform, renderer, toolchain, warmup and fixed inputs.

Capture median/p95/p99 frame times, allocation counts after warmup, transferred bytes, GPU readbacks/uploads, audio underruns and queue depth, and startup time. Headless throughput alone cannot establish UI or audio latency parity.

Acceptance: reproducible baseline and a current ownership map. No performance guarantee is made until candidates are measured against it.

### 1. Consolidate RGSS preparation without changing execution

First extract pure INI parsing, version selection and script/archive discovery into one RGSS-owned implementation. Keep file access and process mutations explicit. Return a launch description; apply cwd/environment changes only at the existing safe startup point.

Preserve desktop behavior deliberately: explicit `--rgss`, script versus directory modes, case-insensitive file lookup, archive validation, and Wine/UNC paths that must not be canonicalized. The adapter currently canonicalizes directories and has different script handling, so sharing code requires choosing compatibility semantics rather than moving either function unchanged.

Next share patch/runtime preparation and Ruby teardown order. Keep CLI parsing, output paths and process restart in the frontend. Route desktop through the adapter only after it can retain scripted input, frame limits, API startup, audio, accelerated packets, capture and reset behavior. Otherwise retain two thin entry points calling the same RGSS lifecycle helpers.

Acceptance: remove duplicated parsing/preparation bodies; run launch fixtures covering RGSS1/2/3, overrides, scripts, archives, mixed-case paths, capture and reset. Ruby finalizers must still run while native registries exist. Never move environment mutation onto a worker merely to reuse `spawn_core`.

### 2. Correct the shared UI dependency boundary

Inventory generic shell types currently inside `rsxp::ui`: workspace information/actions, library/session state, game-view presentation and generic controls. Move those into the UI layer; leave RGSS services and specialized panels with RGSS. Use a small optional panel/service adapter, not a dynamic plugin loader.

Replace `SharedApp`'s mandatory Bitmap input with a neutral frame presentation API. Preserve revision/size-based texture reuse and the existing GPU paint-callback entry point. The RGSS adapter can translate Bitmap metadata without introducing an extra pixel copy.

Avoid a dependency cycle: generic UI must not import the engine that supplies its optional panels. If Cargo packaging requires a separate RGSS UI crate, introduce exactly that boundary; otherwise keep the composition in the frontend.

Acceptance: GBA-only desktop has no dependency path to `rsxp`, `rsxp-rgss-ruby` or Magnus; full desktop/browser still expose all existing RGSS tooling. Check dependency trees as well as compilation. Preserve mobile's existing integration when relocating shared types; a complete mobile migration is separate work.

### 3. Share conversion and routing policy

Move format validation/conversion into portable frontend utilities. Provide direct conversion into reusable packed ARGB and RGBA destinations; avoid normalizing through an intermediate image. Preserve stride, palette, alpha, byte-order and current RGB565 expansion behavior. Skip conversion where source and destination already match.

Migrate desktop and web consumers independently. Remove their duplicated format switches once tests prove equivalence. Browser buffer ownership must account for UI use of the previous frame; do not reuse storage while a consumer still borrows it.

Expose catalog selection from a probe supplied by the caller. Native routing can read a header, while browser routing uses already loaded bytes. Preserve RGSS-project precedence over directories containing a single ROM. Keep backend feature registration centralized without pulling native MRI into wasm.

Acceptance: all formats, padded rows, palettes and invalid frames have deterministic conversion tests; steady-state desktop allocation/copy counts do not increase; browser allocations improve or remain unchanged; routing fixtures agree across supported hosts.

### 4. Isolate desktop session supervision

Move process ownership, startup/handshake, TCP/stdio transport and receive/control workers out of `dashboard.rs` into a desktop session module. Reuse the existing `SessionTransport` abstraction. Keep operating-system differences behind its implementations rather than spreading cfg branches through the UI.

Give the UI a bounded session snapshot and command interface. Represent CPU/GPU presentation state explicitly where that eliminates independently maintained flags. Keep GPU resource deltas ordered even when obsolete draw lists are dropped. Use current payload bounds, queue behavior and shutdown policy initially.

Only after this extraction, consolidate duplicated protocol parsing/encoding helpers between producer and consumer where demonstrated. Do not replace the binary GPU/pixel payloads with a generic JSON frame representation.

Acceptance: dashboard no longer owns socket/process internals; stop, startup failure, disconnect, slow peers and child cleanup retain behavior. Accelerated ordinary frames still avoid explicit readback, and screenshots after skipped views remain correct.

### 5. Remove leftovers and stop abstraction growth

After each migration, delete the replaced implementation, temporary adapters and unused imports/dependencies. Check all feature combinations and test/benchmark consumers before removing a path. `window.rs`, GPU helpers and serialized Ruby commands are not dead merely because the main dashboard uses a different entry point.

Review large UI files only for repeated concrete parsing, request state or row-building logic. Extract shared helpers when semantics match; retain separate panels with distinct responsibilities. Do not merge GBA/DS execution loops into a generic emulator framework just because command dispatch looks similar.

Acceptance: each refactor has one clear owner, fewer duplicate implementations or dependency edges, and no permanent migration switch. New abstractions must remove more complexity than they introduce.

## Performance and compatibility guardrails

Preserve retained scene plans and invalidation, copy-on-write bitmaps, revision-based uploads, tile/atlas caches, batching, and software fallback. Keep typed desktop Ruby bindings and portable serialized bindings as separate boundary adapters to common engine operations; forcing desktop through serialization would be a poor simplification.

Preserve the preallocated PCM ring, bounded media/save queues, borrowed core frames, source formats where supported, transactional state restore, and content-scoped save identity. Keep browser cooperative scheduling and native execution distinct. Do not alter vendored emulators or compatibility Ruby classes as part of architectural cleanup.

Performance acceptance means no repeatable regression outside baseline measurement noise in representative workloads, with unchanged allocation/copy/readback invariants on affected paths. Establish tolerances from repeated baseline runs rather than inventing a percentage. Run controlled performance comparisons separately from concurrent builds; use deterministic correctness checks in ordinary CI.

## Validation checklist for implementation

- Core contract/storage: `cargo test -p rsxp-core -p rsxp-frontend`.
- Cartridge adapter and catalog: `cargo test -p rsxp-core-gba` and `cargo test -p rsxp-cores --no-default-features --features gba`.
- GBA-only desktop: `cargo check -p rsxp-desktop --no-default-features --features gba`, plus explicit dependency-tree inspection for `rsxp`, Ruby and Magnus.
- DS: run its tests with the documented supported toolchain; see `docs/core-conformance.md` for current requirements.
- RGSS: desktop unit tests, CLI integration tests, launch fixtures, GPU/software parity tests and session transport tests relevant to each change.
- Web: existing `platforms/web/package.json` test scripts, including cartridge/launcher tests, then browser behavior and benchmark fixtures.
- Performance: `session-bench`, `render-bench`, `tools/benchmark-web.mjs`, and fixed-game workloads described in the performance documents. Compare the actual presentation path as well as headless execution.

This review inspected source, manifests, tests and performance documentation. No builds, tests or benchmarks were run: Cargo is not available on the current shell PATH. These commands are implementation gates, not reported passing results. No application source was changed by this review.

## Suggested delivery sequence

1. Documentation/baseline correction and shared RGSS discovery.
2. Generic UI boundary and optional RGSS composition.
3. Shared media conversion and probe-based routing.
4. Desktop session extraction and duplicate protocol-helper removal.
5. RGSS lifecycle convergence after parity, followed by deletion of replaced paths.

Steps 2–4 do not require a wholesale RGSS runtime migration. Complete lifecycle convergence last because it has the widest compatibility surface. This sequence produces useful simplification even if performance evidence prevents a later abstraction.
