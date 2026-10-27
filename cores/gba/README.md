# Game Boy Advance core

`cores/gba/core` implements the shared [`fanplay_core::Core`](../../crates/fanplay-core/src/lib.rs)
contract using [RustBoyAdvance-NG](https://github.com/michelhe/rustboyadvance-ng)
(MIT, revision `1d7ff23a6adb42d803ebd393c3d27d94e4fea15e`).

Why this backend:

- library-first API (`GameBoyAdvance`, `GamepakBuilder`), no SDL/terminal
  frontend required;
- MIT licensed and compatible with FANPLAY's GPL-2.0-or-later distribution;
- frame buffer, input, audio, battery saves and full-machine save states;
- mature debugger and test-ROM coverage upstream.

Third-party license text: [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md).

## BIOS

The open-source Normmatt/VBA-M BIOS replacement is embedded in the core binary
(16 KiB, from [ReGBA](https://github.com/Nebuleon/ReGBA/blob/master/bios/gba_bios.bin)).
This is not Nintendo's official BIOS dump. No external BIOS is required for
basic operation, but a user-provided dump can override it for games that depend
on BIOS behavior not covered by the replacement. The override order is:

1. `FANPLAY_GBA_BIOS` environment variable;
2. `gba_bios.bin` or `bios.bin` beside the ROM;
3. the same names in the frontend save directory;
4. the embedded BIOS.

The embedded BIOS is GPL-2.0-or-later and is compatible with FANPLAY's license;
see [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md). The adapter starts directly
at the cartridge using the backend's post-BIOS register setup, skipping the boot
logo when the replacement BIOS is active. An official BIOS dump runs its normal
reset sequence. In both cases the BIOS remains mapped for SWI services during
gameplay.

## Saves

The cartridge real-time clock is enabled for all regional versions of Pokémon
Ruby, Sapphire, and Emerald, and for ROMs containing the `SIIRTC_V` signature.
The backend uses the host's local time for the clock.

The backend writes battery saves directly to `<rom-stem>.sav` in the
content-scoped `Platform::save_dir()` (`<FANPLAY_HOME>/saves/gba/<content-hash>/save`
on desktop). Save states are serialized by the backend and wrapped in the shared
versioned `StateEnvelope` with the core id, state revision, ROM fingerprint and
integrity check. A corrupt, foreign or different-ROM state is rejected before
restore.

## Video and audio

The native `u32` framebuffer is submitted zero-copy as `Bgrx8` on
little-endian hosts (GBA color channels expanded from BGR555). Big-endian hosts
receive `Rgba8`. Audio is drained from a preallocated ring buffer as interleaved
stereo `i16`; the frontend negotiates the output sample rate per session
(48 kHz when no device constraint is supplied).

## Tests

```sh
cargo test -p fanplay-core-gba
```

The headless test builds a small mode-3 test ROM, loads it through the generic
core contract, checks the presented frame, requests a save state and verifies a
clean frame-limited stop.

To run a real ROM through the same contract:

```sh
cargo run -p fanplay-core-runner -- --content /path/to/game.gba --frames 120 \
  --output /tmp/gba.ppm
```

The runner defaults to a 60-frame safety limit. Use `--fast-forward` for
throughput checks and `--save-state`/`--load-state` for state round trips.
