# Live game MCP API

This API exposes game inspection, Pokémon Essentials party data, and bounded
input controls for a running desktop `rsxp` game. Its OpenAPI contract is
generated from the Rust declarations in `openapi.rs`. Rust owns HTTP and MCP
stdio; Ruby only reads RGSS/Essentials state and injects controls on the game
thread.

Start the game normally. In the egui workspace's left panel, enter a port under
**Game API** and choose **Start HTTP API**. The server listens on `127.0.0.1` and
remains available for the active session. The interactive dashboard starts its
runner with `RSXP_API=0`; use the workspace control to start HTTP for an
interactive session. For headless runs, the environment
variable can still start it automatically:

```sh
RSXP_API=1 RSXP_API_PORT=47821 cargo run --release --bin rsxp -- run --game /path/to/game --headless
```

The egui inspector sends requests through the in-process game-thread queue and
works without starting the HTTP listener. Use **Start HTTP API** when an MCP
client or another external tool needs to connect, or when testing the endpoint.

Build and register the Rust MCP executable:

```sh
cargo build --release --no-default-features --features api-docs --bin rsxp-mcp
cargo api-openapi > api/openapi.json
```

`cargo api-openapi` regenerates the checked-in OpenAPI JSON from Rust endpoint
and schema declarations. The MCP tool definitions are kept in the same Rust
binary and correspond to these REST operations.

```json
{
  "mcpServers": {
    "rsxp-game": {
      "command": "/absolute/path/to/rsxp/target/release/rsxp-mcp",
      "env": { "RSXP_API_PORT": "47821" }
    }
  }
}
```

The REST API provides game status, nearby map contents, RPG Maker map
connections and transfer-event links, actual tile layers, a party summary, and
full details for an individual party slot. The egui world map automatically
loads every Essentials map marked `Outdoor`, renders their actual tiles as one
pan/zoom canvas, and aligns maps using explicit connection offsets or direct
transfer-event source and destination cells. Essentials `MapPosition` metadata
anchors disconnected areas. Clicking a rendered cell selects its exact map
coordinates; the transfer button queues it through the game's normal transfer
helper.
Pokémon-specific operations are isolated below `/v1/pokemon-essentials/` and
are only advertised as Pokémon Essentials MCP tools, so generic game clients
can ignore them. These endpoints expose Essentials availability, party and
individual Pokémon data, current-map encounters, current-map NPC/event details,
and live battle state with legal command options. Encounter slot rates are
identified as table-selection percentages; terrain and game modifiers can
change actual encounter probability. Battle capture odds remain unavailable
when the active Essentials version and selected Poké Ball do not provide a
safe, version-independent calculation.

The Essentials routes are `GET /status`, `GET /party`, `GET /party/{slot}`,
`GET /encounters/current-map`, `GET /npcs/current-map`, `GET /battle`,
`POST /wild-encounter`, and `GET`/`POST /debug`, all under
`/v1/pokemon-essentials`. The battle endpoint
is observational: it lists available Fight, Switch, Bag, and Run choices and move
PP. Use the generic `/v1/input/press` endpoint to navigate the game's own battle
menus, which keeps the active Essentials version's battle logic in control.

`POST /v1/pokemon-essentials/wild-encounter?species=PIKACHU` queues a wild battle
with the selected species. Optional query parameters are `level` (1–100),
`shiny` (`true`/`false`), `form`, `gender` (`male`/`female`), `nature`, `ability`,
`item`, `max_ivs` (`true`/`false`), and `friendship` (0–255). Values are validated
against the running game, and the request is rejected with a 400 when the game is
not on the overworld or when a battle, menu or dialogue is already open. The
battle is scheduled and starts on the next game frame, so the response is never
blocked by a long battle; the 200 body reports `scheduled`, `species`, `name`,
`level` and `shiny`. This is the same operation behind the Pokédex Cheats tab.

`GET /v1/pokemon-essentials/debug` reports whether Essentials debug mode is on,
and `POST /v1/pokemon-essentials/debug?enabled=true` (or `false`, or omitted to
toggle) changes it. This writes the game's own `$DEBUG` flag, which enables its
debug menus, Ctrl-modifiers and shortcuts. The `GET /status` response also
includes the current `debug` value. The Pokémon Essentials panel exposes this as
a "Debug mode" checkbox next to the dex buttons.

Pokémon access works with both
older `$Trainer`/`PokeBattle_Pokemon` games and newer `$player`/`Pokemon`
Essentials versions. Detailed responses include moves, stats, IVs, EVs, owner,
capture metadata, species data, and safely serialized plugin fields where
available. The egui Pokémon details panel displays the stat, base-stat, IV, and
EV maps with stat names, alongside moves, ability, nature, item, owner, and
capture details. MCP tools expose the Essentials operations as
`pokemon_essentials_status`, `pokemon_essentials_party`,
`pokemon_essentials_info`, `pokemon_essentials_encounters`,
`pokemon_essentials_npcs`, `pokemon_essentials_battle`,
`pokemon_essentials_wild_encounter`, and `pokemon_essentials_debug`, alongside
generic game tools such as `game_status`, `world_map`, and `press_key`.

Input uses normal RGSS polling, preserving collision and event behavior. The
API binds only to `127.0.0.1`. The MCP process may set `RSXP_API_HOST` and
`RSXP_API_PORT` to select the connection endpoint.

The game-backed species explorer is available through `GET /v1/pokemon-essentials/pokedex`
and `GET /v1/pokemon-essentials/species/{species}` (exact species/form ID).
Use `?refresh=refresh` on the catalog to rebuild its session cache. The corresponding
MCP tools are `pokemon_essentials_pokedex` and `pokemon_essentials_species_entry`.
The catalog includes all loaded forms and marks known wild, party, evolution and
breeding sources. Scripted acquisitions may not be indexed; unknown sources do
not mean a species is unobtainable. Detail responses preserve full learnsets and
custom game stats, moves, abilities, evolution rules, and wild encounter locations.

`GET /v1/pokemon-essentials/movedex` and `GET /v1/pokemon-essentials/itemdex`
return complete game-backed move/item catalogs (use `?refresh=refresh` to rebuild
session caches). Their MCP tools are `pokemon_essentials_movedex` and
`pokemon_essentials_itemdex`. Movedex records include the game-defined TMs that
teach each move; machines are resolved from the item registry, never invented.

Older Essentials releases that predate the `GameData::*` registries (for example
Pokémon Z, which keeps compiled `PBSpecies`/`PBMoves`/`PBItems` modules and flat
`dexdata.dat`/`moves.dat`/`items.dat` files) are served through the same
endpoints. Species, moves, items, learnsets, evolutions and current-map
encounters are decoded from that layout, and item additions use the legacy bag
methods (`pbCanStore?`/`pbStoreItem`).

`POST /v1/pokemon-essentials/inventory/add?item=POTION&quantity=3` modifies the
current player bag via its public storage methods. Quantity defaults to 1 and
must be an integer from 1 to 9999; game bag capacity still applies. It rejects
unknown items, an uninitialized bag and insufficient capacity before storing.
The result reports `requested`, `added`, `total` and `complete`, including partial
results from custom bag implementations. MCP exposes the same operation as
`pokemon_essentials_add_item` with `item` and optional `quantity` arguments.
