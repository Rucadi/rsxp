# Browser build

This directory builds and serves the engine as a web page. The Rust engine is
compiled to WebAssembly and owns rendering, archive decryption, input and the
native command dispatcher. The game's Ruby (our `rgss.rb` shim plus the game's
script bundle) runs under [ruby.wasm](https://github.com/ruby/ruby.wasm), and
talks to the engine through a small base64 bridge.

```
Ruby (ruby.wasm) --RSXP.call--> JS bridge --web_native_call--> Rust engine wasm
                                                                    |
                                            RGBA framebuffer <------+
                                                                    |
                              eframe WebRunner -> shared egui workspace
```

Because ruby.wasm cannot embed MRI the way the desktop build does, the Ruby
interpreter is a separate wasm module. `rgss.rb` is unchanged: it calls
`__rsxp_native`, which the page defines to call the engine.

The browser enables the same `ui` and `game-api` library features as the desktop
workspace. Game status, map tiles/world-map loading, encounters, NPCs, party,
Pokédex, move/item tools and debug actions use the same panels, request queue
and `crates/rsxp-rgss/api/game_bridge/` adapter. Press **F1** to reveal the workspace and
**F2** for Tools. Map previews decode tileset/autotile images from the browser's
virtual game files. No HTTP listener is needed for these tools.

Presentation and the workspace UI use **eframe** (the same app and widget set
as native). The page contributes only a `<canvas>`; there is no HTML chrome and
no 2D `putImageData` presenter. The engine uploads the finished frame to a
texture, draws it letterboxed, and runs the shared workspace (toolbar with
panel toggles, uncapped mode, FPS, reset, save export/import; runtime-patches
panel; performance panel) on top. Gameplay starts with the workspace hidden. F1 toggles
all workspace UI, F2 Tools, F3 Runtime output and F10 Performance. `WebRunner`
owns the browser canvas, input events, resize handling and wgpu presentation.
Game composition remains in the shared CPU renderer.

The library launcher is also shared Rust UI (`SessionPanel`). The browser boots
that workspace before a game is selected; its adapter supplies folder/ZIP/torrent
imports and the prepared game appears in the shared library. Select **Play game**
to start. Desktop keeps native path/ROM selection, recent games, backups, and JIT
options through host capabilities. See [shared frontend migration](../docs/shared-frontend.md)
for the eframe, Android, filesystem, and future iOS boundaries.

## Layout

| Path | Purpose |
| --- | --- |
| `Cargo.toml`, `src/` | Native `rsxp-web` executable: serve, export, embed |
| `engine/` | WASM frontend wrapping the shared `rsxp` library |
| `browser/` | Browser source and the player HTML template |
| `dist/` | Generated browser runtime, bindings, `player.html` (ignored) |
| `target/` | Native executable and Cargo build artifacts (ignored) |
| `tests/` | Native CLI integration and packaged-player checks |
| `xtask/` | Rust build orchestration, invoked with `cargo xtask web` |

## Build and run

```sh
# Inside `nix-shell` (provides the build tools):
cargo xtask web
platforms/web/target/release/rsxp-web serve                 # generic folder picker
platforms/web/target/release/rsxp-web serve ericlostie      # selected game
platforms/web/target/release/rsxp-web export player.html    # reusable single-file player
platforms/web/target/release/rsxp-web embed ericlostie game.html # game + runtime in one file
# serve prints the URL, normally http://127.0.0.1:8000/
```

After building, `npm --prefix platforms/web run benchmark` measures the actual Ruby WASM
to JavaScript to Rust WASM pipeline without game assets or a DOM. It reports
static/changing sprite synchronization and full update/render time, Ruby
allocations, bridge calls, and a deterministic framebuffer hash. Timings exclude
startup and canvas presentation; compare the same workload on the same host.
See [web performance](../docs/web-performance.md) for the optimization details.

`serve` defaults to loopback. `--port PORT` and `--bind ADDRESS` change the listener.
Assets resolve case-insensitively; hidden files and symlinks are excluded.
`embed` includes ordinary files, including saves: prepare a distribution folder
first if necessary. The output file itself is excluded when inside the game folder.
Embedded games show a Play game button to satisfy browser audio restrictions.
Game bytes are stored in small non-executable data blocks; HTML exports add
roughly a third to asset sizes due to base64 encoding.

## Reusable single-file player

`cargo xtask web` also writes **`platforms/web/dist/player.html`** (about 53 MiB). It embeds
both WASM runtimes, the shared application, RGSS shim and workspace. Copy that
one file wherever convenient, open it in a modern browser, click **Choose game
folder**, and select the folder containing `Game.ini` and `Data`. The selected
files stay local; this mode makes no asset requests and needs no local server.
The folder picker uses `webkitdirectory`; the browser may show a confirmation
before granting the page access to the selected files.

The running game uses the same renderer and Ruby bridge as the served build.
Assets and `patches/*.rb` are loaded into the shared virtual filesystem and
runtime-patch catalog, with case-insensitive lookup;
root `Game.rgssad`, `Game.rgss2a` and `Game.rgss3a` archives are supported. The
player HTML itself and hidden folders are excluded from preloading. Large
images/audio are kept out of Ruby's directory mirror to avoid duplicating them.

Saves use the existing browser storage/export/import flow, partitioned by selected
folder name. Browser storage for files opened directly from disk varies; keep a
backup with **Export save**. Reload/Reset asks you to select the folder again.
Folders with the same name share a save namespace inside a given player origin.

To export a copy after building:

```sh
platforms/web/target/release/rsxp-web export /tmp/player.html
```

The generated HTML is ignored by Git. No game assets are embedded in the generic
player; use `embed` for that.

## Assets

All three player modes use the same loader and virtual filesystem. The folder
picker reads selected files, `serve GAME` reads them from `/game/`, and `embed`
decodes the data blocks in the HTML. Assets are preloaded with bounded concurrency;
images and audio remain in the Rust engine rather than being duplicated in Ruby.

`browser/assets.json` is the smaller warm manifest mirrored into Ruby's filesystem,
alongside fonts, Ruby sources, the configured script bundle and Game.ini.
Ruby file APIs route through the shared engine for case-insensitive asset access.
Per-game `patches/*.rb` enter the shared runtime patch catalog.

## Log panel

The page shows a scrolling log of everything Ruby writes to stdout/stderr
(ruby.wasm's console printer), JavaScript errors, and status messages. When the
game stops, the real error appears here — for example a missing asset or a Ruby
exception — instead of only the fiber ending.

The `FPS (F10)` control toggles the performance panel, which shows the current
frame rate and frame count.

## How frames advance

`ruby.wasm` runs synchronously, so the game's `Main` loop cannot return to the
browser on its own. The page runs the whole game inside a Ruby `Fiber` and
replaces `Graphics.update` with a version that calls `Fiber.yield` after each
frame. The page then presents the framebuffer, waits for
`requestAnimationFrame`, and resumes the fiber. Input events are queued into the
engine from DOM handlers between resumes. Ruby startup also yields between
batches of asset copies and script decompression/evaluation, keeping the
workspace responsive while game scripts load. This avoids a Web Worker and
`SharedArrayBuffer`.

If Ruby raises an exception or the game exits, the frame pump stops resuming
Ruby but continues drawing the workspace over the last frame. Controls and
runtime output are revealed automatically, including Ruby's error output;
Reset game reloads the page. The normal game path keeps the same renderer,
frame pacing and memory ownership, with no worker or framebuffer transfer.
A Ruby loop that never yields still blocks the browser thread.

Because the game runs inside a `Fiber`, the page also raises
`RUBY_FIBER_VM_STACK_SIZE`/`RUBY_FIBER_MACHINE_STACK_SIZE` (and the thread
equivalents) via the VM environment to 64 MiB VM / 16 MiB machine. Fibers
otherwise get a much smaller stack than the main thread, which makes deeply
recursive game scripts raise `SystemStackError` over long play sessions
(surfaced as a `FiberError` when the pump resumes the dead fiber). The page
sizes can be overridden with the `?rubyStack=<bytes>` and
`?rubyMachineStack=<bytes>` query parameters; larger values trap ruby.wasm's
linear memory.

## Browser compatibility shims

`rgss.rb` and the page apply a few adaptations for ruby.wasm:

- `socket`, `io/wait` and `openssl` are marked as loaded if absent, so plugins
  that only `require` them (multiplayer features) load.
- `Thread.new` runs its block synchronously (ruby.wasm has no threads).
- The shared `crates/rsxp-rgss/src/ruby/` bundle supplies a portable `Table` fallback when native bindings are absent.
- `System.data_directory` tolerates a read-only filesystem.

The page and headless runner import the same VM setup from `browser/ruby-host.js`.
`crates/rsxp-rgss/src/ruby/`, the game adapter, engine and tool panels remain authoritative
shared sources. Only interpreter embedding, frame scheduling, files, media,
presentation and external transport need platform adapters. Desktop Magnus
bindings and retained/GPU fast paths are still platform-specific optimizations;
sharing capabilities does not imply equal browser performance.

## Known limitations

- Browser audio uses the browser's media decoder and Web Audio gain nodes from
  the Rust frontend. Browsers may require a click, tap or key press before the
  first sound can play; supported formats depend on the browser (MIDI is not
  supported by the native media element). Desktop Rust audio remains on
  Kira/CPAL.
- External HTTP/TCP listeners and cable services are desktop adapters. Browser game tools use the shared local queue; a future browser networking adapter can reuse it. `http`/`socket`-based plugins remain inert.
- Ruby files live in memory, but the page mirrors selected `.rxdata` save writes
  to localStorage under a key derived from the game root and restores them at
  startup. The shim filters writes to `System.data_directory` (with a
  `Game.rxdata` exception); other formats and write paths are not guaranteed to
  persist. Export/import moves the mirrored save bundle as JSON. Storage quota,
  browser storage clearing and game-root changes can affect persistence.

## Headless verification

`tests/game-e2e.mjs` runs the same pipeline as the page (engine wasm + ruby.wasm +
bridge + fiber pump) under Node and writes a frame to a PPM:

```sh
npm --prefix platforms/web test
npm --prefix platforms/web run test:cli
npm --prefix platforms/web run test:game -- 600 /tmp/web-frame.ppm
```

The headless check also requests game status, map 3 and the world map through
the shared Ruby inspector. It validates tile layers, maps outside the warm
manifest, outdoor metadata, and virtual `File.file?`/UTF-8 `File.foreach` access
without copying those files into Ruby’s filesystem. `npm test` verifies the wasm
command queue, replies, unsupported operations and framebuffer rendering.

After changing browser sources, run `cargo xtask web` to refresh the bundled player.
Rust checks:

```sh
cargo test --manifest-path platforms/web/xtask/Cargo.toml
cargo test --manifest-path platforms/web/Cargo.toml
```

### Build prerequisites

The build command is implemented in Rust. It invokes Cargo and `wasm-bindgen`,
then runs Binaryen's `wasm-opt -O3` on the engine before embedding it in the
player and host executable. It uses `wasm-ld` as the WASM linker.
On the first build it runs `npm ci` if the
pinned Ruby runtime is missing.
Use the project's `nix-shell`, or supply Rust's `wasm32-unknown-unknown` target,
Node/npm, `wasm-ld`, and Binaryen (`wasm-opt`). The build automatically installs and caches
`wasm-bindgen` **0.2.129** when the matching tool is not on PATH. `WASM_BINDGEN`, `WASM_LD`, and `WASM_OPT` can
select alternative executable paths.

Ruby WASM is a prebuilt upstream dependency, not rebuilt by rsxp. Running
`cargo xtask web` provisions it automatically. For an existing runtime, supply a folder
containing `browser.umd.js` and `ruby+stdlib.wasm` (Ruby 4.0, matching the pinned
`@ruby` packages **2.10.1**):

```sh
cargo xtask web --ruby-runtime /path/to/ruby-runtime
```

The runtime is cached in `platforms/web/dist/ruby/` for subsequent builds, and the matching
build tool is cached in `platforms/web/target/tools/`. The first build needs network access;
subsequent builds reuse these dependencies. The build also reads the pinned
packages from `platforms/web/node_modules/` if already present. Node is used for headless browser
tests and benchmarks, which are separate from the build.

Rust build-tool tests: `cargo test --manifest-path platforms/web/xtask/Cargo.toml`.

## ZIP and WebTorrent imports

The generic player accepts a local game folder, a game ZIP (file picker or drop
zone), or a WebTorrent magnet link / HTTP(S) URL to a `.torrent` file. ZIPs may
contain the game at the archive root or inside a wrapper folder. ZIP entries are
extracted lazily as the existing game loader preloads assets. Archives containing
multiple games or unsafe paths are rejected. Password-protected ZIPs are not
supported by this UI.

Paste a torrent identifier and press **Download game**. The launcher shows
progress, transfer speed and peer count; **Cancel** closes the download. Torrents
may contain a game directory or one ZIP file. Once ready, press **Start game**.
The browser needs WebRTC-compatible seeders or HTTP web seeds with appropriate
CORS access; ordinary TCP/UDP-only BitTorrent peers cannot serve a browser.
A `.torrent` URL also needs CORS access. See the [WebTorrent documentation](https://webtorrent.io/docs).

Downloading shares pieces with peers until game preload finishes, then the
client and its session storage are released. Torrents are not retained across
reloads; saves still use the normal save export/import flow. Browser memory
limits apply, especially for large torrents. ZIP and torrent support is bundled
in exported HTML, with no CDN needed; network sources still require internet.
The pinned third-party bundles and licenses live in `browser/vendor/`.
