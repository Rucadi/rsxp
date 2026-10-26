# Remaining architecture work

This is a backlog, not a description of unimplemented cores. Cartridge cores are
implemented. Shared workspace source now lives in `fanplay-ui`; interface code
lives in `fanplay-gba` and `fanplay-nds`; emulator code remains under `cores/`.
See [architecture](architecture.md) for the current graph.

Remaining boundaries visible in the implementation:

1. `fanplay-ui` still depends on RGSS bitmap, input and runtime services.
2. Desktop RGSS preparation and the generic RGSS adapter have separate paths.
3. The desktop dashboard still requires `gpu` plus `rgss` features.
4. Browser cartridge storage is not yet durable across sessions.
5. Mobile dependency metadata does not amount to a complete mobile host.

Changes should preserve existing behavior and use the checks in
[development](development.md). Keep emulator tests separate from UI tests;
do not move cartridge emulation into interface crates to resolve dependency work.
Prior benchmark records are historical evidence, not acceptance measurements
for an untested implementation.
