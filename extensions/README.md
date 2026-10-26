# Rhai plugins

The bundled plugins in this directory are embedded into the native and
browser UI hosts. Native hosts can additionally load plugins from disk; each
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
Pokédex index/detail/encounter Ruby bindings, HTTP routes, and its browser-safe
HTML explorer. `essentials-movedex.rhai` follows the same pattern for the move
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
In the browser the same plugin HTML is placed in an iframe overlay; its
`fetch('/v1/...')` calls are routed through the WebAssembly game API bridge,
so plugins do not need separate native and browser source files. The browser
view's close button removes the view; a plugin can also call
`webview_close("main")`.

`essentials-game-api` uses the same mechanism for its game-facing API: its
`main.rhai` injects the package's `game_api.rb`, waits for registration, then
queries `pokemon_essentials_status` to populate its panel. The core Ruby shim
only provides the transport needed to bootstrap that first injection.
