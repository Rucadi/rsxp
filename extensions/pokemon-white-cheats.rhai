// fanplay:cores = nds
// fanplay:panel = pokemon-white-cheats, Pokémon White (Spain/EUR) Cheats

// Pokémon White (Spain/EUR) stores the money value at 0x0223CD6C.
// Keep the code in a function so it is visible from Rhai event callbacks.
fn max_money_code() {
    "0223CD6C 0098967F\nD2000000 00000000"
}

fn build_ui() {
    [
        ui_group("pokemon-white-cheats.main", "Cheats", [
            ui_button("pokemon-white-cheats.enable", "Enable max money"),
            ui_button("pokemon-white-cheats.disable", "Disable all cheats")
        ])
    ]
}

fn on_activate() {
    state_set("status", "ready");
}

fn on_ui_event(id, _value) {
    if id == "pokemon-white-cheats.enable" {
        state_set("request", nds_add_cheat(max_money_code()));
    } else if id == "pokemon-white-cheats.disable" {
        state_set("request", nds_clear_cheats());
    }
}

fn on_operation_result(id, result) {
    state_set("last_request", id);
    state_set("last_result", result);
}

fn on_shutdown() {
    nds_clear_cheats();
}
