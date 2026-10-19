// SPDX-License-Identifier: GPL-2.0-or-later
//! Shared local game-command queue and HTTP routing used by the desktop and
//! browser tools (the egui inspector, the desktop workspace, and `rsxp-mcp`).
//!
//! The bridge is a process-local request queue. A caller submits a named
//! operation ([`send_from_ui`], or the desktop HTTP transport) and the *game
//! thread* drains it from Ruby's `Input.update` via [`poll`], then answers with
//! [`respond`]. Submission never blocks the game thread: the egui path returns a
//! request id whose reply is polled later with [`take_ui_response`].
//!
//! ## Wire format
//! [`poll`] yields zero or more newline-terminated requests, each
//! `id<TAB>method<TAB>arg…`, where `id` is a monotonic `u64`. `route_http` maps
//! an HTTP method and target (`/v1/…`, with URL-decoded query parameters) to
//! `(operation, args)` or an HTTP `(status, message)`, validating map/slot ids,
//! item names and quantities. `respond` completes the request with a JSON
//! string.
//!
//! ## Threading and lifecycle
//! `Bridge` holds a `std::sync::mpsc` request channel, the reply senders for
//! requests handed to Ruby, and the receivers polled by the egui workspace.
//! [`initialize`] creates it lazily. The network transports ([`start`],
//! [`start_from_env`], [`listening_port`]) are desktop-only and live in the
//! private `transport` submodule; on WASM they are stubbed because external
//! networking is unavailable. The runner side (a separate process) uses the
//! [`relay_enable`] / [`relay_requests`] / [`relay_replies`] thread-locals to
//! shuttle the same protocol across the session boundary. [`end_session`] fails
//! any in-flight requests when the selected game ends, so they are never
//! replayed into another game.
use std::sync::{atomic::AtomicBool, Arc};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender, SyncSender},
        Mutex, OnceLock,
    },
};
#[cfg(not(target_arch = "wasm32"))]
mod transport;
#[cfg(not(target_arch = "wasm32"))]
pub use transport::{listening_port, start, start_from_env};
#[cfg(target_arch = "wasm32")]
pub fn listening_port() -> Option<u16> {
    None
}
#[cfg(target_arch = "wasm32")]
pub fn start(_port: u16) -> Result<(), String> {
    Err("External networking is not available in this browser host".into())
}

struct Request {
    id: u64,
    method: String,
    args: Vec<String>,
    reply: SyncSender<String>,
    canceled: Arc<AtomicBool>,
}

struct Bridge {
    requests: Sender<Request>,
    ready: Mutex<Receiver<Request>>,
    pending: Mutex<HashMap<u64, SyncSender<String>>>,
    ui_pending: Mutex<HashMap<u64, Receiver<String>>>,
    next_id: AtomicU64,
}

static BRIDGE: OnceLock<Bridge> = OnceLock::new();

/// Initializes the game-thread request queue without starting a network
/// listener. The egui inspector uses this local path directly.
pub fn initialize() {
    BRIDGE.get_or_init(|| {
        let (sender, receiver) = mpsc::channel();
        Bridge {
            requests: sender,
            ready: Mutex::new(receiver),
            pending: Mutex::new(HashMap::new()),
            ui_pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    });
}

pub fn route_http(
    method: &str,
    target: &str,
) -> Result<(&'static str, Vec<String>), (u16, String)> {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let query: Vec<(String, String)> = query
        .split('&')
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            Some((url_decode(key)?, url_decode(value)?))
        })
        .collect();
    let value = |name: &str| {
        query
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };
    Ok(match (method, path) {
        ("GET", "/v1/game/status") => ("game_status", vec![]),
        ("GET", "/v1/game/surroundings") => (
            "get_surroundings",
            vec![value("radius").unwrap_or("5").to_string()],
        ),
        ("GET", "/v1/game/world-map") => ("world_map", vec![]),
        ("GET", path) if path.starts_with("/v1/game/maps/") && path.ends_with("/tiles") => {
            let map_id = &path["/v1/game/maps/".len()..path.len() - "/tiles".len()];
            if map_id.is_empty() || !map_id.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err((400, "map ID must be a positive integer".into()));
            }
            ("map_preview", vec![map_id.to_string()])
        }
        ("POST", "/v1/game/teleport") => (
            "teleport",
            vec![
                value("map_id").unwrap_or("").to_string(),
                value("x").unwrap_or("").to_string(),
                value("y").unwrap_or("").to_string(),
            ],
        ),
        ("POST", "/v1/pokemon-essentials/inventory/add") => {
            let item = value("item").unwrap_or("");
            let quantity = value("quantity").unwrap_or("1");
            if item.is_empty()
                || !item
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            {
                return Err((400, "item must be a game item ID".into()));
            }
            if !quantity
                .parse::<u32>()
                .ok()
                .is_some_and(|quantity| (1..=9999).contains(&quantity))
            {
                return Err((400, "quantity must be 1..9999".into()));
            }
            (
                "pokemon_essentials_add_item",
                vec![item.to_owned(), quantity.to_owned()],
            )
        }
        ("GET", "/v1/pokemon-essentials/movedex") => (
            "pokemon_essentials_movedex",
            vec![value("refresh").unwrap_or("").to_string()],
        ),
        ("GET", "/v1/pokemon-essentials/itemdex") => (
            "pokemon_essentials_itemdex",
            vec![value("refresh").unwrap_or("").to_string()],
        ),
        ("GET", "/v1/pokemon-essentials/pokedex") => (
            "pokemon_essentials_pokedex",
            vec![value("refresh").unwrap_or("").to_string()],
        ),
        ("GET", path) if path.starts_with("/v1/pokemon-essentials/species/") => {
            let species = &path["/v1/pokemon-essentials/species/".len()..];
            if species.is_empty()
                || !species
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            {
                return Err((400, "species must be a game species ID".into()));
            }
            (
                "pokemon_essentials_species_entry",
                vec![species.to_string()],
            )
        }
        ("GET", "/v1/pokemon-essentials/status") => ("pokemon_essentials_status", vec![]),
        ("GET", "/v1/pokemon-essentials/party") => ("pokemon_essentials_party", vec![]),
        ("GET", path) if path.starts_with("/v1/pokemon-essentials/party/") => {
            let slot = &path["/v1/pokemon-essentials/party/".len()..];
            if slot.is_empty() || !slot.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err((400, "party slot must be a non-negative integer".into()));
            }
            ("pokemon_essentials_info", vec![slot.to_string()])
        }
        ("GET", "/v1/pokemon-essentials/encounters/current-map") => {
            ("pokemon_essentials_encounters", vec![])
        }
        ("GET", "/v1/pokemon-essentials/npcs/current-map") => ("pokemon_essentials_npcs", vec![]),
        ("GET", "/v1/pokemon-essentials/battle") => ("pokemon_essentials_battle", vec![]),
        ("POST", "/v1/pokemon-essentials/wild-encounter") => {
            let species = value("species").unwrap_or("");
            if species.is_empty()
                || !species
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            {
                return Err((400, "species must be a game species ID".into()));
            }
            (
                "pokemon_essentials_wild_encounter",
                vec![
                    species.to_owned(),
                    value("level").unwrap_or("5").to_owned(),
                    value("shiny").unwrap_or("").to_owned(),
                    value("form").unwrap_or("").to_owned(),
                    value("gender").unwrap_or("").to_owned(),
                    value("nature").unwrap_or("").to_owned(),
                    value("ability").unwrap_or("").to_owned(),
                    value("item").unwrap_or("").to_owned(),
                    value("max_ivs").unwrap_or("").to_owned(),
                    value("friendship").unwrap_or("").to_owned(),
                ],
            )
        }
        ("GET", "/v1/pokemon-essentials/debug") => {
            ("pokemon_essentials_debug", vec![String::new()])
        }
        ("POST", "/v1/pokemon-essentials/debug") => {
            let enabled = value("enabled").unwrap_or("toggle");
            if !["true", "false", "toggle"].contains(&enabled) {
                return Err((400, "enabled must be true, false, or toggle".into()));
            }
            ("pokemon_essentials_debug", vec![enabled.to_owned()])
        }
        ("POST", "/v1/input/press") => (
            "press_key",
            vec![
                value("key").unwrap_or("").to_string(),
                value("frames").unwrap_or("1").to_string(),
            ],
        ),
        ("DELETE", "/v1/input/held") => ("release_all_keys", vec![]),
        _ if path.starts_with("/v1/") => return Err((404, "unknown API endpoint".into())),
        _ => return Err((404, "unknown API path".into())),
    })
}

fn url_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                decoded.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16)? as u8;
                let lo = (bytes[i + 2] as char).to_digit(16)? as u8;
                decoded.push((hi << 4) | lo);
                i += 3;
            }
            b'%' => return None,
            byte => {
                decoded.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(decoded).ok()
}

/// Queues an API operation from the egui workspace without blocking the game
/// thread. The caller should poll [`take_ui_response`] on later UI frames.
pub fn send_from_ui(method: &str, args: Vec<String>) -> Result<u64, String> {
    if !is_supported_operation(method) {
        return Err("unknown game API operation".into());
    }
    initialize();
    let bridge = BRIDGE.get().ok_or("game API is unavailable")?;
    let id = bridge.next_id.fetch_add(1, Ordering::Relaxed);
    let (reply_tx, reply_rx) = mpsc::sync_channel(1);
    bridge
        .requests
        .send(Request {
            id,
            method: method.into(),
            args,
            reply: reply_tx,
            canceled: Arc::new(AtomicBool::new(false)),
        })
        .map_err(|_| "game API request queue is closed".to_string())?;
    bridge
        .ui_pending
        .lock()
        .map_err(|_| "game API response queue is unavailable")?
        .insert(id, reply_rx);
    Ok(id)
}

/// Returns and removes a completed egui request response, or `None` if it is
/// still waiting for the game thread to process it.
pub fn take_ui_response(id: u64) -> Option<Result<String, String>> {
    let bridge = BRIDGE.get()?;
    let mut pending = bridge.ui_pending.lock().ok()?;
    let receiver = pending.get(&id)?;
    match receiver.try_recv() {
        Ok(response) => {
            pending.remove(&id);
            Some(Ok(response))
        }
        Err(mpsc::TryRecvError::Empty) => None,
        Err(mpsc::TryRecvError::Disconnected) => {
            pending.remove(&id);
            Some(Err("game API response channel closed".into()))
        }
    }
}

fn is_supported_operation(method: &str) -> bool {
    matches!(
        method,
        "game_status"
            | "get_surroundings"
            | "world_map"
            | "map_preview"
            | "teleport"
            | "pokemon_essentials_status"
            | "pokemon_essentials_party"
            | "pokemon_essentials_info"
            | "pokemon_essentials_add_item"
            | "pokemon_essentials_movedex"
            | "pokemon_essentials_itemdex"
            | "pokemon_essentials_pokedex"
            | "pokemon_essentials_species_entry"
            | "pokemon_essentials_encounters"
            | "pokemon_essentials_npcs"
            | "pokemon_essentials_battle"
            | "pokemon_essentials_wild_encounter"
            | "pokemon_essentials_debug"
            | "start_cable_club"
            | "press_key"
            | "release_all_keys"
    )
}

/// Moves queued client requests to Ruby; called by Input.update on the game thread.
pub fn poll() -> Vec<u8> {
    if RELAY.get() {
        return RELAY_REQUESTS.with(|r| std::mem::take(&mut *r.borrow_mut()));
    }
    let Some(bridge) = BRIDGE.get() else {
        return Vec::new();
    };
    let Ok(receiver) = bridge.ready.lock() else {
        return Vec::new();
    };
    let mut output = String::new();
    loop {
        match receiver.try_recv() {
            Ok(request) => {
                if request.canceled.load(Ordering::Relaxed) {
                    continue;
                }
                let mut fields = vec![request.id.to_string(), request.method];
                fields.extend(request.args);
                output.push_str(&fields.join("\t"));
                output.push('\n');
                if let Ok(mut pending) = bridge.pending.lock() {
                    pending.insert(request.id, request.reply);
                }
            }
            Err(_) => break,
        }
    }
    output.into_bytes()
}

/// Completes a request previously returned by `poll` with a JSON payload.
pub fn respond(id: u64, payload: &str) -> Result<(), String> {
    if RELAY.get() {
        RELAY_REPLIES.with(|r| r.borrow_mut().push((id, payload.into())));
        return Ok(());
    }
    let bridge = BRIDGE.get().ok_or("game API bridge is not active")?;
    let sender = bridge
        .pending
        .lock()
        .map_err(|_| "game API response queue is unavailable")?
        .remove(&id)
        .ok_or("unknown or expired game API request")?;
    sender
        .send(payload.to_string())
        .map_err(|_| "MCP client disconnected".into())
}

#[cfg(any(test, not(target_arch = "wasm32")))]
fn json_string(value: &str) -> String {
    let mut result = String::from("\"");
    for character in value.chars() {
        match character {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c if c.is_control() => result.push_str(&format!("\\u{:04x}", c as u32)),
            c => result.push(c),
        }
    }
    result.push('"');
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_and_registry_routes_validate_mutations() {
        for (path, operation) in [
            ("movedex", "pokemon_essentials_movedex"),
            ("itemdex", "pokemon_essentials_itemdex"),
        ] {
            assert_eq!(
                route_http(
                    "GET",
                    &format!("/v1/pokemon-essentials/{path}?refresh=refresh")
                )
                .unwrap(),
                (operation, vec!["refresh".into()])
            );
        }
        assert_eq!(
            route_http(
                "POST",
                "/v1/pokemon-essentials/inventory/add?item=TM99&quantity=3"
            )
            .unwrap(),
            (
                "pokemon_essentials_add_item",
                vec!["TM99".into(), "3".into()]
            )
        );
        for target in [
            "item=TM99&quantity=0",
            "item=TM99&quantity=10000",
            "item=../TM99&quantity=1",
        ] {
            assert!(route_http(
                "POST",
                &format!("/v1/pokemon-essentials/inventory/add?{target}")
            )
            .is_err());
        }
        assert!(route_http("GET", "/v1/pokemon-essentials/inventory/add?item=TM99").is_err());
    }

    #[test]
    fn routes_species_catalog_and_form_details() {
        assert_eq!(
            route_http("GET", "/v1/pokemon-essentials/pokedex?refresh=refresh").unwrap(),
            ("pokemon_essentials_pokedex", vec!["refresh".into()])
        );
        assert_eq!(
            route_http("GET", "/v1/pokemon-essentials/species/BULBASAUR_1").unwrap(),
            (
                "pokemon_essentials_species_entry",
                vec!["BULBASAUR_1".into()]
            )
        );
        assert!(route_http("GET", "/v1/pokemon-essentials/species/").is_err());
        assert!(route_http("GET", "/v1/pokemon-essentials/species/BULBA/SAUR").is_err());
    }

    #[test]
    fn routes_openapi_game_and_pokemon_operations() {
        assert_eq!(
            route_http("GET", "/v1/game/status").unwrap(),
            ("game_status", vec![])
        );
        assert_eq!(
            route_http("GET", "/v1/game/world-map").unwrap(),
            ("world_map", vec![])
        );
        assert_eq!(
            route_http("GET", "/v1/game/maps/25/tiles").unwrap(),
            ("map_preview", vec!["25".into()])
        );
        assert_eq!(
            route_http("POST", "/v1/game/teleport?map_id=3&x=34&y=14").unwrap(),
            ("teleport", vec!["3".into(), "34".into(), "14".into()])
        );
        assert_eq!(
            route_http("GET", "/v1/pokemon-essentials/party").unwrap(),
            ("pokemon_essentials_party", vec![])
        );
        assert_eq!(
            route_http("GET", "/v1/pokemon-essentials/party/2").unwrap(),
            ("pokemon_essentials_info", vec!["2".into()])
        );
        assert_eq!(
            route_http("GET", "/v1/pokemon-essentials/encounters/current-map")
                .unwrap()
                .0,
            "pokemon_essentials_encounters"
        );
        assert_eq!(
            route_http("GET", "/v1/pokemon-essentials/npcs/current-map")
                .unwrap()
                .0,
            "pokemon_essentials_npcs"
        );
        assert_eq!(
            route_http("GET", "/v1/pokemon-essentials/battle")
                .unwrap()
                .0,
            "pokemon_essentials_battle"
        );
        assert_eq!(
            route_http(
                "POST",
                "/v1/pokemon-essentials/wild-encounter?species=PIKACHU&level=30&shiny=true"
            )
            .unwrap(),
            (
                "pokemon_essentials_wild_encounter",
                vec![
                    "PIKACHU".into(),
                    "30".into(),
                    "true".into(),
                    "".into(),
                    "".into(),
                    "".into(),
                    "".into(),
                    "".into(),
                    "".into(),
                    "".into()
                ]
            )
        );
        assert!(route_http("POST", "/v1/pokemon-essentials/wild-encounter?species=").is_err());
        assert!(route_http("POST", "/v1/pokemon-essentials/wild-encounter?species=A/B").is_err());
        assert_eq!(
            route_http("GET", "/v1/pokemon-essentials/debug").unwrap(),
            ("pokemon_essentials_debug", vec![String::new()])
        );
        assert_eq!(
            route_http("POST", "/v1/pokemon-essentials/debug?enabled=true").unwrap(),
            ("pokemon_essentials_debug", vec!["true".into()])
        );
        assert_eq!(
            route_http("POST", "/v1/pokemon-essentials/debug").unwrap(),
            ("pokemon_essentials_debug", vec!["toggle".into()])
        );
        assert!(route_http("POST", "/v1/pokemon-essentials/debug?enabled=maybe").is_err());
        assert_eq!(
            route_http("POST", "/v1/input/press?key=up&frames=8").unwrap(),
            ("press_key", vec!["up".into(), "8".into()])
        );
    }

    #[test]
    fn rejects_bad_party_slots_and_unknown_api_paths() {
        assert!(route_http("GET", "/v1/pokemon-essentials/party/nope").is_err());
        assert!(route_http("GET", "/v1/pokemon/party").is_err());
        assert!(route_http("GET", "/v1/unknown").is_err());
    }

    #[test]
    fn egui_requests_use_the_game_queue_without_starting_http() {
        assert_eq!(listening_port(), None);
        let id = send_from_ui("game_status", Vec::new()).unwrap();
        let request = String::from_utf8(poll()).unwrap();
        assert_eq!(request, format!("{id}\tgame_status\n"));

        respond(id, r#"{"ready":true}"#).unwrap();
        assert_eq!(take_ui_response(id), Some(Ok(r#"{"ready":true}"#.into())));
        assert_eq!(listening_port(), None);

        let id = send_from_ui("start_cable_club", Vec::new()).unwrap();
        let request = String::from_utf8(poll()).unwrap();
        assert_eq!(request, format!("{id}\tstart_cable_club\n"));
        respond(id, r#"{"started":true}"#).unwrap();
        assert_eq!(take_ui_response(id), Some(Ok(r#"{"started":true}"#.into())));
    }
}

// Runner-side relay preserves the existing inspector protocol across a process
// boundary. Only the isolated game thread accesses these cells.
thread_local! {
    static RELAY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static RELAY_REQUESTS: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
    static RELAY_REPLIES: std::cell::RefCell<Vec<(u64, String)>> = const { std::cell::RefCell::new(Vec::new()) };
}
pub fn relay_enable() {
    RELAY.set(true);
}
pub fn relay_requests(bytes: &[u8]) {
    RELAY_REQUESTS.with(|r| r.borrow_mut().extend_from_slice(bytes));
}
pub fn relay_replies() -> Vec<(u64, String)> {
    RELAY_REPLIES.with(|r| std::mem::take(&mut *r.borrow_mut()))
}

/// Fail requests when the selected game ends; never replay them into another game.
pub fn end_session() {
    let Some(bridge) = BRIDGE.get() else { return };
    if let Ok(ready) = bridge.ready.lock() {
        while let Ok(request) = ready.try_recv() {
            let _ = request
                .reply
                .send("{\"error\":\"Game session ended\"}".into());
        }
    }
    if let Ok(mut pending) = bridge.pending.lock() {
        for (_, sender) in pending.drain() {
            let _ = sender.send("{\"error\":\"Game session ended\"}".into());
        }
    }
    if let Ok(mut pending) = bridge.ui_pending.lock() {
        pending.clear();
    }
}
