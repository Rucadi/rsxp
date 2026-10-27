# Rhai plugins

The bundled plugins in this directory are embedded into the native UI host.
Native hosts can additionally load plugins from disk; each
plugin owns form state, UI layout, request orchestration, loading state and
user-facing feedback, while the RGSS bridge remains authoritative for game
data and mutations.

A plugin may opt into live Ruby patching by declaring both `ruby.inject`
and `ruby.unload` in `capabilities`. Its Rhai entry point can then use:

```rhai
let handle = ruby_inject("my-patch", "{ apply: proc {}, revert: proc {} }");
// After the injection response arrives:
let unload_request = ruby_unload(handle);
```

The Ruby source must evaluate to a hash containing `apply` and `revert` procs.
The returned value is an asynchronous request handle, not a synchronous Ruby
result. The host installs it as a reversible runtime patch and unloads it by
running `revert` before removing the patch. The patch is scoped to the current
RGSS session and is discarded when that session ends.

## Dynamically loaded plugins

Native hosts also load Rhai plugins from `FANPLAY_EXTENSIONS_DIR`. The
variable may point to one `.rhai` file or to a directory whose immediate
children are `.rhai` files. A
single Rhai file uses its filename as the ID and can override identity with
`// fanplay:key=value` metadata comments; no companion manifest is required. The
same header can declare capabilities, operations, panels, and routes, for
example:

```rhai
// fanplay:capabilities = ruby.inject,ruby.unload
// fanplay:operation = my_operation
// fanplay:route = GET /v1/my-operation my_operation value
// fanplay:panel = main,My Plugin
```

From the repository root, run:

```sh
FANPLAY_EXTENSIONS_DIR="$PWD/extensions" cargo run -p fanplay-desktop --bin fanplay
```

The bundled plugins are skipped and [`dynamic-demo.rhai`](dynamic-demo.rhai)
is loaded from disk. It is a UI-only example that can verify dynamic loading
without a game-specific operation.

Applications can also call `SharedApp::paste_plugin(id, source)` to load Rhai
text directly, or `SharedApp::import_plugin(path)` on native hosts. The Runtime
tools panel exposes both the paste and file-import actions.
Rhai files that need Ruby can declare `ruby.inject,ruby.unload` in metadata and
embed the implementation directly in the same file:

```rhai
/* fanplay:text ruby-begin
module MyPlugin; end
fanplay:text ruby-end */
```

The host removes that block before compiling Rhai and exposes it to
`text_block("ruby")`, so the script explicitly decides when to insert it into
its runtime patch. The same format works for large HTML/CSS/JavaScript
templates, for example `text_block("html")`; it avoids escaping the template
as a giant Rhai string. `essentials-pokedex.rhai` uses both blocks: it owns the
Pokédex index/detail/encounter Ruby bindings, HTTP routes, and its HTML explorer.
`essentials-movedex.rhai` follows the same pattern for the move
registry, including its filters, machine actions, and responsive HTML explorer.
`essentials-world-map.rhai` follows the same pattern for map metadata, tile
previews, connected-map layout, Canvas tile composition, interior selection,
and teleport controls. The Rust workspace has no explorer-specific UI or
world-map rendering/data code. `essentials-itemdex.rhai` follows it for the
active game's item registry, legacy item decoding, item icons, filtering,
inventory actions, and the HTML item explorer.
`essentials-encounters.rhai` provides current-map encounter tables and polls
the generic status endpoint so map transitions reload the view.

Plugin routes are intentionally not compiled into the host OpenAPI document;
the live `/v1/extensions/routes` discovery endpoint is the source of truth for
Rhai-owned API routes.

## NDS memory and cheats

An NDS plugin opts into the NDS session with:

```rhai
// fanplay:cores = nds
```

NDS memory helpers are asynchronous and return a request ID. Results arrive in
the normal `on_operation_result(id, result)` callback. `nds_read` reads a
little-endian 1-, 2-, or 4-byte value; `nds_read_bytes` reads up to 1 MiB;
`nds_write` writes RAM or save RAM. The `space` argument may be
`"cartridge"`/`"rom"`, `"ram"`/`"memory"`, or `"save"`/`"save_ram"`.
Cartridge addresses are byte offsets into the ROM. RAM addresses use the DS CPU
address space, so ARM9 main RAM and ARM7 work RAM can both be inspected.

```rhai
// fanplay:cores = nds

fn on_activate() {
    nds_read("cartridge", 0x100, 4);
    nds_read("ram", 0x02000000, 4);
    nds_read_bytes("cartridge", 0, 16);
}

fn on_operation_result(id, result) {
    state_set("last_nds_request", id);
    state_set("last_nds_result", result);
}
```

`nds_add_cheat(code)` and `nds_remove_cheat(code)` accept standard Action
Replay DS lines (`XXXXXXXX YYYYYYYY`, with optional spaces or hyphens),
including multi-line conditional and repeat codes. `nds_clear_cheats()` removes
all active codes. Cheats are applied once per emulated frame; cartridge ROM is
read-only, while RAM and save RAM are the writable spaces.

### Pokémon White Spain/EUR example

[`pokemon-white-cheats.rhai`](pokemon-white-cheats.rhai) is a complete NDS
plugin with a small UI for Pokémon White (Spain/EUR). It applies the confirmed
max-money code at `0x0223CD6C` and removes all active cheats when disabled or
when the plugin shuts down. The code is intentionally kept as a dynamically
loaded example instead of being bundled into every session.

Load it from the repository root with:

```sh
FANPLAY_EXTENSIONS_DIR="$PWD/extensions/pokemon-white-cheats.rhai" \
  cargo run -p fanplay-desktop --bin fanplay
```

It can also be loaded through Runtime tools → Import plugin. After Pokémon
White (Spain/EUR) is running, open the plugin panel and select **Enable max
money**. This address is region-specific; do not use it for the US/NA release
or Pokémon White 2.

## GBA memory and cheats

A GBA plugin opts into the GBA session with:

```rhai
// fanplay:cores = gba
```

The GBA helpers mirror the NDS ones and are asynchronous. They return a request
ID and deliver their result to `on_operation_result(id, result)`. `gba_read`
reads a little-endian 1-, 2-, or 4-byte value; `gba_read_bytes` reads up to
1 MiB; `gba_write` writes live console memory. The `space` argument may be
`"ram"`/`"memory"` (GBA CPU address space: EWRAM, IWRAM, I/O, palette, VRAM,
OAM), `"cartridge"`/`"rom"` (byte offsets into the ROM, or `0x08000000`-based
ROM addresses), or `"save"`/`"save_ram"` (read-only battery save contents).
Cartridge ROM and save RAM are read-only from plugins.

```rhai
// fanplay:cores = gba

fn on_activate() {
    gba_read("ram", 0x02000000, 4);
    gba_read_bytes("cartridge", 0, 16);
}

fn on_operation_result(id, result) {
    state_set("last_gba_request", id);
    state_set("last_gba_result", result);
}
```

`gba_add_cheat(code)`, `gba_remove_cheat(code)`, and `gba_clear_cheats()` accept
the usual GBA formats:

- CodeBreaker `XXXXXXXX YYYY`, for example `82005274 0044`.
- GameShark / Action Replay `XXXXXXXX YYYYYYYY`. Plain codes and
  TEA-encrypted GameShark Advance v1/v2 and Action Replay v3 codes are detected
  automatically.
- VBA `ADDRESS:VALUE`, for example `02000000:63` (1 by default), or
  `02000000:0063` / `02000000:00000063` for 2/4-byte writes.

Ordinary memory writes, fills, 8/16-bit arithmetic (`OR`/`AND`/`add`), and the
one- or two-line conditional forms are applied once per emulated frame. ROM
hooks/patches, EEPROM commands, keypad action codes, and PARv3 block conditions
are rejected with an explicit error rather than silently ignored. Titles,
comments, blank lines, and `#` lines are skipped; `//` starts a trailing
comment and a leading `;` is ignored.

## HTML plugin views

A plugin that declares `ui.webview` can open an HTML GUI from its Rhai event
handler:

```rhai
let html = "<button onclick=\"fetch('/v1/game/status').then(r => r.json()).then(x => document.body.innerText = JSON.stringify(x))\">Refresh</button>";
webview_open("main", "My plugin", html);
```

On desktop this creates a native Wry/WebKit view (a separate resizable window
on Linux and macOS). The host starts the loopback game API when the view is
opened and rewrites relative `/v1/...` requests to that API. The transport
includes CORS headers for plugin pages.
The view's close button removes the view; a plugin can also call
`webview_close("main")`.

`essentials-game-api` uses the same mechanism for its game-facing API: its
`main.rhai` injects the package's `game_api.rb`, waits for registration, then
queries `pokemon_essentials_status` to populate its panel. The core Ruby shim
only provides the transport needed to bootstrap that first injection.
