# Nintendo DS core

`rsxp-core-nds` adapts
[Dust](https://github.com/kelpsyberry/dust) at pinned commit
`76d79f25e4ea3ce355c4ac868f5ac9a233e64b89`. It is enabled in the default
desktop, core registry and headless runner features. Dust uses unstable Rust
features and the Rust 1.97 SIMD APIs; the Nix shell enables them for its
bundled compiler. No Nintendo ROM, BIOS or firmware is bundled.

```sh
cargo run -p rsxp-desktop -- run --game game.nds
cargo run -p rsxp-core-runner -- \
  --content game.nds --frames 120 --output frame.ppm
cargo test -p rsxp-core-nds
```

Both screens are presented as a single 256×384 RGBA frame (top screen first).
The backend uses Dust's interpreter and stereo PCM at 48 kHz or the desktop
device rate. Desktop sessions use Dust's wgpu 2D/3D renderers when available;
the 3D pass and 2D compositor submissions are serialized on the graphics
thread to avoid cross-thread wgpu queue submission stalls. They fall back to
Dust's synchronous software renderers if wgpu initialization fails.
WebAssembly uses the same Dust software 2D/3D renderers and shared emulation/UI
code; browser GPU threading is not required. Because the current cartridge
transport carries CPU `VideoFrame`s between processes, desktop wgpu frames are
read back before presentation. Pause, fast-forward,
reset, frame limits, screenshots, cartridge saves and versioned save states
use the shared core contract. RTC reads the host's local clock.

## Controls

| DS control | Keyboard / mouse |
| --- | --- |
| D-pad | Arrow keys |
| A / B | X / Z |
| X / Y | S / A |
| L / R | Q / W |
| Start / Select | Enter / Backspace |
| Touch | Hold the left mouse button on the lower screen |

Controller face buttons map by position, with shoulder buttons for L/R.
Core clients submit touch using `Button::Extra0` and pointer coordinates in
256×384 output space; leaving the lower screen releases the stylus.

## System files

Direct boot uses Dust's BIOS HLE and generated firmware by default. Some
cartridges require real system files, including ARM7 BIOS for encrypted ROMs.
Place `bios7.bin` (16 KiB), `bios9.bin` (4 KiB), and optionally `firmware.bin`
beside the ROM, or use `RSXP_NDS_BIOS7`, `RSXP_NDS_BIOS9`, and
`RSXP_NDS_FIRMWARE`. Explicit overrides with invalid sizes or unreadable paths
produce errors. Supply your own dumps when needed.

## Saves

Cartridge saves use `<sanitized-rom-stem>.sav` in the content-scoped
`Platform::save_dir()` (`<data>/rsxp/saves/nds/<content-hash>/save` on desktop).
The bundled Dust game database chooses EEPROM/FRAM/FLASH; an existing save's
length takes precedence. Unknown homebrew defaults to no save device unless
an existing save identifies one. NAND save cartridges are rejected because
Dust does not implement their saves. Modified saves flush every 300 frames,
on reset, on an explicit save request and on normal exit.

Save states use the shared versioned `StateEnvelope`: core id, state revision,
ROM fingerprint, cartridge save, backend payload and integrity checksum.
Loading validates the envelope and decodes into a new machine before replacing
the running one. They are tied to this backend revision, are not Dust desktop
state files, and are not a portable archival format. The frontend's
save/load-state commands work with this core; the desktop runner exposes save
states through the cartridge command queue.

## Scope and compatibility

Wi-Fi, microphone input, DSi mode, DLDI storage, and alternate screen layouts
are not exposed. The RTC host offset is not serialized.
The tests use generated ARM programs without commercial ROMs; they do not
establish retail-game compatibility or real-time performance for every title.

The local compatibility patches are documented in [THIRD_PARTY.md](THIRD_PARTY.md).
DS-enabled binaries include GPL-3.0 Dust code. The existing Nix packages keep
DS disabled; their dependency hashes include Dust so default builds continue
to resolve the shared lockfile.
