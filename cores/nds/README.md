# Nintendo DS core

`fanplay-core-nds` adapts
[Dust](https://github.com/kelpsyberry/dust) at pinned commit
`76d79f25e4ea3ce355c4ac868f5ac9a233e64b89`. It is enabled in the default
desktop, core registry and headless runner features. Dust uses unstable Rust
features and pinned SIMD APIs; the Nix shell enables them for its
bundled compiler. No Nintendo ROM, BIOS or firmware is bundled.

```sh
cargo run -p fanplay-desktop -- run --game game.nds
cargo run -p fanplay-core-runner -- \
  --content game.nds --frames 120 --output frame.ppm
cargo test -p fanplay-core-nds
```

Both screens are presented as a single 256×384 RGBA frame (top screen first).
The backend uses Dust's interpreter and stereo PCM at 48 kHz or the desktop
device rate. Desktop sessions use Dust's optimized software 2D/3D renderers
by default. The experimental wgpu path can be selected with
`FANPLAY_NDS_RENDERER=wgpu`, but it currently has to read each frame back to
the CPU for the desktop cartridge transport, so it is not the recommended
gameplay path. The 3D pass and 2D compositor submissions are serialized on the
graphics thread to avoid cross-thread wgpu queue submission stalls. wgpu falls
back to Dust's synchronous software renderers if initialization fails.
Because the current cartridge transport carries CPU `VideoFrame`s between
processes, desktop wgpu frames are
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
beside the ROM, or use `FANPLAY_NDS_BIOS7`, `FANPLAY_NDS_BIOS9`, and
`FANPLAY_NDS_FIRMWARE`. Explicit overrides with invalid sizes or unreadable paths
produce errors. Supply your own dumps when needed.

## Saves

Cartridge saves use `<sanitized-rom-stem>.sav` in the content-scoped
`Platform::save_dir()` (`<FANPLAY_HOME>/saves/nds/<content-hash>/save` on desktop).
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

## Frame timing and benchmarks

For gameplay, use a release build:

```sh
nix-shell --run 'cargo run --release -p fanplay-desktop -- run --game "/path/to/game.nds"'
```

The desktop launcher
preserves the core's software renderer default; an explicit
`FANPLAY_NDS_RENDERER=wgpu` environment or per-game setting still opts into the
experimental GPU path. Desktop NDS sessions use a three-slot shared-memory
frame ring, so the runner sends only slot metadata over the session channel and
reuses its frame buffers; the dashboard still performs the final bitmap/GPU
texture upload. Incoming frames wake the desktop UI immediately.

The frame pacer includes emulation time in the first frame's budget and resumes
after a long stall without adding a full frame of sleep. Small overruns retain
the absolute schedule. Native timing is approximately 59.826 Hz, so a fixed
60 Hz display can still occasionally repeat a frame even with ample CPU headroom.

Measure core frame-time percentiles independently of desktop presentation:

```sh
nix-shell --run 'cargo run --release -p fanplay-core-nds --example frame_bench -- "/path/to/game.nds" 1800'
```

This runs unpaced, skips 120 warmup frames, and reports mean, p50/p95/p99,
maximum and frames exceeding the native 16.72 ms budget. It ignores existing
saves and discards writes. With no input it measures the opening sequence only;
it does not measure IPC, audio-device scheduling, display latency or gameplay.
Set `FANPLAY_NDS_RENDERER=wgpu` to compare the experimental renderer.

## Scope and compatibility

Wi-Fi, microphone input, DSi mode and DLDI storage are not exposed. Alternate
screen layouts are implemented by `fanplay-nds`, separately from this core. The RTC host offset is not serialized.
The tests use generated ARM programs without commercial ROMs; they do not
establish retail-game compatibility or real-time performance for every title.

The local compatibility patches are documented in [THIRD_PARTY.md](THIRD_PARTY.md).
DS-enabled binaries include GPL-3.0 Dust code. The default desktop/Nix package includes DS; dependency hashes and vendor
patches are recorded in the Nix configuration and Cargo lockfiles.
