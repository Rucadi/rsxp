// fanplay:id = essentials-inventory
// fanplay:name = Essentials Inventory
// fanplay:cores = rgss
// fanplay:version = 0.1.0
// fanplay:capabilities = pokemon_essentials.inventory.read,pokemon_essentials.inventory.write
// fanplay:operation = pokemon_essentials_add_item
// fanplay:operation-capabilities = pokemon_essentials_add_item pokemon_essentials.inventory.write
// fanplay:operation-mutates = pokemon_essentials_add_item
// fanplay:panel = inventory,Essentials Inventory

// Form state, validation of obvious user input, loading state and feedback
// belong to the plugin. The Ruby bridge remains authoritative for item
// existence, bag capacity and the actual mutation.
fn on_activate() {
    state_set("item", "");
    state_set("quantity", "1");
    state_set("pending", ());
    state_set("status", "");
}

fn build_ui() {
    let nodes = [
        ui_text_input("inventory.item", "Item ID", state_get("item")),
        ui_text_input("inventory.quantity", "Quantity", state_get("quantity")),
        ui_button("inventory.submit", "Add item")
    ];
    let pending = state_get("pending");
    if pending != () {
        let loading = ui_loading("inventory.loading", "Adding item…");
        let loading_nodes = [loading];
        let group = ui_group("inventory.form", "Add an item", loading_nodes);
        return [group];
    }
    let status = state_get("status");
    if status != "" {
        nodes.push(ui_label("inventory.status", status));
    }
    [ui_group("inventory.form", "Add an item", nodes)]
}

fn on_ui_event(widget_id, value) {
    if widget_id == "inventory.item" {
        state_set("item", value);
    }
    if widget_id == "inventory.quantity" {
        state_set("quantity", value);
    }
    if widget_id == "inventory.submit" {
        let item = state_get("item");
        let quantity = state_get("quantity");
        if item == "" {
            state_set("status", "Enter an item ID.");
        } else if quantity == "" {
            state_set("status", "Enter a quantity.");
        } else {
            state_set("status", "");
            let arguments = #{item: item, quantity: quantity};
            let request_id = request("pokemon_essentials_add_item", arguments);
            state_set("pending", request_id);
        }
    }
}

fn on_operation_result(_request_id, result) {
    state_set("pending", ());
    if result["complete"] == true {
        state_set("status", "Item added.");
    } else {
        let error = last_error();
        if error != "" {
            state_set("status", error);
        }
    }
}

fn on_shutdown() { }
