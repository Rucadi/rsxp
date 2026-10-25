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
//! `(operation, args)` or an HTTP `(status, message)`. All HTTP routes are
//! registered by the host from loaded extensions and only have their path
//! arguments extracted here. Extension Ruby owns validation,
//! game-state access, and the JSON response. `respond` completes the request
//! with that JSON string unchanged.
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
    collections::{HashMap, HashSet},
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

/// A route supplied by an extension. The core only matches the path and
/// extracts ordered string arguments; validation and the response remain in
/// the extension's Ruby dispatcher.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtensionRoute {
    pub method: String,
    pub path: String,
    pub operation: String,
    pub arguments: Vec<String>,
}

static EXTENSION_ROUTES: OnceLock<Mutex<Vec<ExtensionRoute>>> = OnceLock::new();
static EXTENSION_OPERATIONS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

/// Registers route patterns and operation names exposed by the active Rhai
/// extensions. Re-registering a route replaces its previous definition so a
/// UI rebuild cannot accumulate stale handlers.
pub fn register_extension_routes<I>(routes: I)
where
    I: IntoIterator<Item = (String, String, String, Vec<String>)>,
{
    let routes = routes
        .into_iter()
        .map(|(method, path, operation, arguments)| ExtensionRoute {
            method: method.to_ascii_uppercase(),
            path,
            operation,
            arguments,
        })
        .collect::<Vec<_>>();
    let route_store = EXTENSION_ROUTES.get_or_init(|| Mutex::new(Vec::new()));
    let operation_store = EXTENSION_OPERATIONS.get_or_init(|| Mutex::new(HashSet::new()));
    if let Ok(mut registered) = route_store.lock()
        && let Ok(mut operations) = operation_store.lock()
    {
        for route in routes {
            operations.insert(route.operation.clone());
            if let Some(existing) = registered
                .iter_mut()
                .find(|existing| existing.method == route.method && existing.path == route.path)
            {
                *existing = route;
            } else {
                registered.push(route);
            }
        }
    }
}

/// Snapshot of the route aliases currently supplied by loaded extensions.
pub fn extension_route_list() -> Vec<ExtensionRoute> {
    EXTENSION_ROUTES
        .get()
        .and_then(|routes| routes.lock().ok())
        .map(|routes| routes.clone())
        .unwrap_or_default()
}

/// Serializes the route aliases currently supplied by loaded extensions.
/// External adapters use this discovery endpoint to build their own tool
/// lists; the bridge intentionally exposes no game-specific schemas here.
pub fn extension_routes_json() -> String {
    let routes = extension_route_list();
    let mut output = String::from("{\"routes\":[");
    for (index, route) in routes.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        output.push_str("{\"method\":");
        output.push_str(&json_string(&route.method));
        output.push_str(",\"path\":");
        output.push_str(&json_string(&route.path));
        output.push_str(",\"operation\":");
        output.push_str(&json_string(&route.operation));
        output.push_str(",\"arguments\":[");
        for (argument_index, argument) in route.arguments.iter().enumerate() {
            if argument_index > 0 {
                output.push(',');
            }
            output.push_str(&json_string(argument));
        }
        output.push_str("]}");
    }
    output.push_str("]}");
    output
}

pub fn route_http(method: &str, target: &str) -> Result<(String, Vec<String>), (u16, String)> {
    route_extension(method, target)
}

fn route_extension(method: &str, target: &str) -> Result<(String, Vec<String>), (u16, String)> {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let query = query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            Some((url_decode(key)?, url_decode(value)?))
        })
        .collect::<HashMap<_, _>>();
    let routes = EXTENSION_ROUTES
        .get()
        .and_then(|routes| routes.lock().ok())
        .map(|routes| routes.clone())
        .unwrap_or_default();
    for route in routes {
        if route.method != method.to_ascii_uppercase() {
            continue;
        }
        let Some(path_values) = match_route_path(&route.path, path) else {
            continue;
        };
        let arguments = route
            .arguments
            .iter()
            .map(|name| {
                path_values
                    .get(name)
                    .or_else(|| query.get(name))
                    .cloned()
                    .unwrap_or_default()
            })
            .collect();
        return Ok((route.operation, arguments));
    }
    if path.starts_with("/v1/") {
        Err((404, "unknown API endpoint".into()))
    } else {
        Err((404, "unknown API path".into()))
    }
}

fn match_route_path(template: &str, path: &str) -> Option<HashMap<String, String>> {
    let template_parts = template.split('/').collect::<Vec<_>>();
    let path_parts = path.split('/').collect::<Vec<_>>();
    if template_parts.len() != path_parts.len() {
        return None;
    }
    let mut values = HashMap::new();
    for (template, value) in template_parts.iter().zip(path_parts) {
        if let Some(name) = template.strip_prefix('{').and_then(|v| v.strip_suffix('}')) {
            if name.is_empty() {
                return None;
            }
            values.insert(name.to_owned(), url_decode(value)?);
        } else if *template != value {
            return None;
        }
    }
    Some(values)
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
    let builtin = matches!(method, "ruby_inject" | "ruby_unload" | "start_cable_club");
    builtin
        || EXTENSION_OPERATIONS
            .get()
            .and_then(|operations| operations.lock().ok())
            .is_some_and(|operations| operations.contains(method))
}

/// Encodes arbitrary UTF-8 text for the bridge's tab-separated argument wire
/// format. The game-side adapter decodes this before passing it as a native
/// command payload, so Ruby source never has to be escaped into a header.
pub fn hex_encode(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value.as_bytes() {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
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

    fn register_test_extension_routes() {
        register_extension_routes([
            (
                "GET".into(),
                "/v1/test/catalog".into(),
                "test_catalog".into(),
                vec!["refresh".into()],
            ),
            (
                "GET".into(),
                "/v1/game/status".into(),
                "game_status".into(),
                vec![],
            ),
            (
                "GET".into(),
                "/v1/game/surroundings".into(),
                "get_surroundings".into(),
                vec!["radius".into()],
            ),
            (
                "GET".into(),
                "/v1/game/world-map".into(),
                "world_map".into(),
                vec![],
            ),
            (
                "GET".into(),
                "/v1/game/maps/{map_id}/tiles".into(),
                "map_preview".into(),
                vec!["map_id".into()],
            ),
            (
                "POST".into(),
                "/v1/game/teleport".into(),
                "teleport".into(),
                vec!["map_id".into(), "x".into(), "y".into()],
            ),
            (
                "POST".into(),
                "/v1/input/press".into(),
                "press_key".into(),
                vec!["key".into(), "frames".into()],
            ),
            (
                "DELETE".into(),
                "/v1/input/held".into(),
                "release_all_keys".into(),
                vec![],
            ),
            (
                "GET".into(),
                "/v1/pokemon-essentials/pokedex".into(),
                "pokemon_essentials_pokedex".into(),
                vec!["refresh".into()],
            ),
            (
                "POST".into(),
                "/v1/pokemon-essentials/inventory/add".into(),
                "pokemon_essentials_add_item".into(),
                vec!["item".into(), "quantity".into()],
            ),
            (
                "GET".into(),
                "/v1/pokemon-essentials/species/{species}".into(),
                "pokemon_essentials_species_entry".into(),
                vec!["species".into()],
            ),
            (
                "GET".into(),
                "/v1/pokemon-essentials/status".into(),
                "pokemon_essentials_status".into(),
                vec![],
            ),
            (
                "GET".into(),
                "/v1/pokemon-essentials/party".into(),
                "pokemon_essentials_party".into(),
                vec![],
            ),
            (
                "GET".into(),
                "/v1/pokemon-essentials/party/{slot}".into(),
                "pokemon_essentials_info".into(),
                vec!["slot".into()],
            ),
            (
                "GET".into(),
                "/v1/pokemon-essentials/encounters/current-map".into(),
                "pokemon_essentials_encounters".into(),
                vec![],
            ),
            (
                "GET".into(),
                "/v1/pokemon-essentials/npcs/current-map".into(),
                "pokemon_essentials_npcs".into(),
                vec![],
            ),
            (
                "GET".into(),
                "/v1/pokemon-essentials/battle".into(),
                "pokemon_essentials_battle".into(),
                vec![],
            ),
            (
                "POST".into(),
                "/v1/pokemon-essentials/wild-encounter".into(),
                "pokemon_essentials_wild_encounter".into(),
                vec![
                    "species".into(),
                    "level".into(),
                    "shiny".into(),
                    "form".into(),
                    "gender".into(),
                    "nature".into(),
                    "ability".into(),
                    "item".into(),
                    "max_ivs".into(),
                    "friendship".into(),
                ],
            ),
            (
                "GET".into(),
                "/v1/pokemon-essentials/debug".into(),
                "pokemon_essentials_debug".into(),
                vec![],
            ),
            (
                "POST".into(),
                "/v1/pokemon-essentials/debug".into(),
                "pokemon_essentials_debug".into(),
                vec!["enabled".into()],
            ),
        ]);
    }

    #[test]
    fn inventory_and_registry_routes_validate_mutations() {
        register_test_extension_routes();
        assert_eq!(
            route_http("GET", "/v1/test/catalog?refresh=refresh").unwrap(),
            ("test_catalog".to_string(), vec!["refresh".into()])
        );
        assert_eq!(
            route_http(
                "POST",
                "/v1/pokemon-essentials/inventory/add?item=TM99&quantity=3"
            )
            .unwrap(),
            (
                "pokemon_essentials_add_item".to_string(),
                vec!["TM99".into(), "3".into()]
            )
        );
        assert_eq!(
            route_http(
                "POST",
                "/v1/pokemon-essentials/inventory/add?item=TM99&quantity=0"
            )
            .unwrap()
            .1,
            vec!["TM99", "0"]
        );
    }

    #[test]
    fn routes_species_catalog_and_form_details() {
        register_test_extension_routes();
        assert_eq!(
            route_http("GET", "/v1/pokemon-essentials/pokedex?refresh=refresh").unwrap(),
            (
                "pokemon_essentials_pokedex".to_string(),
                vec!["refresh".into()]
            )
        );
        assert_eq!(
            route_http("GET", "/v1/pokemon-essentials/species/BULBASAUR_1").unwrap(),
            (
                "pokemon_essentials_species_entry".to_string(),
                vec!["BULBASAUR_1".into()]
            )
        );
        assert_eq!(
            route_http("GET", "/v1/pokemon-essentials/species/")
                .unwrap()
                .1,
            vec![""]
        );
        assert!(route_http("GET", "/v1/pokemon-essentials/species/BULBA/SAUR").is_err());
    }

    #[test]
    fn routes_openapi_game_and_pokemon_operations() {
        register_test_extension_routes();
        assert_eq!(
            route_http("GET", "/v1/game/status").unwrap(),
            ("game_status".to_string(), vec![])
        );
        assert_eq!(
            route_http("GET", "/v1/game/world-map").unwrap(),
            ("world_map".to_string(), vec![])
        );
        assert_eq!(
            route_http("GET", "/v1/game/maps/25/tiles").unwrap(),
            ("map_preview".to_string(), vec!["25".into()])
        );
        assert_eq!(
            route_http("POST", "/v1/game/teleport?map_id=3&x=34&y=14").unwrap(),
            (
                "teleport".to_string(),
                vec!["3".into(), "34".into(), "14".into()]
            )
        );
        assert_eq!(
            route_http("GET", "/v1/pokemon-essentials/party").unwrap(),
            ("pokemon_essentials_party".to_string(), vec![])
        );
        assert_eq!(
            route_http("GET", "/v1/pokemon-essentials/party/2").unwrap(),
            ("pokemon_essentials_info".to_string(), vec!["2".into()])
        );
        assert_eq!(
            route_http("GET", "/v1/pokemon-essentials/encounters/current-map")
                .unwrap()
                .0,
            "pokemon_essentials_encounters".to_string()
        );
        assert_eq!(
            route_http("GET", "/v1/pokemon-essentials/npcs/current-map")
                .unwrap()
                .0,
            "pokemon_essentials_npcs".to_string()
        );
        assert_eq!(
            route_http("GET", "/v1/pokemon-essentials/battle")
                .unwrap()
                .0,
            "pokemon_essentials_battle".to_string()
        );
        assert_eq!(
            route_http(
                "POST",
                "/v1/pokemon-essentials/wild-encounter?species=PIKACHU&level=30&shiny=true"
            )
            .unwrap(),
            (
                "pokemon_essentials_wild_encounter".to_string(),
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
        assert_eq!(
            route_http("POST", "/v1/pokemon-essentials/wild-encounter?species=")
                .unwrap()
                .1[0],
            ""
        );
        assert_eq!(
            route_http("GET", "/v1/pokemon-essentials/debug").unwrap(),
            ("pokemon_essentials_debug".to_string(), vec![])
        );
        assert_eq!(
            route_http("POST", "/v1/pokemon-essentials/debug?enabled=true").unwrap(),
            ("pokemon_essentials_debug".to_string(), vec!["true".into()])
        );
        assert_eq!(
            route_http("POST", "/v1/pokemon-essentials/debug").unwrap(),
            ("pokemon_essentials_debug".to_string(), vec!["".into()])
        );
        assert_eq!(
            route_http("POST", "/v1/pokemon-essentials/debug?enabled=maybe")
                .unwrap()
                .1,
            vec!["maybe"]
        );
        assert_eq!(
            route_http("POST", "/v1/input/press?key=up&frames=8").unwrap(),
            ("press_key".to_string(), vec!["up".into(), "8".into()])
        );
    }

    #[test]
    fn extension_route_discovery_is_generic_json() {
        register_test_extension_routes();
        let catalog = extension_routes_json();
        assert!(catalog.contains("/v1/pokemon-essentials/status"));
        assert!(catalog.contains("\"operation\":\"pokemon_essentials_status\""));
        assert!(catalog.contains("\"arguments\":[\"species\"]"));
    }

    #[test]
    fn rejects_bad_party_slots_and_unknown_api_paths() {
        register_test_extension_routes();
        assert_eq!(
            route_http("GET", "/v1/pokemon-essentials/party/nope")
                .unwrap()
                .1,
            vec!["nope"]
        );
        assert!(route_http("GET", "/v1/pokemon/party").is_err());
        assert!(route_http("GET", "/v1/unknown").is_err());
    }

    #[test]
    fn egui_requests_use_the_game_queue_without_starting_http() {
        assert_eq!(listening_port(), None);
        register_test_extension_routes();
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
