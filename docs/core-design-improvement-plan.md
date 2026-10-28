# Core boundaries and remaining work

The implemented core contract, static registry, native worker, save envelopes,
and cartridge adapters are documented in [multi-core architecture](multi-core-architecture.md).
This file is a future-work list, not another competing architecture guide.

- Make the shared graphical shell less dependent on RGSS services.
- Consolidate duplicated RGSS preparation while preserving native and headless behavior.
- Measure backend allocations and GPU readback before claiming throughput gains.
- Add new cores only with metadata, routing, save identity and deterministic tests.

Preserve the boundary: `cores/<system>` owns emulation; `fanplay-<system>` owns
Fanplay interface/connectivity integration. Native core lifecycle remains a
host concern.
