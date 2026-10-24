# Core design review and improvement plan

Reviewed 2026-10-24 against the current working tree, including the pending DS integration. This is a plan, not an implementation change.

> **Status (2026-10-24):** the desktop convergence items are implemented:
> panic-safe completion and terminal events, content-keyed storage with a
> versioned state envelope, bounded media/save workers, catalog routing, a
> feature-gated cartridge host, and the core conformance matrix. Web/mobile
> migration, measured transport latency baselines and loadable plugins remain
> targets. See `docs/multi-core-architecture.md` and
> `docs/core-conformance.md` for the implemented-versus-target split.

## Assessment

Keep the overall architecture: a small core contract, factories constructed on the execution thread, core-owned execution loops, borrowed output buffers, atomic controls, and feature-selected static registration. These accommodate RGSS's Ruby-driven execution and hardware emulators without forcing either into the other's model. A plugin ABI is premature.

The main gap is between that contract and the production frontend. `rsxp-frontend` currently provides routing and thread startup; desktop still has separate RGSS and cartridge paths, and web/mobile depend directly on the RGSS engine. The architecture document describes a target, not a completed migration.

Core-owned `run` is useful for lifecycle and RGSS compatibility, but does not itself establish a performance advantage over a monomorphic frame-step loop. Keep the interface and validate performance empirically; an optional deterministic execution mode need not replace it.

## Findings grounded in the code

| Priority | Finding | Evidence and consequence |
| --- | --- | --- |
| P0 | Completion is not panic-safe | `crates/rsxp-frontend/src/lib.rs`: `spawn_core` sets `finished` only after normal return. `frontends/desktop/src/gba.rs` polls this flag before joining. A core panic can leave that loop waiting indefinitely. |
| P1 | Production routing bypasses the catalog | `frontends/desktop/src/runtime/mod.rs` branches on `.gba`/`.nds`; `gba.rs` directly constructs registrations. Adding a core still requires editing frontend behavior. |
| P1 | Desktop cannot exclude RGSS | `frontends/desktop/Cargo.toml` unconditionally links `rsxp` and the Ruby crate. Cartridge input, bitmap and session handling also use existing RGSS infrastructure. Feature isolation exists in the core catalog, not throughout the application. |
| P1 | Hot-path rules stop at the trait boundary | `gba.rs::bitmap_from_frame` allocates and converts every frame; `Video::present` locks the runner; `PcmAudio` uses a mutex-backed, growable deque in producer and device callback. This contradicts the documented end-to-end aspirations. No latency regression is claimed without measurement. |
| P1 | Save identity and validation differ by core | Desktop directories use sanitized titles, allowing same-named content to share storage. GBA emits raw backend states; DS already checks version magic, ROM hash and checksum and restores into a candidate machine. There is no shared envelope or storage policy. |
| P1 | Storage ownership is split | GBA delegates native save-file writes to its backend; DS reads native paths but writes through `Platform`, including periodic synchronous flushes. Desktop event handlers also write synchronously. Virtual storage and consistent failure handling are not yet supported end to end. |
| P1 | Runtime negotiation is implicit | Desktop sets process environment variables for cartridge audio rates, while `CoreContext` has no session configuration or sink capability negotiation. `CoreInfo` lacks option definitions and detailed input/display descriptors. |
| P2 | Media semantics are underspecified | `Indexed8` has no palette field and desktop treats indices as grayscale. `VideoFrame` validates only with debug assertions, while fields remain public. PCM has no acceptance/queue-depth feedback or discontinuity/reset operation. |
| P2 | Multi-screen support is a flag, not a model | DS publishes a stacked 256×384 frame and interprets pointer Y ≥ 192 as touch. Desktop contains DS-specific mapping. Layout, rotation and separate screen presentation require explicit screen/input metadata. |
| P2 | Lifecycle and commands lack shared outcomes | Commands have no request ID or completion result; some unsupported commands are silently ignored. `Loaded` is core-emitted and `Exited` is not consistently emitted. Drop requests stop but detaches the thread; shutdown ownership needs explicit semantics. |
| P2 | Documentation exceeds demonstrated validation | Core/frontend tests cover basic routing and operation. GBA headless tests check pixels and state production, not deterministic state replay. DS tests cover more state validation. Docs contain stale default-feature and BIOS-startup descriptions. |

## Ordered implementation plan

### 1. Make session completion reliable

- Make completion observable on normal exit, load failure and unwind, using the join handle or an unwind-safe completion guard. Preserve the panic result for reporting; do not continue using a panicked core.
- Define one session owner for terminal status and reporting. Specify stop-during-load behavior, cooperative shutdown, and process timeout/termination policy for isolated sessions.
- Add focused tests for factory/load/run panics, normal failure, stop and nonblocking handle drop. Test the desktop polling behavior against a failed worker.

Acceptance: every terminated worker becomes observable without an unbounded polling loop; a single terminal outcome reaches the host. Native aborts remain the process supervisor's responsibility.

### 2. Standardize saves before moving more UI

- Introduce frontend storage keyed by core ID and content identity, with explicit migration of existing title-based saves. Preserve old files during migration; allow users to intentionally share saves between compatible ROM revisions.
- Define a versioned state envelope: core ID, backend/state-format revision, content identity, payload size and integrity check. Reuse DS's transactional restore approach and apply equivalent validation to GBA.
- Separate content reads, system-file reads and save storage in the platform contract. Return typed I/O failures instead of collapsing unreadable and missing files into `None`.
- Move persistence to a bounded frontend worker using atomic replacement, acknowledged writes and an orderly shutdown flush. Audit/adapt GBA's backend file writes first so they do not bypass this service. Surface write failures without silently losing pending saves.

Acceptance: same-title games do not collide; incompatible/corrupt states preserve the running machine; failed writes are visible and retryable; in-memory storage works in core tests.

### 3. Establish a measured media path

- Record baseline allocation counts, copied bytes, sink blocking time, audio queue depth/underruns, and frame-time percentiles for both a headless sink and the actual desktop transport.
- Replace PCM deque/mutex with a preallocated bounded SPSC ring. Negotiate sample rate/channels explicitly and define overload behavior, target latency, pause, fast-forward, reset and state-load flushing.
- Preserve source frame format/stride through the cartridge transport where supported. Where conversion or asynchronous ownership is necessary, use reusable bounded buffers with an explicit drop/backpressure policy.
- Separate input/control handling from video transport locking. Preserve RGSS's accelerated packet path and benchmark it separately.

Acceptance: no steady-state allocation or mutex in the new cartridge media bridge after warm-up; bounded audio/video memory; measured latency and underruns reported against the same baseline. Backend allocations are measured separately, not assumed absent.

### 4. Make the production frontend use the contract

- Route desktop content through `rsxp-cores::catalog()` and replace `gba.rs` with a generic cartridge/session host.
- Add typed session configuration, option schemas and sink capability negotiation. Replace audio-rate environment mutation with per-session configuration.
- Describe input controls and screens, including touch regions, independently of core IDs. Advertise supported pixel/audio formats; add a palette representation or reject indexed output explicitly.
- Gate generic UI actions by capabilities. Add command request IDs and success/unsupported/failure outcomes, with bounded payload sizes and queue-full handling.
- Extract generic session transport, audio, input and presentation types from RGSS dependencies; then make RGSS and its panels optional in desktop. Retain process isolation for MRI/global working-directory state.

Acceptance: a GBA-only desktop builds without MRI or RGSS; registering a new comparable core requires no extension branch in the host; RGSS desktop rendering and tooling retain existing behavior.

### 5. Build conformance and performance gates

- Reuse generated/homebrew fixtures in a shared harness: fixed input and clock, exact frame budget, frame/audio hashes, save–advance–restore–replay equivalence, corrupt/wrong-game state rejection, pause/reset/stop and storage failures.
- Inject deterministic RTC/time where needed; keep emulated time distinct from wall-clock pacing. A bounded execution option can live inside `run`; add a separate step/debug API only if a concrete consumer needs it.
- Add feature-matrix CI for core-only, GBA-only, DS-enabled and RGSS-enabled builds with their appropriate toolchains. Verify dependency isolation as well as compilation.
- Keep timing thresholds on controlled benchmark hosts; use deterministic correctness and allocation invariants in ordinary CI.
- Update architecture and runner docs to distinguish implemented behavior from targets, including DS defaults/toolchain requirements and GBA's current `skip_bios` startup.

Acceptance: each shipped capability has a contract test, state replay is deterministic under controlled inputs, and performance claims have reproducible measurements.

### 6. Extend portability after desktop convergence

- Migrate web/mobile to the proven configuration, storage and media services incrementally.
- Prototype browser execution scheduling before claiming the blocking/threaded `run` contract is portable: decide between a worker-based loop and a cooperative execution adapter.
- Defer dynamic plugin ABI, rewind, netplay and broader core additions until lifecycle, persistence and deterministic replay are stable.

## Scope and validation of this review

- Inspected contract, registry/thread driver, RGSS/GBA/DS adapters, desktop cartridge media/storage path, manifests, tests and architecture docs.
- `cargo test -p rsxp-core -p rsxp-frontend --offline`: 10 tests passed; one documentation example is ignored.
- `cargo test -p rsxp-core-gba --offline`: 5 tests passed, including the headless boot/reset and state-production tests.
- No GUI, audio-device, real-game performance or cross-platform behavior was exercised. Performance findings describe concrete code paths, not measured runtime regressions.
- Existing staged and unstaged changes were left intact; this plan is the only review artifact added.
