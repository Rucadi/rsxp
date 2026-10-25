# Unified control API

RSXP now has **one loopback control contract** for the launcher and the player.
The launcher opens the listener as soon as the desktop shell starts and keeps it
open while games start, stop and restart. A game never opens a second HTTP
server on the same port. Headless runs keep the same discovery document and
route names, with launcher-only operations returning an explicit `409` instead
of a different URL scheme.

```
Desktop shell (launcher)                 Isolated game runner
┌──────────────────────────┐            ┌──────────────────────────┐
│ eframe UI + launcher     │            │ MRI/RGSS or cartridge    │
│ ControlServer :47821     │  existing  │ api_bridge queue         │
│  ├─ launcher/settings    │◀──────────▶│  ├─ game status/input    │
│  ├─ catalogs/saves       │  session   │  └─ Rhai plugin routes   │
│  └─ proxies /v1/game/*   │  transport │                          │
└──────────────────────────┘            └──────────────────────────┘
```

Browser players cannot bind a TCP socket. They expose the same catalog through
`window.RSXP.control` in the page instead of pretending to be a local server.
Discovery, parameter types, status and game operations keep their names; save
snapshots and settings use browser local storage.

## Why one server

The previous implementation could start a game API in the dashboard process and
another one in a headless runner. Their discovery documents and operation sets
differed, so an external tool had to know which process answered before it could
build a request. That is the same failure mode desktop emulators avoid when a
debugger or remote-control service is bound once per application instance: the
address is stable, the contract is stable, and capabilities are reported rather
than inferred from a separate program.

Rules:

1. `127.0.0.1:47821` is the default endpoint. The port is stored in the RSXP
   configuration and can be changed from the API panel or `RSXP_API_PORT`.
2. The launcher owns the listener. Starting a game does not move or replace it.
3. Headless runs own their listener because no launcher exists, but publish the
   same discovery shape and mark launcher operations unavailable.
4. `RSXP_API=0` disables the listener for one process. `RSXP_API=1` forces it
   on. Otherwise `config/config.json` decides.
5. The listener never binds a public interface unless a future caller passes an
   explicit loopback address.
6. Cross-origin browser access is opt-in with `RSXP_API_CORS=1`. A wildcard
   CORS header would let any website control the local launcher, so it is not
   enabled by default. CLI tools, MCP clients and the desktop UI do not need it.

## Discovery and types

`GET /v1/discovery` is the primary document for people, scripts and AI agents.
Every endpoint includes:

```json
{
  "id": "launcher.launch",
  "method": "POST",
  "path": "/v1/launcher/launch",
  "scope": "launcher",
  "summary": "Launch a game",
  "description": "Launches a local path or a selected catalog game…",
  "mutates_state": true,
  "parameters": [
    {
      "name": "path",
      "in": "body",
      "type": "string",
      "required": false,
      "description": "Local game directory, Game.ini, or ROM path"
    }
  ],
  "request": { "type": "object" },
  "response": { "type": "object" },
  "curl": "curl -s -X POST 'http://127.0.0.1:47821/v1/launcher/launch' …"
}
```

Parameter locations are `path`, `query` or `body`; types are `string`,
`integer`, `number`, `boolean`, `object` or `array`. Required flags, defaults,
enums and numeric ranges are included where they apply. The same catalog is
rendered as OpenAPI 3.1 at `GET /v1/openapi.json`.

The MCP executable now builds its tool list from this document rather than from
plugin routes alone, so an AI sees launcher, settings, catalog, save and game
operations with their real descriptions and input schemas. Tool names replace
dots with underscores (`launcher_launch`) while discovery keeps the original
operation id. Plugin routes are merged into the catalog at runtime and remain
discoverable while their plugin is loaded.

## Operation groups

| Group | Examples |
| --- | --- |
| Server | `/v1/status`, `/v1/discovery`, `/v1/openapi.json` |
| Launcher | status, library, launch, stop, restart, pause, live settings |
| Folders | resolve or open `game`, `save`, `patches`, `backups` |
| Settings | common settings and per-game overrides, effective merge |
| Catalogs | permanent list, add, remove, select, refresh, cached games |
| Saves | info, create snapshot, restore, delete |
| Game | status, world map, tiles, surroundings, teleport, input, reset |
| Extensions | every route declared by a loaded Rhai plugin |

Launcher operations remain available while a game is running. Game operations
return `409 Conflict` when no game is active. Unknown paths return `404` with a
link to discovery; wrong methods return `405` with the allowed method list.
Mutating operations accept the same named parameters in the JSON body or in the
query string, which keeps simple command-line calls convenient without adding a
second schema.

## Settings model

Two layers are stored, and the effective value is computed at launch:

1. **Common settings** live in `<RSXP_HOME>/config/config.json`. They apply to
   every game.
2. **Per-game overrides** are stored in the `games` map of the same
   `config/config.json`, keyed by a stable game key (`repository/game` or
   `local:/absolute/path`). They override common values for one game. Older
   `config/games/<hash>.json` files are read as a fallback and migrate on save.

Repository catalog metadata can supply defaults. The merge order is
`repository defaults → common settings → per-game override`. Patches, keyboard
bindings and patch parameters are additive; scalar options from a later layer
win. `fast_forward` is a positive common flag and cannot be disabled per game,
which matches the workspace's uncapped mode.

Every option that can be changed from the desktop UI also has an API operation.
Launch-time options (renderer, JIT, RGSS version, BIOS paths) require a restart;
live options (fast-forward, frame-rate cap, pause) apply immediately through
`PATCH /v1/launcher/live`.

## Catalogs

JSON catalogs are permanent documents, not a transient refresh result:

- Adding a source writes it to `config/config.json`.
- Refreshing writes a verified cache to `repositories/cache/<id>.json`.
- A failed refresh keeps the last good cache and records `last_error`.
- Removing a catalog removes it from the selection list but keeps its cache, so
  re-adding the source is instant.
- Each catalog can be selected or deselected without being deleted.

Catalog ids are stable and derived from the source URL/path, so two different
publishers cannot silently replace each other's entries. The launcher's
**Repositories** page exposes selection checkboxes and per-catalog refresh.

## Save snapshots

Save safety has two layers:

- The existing desktop directory backup keeps its explicit source/destination
  controls for unusual layouts.
- Named snapshots under `<RSXP_HOME>/backups/<game-key>/` are created by the
  player's **Saves** panel, the in-game **Save files** menu, and
  `POST /v1/saves/backup`. They are listed with file count and byte size, and
  restored through the same staged, atomic replacement used by the desktop
  backup code. Restore preserves overwritten originals in
  `before-restore-<nonce>` and refuses symlinks, overlapping trees and
  incompatible destination types.
- Restore is disabled while a game is running.
- Browser snapshots use the same names and metadata shape but live in
  `localStorage`; **Export saves** / **Import saves** move the bundle between
  browsers.

Cartridge saves are resolved by content identity under
`<RSXP_HOME>/saves/<core>/<content-id>`, so two ROMs with the same filename do
not share progress. RGSS saves use the conventional `saves/rgss/<title>` path.
The API returns the resolved paths from `GET /v1/saves`; no caller needs to
guess.

## Folders

`GET /v1/launcher/games/{key}/folders` returns the game installation, save,
patches and backups directories. `POST …/open-folder` with
`{"kind":"game|save|patches|backups"}` opens the folder in the host file manager
and returns `opened:false` with the path when no opener is available. The
launcher's game settings window and the in-game **Folders** menu expose the same
locations. Creating a save or patches folder is explicit and idempotent; merely
reading a path never mutates the game tree.

## What was borrowed from other emulators

Dolphin, DuckStation and RetroArch each contribute a useful idea, without
copying their architecture:

- **Dolphin-style durable configuration:** paths and user options belong in a
  user-owned configuration document, not in transient UI state. RSXP keeps this
  in `config/config.json` and validates it on load.
- **Content-scoped saves:** modern emulators avoid title collisions by keying
  saves to content identity. RSXP already did this for cartridges and now uses
  the same path in the control API.
- **DuckStation/RetroArch-style layered options:** global defaults plus
  per-game overrides are easier to reason about than a single mutable settings
  blob. RSXP merges them deterministically.
- **Emulator debugger philosophy:** one local endpoint with capability
  discovery is more useful to tools than several ports that depend on process
  mode. RSXP keeps the launcher alive as the control plane.
- **Snapshot/export distinction:** save states and battery saves are different
  things. RSXP's snapshots copy save files only; they are not VM state and the
  API says so.

Deliberately not copied: public network listeners, automatic port hopping,
unauthenticated remote control, and a separate per-game server. They add
surprise and make reproducible automation harder.

## Additional improvements in this change

- **Atomic, self-healing configuration.** `config.json` is written through a
  temporary file and rename. A corrupt file is moved aside instead of blocking
  startup.
- **Stable catalog identity.** Catalog ids derive from the source, so refreshing
  a source cannot overwrite a different catalog's cache.
- **Shared backup engine.** The desktop path editor, named snapshots and the
  HTTP API use one staging/rollback implementation instead of three copies.
- **Honest availability.** Headless and browser discovery include every
  operation but report which host can execute it; clients get `409` with a
  reason instead of a misleading `404`.
- **Live settings without a restart.** Fast-forward, frame-rate cap and pause
  are separated from launch-time options, so tools do not accidentally require
  a game restart to change pacing.
- **Browser parity where it is possible.** The browser gets the same operation
  names, typed discovery, settings merge and save-snapshot shape; filesystem and
  process operations are explicitly rejected rather than silently missing.
- **MCP discovery from the live catalog.** The MCP tool list is no longer a
  separately maintained guess that can drift from the HTTP routes.
- **Folder actions are explicit.** Reading a path never creates directories;
  opening a save or patches folder creates it only when the user asks to open
  it.

## Compatibility notes

- Existing `/v1/game/...`, `/v1/input/...`, `/v1/pokemon-essentials/...` and
  `/v1/extensions/routes` paths are unchanged.
- `RSXP_API=1` still starts a server in a headless run; it now also serves
  `/v1/discovery`, `/v1/openapi.json` and `/v1/status`.
- The desktop launcher keeps its display-side `api_bridge` for plugin webviews;
  the public HTTP listener is the unified control server.
- Browser pages expose the same names through a JavaScript bridge. They cannot
  provide a TCP socket, native folder dialogs or arbitrary filesystem paths; the
  discovery status marks those capabilities as unavailable.
