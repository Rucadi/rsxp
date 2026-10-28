# Game Boy / Game Boy Color core

`fanplay-core-gbc` adapts the portable Rust emulator from
[`mvdnes/rboy`](https://github.com/mvdnes/rboy) to FANPLAY's generic core
contract. The cartridge header selects DMG or CGB mode automatically, so both
`.gb` and `.gbc` cartridges use the same core and 160×144 presentation path.

The adapter owns the FANPLAY frame loop, input mapping, audio conversion,
battery-save path, screenshots and versioned save-state envelope. The upstream
core's known hardware trade-offs remain applicable; see its README for the
accuracy and compatibility notes.

The upstream project is MIT licensed; its notice is retained in
`THIRD_PARTY_LICENSES.md`.
