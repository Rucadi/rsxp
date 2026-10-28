// fanplay:id = essentials-game-api
// fanplay:name = Essentials Game API
// fanplay:cores = rgss
// fanplay:version = 0.1.0
// fanplay:capabilities = ruby.inject,ruby.unload,pokemon_essentials.read,pokemon_essentials.write
// fanplay:operation = pokemon_essentials_add_item
// fanplay:operation-capabilities = pokemon_essentials_add_item pokemon_essentials.write
// fanplay:operation-mutates = pokemon_essentials_add_item
// fanplay:route = POST /v1/pokemon-essentials/inventory/add pokemon_essentials_add_item item,quantity
// fanplay:operation = pokemon_essentials_status
// fanplay:operation-capabilities = pokemon_essentials_status pokemon_essentials.read
// fanplay:route = GET /v1/pokemon-essentials/status pokemon_essentials_status
// fanplay:operation = pokemon_essentials_party
// fanplay:operation-capabilities = pokemon_essentials_party pokemon_essentials.read
// fanplay:route = GET /v1/pokemon-essentials/party pokemon_essentials_party
// fanplay:operation = pokemon_essentials_info
// fanplay:operation-capabilities = pokemon_essentials_info pokemon_essentials.read
// fanplay:route = GET /v1/pokemon-essentials/party/{slot} pokemon_essentials_info slot
// fanplay:operation = pokemon_essentials_encounters
// fanplay:operation-capabilities = pokemon_essentials_encounters pokemon_essentials.read
// fanplay:route = GET /v1/pokemon-essentials/encounters/current-map pokemon_essentials_encounters
// fanplay:operation = pokemon_essentials_npcs
// fanplay:operation-capabilities = pokemon_essentials_npcs pokemon_essentials.read
// fanplay:route = GET /v1/pokemon-essentials/npcs/current-map pokemon_essentials_npcs
// fanplay:operation = pokemon_essentials_battle
// fanplay:operation-capabilities = pokemon_essentials_battle pokemon_essentials.read
// fanplay:route = GET /v1/pokemon-essentials/battle pokemon_essentials_battle
// fanplay:operation = pokemon_essentials_debug
// fanplay:operation-capabilities = pokemon_essentials_debug pokemon_essentials.write
// fanplay:operation-mutates = pokemon_essentials_debug
// fanplay:route = GET /v1/pokemon-essentials/debug pokemon_essentials_debug
// fanplay:route = POST /v1/pokemon-essentials/debug pokemon_essentials_debug enabled
// fanplay:panel = game-api,Essentials Game API

// This package is the semi-external game API module. Its Ruby implementation
// is package data, while Rhai owns installation, lifecycle and the inspector
// UI that queries it through the normal plugin request queue.
fn on_activate() {
    let ruby = ruby_source();
    let apply = "{ apply: proc { " + ruby + " }, revert: proc { FANPLAY.send(:remove_const, :GameAPI) if defined?(FANPLAY::GameAPI) } }";
    state_set("pending", ruby_inject("essentials-game-api", apply));
    state_set("status_attempts", 0);
}

fn build_ui() {
    let pending = state_get("pending");
    if pending != () {
        return [ui_loading("game-api.loading", "Installing Essentials game API…")];
    }
    if state_get("status_request") != () {
        return [ui_loading("game-api.querying", "Checking whether Essentials has finished starting…")];
    }
    let status = state_get("status");
    if status == () {
        return [ui_loading("game-api.querying", "Querying Essentials…")];
    }
    [
        ui_label("game-api.status", status),
        ui_button("game-api.refresh", "Check again")
    ]
}

fn on_ui_event(widget_id, _value) {
    if widget_id == "game-api.refresh" && state_get("status_request") == () {
        state_set("status", ());
        state_set("status_attempts", 0);
        state_set("status_request", request("pokemon_essentials_status", #{}));
    }
}

fn on_operation_result(request_id, result) {
    let pending = state_get("pending");
    if pending == request_id {
        state_set("pending", ());
        if result == () {
            state_set("status_request", ());
            state_set("status", "Essentials Game API error: " + last_error());
            return;
        }
        state_set("patch_handle", result["patch_id"]);
        state_set("status_request", request("pokemon_essentials_status", #{}));
        return;
    }
    if state_get("status_request") == request_id {
        if result == () {
            state_set("status_request", ());
            state_set("status", "Essentials status error: " + last_error());
            return;
        }
        if result["available"] == true {
            state_set("status_request", ());
            state_set("status", "Essentials is active.");
        } else if state_get("status_attempts") < 60 {
            // During game startup the bridge can answer before Essentials has
            // registered its API. Treat that response as inconclusive and
            // keep asking for a short startup window.
            state_set("status_attempts", state_get("status_attempts") + 1);
            state_set("status_request", request("pokemon_essentials_status", #{}));
        } else {
            state_set("status_request", ());
            state_set("status", "Essentials has not reported itself active yet.");
        }
    }
}

fn on_shutdown() {
    let handle = state_get("patch_handle");
    if handle != () {
        ruby_unload(handle);
    }
}
