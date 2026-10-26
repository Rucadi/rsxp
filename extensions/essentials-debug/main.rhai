// fanplay:id = pokemon-essentials-debug
// fanplay:name = Essentials Debug
// fanplay:cores = rgss
// fanplay:version = 0.1.0
// fanplay:capabilities = pokemon_essentials.debug.read,pokemon_essentials.debug.write
// fanplay:operation = pokemon_essentials_debug
// fanplay:operation-capabilities = pokemon_essentials_debug pokemon_essentials.debug.read
// fanplay:operation-mutates = pokemon_essentials_debug
// fanplay:panel = debug,Essentials Debug

// The script orchestrates the feature; the RGSS bridge remains authoritative.
fn on_activate() {
    state_set("pending", request("pokemon_essentials_debug", #{}));
}

fn build_ui() {
    let pending = state_get("pending");
    if pending != () {
        return [ui_loading("debug.loading", "Reading Essentials debug mode…")];
    }
    let error = last_error();
    if error != "" {
        return [ui_label("debug.error", "Essentials debug unavailable: " + error)];
    }
    [ui_checkbox("debug.enabled", "Enable Essentials debug mode", state_get("debug") == true)]
}

fn on_ui_event(widget_id, value) {
    if widget_id == "debug.enabled" {
        state_set("pending", request("pokemon_essentials_debug", #{enabled: value}));
    }
}

fn on_operation_result(_request_id, result) {
    state_set("pending", ());
    if result["debug"] != () {
        state_set("debug", result["debug"]);
    }
}

fn on_shutdown() { }
