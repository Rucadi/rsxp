# RPG Maker runtime and compatibility

`fanplay-rgss` is the Rust engine package. `cores/rgss/ruby` embeds native MRI
and supplies Magnus bindings. RGSS 1, 2 and 3 are selectable, but
selection does not imply complete XP/VX/VX Ace or plugin compatibility.

## Startup

The native desktop path is `frontends/desktop/src/runtime/{mod,game,options}.rs`.
The generic-core path is `cores/rgss/core/{lib,game,host}.rs`. Both prepare the
game and archives before running Ruby. Keep these routes consistent when
changing Game.ini/script selection. `--script` runs an explicit file;
`--game` accepts a game directory or Game.ini, and the dispatcher also routes
cartridge files to cartridge cores. The run parser requires one content source.

The Ruby shim, compatibility helpers and RPG data definitions are assembled
from `crates/fanplay-rgss/src/ruby/`. The native bindings live under
`cores/rgss/ruby/src/bindings/`. The engine's command dispatcher is
`crates/fanplay-rgss/src/engine/commands.rs`; those files are the authoritative
API surface, not a list copied from another RGSS implementation.

## Implemented areas

The runtime has graphics/frame execution, input, bitmap/font resources,
sprites, planes/viewports, tilemap and retained-map paths, Tables, audio host
calls, archive loading, animated GIF and Theora movie decoding. CPU rendering
lives in `graphics`; optional native wgpu composition lives in `gpu`. Platform
services such as files, text/audio and presentation are supplied by hosts.
The game logic remains Ruby.

Game-specific functionality such as Essentials explorers and online-play
controls is supplied by Rhai plugins and their Ruby implementations. It is
separate from generic RGSS compatibility. See [extensions](../extensions/README.md).

## Limits

The native interpreter is modern MRI, not RPG Maker's original embedded Ruby.
Language differences, Windows APIs, arbitrary DLL extensions, RTP conventions
and game/plugin assumptions can still prevent execution. Targeted compatibility
helpers do not reproduce a complete Windows environment.

Theora movie decoding is implemented, but the movie path does not provide a
complete general-purpose media player. Unsupported operations/effects can fail
or take a software path; inspect the dispatcher and renderers before claiming
specific APIs or visual effects are supported.

## Running and diagnosing

```sh
cargo run --release -p fanplay-desktop -- run --game /path/to/game --rgss 1
cargo run --release -p fanplay-desktop -- run \
  --script crates/fanplay-rgss/testdata/rgss_demo.rb --rgss 3 \
  --headless --frames 60 --output /tmp/rgss.ppm
FANPLAY_INPUT=60:40:3 cargo run --release -p fanplay-desktop -- run \
  --script crates/fanplay-rgss/testdata/rgss_demo.rb --headless --frames 90
```

`--frames` limits completed graphics updates. Headless execution still runs
scripts and composition, with silent host audio; it is not a parser-only test.
`FANPLAY_INPUT` supplies deterministic frame/scancode/hold input.
`--trace-scripts` logs script sections. `--renderer` selects RGSS composition;
headless auto uses software, interactive auto uses the GPU path.

Native JIT settings are `off`, `yjit`, or `zjit`, subject to the actual Ruby
build. Generic patches and frame pacing are configured through settings and
the workspace; the `run` parser does not accept `--fps` or `--uncapped`.
Crashes, backtraces and output appear in the host/session logs. Generated
fixtures and unit tests are regression checks, not a compatibility list for
all fangames.
