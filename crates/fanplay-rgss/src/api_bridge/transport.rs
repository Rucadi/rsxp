// SPDX-License-Identifier: GPL-2.0-or-later
//! Optional desktop HTTP transport for the game-command bridge.
//!
//! Local tools (the egui inspector, the workspace) call the shared queue
//! directly and never need this module. When enabled, it exposes the same
//! operations over a loopback-only HTTP/1.1 service so external processes (for
//! example `fanplay-mcp`) can drive the game. Extensions register additional
//! route aliases with the bridge; this transport does not know their domains
//! or response schemas.
//!
//! Environment: [`start_from_env`] starts the server only when `FANPLAY_API=1`,
//! reading the port from `FANPLAY_API_PORT` (default `47821`). [`start`] can be
//! called at runtime by the desktop workspace instead. The listener binds
//! `127.0.0.1` only and is idempotent for the same port while rejecting a
//! different one.
//!
//! Threading: one background thread (`fanplay-game-api`) accepts connections and
//! spawns a short-lived `fanplay-game-api-client` thread per connection. Each
//! client parses the request line and headers (bounded to 16 KiB), routes them
//! with `route_http`, and forwards the operation to the game thread, waiting up
//! to 10 seconds for the reply and cancelling the request on timeout. Responses
//! are JSON with `Connection: close`.
use super::*;
use fanplay_control::{Catalog, Endpoint, Parameter, ParameterType, Scope};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    sync::atomic::AtomicU16,
    thread,
    time::Duration,
};
static HTTP_PORT: AtomicU16 = AtomicU16::new(0);
static HTTP_START_LOCK: Mutex<()> = Mutex::new(());
/// Starts the optional API server when FANPLAY_API=1. It is loopback-only.
pub fn start_from_env() -> Result<(), String> {
    initialize();
    if std::env::var("FANPLAY_API").as_deref() != Ok("1") {
        return Ok(());
    }
    let port = std::env::var("FANPLAY_API_PORT")
        .unwrap_or_else(|_| "47821".into())
        .parse::<u16>()
        .map_err(|_| "FANPLAY_API_PORT must be 1..65535")?;
    if port == 0 {
        return Err("FANPLAY_API_PORT must be 1..65535".into());
    }
    start(port)
}

/// Starts the loopback API on `port`; the desktop workspace can call this at
/// runtime, without requiring an environment variable or restarting the game.
pub fn start(port: u16) -> Result<(), String> {
    if port == 0 {
        return Err("API port must be 1..65535".into());
    }
    initialize();
    let _start_guard = HTTP_START_LOCK
        .lock()
        .map_err(|_| "game API startup lock is unavailable")?;
    let existing_port = HTTP_PORT.load(Ordering::Acquire);
    if existing_port != 0 {
        return if existing_port == port {
            Ok(())
        } else {
            Err(format!(
                "game API is already listening on 127.0.0.1:{}",
                existing_port
            ))
        };
    }
    let listener = TcpListener::bind(("127.0.0.1", port))
        .map_err(|error| format!("cannot start game API on 127.0.0.1:{port}: {error}"))?;
    thread::Builder::new()
        .name("fanplay-game-api".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = thread::Builder::new()
                    .name("fanplay-game-api-client".into())
                    .spawn(move || serve(stream));
            }
        })
        .map_err(|error| format!("cannot start game API listener: {error}"))?;
    HTTP_PORT.store(port, Ordering::Release);
    eprintln!("[GameAPI] listening on 127.0.0.1:{port}");
    Ok(())
}

/// Returns the active loopback port, if the API has been started.
pub fn listening_port() -> Option<u16> {
    match HTTP_PORT.load(Ordering::Acquire) {
        0 => None,
        port => Some(port),
    }
}

fn control_status(port: u16) -> Value {
    json!({
        "server": "fanplay-control",
        "api_version": fanplay_control::API_VERSION,
        "mode": "game",
        "pid": std::process::id(),
        "port": port,
        "base_url": format!("http://127.0.0.1:{port}"),
        "game_running": true,
        "game_available": true,
        "launcher_available": false,
        "notes": "Headless game process. Launcher, catalog, save and settings endpoints are not available here; use the desktop launcher or its control API for those.",
    })
}

fn control_catalog() -> Catalog {
    let mut catalog = Catalog::standard();
    catalog.merge_dynamic(super::extension_route_list().into_iter().map(|route| {
        let mut endpoint = Endpoint::new(
            &route.operation,
            &route.method,
            &route.path,
            format!("Game operation {}", route.operation),
            "Route supplied by a loaded Rhai plugin and executed by the game bridge.",
            Scope::Extensions,
        );
        for argument in &route.arguments {
            let in_path = route.path.contains(&format!("{{{argument}}}"));
            endpoint = endpoint.parameter(if in_path {
                Parameter::path(
                    argument.as_str(),
                    ParameterType::String,
                    "Plugin route argument",
                )
            } else {
                Parameter::query(
                    argument.as_str(),
                    ParameterType::String,
                    false,
                    "Plugin route argument",
                )
            });
        }
        endpoint
    }));
    catalog
}

fn serve(stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let read = reader.read_line(&mut line);
    let result = if read.is_ok_and(|size| size > 0 && size <= 4096) {
        let mut headers = 0usize;
        loop {
            let mut header = String::new();
            match reader.read_line(&mut header) {
                Ok(size) if size > 0 => {
                    headers += size;
                    if headers > 16_384 {
                        break Err((431, "request headers are too large".to_string()));
                    }
                    if header == "\r\n" || header == "\n" {
                        break handle_http_request(&line);
                    }
                }
                _ => break Err((400, "incomplete HTTP request".to_string())),
            }
        }
    } else {
        Err((400, "request line is missing or too long".to_string()))
    };
    let mut stream = reader.into_inner();
    match result {
        Ok(body) => write_http_response(&mut stream, 200, &body),
        Err((status, message)) => {
            let body = format!("{{\"error\":{}}}", json_string(&message));
            write_http_response(&mut stream, status, &body);
        }
    }
}

fn write_http_response(stream: &mut TcpStream, status: u16, body: &str) {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        409 => "Conflict",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        _ => "Error",
    };
    let cors = if std::env::var("FANPLAY_API_CORS").as_deref() == Ok("1") {
        "Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, DELETE, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\n"
    } else {
        ""
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json; charset=utf-8\r\n{cors}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

fn handle_http_request(line: &str) -> Result<String, (u16, String)> {
    let mut parts = line.split_whitespace();
    let method = parts.next().ok_or((400, "missing HTTP method".into()))?;
    let target = parts.next().ok_or((400, "missing HTTP path".into()))?;
    if method == "OPTIONS" {
        return Ok("{}".into());
    }
    let path = target.split('?').next().unwrap_or(target);
    if method == "GET" {
        let port = listening_port().unwrap_or(0);
        match path {
            "/v1" => {
                return Ok(json!({
                    "server": "fanplay-control",
                    "api_version": fanplay_control::API_VERSION,
                    "discovery": format!("/v1/discovery"),
                    "openapi": "/v1/openapi.json",
                    "status": "/v1/status",
                })
                .to_string());
            }
            "/v1/discovery" => {
                return Ok(control_catalog()
                    .discovery(control_status(port))
                    .to_string());
            }
            "/v1/openapi.json" => {
                return Ok(control_catalog().openapi(control_status(port)).to_string());
            }
            "/v1/status" => return Ok(control_status(port).to_string()),
            _ => {}
        }
    }
    if method == "GET" && target == "/v1/extensions/routes" {
        return Ok(extension_routes_json());
    }
    if path == "/v1/game/settings" {
        return Err((
            409,
            "game settings require the desktop launcher control API".into(),
        ));
    }
    if method == "POST" && path == "/v1/game/reset" {
        crate::runtime_settings::request_reset();
        return Ok("{\"reset\":true}".into());
    }
    if path.starts_with("/v1/launcher")
        || path.starts_with("/v1/catalogs")
        || path.starts_with("/v1/settings")
        || path.starts_with("/v1/saves")
    {
        return Err((
            409,
            "this operation requires the desktop launcher control API".into(),
        ));
    }
    let (operation, args) = route_http(method, target)?;
    request(&operation, args).map_err(|message| (400, message))
}

fn request(method: &str, args: Vec<String>) -> Result<String, String> {
    if !is_supported_operation(method) {
        return Err("unknown game API operation".into());
    }
    let bridge = BRIDGE.get().ok_or("game API bridge is not active")?;
    let id = bridge.next_id.fetch_add(1, Ordering::Relaxed);
    let (reply_tx, reply_rx) = mpsc::sync_channel(1);
    let canceled = Arc::new(AtomicBool::new(false));
    bridge
        .requests
        .send(Request {
            id,
            method: method.into(),
            args,
            reply: reply_tx,
            canceled: Arc::clone(&canceled),
        })
        .map_err(|_| "game API bridge is closed".to_string())?;
    match reply_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(response) => Ok(response),
        Err(_) => {
            canceled.store(true, Ordering::Relaxed);
            Err("timed out waiting for the game thread; is the game responsive?".into())
        }
    }
}
