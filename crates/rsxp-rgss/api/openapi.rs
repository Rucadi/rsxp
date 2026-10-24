// SPDX-License-Identifier: GPL-2.0-or-later
#![allow(dead_code)]
//! Code-first OpenAPI contract for the live game HTTP API.
//!
//! This module is compiled into the `rsxp-mcp` binary from `api/main.rs` (via
//! `#[path = "openapi.rs"]`) and describes the built-in REST compatibility
//! contract. Extension route aliases are discovered at runtime and are not
//! hard-coded into this document. `ApiDoc` aggregates the operation metadata and schemas:
//! the `#[utoipa::path]` stub functions below give each route its method, tag,
//! query/path parameters and response bodies, while the `ToSchema` structs
//! define the shared JSON shapes (game state, maps/tiles, Pokémon Essentials
//! party/dex/encounter/battle data, and input results). The functions are
//! documentation-only and intentionally empty. `rsxp-mcp --openapi` (or the
//! `cargo api-openapi` alias) pretty-prints this contract; the result is checked
//! in as `api/openapi.json`.
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "rsxp Live Game API",
        version = "0.2.0",
        description = "Loopback API for game state, Pokémon Essentials party data, and bounded input. Start it from the egui Game API panel, or use RSXP_API=1 for headless runs."
    ),
    servers((url = "http://127.0.0.1:47821", description = "Running local game process")),
    tags((name = "Game"), (name = "World map"), (name = "Pokémon Essentials"), (name = "Input")),
    paths(
        get_game_status,
        get_surroundings,
        get_world_map,
        get_map_tiles,
        teleport,
        add_inventory_item,
        get_pokedex,
        get_species_entry,
        get_pokemon_essentials_status,
        get_pokemon_essentials_party,
        get_pokemon_essentials_details,
        get_pokemon_essentials_encounters,
        get_pokemon_essentials_npcs,
        get_pokemon_essentials_battle,
        post_wild_encounter,
        get_essentials_debug,
        set_essentials_debug,
        press_game_key,
        release_game_keys
    ),
    components(schemas(
        ApiError, GameStatus, Surroundings, WorldMap, WorldMapEntry, MapTiles,
        TeleportResult,
        TileInfo, EventInfo,
        PokemonEssentialsStatus, PokemonParty, PokemonSummary, PokemonDetails, PokemonMove,
        PokemonOwner, PokemonAbility, PokemonNature, PokemonItem, SpeciesInfo,
        GamePokedex, GameSpeciesSummary, GameSpeciesEntry, GameInventoryChange,
        PokemonEncounters, EncounterTable, EncounterEntry, PokemonNpcs, PokemonNpc, PokemonNpcPage,
        PokemonBattle, PokemonBattler, BattleCommands, BattleSwitchOption,
        GameEncounterResult,
        GameDebugMode,
        KeyPressResult, ReleaseResult
    ))
)]
pub struct ApiDoc;

#[utoipa::path(
    get,
    path = "/v1/game/status",
    tag = "Game",
    operation_id = "getGameStatus",
    responses(
        (status = 200, description = "Current scene, screen, map, and player state", body = GameStatus),
        (status = 400, description = "Game state is not available", body = ApiError)
    )
)]
fn get_game_status() {}

#[utoipa::path(
    get,
    path = "/v1/game/surroundings",
    tag = "Game",
    operation_id = "getSurroundings",
    params(("radius" = Option<i32>, Query, description = "Tile radius around the player", minimum = 1, maximum = 15)),
    responses(
        (status = 200, description = "Nearby tile layers, terrain tags, and events", body = Surroundings),
        (status = 400, description = "Invalid radius or unavailable map", body = ApiError)
    )
)]
fn get_surroundings() {}

#[utoipa::path(
    get,
    path = "/v1/game/world-map",
    tag = "World map",
    operation_id = "getWorldMap",
    responses(
        (status = 200, description = "RPG Maker map metadata, physical map connections, and transfer-event links", body = WorldMap),
        (status = 400, description = "Map metadata is unavailable", body = ApiError)
    )
)]
fn get_world_map() {}

#[utoipa::path(
    get,
    path = "/v1/game/maps/{map_id}/tiles",
    tag = "World map",
    operation_id = "getMapTiles",
    params(("map_id" = i32, Path, description = "Map ID", minimum = 1)),
    responses(
        (status = 200, description = "Actual RGSS tile layers and bitmap names for one map", body = MapTiles),
        (status = 400, description = "Map or tileset is unavailable", body = ApiError)
    )
)]
fn get_map_tiles() {}

#[utoipa::path(
    post,
    path = "/v1/game/teleport",
    tag = "World map",
    operation_id = "teleportPlayer",
    params(
        ("map_id" = i32, Query, description = "Destination game map ID"),
        ("x" = i32, Query, description = "Destination tile X"),
        ("y" = i32, Query, description = "Destination tile Y")
    ),
    responses(
        (status = 200, description = "Transfer queued through the game's normal player transfer support", body = TeleportResult),
        (status = 400, description = "Map/tile is invalid or transfer is unavailable", body = ApiError)
    )
)]
fn teleport() {}

#[utoipa::path(
    get,
    path = "/v1/pokemon-essentials/status",
    tag = "Pokémon Essentials",
    operation_id = "getPokemonEssentialsStatus",
    responses(
        (status = 200, description = "Whether Pokémon Essentials data is available and current trainer, map, and battle readiness", body = PokemonEssentialsStatus),
        (status = 400, description = "Essentials state is unavailable", body = ApiError)
    )
)]
fn get_pokemon_essentials_status() {}

#[utoipa::path(
    get,
    path = "/v1/pokemon-essentials/party",
    tag = "Pokémon Essentials",
    operation_id = "getEssentialsPokemonParty",
    responses(
        (status = 200, description = "Party summaries in party order", body = PokemonParty),
        (status = 400, description = "Trainer or party is not initialized", body = ApiError)
    )
)]
fn get_pokemon_essentials_party() {}

#[utoipa::path(
    get,
    path = "/v1/pokemon-essentials/party/{slot}",
    tag = "Pokémon Essentials",
    operation_id = "getEssentialsPokemonDetails",
    params(("slot" = usize, Path, description = "Zero-based party position", minimum = 0, maximum = 5)),
    responses(
        (status = 200, description = "Stats, IVs, EVs, moves, owner, capture metadata, species data, and plugin fields", body = PokemonDetails),
        (status = 400, description = "Invalid or empty party slot", body = ApiError)
    )
)]
fn get_pokemon_essentials_details() {}

#[utoipa::path(
    get,
    path = "/v1/pokemon-essentials/encounters/current-map",
    tag = "Pokémon Essentials",
    operation_id = "getEssentialsMapEncounters",
    responses(
        (status = 200, description = "Current map's encounter types, species, level ranges, and per-slot table selection percentages", body = PokemonEncounters),
        (status = 400, description = "Encounter data is unavailable", body = ApiError)
    )
)]
fn get_pokemon_essentials_encounters() {}

#[utoipa::path(
    get,
    path = "/v1/pokemon-essentials/npcs/current-map",
    tag = "Pokémon Essentials",
    operation_id = "getEssentialsMapNpcs",
    responses(
        (status = 200, description = "Current map events/NPCs with positions, page triggers, command excerpts, and likely trainer battle indicators", body = PokemonNpcs),
        (status = 400, description = "Current map is unavailable", body = ApiError)
    )
)]
fn get_pokemon_essentials_npcs() {}

#[utoipa::path(
    get,
    path = "/v1/pokemon-essentials/battle",
    tag = "Pokémon Essentials",
    operation_id = "getEssentialsBattle",
    responses(
        (status = 200, description = "Active battle state, own and opponent battlers, HP/status/stages/moves/PP, and available command options", body = PokemonBattle),
        (status = 400, description = "Battle state is unavailable", body = ApiError)
    )
)]
fn get_pokemon_essentials_battle() {}

#[utoipa::path(
    post,
    path = "/v1/pokemon-essentials/wild-encounter",
    tag = "Pokémon Essentials",
    operation_id = "postEssentialsWildEncounter",
    params(
        ("species" = String, Query, description = "Exact game species/form ID", example = "PIKACHU"),
        ("level" = Option<i32>, Query, description = "Wild level (default 5)", minimum = 1, maximum = 100),
        ("shiny" = Option<bool>, Query, description = "Force shiny (true) or non-shiny (false)"),
        ("form" = Option<i32>, Query, description = "Alternate form index", minimum = 0),
        ("gender" = Option<String>, Query, description = "Force male or female"),
        ("nature" = Option<String>, Query, description = "Nature ID, for example JOLLY"),
        ("ability" = Option<String>, Query, description = "Ability ID"),
        ("item" = Option<String>, Query, description = "Held item ID"),
        ("max_ivs" = Option<bool>, Query, description = "Set every IV to 31"),
        ("friendship" = Option<i32>, Query, description = "Friendship value", minimum = 0, maximum = 255)
    ),
    responses(
        (status = 200, description = "Encounter scheduled to start on the next game frame", body = GameEncounterResult),
        (status = 400, description = "Invalid options, or the game is not on the overworld", body = ApiError)
    )
)]
fn post_wild_encounter() {}

#[utoipa::path(
    get,
    path = "/v1/pokemon-essentials/debug",
    tag = "Pokémon Essentials",
    operation_id = "getEssentialsDebugMode",
    responses(
        (status = 200, description = "Whether Pokémon Essentials debug mode is currently enabled", body = GameDebugMode),
        (status = 400, description = "Essentials state is unavailable", body = ApiError)
    )
)]
fn get_essentials_debug() {}

#[utoipa::path(
    post,
    path = "/v1/pokemon-essentials/debug",
    tag = "Pokémon Essentials",
    operation_id = "setEssentialsDebugMode",
    params(("enabled" = Option<bool>, Query, description = "true or false to set debug mode; omit to toggle")),
    responses(
        (status = 200, description = "The resulting debug mode value", body = GameDebugMode),
        (status = 400, description = "Invalid value or Essentials state is unavailable", body = ApiError)
    )
)]
fn set_essentials_debug() {}

#[utoipa::path(
    post,
    path = "/v1/input/press",
    tag = "Input",
    operation_id = "pressGameKey",
    params(
        ("key" = String, Query, description = "RGSS key to press", example = "up"),
        ("frames" = Option<i32>, Query, description = "Number of game updates to hold the key", minimum = 1, maximum = 60)
    ),
    responses(
        (status = 200, description = "Input accepted by the game thread", body = KeyPressResult),
        (status = 400, description = "Invalid key or duration", body = ApiError)
    )
)]
fn press_game_key() {}

#[utoipa::path(
    delete,
    path = "/v1/input/held",
    tag = "Input",
    operation_id = "releaseGameKeys",
    responses((status = 200, description = "API-held keys released", body = ReleaseResult))
)]
fn release_game_keys() {}

#[derive(utoipa::ToSchema)]
struct ApiError {
    error: String,
}

#[derive(utoipa::ToSchema)]
struct GameStatus {
    ready: bool,
    scene: Option<String>,
    frame: Option<i64>,
    #[schema(value_type = Object)]
    resolution: String,
    #[schema(value_type = Option<Object>)]
    map: Option<String>,
    #[schema(value_type = Option<Object>)]
    player: Option<String>,
}

#[derive(utoipa::ToSchema)]
struct Surroundings {
    map_id: Option<i64>,
    map_name: Option<String>,
    width: i32,
    height: i32,
    radius: i32,
    tiles: Vec<TileInfo>,
    events: Vec<EventInfo>,
}

#[derive(utoipa::ToSchema)]
struct WorldMap {
    maps: Vec<WorldMapEntry>,
    connections: Vec<Vec<i32>>,
    transfers: Vec<Vec<i32>>,
    current_map_id: Option<i32>,
}

#[derive(utoipa::ToSchema)]
struct WorldMapEntry {
    id: i32,
    name: String,
    width: i32,
    height: i32,
    tileset_id: i32,
    outdoor: bool,
    region: Option<i32>,
    world_x: Option<i32>,
    world_y: Option<i32>,
}

#[derive(utoipa::ToSchema)]
struct MapTiles {
    map_id: i32,
    width: i32,
    height: i32,
    tileset: String,
    autotiles: Vec<String>,
    layers: Vec<Vec<Vec<i32>>>,
}

#[derive(utoipa::ToSchema)]
struct TeleportResult {
    accepted: bool,
    map_id: i32,
    x: i32,
    y: i32,
}

#[derive(utoipa::ToSchema)]
struct TileInfo {
    x: i32,
    y: i32,
    layers: Vec<i32>,
    terrain_tag: Option<i32>,
}

#[derive(utoipa::ToSchema)]
struct EventInfo {
    id: Option<i32>,
    name: Option<String>,
    x: Option<i32>,
    y: Option<i32>,
    direction: Option<i32>,
    moving: Option<bool>,
}

#[derive(utoipa::ToSchema)]
struct PokemonParty {
    count: usize,
    pokemon: Vec<PokemonSummary>,
}

#[derive(utoipa::ToSchema)]
struct PokemonEssentialsStatus {
    available: bool,
    engine: Option<String>,
    version: Option<String>,
    #[schema(value_type = Option<Object>)]
    trainer: Option<String>,
    map_id: Option<i32>,
    in_battle: bool,
    debug: bool,
}

#[derive(utoipa::ToSchema)]
struct PokemonEncounters {
    available: bool,
    map_id: i32,
    encounter_version: i32,
    current_encounter_type: Option<String>,
    probability_note: Option<String>,
    tables: Vec<EncounterTable>,
}

#[derive(utoipa::ToSchema)]
struct EncounterTable {
    #[schema(value_type = Value)]
    r#type: String,
    name: String,
    step_chance: Option<f64>,
    entries: Vec<EncounterEntry>,
}

#[derive(utoipa::ToSchema)]
struct EncounterEntry {
    slot: usize,
    #[schema(value_type = Value)]
    species: Option<String>,
    name: Option<String>,
    min_level: Option<i32>,
    max_level: Option<i32>,
    weight: f64,
    selection_percent: f64,
    step_percent: Option<f64>,
}

#[derive(utoipa::ToSchema)]
struct PokemonNpcs {
    map_id: Option<i32>,
    map_name: Option<String>,
    npcs: Vec<PokemonNpc>,
}

#[derive(utoipa::ToSchema)]
struct PokemonNpc {
    id: Option<i32>,
    name: Option<String>,
    x: Option<i32>,
    y: Option<i32>,
    direction: Option<i32>,
    moving: Option<bool>,
    erased: Option<bool>,
    pages: Vec<PokemonNpcPage>,
}

#[derive(utoipa::ToSchema)]
struct PokemonNpcPage {
    trigger: Option<i32>,
    #[schema(value_type = Object)]
    graphic: String,
    #[schema(value_type = Vec<Object>)]
    commands: Vec<String>,
    text_excerpt: Option<String>,
    trainer_battle_heuristic: bool,
}

#[derive(utoipa::ToSchema)]
struct PokemonBattle {
    active: bool,
    class: Option<String>,
    turn: Option<i32>,
    decision: Option<i32>,
    can_run: Option<bool>,
    capture_odds: Option<f64>,
    capture_odds_note: Option<String>,
    battlers: Vec<PokemonBattler>,
}

#[derive(utoipa::ToSchema)]
struct PokemonBattler {
    index: usize,
    side: String,
    name: Option<String>,
    #[schema(value_type = Option<Object>)]
    pokemon: Option<String>,
    hp: Option<i32>,
    total_hp: Option<i32>,
    #[schema(value_type = Value)]
    status: Option<String>,
    status_count: Option<i32>,
    #[schema(value_type = Object)]
    stages: String,
    moves: Vec<PokemonMove>,
    #[schema(value_type = Object)]
    commands: String,
}

#[derive(utoipa::ToSchema)]
struct BattleCommands {
    fight: bool,
    switch: bool,
    switch_options: Vec<BattleSwitchOption>,
    bag: bool,
    run: Option<bool>,
}

#[derive(utoipa::ToSchema)]
struct BattleSwitchOption {
    slot: usize,
    #[schema(value_type = Value)]
    species: Option<String>,
    name: Option<String>,
    level: Option<i32>,
}

#[derive(utoipa::ToSchema)]
struct GameEncounterResult {
    scheduled: bool,
    species: String,
    name: String,
    level: i32,
    shiny: bool,
}

#[derive(utoipa::ToSchema)]
struct GameDebugMode {
    debug: bool,
}

#[derive(utoipa::ToSchema)]
struct PokemonSummary {
    slot: usize,
    #[schema(value_type = Value)]
    species: Option<String>,
    species_name: Option<String>,
    name: Option<String>,
    level: Option<i32>,
    #[schema(value_type = Value)]
    form: Option<String>,
    #[schema(value_type = Value)]
    gender: Option<String>,
    shiny: Option<bool>,
    egg: Option<bool>,
    hp: Option<i32>,
    total_hp: Option<i32>,
    #[schema(value_type = Value)]
    status: Option<String>,
    #[schema(value_type = Option<Vec<Value>>)]
    types: Option<Vec<String>>,
    empty: Option<bool>,
}

#[derive(utoipa::ToSchema)]
struct PokemonDetails {
    slot: usize,
    #[schema(value_type = Value)]
    species: Option<String>,
    species_name: Option<String>,
    name: Option<String>,
    level: Option<i32>,
    #[schema(value_type = Value)]
    form: Option<String>,
    #[schema(value_type = Value)]
    gender: Option<String>,
    shiny: Option<bool>,
    egg: Option<bool>,
    hp: Option<i32>,
    total_hp: Option<i32>,
    #[schema(value_type = Value)]
    status: Option<String>,
    #[schema(value_type = Option<Vec<Value>>)]
    types: Option<Vec<String>>,
    personal_id: Option<u64>,
    experience: Option<u64>,
    growth_rate: Option<String>,
    ability: Option<PokemonAbility>,
    ability_index: Option<i32>,
    nature: Option<PokemonNature>,
    held_item: Option<PokemonItem>,
    #[schema(value_type = Object)]
    stats: String,
    #[schema(value_type = Object)]
    base_stats: String,
    #[schema(value_type = Object)]
    ivs: String,
    #[schema(value_type = Object)]
    evs: String,
    moves: Vec<PokemonMove>,
    #[schema(value_type = Option<Object>)]
    owner: Option<PokemonOwner>,
    #[schema(value_type = Object)]
    obtain: String,
    #[schema(value_type = Option<Object>)]
    species_data: Option<SpeciesInfo>,
    #[schema(value_type = Object)]
    extensions: String,
    #[schema(value_type = Value)]
    status_count: Option<String>,
    happiness: Option<i32>,
    pokerus: Option<i32>,
    #[schema(value_type = Value)]
    ribbons: Option<String>,
    #[schema(value_type = Value)]
    markings: Option<String>,
}

#[derive(utoipa::ToSchema)]
struct PokemonMove {
    id: Option<String>,
    name: Option<String>,
    pp: Option<i32>,
    usable_by_pp: Option<bool>,
    total_pp: Option<i32>,
    pp_up: Option<i32>,
    power: Option<i32>,
    #[schema(rename = "type")]
    r#type: Option<String>,
    category: Option<String>,
    accuracy: Option<i32>,
    function_code: Option<String>,
}

#[derive(utoipa::ToSchema)]
struct PokemonOwner {
    name: Option<String>,
    id: Option<u64>,
    public_id: Option<u64>,
    gender: Option<i32>,
    language: Option<i32>,
}

#[derive(utoipa::ToSchema)]
struct PokemonAbility {
    #[schema(value_type = Value)]
    id: Option<String>,
    name: Option<String>,
}

#[derive(utoipa::ToSchema)]
struct PokemonNature {
    #[schema(value_type = Value)]
    #[schema(value_type = Value)]
    id: Option<String>,
    name: Option<String>,
}

#[derive(utoipa::ToSchema)]
struct PokemonItem {
    #[schema(value_type = Value)]
    #[schema(value_type = Value)]
    id: Option<String>,
    name: Option<String>,
}

#[derive(utoipa::ToSchema)]
struct SpeciesInfo {
    #[schema(value_type = Value)]
    id: Option<String>,
    name: Option<String>,
    category: Option<String>,
    height: Option<f64>,
    weight: Option<f64>,
    abilities: Vec<String>,
    hidden_abilities: Option<Vec<String>>,
}

#[derive(utoipa::ToSchema)]
struct KeyPressResult {
    key: String,
    frames: i32,
    accepted: bool,
}

#[derive(utoipa::ToSchema)]
struct ReleaseResult {
    released: bool,
}

#[utoipa::path(
    get, path = "/v1/pokemon-essentials/pokedex", tag = "Pokémon Essentials",
    operation_id = "getEssentialsPokedex",
    params(("refresh" = Option<String>, Query, description = "Use refresh to rebuild the session catalog")),
    responses((status = 200, description = "All loaded game species and forms with known acquisition evidence", body = GamePokedex),
              (status = 400, description = "Species data unavailable", body = ApiError))
)]
fn get_pokedex() {}

#[utoipa::path(
    get, path = "/v1/pokemon-essentials/species/{species}", tag = "Pokémon Essentials",
    operation_id = "getEssentialsSpecies",
    params(("species" = String, Path, description = "Exact game species/form ID", example = "BULBASAUR")),
    responses((status = 200, description = "Game-specific stats, abilities, complete learnsets, evolution methods and encounter locations", body = GameSpeciesEntry),
              (status = 400, description = "Unknown species or unavailable game data", body = ApiError))
)]
fn get_species_entry() {}

#[derive(utoipa::ToSchema)]
struct GamePokedex {
    species: Vec<GameSpeciesSummary>,
    note: String,
}
#[derive(utoipa::ToSchema)]
struct GameSpeciesSummary {
    id: String,
    name: String,
    number: Option<i32>,
    form: i32,
    form_name: Option<String>,
    types: Vec<String>,
    source: Option<String>,
    map_count: usize,
}
#[derive(utoipa::ToSchema)]
struct GameSpeciesEntry {
    id: String,
    name: String,
    number: Option<i32>,
    form: i32,
    form_name: Option<String>,
    types: Vec<String>,
    source: Option<String>,
    map_count: usize,
    entry: Option<String>,
    #[schema(value_type = Object)]
    stats: String,
    #[schema(value_type = Object)]
    facts: String,
    #[schema(value_type = Vec<Object>)]
    abilities: Vec<String>,
    #[schema(value_type = Vec<Object>)]
    hidden_abilities: Vec<String>,
    #[schema(value_type = Vec<Object>)]
    locations: Vec<String>,
    #[schema(value_type = Vec<Object>)]
    moves: Vec<String>,
    #[schema(value_type = Vec<Object>)]
    evolutions: Vec<String>,
    #[schema(value_type = Vec<Object>)]
    previous: Vec<String>,
    #[schema(value_type = Object)]
    raw: String,
}

#[utoipa::path(
    post, path = "/v1/pokemon-essentials/inventory/add", tag = "Pokémon Essentials",
    operation_id = "addEssentialsInventoryItem",
    params(("item" = String, Query, description = "Exact game item ID", example = "POTION"),
           ("quantity" = Option<i32>, Query, description = "Quantity to add (default 1); game bag capacity still applies", minimum = 1, maximum = 9999)),
    responses((status = 200, description = "Confirmed added quantity and resulting bag total", body = GameInventoryChange),
              (status = 400, description = "Invalid item/quantity, unavailable bag or insufficient capacity", body = ApiError))
)]
fn add_inventory_item() {}
#[derive(utoipa::ToSchema)]
struct GameInventoryChange {
    item: String,
    name: String,
    requested: i32,
    added: i32,
    total: Option<i32>,
    complete: bool,
}
