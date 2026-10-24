// rsxp:id = essentials-online-play
// rsxp:name = Essentials Online Play
// rsxp:version = 0.1.0
// rsxp:capabilities = pokemon_essentials.read,pokemon_essentials.write
// rsxp:operation = start_cable_club
// rsxp:operation-capabilities = start_cable_club pokemon_essentials.write
// rsxp:operation-mutates = start_cable_club
// rsxp:panel = online-play,Essentials Online Play

// Cable Club is an Essentials-specific feature. This plugin owns its panel
// state and presentation; the native host only exposes generic asynchronous
// host operations for the optional local relay.
fn request_status() {
    let request_id = request("__host.cable_status", #{});
    state_set("status_request", request_id);
}

fn relay_nodes(status) {
    let nodes = [];
    let address = status["address"];
    if address != () && address != "" {
        nodes.push(ui_label("online-play.relay-address", "Relay listening: " + address));
        nodes.push(ui_button("online-play.stop", "Stop relay"));
        let clients = status["clients"];
        if clients != () && clients.len() > 0 {
            let count = clients.len().to_string();
            nodes.push(ui_label("online-play.clients", "Connected clients: " + count));
        } else {
            nodes.push(ui_label("online-play.waiting", "Waiting for game clients…"));
        }
        nodes.push(ui_label("online-play.relay-help", "Forward this TCP port for internet play."));
    } else {
        nodes.push(ui_text_input("online-play.bind", "Bind", state_get("bind")));
        nodes.push(ui_text_input("online-play.port", "Port", state_get("port")));
        nodes.push(ui_button("online-play.start", "Start relay"));
    }
    nodes
}

fn on_activate() {
    state_set("bind", "0.0.0.0");
    state_set("port", "25565");
    state_set("client_host", "127.0.0.1");
    state_set("client_port", "25565");
    state_set("status", ());
    state_set("status_request", ());
    state_set("pending", ());
    state_set("notice", "");
    request_status();
}

fn build_ui() {
    let nodes = [];
    let status = state_get("status");
    let pending = state_get("pending");
    if status == () {
        nodes.push(ui_loading("online-play.status-loading", "Checking relay status…"));
    } else {
        let relay = relay_nodes(status);
        for node in relay {
            nodes.push(node);
        }
    }

    nodes.push(ui_label("online-play.client-heading", "Game client connection"));
    nodes.push(ui_button("online-play.start-game", "Start Cable Club in game"));
    nodes.push(ui_text_input("online-play.client-host", "Host", state_get("client_host")));
    nodes.push(ui_text_input("online-play.client-port", "Port", state_get("client_port")));
    nodes.push(ui_button("online-play.save", "Save for game"));
    nodes.push(ui_button("online-play.refresh", "Refresh status"));

    let notice = state_get("notice");
    if notice != "" {
        nodes.push(ui_label("online-play.notice", notice));
    }
    if pending != () {
        nodes.push(ui_loading("online-play.action-loading", "Working…"));
    }
    [ui_group("online-play.panel", "Cable Club", nodes)]
}

fn on_ui_event(widget_id, value) {
    if widget_id == "online-play.bind" { state_set("bind", value); }
    if widget_id == "online-play.port" { state_set("port", value); }
    if widget_id == "online-play.client-host" { state_set("client_host", value); }
    if widget_id == "online-play.client-port" { state_set("client_port", value); }

    if widget_id == "online-play.refresh" && state_get("status_request") == () {
        request_status();
    }
    if widget_id == "online-play.start" && state_get("pending") == () {
        state_set("notice", "");
        state_set("pending", request("__host.cable_start", #{bind: state_get("bind"), port: state_get("port")}));
    }
    if widget_id == "online-play.stop" && state_get("pending") == () {
        state_set("notice", "");
        state_set("pending", request("__host.cable_stop", #{}));
    }
    if widget_id == "online-play.start-game" && state_get("pending") == () {
        state_set("notice", "");
        state_set("pending", request("start_cable_club", #{}));
    }
    if widget_id == "online-play.save" && state_get("pending") == () {
        state_set("notice", "");
        state_set("pending", request("__host.cable_save", #{host: state_get("client_host"), port: state_get("client_port")}));
    }
}

fn on_operation_result(request_id, result) {
    if request_id == state_get("status_request") {
        state_set("status_request", ());
        if result != () {
            state_set("status", result);
        } else {
            state_set("notice", last_error());
        }
        return;
    }
    if request_id == state_get("pending") {
        state_set("pending", ());
        if result == () {
            state_set("notice", last_error());
        } else if result["started"] == true {
            state_set("notice", "Cable Club started in the game.");
        } else if result["saved"] == true {
            state_set("notice", "Game endpoint saved.");
        } else {
            state_set("notice", "Relay updated.");
        }
        request_status();
    }
}

fn on_shutdown() { }
