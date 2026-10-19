# Runtime behavior and RGSS compatibility

This page describes the implemented runtime rather than a completed compatibility
specification. See the [README](../README.md) for launch commands and
[architecture](architecture.md) for ownership and module boundaries.

## Game loading

`desktop/runtime/options.rs` accepts exactly one of `--script` and `--game`.
Script mode canonicalizes the script and uses its parent as the working directory;
game mode uses the selected directory. Relative capture output paths are resolved
before that directory change.

`desktop/runtime/game.rs` reads `Title` and `Scripts` from Game.ini. `--rgss 1|2|3`
overrides inference; otherwise the configured script extension, standard archive
names and standard script files select the version, falling back to RGSS1.
Default bundles are `Data/Scripts.rxdata`, `.rvdata` and `.rvdata2`. Recognized
`Game.rgssad`, `Game.rgss2a` and `Game.rgss3a` archives are loaded into memory.
Engine asset lookup supports loose files, archives, normalized separators and
ASCII case-insensitive resolution.

`rust/ruby/` supplies the Ruby API and script loader, assembled at compile time
as `rsxp::ruby::RGSS_SOURCE`. Bundles contain Marshal
arrays of `[id, name, zlib_source]`; sources are decoded and evaluated in order
in TOPLEVEL_BINDING, with decoded text retained in `$RGSS_SCRIPTS` slot 3.
Section names appear in error backtraces. Embedded XP/VX/VX Ace RPG class
definitions let Marshal reconstruct RPG data objects. A game `preload.rb`
runs before the main scripts.

This uses modern MRI, not the original RPG Maker Ruby interpreter. Legacy
language semantics, Windows code pages and arbitrary DLL extensions are not
automatically reproduced. The shim includes targeted adaptations such as old
`File.exists?`/`Dir.exists?` aliases, string encoding handling, Kernel helper
forwarding and in-memory repairs of redundant plugin self-inheritance.

## Implemented API areas

The definitions in `rust/ruby/`, desktop `rust/ruby/desktop/bindings/`, and operations in
`rust/engine/commands.rs` are the authoritative API surface. This table is an
overview, not a promise that every original RGSS method is supported.

| Area | Implemented behavior |
| --- | --- |
| Graphics | Updates, waits, dimensions/resizing, frame counters/rate, real-time `delta`/`average_frame_rate`, display size, window resize, screenshots, frameskip/threadsafe, scene-recompositing snapshots, freeze, basic transitions, brightness/fades, Theora (`.ogv`) `play_movie` and host window controls |
| Input | Button polling and repeat/trigger/release state, directions, extended scancodes, mouse and text input, in-process clipboard, native/browser controller actions with saved remapping, and controller presence |
| Bitmap / Font | Creation, image loading (including animated GIF frames), text drawing/measurement, fills, linear gradients, box/radial blur, pixel access, raw RGBA data, blits/stretching, hue changes, static snapshots, PNG export, hi-res aliasing, KGL2 operations, cloning and disposal |
| Sprite | Position/origin, source rectangle, zoom/rotation, opacity, blend modes, mirror, color/tone, bush transparency, pattern overlays and viewport membership |
| Plane / Viewport | Tiling/scrolling, clipping, ordering, color/tone and grouped viewport composition |
| Tilemap | RGSS1 visible tile sprites, animated autotiles and priority handling; shared native atlas/geometry optimization |
| Table / values | Table storage/indexing/resizing and Marshal payloads; Rect, Color and Tone values and serialization |
| Audio | BGM/BGS/ME/SE playback, stopping, fades, volume and positions through platform backends |
| Data and system helpers | `load_data`, `save_data`, RGSS entry helpers, console message boxes, time/platform/title/data-directory helpers, and `System` compatibility (`memory`, `nproc`, `file_exist?`, `parse_csv`, `window_title`, `delta`, `desensitize`, BSD flags) plus the `CFG` config surface |
| HTTPLite | JSON helpers; network methods explicitly raise `RSXPError` |
| Win32API | Targeted emulation plus native desktop DLL handling; the `KGL2.klib` light-blending and shadow functions are emulated portably (`KGL2_Impl`) |

Known explicit restrictions include unsupported blend values and
filename-based pattern transitions. `Graphics.play_movie` decodes Theora video
in-engine on every platform but does not mix the movie's Vorbis audio track yet.
Remaining mkxp-z APIs that rsxp does not implement include the native `Window`/
`WindowVX` and `TilemapVX`/`TilemapAutotiles` classes, `MiniFFI`, Steam
(`SteamLite`/achievements), Bitmap frame editing (`add_frame`/`remove_frame`/
`next_frame`/`previous_frame`/`looping`), rendered `Sprite` wave effects, OS
clipboard (rsxp keeps an in-process clipboard), raw controller axes, battery
and raw button-query helpers, hi-res asset replacement, and `System.launch`/
`mount`/`unmount`. HTTP game-script methods are
not supplied by the separate live game inspection API. RTP discovery and
arbitrary engine/plugin extensions should not be assumed. Unsupported native
operations return an error rather than silently gaining compatibility.

The engine limits bitmaps and Tables to 16,777,216 pixels/elements. Desktop
Tables use native Magnus bindings and Ruby GC; browser Tables are pure Ruby.
Bitmap/sprite/plane/viewport objects refer to native resource IDs and release
them on disposal or finalization.

## Frame limits, input and failures

`--frames N` requires a positive integer and counts completed Graphics updates
independently of the writable `Graphics.frame_count`. A frame limit or host
close produces the internal clean-stop signal. Ruby errors report backtraces
and fail the process; Ruby exit status is propagated. Escape is a game input,
so scripts decide what it does.

Headless runs omit window polling/presentation and pacing; they do not skip
Ruby logic or rendering. Desktop headless audio is silent. Inject repeatable
input with physical scancodes:

```sh
RSXP_INPUT=60:40:3 cargo run --release -p rsxp-desktop --bin rsxp -- run \
  --script rust/testdata/rgss_demo.rb --headless --frames 90
```

Each comma-separated `FRAME:SCANCODE[:HOLD]` entry presses a key for HOLD updates
(default one); scancode 40 is Return. This is separate from game-specific input
helpers and the live game API's queued controls.

## Runtime controls and diagnostics

Frame rate starts at 40 for RGSS1 and 60 for RGSS2/3. Workspace controls can
set a rate override or disable pacing; uncapped execution speeds per-frame
game logic up in wall-clock time. Use F10 for performance and F11 for uncapped
execution. These are runtime controls, not `--fps`/`--uncapped` CLI arguments.

| Setting | Purpose |
| --- | --- |
| `--renderer auto\|software\|wgpu`, `RSXP_RENDERER` | Select compositor; headless auto uses software |
| `--trace-scripts` | Log script sections during loading |
| `RSXP_YJIT=0` | Disable the desktop attempt to enable YJIT |
| `RSXP_INPUT` | Scripted frame/scancode input |
| `RSXP_PATCH=ID[,ID...]` | Enable runtime patches at startup |
| `RSXP_GPU_BATCH=0` | Disable disjoint GPU draw grouping for comparison |
| `RSXP_NATIVE_TILEMAP=0` | Disable native tile sprite optimization |
| `RSXP_SCENE_CACHE=0` | Disable retained scene ordering |

Runtime patches are cataloged in `rust/patches/mod.rs`, with Ruby bodies beside
it. The engine tracks toggles/values; the Ruby shim fetches a newly enabled
patch, evaluates it and calls its apply/revert/value callbacks. The built-in
`map_panoramic` patch targets Essentials map rendering and stays inert when its
required classes are absent. Patches are game-specific modifications, not
universal RGSS compatibility fixes.

## Browser differences and validation

The browser uses a separate Ruby WASM module, a serialized bridge and a Fiber
scheduler. Its filesystem, audio, save persistence and thread/network shims have
additional constraints; see [browser documentation](../web/README.md).

`rust/tests/cli.rs` exercises embedded Ruby, all three RGSS modes, rendering,
Table GC/Marshal, scripted input, failures/exit status, frame limits, compressed
bundles, encrypted games and isolated runner transport. Unit tests cover core
rendering/input/data operations. GPU checks and some local-game checks are
ignored by default. A passing fixture establishes that fixture's behavior,
not full compatibility with every game using the same RGSS version.
