# Emulator cores

Emulation stays here. Fanplay interface/GUI/connectivity integration belongs in
`crates/fanplay-gba` and `crates/fanplay-nds`, separately from the emulator backends.

| Directory | Package / role |
| --- | --- |
| `gba/core` | `fanplay-core-gba`: RustBoyAdvance-NG adapter, BIOS assets and tests |
| `nds/core` | `fanplay-core-nds`: Dust adapter and game database |
| `nds/vendor` | Local compatibility/rendering patches for Dust dependencies |
| `rgss/core` | `fanplay-core-rgss`: generic core adapter for the RPG Maker runtime |
| `rgss/ruby` | `fanplay-rgss-ruby`: native MRI/Magnus embedding |

All adapters implement the contract in
[`fanplay-core`](../crates/fanplay-core/src/lib.rs). The build-time registry is
[`fanplay-cores`](../crates/fanplay-cores/src/lib.rs). Defaults include GBA and
DS; RGSS is opt-in in the generic registry/runner and enabled by the default
desktop features. There is no implemented GB/GBC core.

For a new core, provide content detection, metadata, a core factory, platform
services integration, bounded frame/audio output and deterministic tests.
Use capability declarations for commands a core really supports. Keep the
interface crate independent of the backend. Native workers and browser
cooperative scheduling have different lifecycle requirements.

Release LTO is intentionally disabled in the workspace because of the upstream
GBA interpreter. Do not enable it as a generic optimization without checking
real cartridge behavior. Allocation and frame-time claims require measurements.

See [core architecture](../docs/multi-core-architecture.md),
[conformance](../docs/core-conformance.md), [GBA](gba/README.md), and
[DS](nds/README.md).
