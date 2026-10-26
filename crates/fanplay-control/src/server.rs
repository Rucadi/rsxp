// SPDX-License-Identifier: GPL-2.0-or-later
//! Small loopback HTTP server for the typed control catalog.
//!
//! The server deliberately implements only what the contract needs: request
//! line, bounded headers, bounded JSON bodies, path/query parameter parsing and
//! JSON responses. There is no HTTP/2, TLS, chunked upload, or keep-alive. That
//! keeps the attack surface small for a local-only control plane and makes the
//! same behavior available on every desktop platform.
//!
//! Every connection is handled on its own short-lived thread. A slow client
//! cannot block the accept loop, and read/write timeouts bound abandoned
//! connections. The listener binds `127.0.0.1` unless the caller explicitly
//! passes another loopback address to [`start_on`].

use crate::{Catalog, Endpoint, Invocation, API_VERSION};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_REQUEST_LINE: usize = 8 * 1024;
const MAX_HEADERS: usize = 32 * 1024;
const MAX_BODY: usize = 2 * 1024 * 1024;

/// A parsed control request. `path` and query parameters are decoded before
/// the handler sees them, so adapters never reimplement URL parsing.
#[derive(Clone, Debug)]
pub struct ControlRequest {
    pub invocation: Invocation,
    /// Peer IP, useful for diagnostics. Never used for authentication: the
    /// listener is loopback-only and local processes are trusted.
    pub remote: String,
}

/// A response from a host adapter. `body` must be JSON; the server adds the
/// HTTP envelope, CORS headers and content length.
#[derive(Clone, Debug, PartialEq)]
pub struct ControlResponse {
    pub status: u16,
    pub body: Value,
}

impl ControlResponse {
    pub fn ok(body: impl Into<Value>) -> Self {
        Self {
            status: 200,
            body: body.into(),
        }
    }

    pub fn error(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            body: json!({ "error": message.into() }),
        }
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::error(409, message)
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::error(400, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::error(404, message)
    }
}

/// Host implementation behind the catalog.
pub trait ControlHandler: Send + Sync + 'static {
    fn handle(&self, request: ControlRequest) -> ControlResponse;

    /// Endpoints contributed by loaded extensions at runtime.
    fn dynamic_endpoints(&self) -> Vec<Endpoint> {
        Vec::new()
    }

    /// Additional fields merged into `/v1/status`.
    fn status(&self) -> Value {
        json!({})
    }

    /// `launcher` or `game`; included in discovery so a client can label the
    /// endpoint even before it calls `/v1/status`.
    fn mode(&self) -> &'static str {
        "launcher"
    }
}

struct ServerState {
    catalog: Catalog,
    handler: Arc<dyn ControlHandler>,
    port: u16,
    started: Instant,
    stop: AtomicBool,
    connections: AtomicU64,
    bind: String,
}

/// Owns the listener thread. Dropping the handle stops the server.
pub struct ServerHandle {
    state: Arc<ServerState>,
    address: std::net::SocketAddr,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl ServerHandle {
    pub fn port(&self) -> u16 {
        self.address.port()
    }

    pub fn address(&self) -> std::net::SocketAddr {
        self.address
    }

    pub fn url(&self) -> String {
        format!("http://{}/", self.address)
    }

    pub fn discovery_url(&self) -> String {
        format!("http://{}/v1/discovery", self.address)
    }

    pub fn stop(&self) {
        self.state.stop.store(true, Ordering::Release);
        // Wake the blocking accept call. The connection is closed immediately.
        let _ = TcpStream::connect_timeout(&self.address, Duration::from_millis(150));
        if let Ok(mut thread) = self.thread.lock()
            && let Some(thread) = thread.take()
        {
            let _ = thread.join();
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Starts the standard catalog on `127.0.0.1:port`.
pub fn start(port: u16, handler: Arc<dyn ControlHandler>) -> Result<ServerHandle, String> {
    start_with_catalog(port, Catalog::standard(), handler)
}

/// Starts the standard catalog on an explicit loopback address.
pub fn start_on(
    bind: &str,
    port: u16,
    handler: Arc<dyn ControlHandler>,
) -> Result<ServerHandle, String> {
    start_with_catalog_on(bind, port, Catalog::standard(), handler)
}

/// Starts a caller-supplied catalog on `127.0.0.1:port`.
pub fn start_with_catalog(
    port: u16,
    catalog: Catalog,
    handler: Arc<dyn ControlHandler>,
) -> Result<ServerHandle, String> {
    start_with_catalog_on("127.0.0.1", port, catalog, handler)
}

/// Starts a caller-supplied catalog on an explicit bind address.
pub fn start_with_catalog_on(
    bind: &str,
    port: u16,
    catalog: Catalog,
    handler: Arc<dyn ControlHandler>,
) -> Result<ServerHandle, String> {
    let listener = TcpListener::bind((bind, port))
        .map_err(|error| format!("cannot start control API on {bind}:{port}: {error}"))?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("cannot inspect control API listener: {error}"))?;
    listener
        .set_nonblocking(false)
        .map_err(|error| format!("cannot configure control API listener: {error}"))?;
    let state = Arc::new(ServerState {
        catalog,
        handler,
        port: address.port(),
        started: Instant::now(),
        stop: AtomicBool::new(false),
        connections: AtomicU64::new(0),
        bind: bind.to_owned(),
    });
    let accept_state = Arc::clone(&state);
    let thread = std::thread::Builder::new()
        .name("fanplay-control-api".into())
        .spawn(move || accept_loop(listener, accept_state))
        .map_err(|error| format!("cannot start control API thread: {error}"))?;
    eprintln!(
        "[ControlAPI] listening on http://{} (discovery http://{}/v1/discovery)",
        address, address
    );
    Ok(ServerHandle {
        state,
        address,
        thread: Mutex::new(Some(thread)),
    })
}

fn accept_loop(listener: TcpListener, state: Arc<ServerState>) {
    for connection in listener.incoming() {
        if state.stop.load(Ordering::Acquire) {
            break;
        }
        let Ok(stream) = connection else {
            continue;
        };
        let id = state.connections.fetch_add(1, Ordering::Relaxed) + 1;
        let state = Arc::clone(&state);
        let _ = std::thread::Builder::new()
            .name(format!("fanplay-control-{id}"))
            .spawn(move || serve(stream, state));
    }
}

fn serve(mut stream: TcpStream, state: Arc<ServerState>) {
    let remote = stream
        .peer_addr()
        .map(|address| address.ip().to_string())
        .unwrap_or_else(|_| "local".into());
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let response = match read_request(&mut stream) {
        Ok(request) => dispatch(&state, request, remote),
        Err((status, message)) => ControlResponse::error(status, message),
    };
    let _ = write_response(&mut stream, response);
}

struct ParsedRequest {
    method: String,
    target: String,
    body: Value,
}

fn read_request(stream: &mut TcpStream) -> Result<ParsedRequest, (u16, String)> {
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .map_err(|error| (500, format!("cannot read control API request: {error}")))?,
    );
    let mut line = String::new();
    let read = reader
        .read_line(&mut line)
        .map_err(|error| (400, format!("cannot read request line: {error}")))?;
    if read == 0 || read > MAX_REQUEST_LINE {
        return Err((400, "request line is missing or too long".into()));
    }
    let mut parts = line.split_whitespace();
    let method = parts
        .next()
        .ok_or((400, "missing HTTP method".into()))?
        .to_ascii_uppercase();
    let target = parts
        .next()
        .ok_or((400, "missing HTTP path".into()))?
        .to_owned();
    let mut content_length = 0usize;
    let mut headers = 0usize;
    let mut expect_continue = false;
    loop {
        let mut header = String::new();
        let read = reader
            .read_line(&mut header)
            .map_err(|error| (400, format!("cannot read request headers: {error}")))?;
        if read == 0 {
            return Err((400, "incomplete HTTP headers".into()));
        }
        headers += read;
        if headers > MAX_HEADERS {
            return Err((431, "request headers are too large".into()));
        }
        if header == "\r\n" || header == "\n" {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                content_length = value
                    .trim()
                    .parse()
                    .map_err(|_| (400, "invalid content-length header".into()))?;
            }
            if name.trim().eq_ignore_ascii_case("expect")
                && value.to_ascii_lowercase().contains("100-continue")
            {
                expect_continue = true;
            }
            if name.trim().eq_ignore_ascii_case("transfer-encoding")
                && value.to_ascii_lowercase().contains("chunked")
            {
                return Err((400, "chunked request bodies are not supported".into()));
            }
        }
    }
    if content_length > MAX_BODY {
        return Err((413, "request body is too large".into()));
    }
    if expect_continue && content_length > 0 {
        stream
            .write_all(b"HTTP/1.1 100 Continue\r\n\r\n")
            .map_err(|error| (400, format!("cannot send 100 Continue: {error}")))?;
        stream
            .flush()
            .map_err(|error| (400, format!("cannot flush 100 Continue: {error}")))?;
    }
    let mut body = Vec::with_capacity(content_length);
    if content_length > 0 {
        body.resize(content_length, 0);
        reader
            .read_exact(&mut body)
            .map_err(|error| (400, format!("cannot read request body: {error}")))?;
    }
    let body = if body.iter().all(u8::is_ascii_whitespace) {
        json!({})
    } else {
        serde_json::from_slice::<Value>(&body)
            .map_err(|error| (400, format!("request body is not valid JSON: {error}")))?
    };
    if !body.is_object() && !body.is_null() {
        return Err((400, "request body must be a JSON object".into()));
    }
    Ok(ParsedRequest {
        method,
        target,
        body: if body.is_null() { json!({}) } else { body },
    })
}

fn dispatch(state: &ServerState, request: ParsedRequest, remote: String) -> ControlResponse {
    let (path, query) = split_target(&request.target);
    if request.method == "OPTIONS" {
        return ControlResponse {
            status: 204,
            body: Value::Null,
        };
    }

    let mut catalog = state.catalog.clone();
    catalog.merge_dynamic(state.handler.dynamic_endpoints());
    let status = server_status(state, &catalog);
    let base_url = status["base_url"]
        .as_str()
        .unwrap_or("http://127.0.0.1")
        .trim_end_matches('/')
        .to_owned();

    // Discovery and documentation are served by the server itself so they stay
    // consistent even while a host adapter is busy or a game is starting.
    match (request.method.as_str(), path.as_str()) {
        ("GET", "/v1") => {
            return ControlResponse::ok(json!({
                "server": "fanplay-control",
                "api_version": API_VERSION,
                "discovery": format!("{base_url}/v1/discovery"),
                "openapi": format!("{base_url}/v1/openapi.json"),
                "status": format!("{base_url}/v1/status"),
            }));
        }
        ("GET", "/v1/discovery") => {
            return ControlResponse::ok(catalog.discovery(status));
        }
        ("GET", "/v1/openapi.json") => {
            return ControlResponse::ok(catalog.openapi(status));
        }
        ("GET", "/v1/status") => {
            return ControlResponse::ok(status);
        }
        _ => {}
    }

    let Some(matched) = catalog.match_path(&request.method, &path) else {
        let allowed = catalog
            .endpoints()
            .iter()
            .filter(|endpoint| same_path(&endpoint.path, &path))
            .map(|endpoint| endpoint.method.clone())
            .collect::<Vec<_>>();
        if !allowed.is_empty() {
            let mut response = ControlResponse::error(
                405,
                format!("method {} is not allowed for {}", request.method, path),
            );
            if let Some(object) = response.body.as_object_mut() {
                object.insert("allowed".into(), json!(allowed));
            }
            return response;
        }
        let mut response = ControlResponse::not_found(format!("unknown control endpoint: {path}"));
        if let Some(object) = response.body.as_object_mut() {
            object.insert(
                "discovery".into(),
                json!(format!("{base_url}/v1/discovery")),
            );
        }
        return response;
    };

    let parameters = match parse_parameters(
        &matched.endpoint,
        matched
            .path_parameters
            .into_iter()
            .map(|(key, value)| (key, url_decode(&value).unwrap_or(value)))
            .collect(),
        query,
        &request.body,
    ) {
        Ok(parameters) => parameters,
        Err((status, message)) => return ControlResponse::error(status, message),
    };
    let invocation = Invocation {
        endpoint: matched.endpoint.id.clone(),
        method: request.method.clone(),
        target: request.target.clone(),
        parameters,
        body: request.body,
    };
    state.handler.handle(ControlRequest { invocation, remote })
}

fn server_status(state: &ServerState, catalog: &Catalog) -> Value {
    let mut status = json!({
        "server": "fanplay-control",
        "api_version": API_VERSION,
        "mode": state.handler.mode(),
        "pid": std::process::id(),
        "uptime_seconds": state.started.elapsed().as_secs(),
        "port": state.port,
        "bind": state.bind,
        "base_url": format!("http://{}", state.bind_address()),
        "endpoint_count": catalog.endpoints().len(),
        "connections": state.connections.load(Ordering::Relaxed),
        "now": unix_seconds(),
    });
    if let (Some(object), Value::Object(extra)) = (status.as_object_mut(), state.handler.status()) {
        for (key, value) in extra {
            object.insert(key, value);
        }
    }
    status
}

impl ServerState {
    fn bind_address(&self) -> String {
        if self.port == 80 {
            self.bind.clone()
        } else {
            format!("{}:{}", self.bind, self.port)
        }
    }
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn split_target(target: &str) -> (String, BTreeMap<String, String>) {
    // Absolute-form request targets are legal for proxies; normalize them.
    let target = if let Some(rest) = target
        .strip_prefix("http://")
        .or_else(|| target.strip_prefix("https://"))
    {
        rest.find('/').map(|index| &rest[index..]).unwrap_or("/")
    } else {
        target
    };
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let mut values = BTreeMap::new();
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if let (Some(key), Some(value)) = (url_decode(key), url_decode(value)) {
            values.insert(key, value);
        }
    }
    let path = if path.is_empty() {
        "/".into()
    } else if path.len() > 1 && path.ends_with('/') {
        path.trim_end_matches('/').to_owned()
    } else {
        path.to_owned()
    };
    (path, values)
}

fn same_path(template: &str, path: &str) -> bool {
    let template = template.split('/').collect::<Vec<_>>();
    let path = path.split('/').collect::<Vec<_>>();
    template.len() == path.len()
        && template
            .iter()
            .zip(path)
            .all(|(template, value)| template.starts_with('{') || *template == value)
}

fn parse_parameters(
    endpoint: &Endpoint,
    path_parameters: BTreeMap<String, String>,
    query: BTreeMap<String, String>,
    body: &Value,
) -> Result<BTreeMap<String, Value>, (u16, String)> {
    let mut parameters = BTreeMap::new();
    for parameter in &endpoint.parameters {
        let raw = match parameter.location {
            crate::ParameterLocation::Path => path_parameters
                .get(&parameter.name)
                .cloned()
                .map(RawParameter::Text),
            crate::ParameterLocation::Query => {
                query.get(&parameter.name).cloned().map(RawParameter::Text)
            }
            crate::ParameterLocation::Body => {
                body.get(&parameter.name).cloned().map(RawParameter::Json)
            }
        };
        let value = match raw {
            Some(RawParameter::Text(raw)) => coerce(parameter, &raw)?,
            Some(RawParameter::Json(value)) => coerce_json(parameter, value)?,
            None => {
                if let Some(default) = &parameter.default {
                    default.clone()
                } else if parameter.required {
                    return Err((
                        400,
                        format!(
                            "missing required {} parameter `{}` ({})",
                            parameter.location.as_str(),
                            parameter.name,
                            parameter.kind.as_str()
                        ),
                    ));
                } else {
                    continue;
                }
            }
        };
        if !parameter.choices.is_empty() {
            let allowed = parameter
                .choices
                .iter()
                .any(|choice| choice == value.as_str().unwrap_or_default());
            if !allowed {
                return Err((
                    400,
                    format!(
                        "parameter `{}` must be one of {}",
                        parameter.name,
                        parameter.choices.join(", ")
                    ),
                ));
            }
        }
        parameters.insert(parameter.name.clone(), value);
    }
    Ok(parameters)
}

enum RawParameter {
    Text(String),
    Json(Value),
}

fn coerce_json(parameter: &crate::Parameter, value: Value) -> Result<Value, (u16, String)> {
    match parameter.kind {
        crate::ParameterType::String => value.as_str().map(|text| json!(text)).ok_or_else(|| {
            (
                400,
                format!("parameter `{}` must be a string", parameter.name),
            )
        }),
        crate::ParameterType::Integer => {
            let integer = value
                .as_i64()
                .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
                .ok_or_else(|| {
                    (
                        400,
                        format!("parameter `{}` must be an integer", parameter.name),
                    )
                })?;
            check_range(parameter, integer as f64)?;
            Ok(json!(integer))
        }
        crate::ParameterType::Number => {
            let number = value
                .as_f64()
                .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
                .filter(|value: &f64| value.is_finite())
                .ok_or_else(|| {
                    (
                        400,
                        format!("parameter `{}` must be a finite number", parameter.name),
                    )
                })?;
            check_range(parameter, number)?;
            Ok(json!(number))
        }
        crate::ParameterType::Boolean => match &value {
            Value::Bool(value) => Ok(json!(value)),
            Value::String(text) => coerce(parameter, text),
            _ => Err((
                400,
                format!("parameter `{}` must be true or false", parameter.name),
            )),
        },
        crate::ParameterType::Object if value.is_object() => Ok(value),
        crate::ParameterType::Array if value.is_array() => Ok(value),
        crate::ParameterType::Object => Err((
            400,
            format!("parameter `{}` must be a JSON object", parameter.name),
        )),
        crate::ParameterType::Array => Err((
            400,
            format!("parameter `{}` must be a JSON array", parameter.name),
        )),
    }
}

fn coerce(parameter: &crate::Parameter, raw: &str) -> Result<Value, (u16, String)> {
    let value = match parameter.kind {
        crate::ParameterType::String => Value::String(raw.to_owned()),
        crate::ParameterType::Integer => {
            let value = raw.parse::<i64>().map_err(|_| {
                (
                    400,
                    format!("parameter `{}` must be an integer", parameter.name),
                )
            })?;
            check_range(parameter, value as f64)?;
            json!(value)
        }
        crate::ParameterType::Number => {
            let value = raw.parse::<f64>().map_err(|_| {
                (
                    400,
                    format!("parameter `{}` must be a number", parameter.name),
                )
            })?;
            if !value.is_finite() {
                return Err((
                    400,
                    format!("parameter `{}` must be finite", parameter.name),
                ));
            }
            check_range(parameter, value)?;
            json!(value)
        }
        crate::ParameterType::Boolean => match raw {
            "true" | "1" | "yes" | "on" => json!(true),
            "false" | "0" | "no" | "off" => json!(false),
            _ => {
                return Err((
                    400,
                    format!("parameter `{}` must be true or false", parameter.name),
                ));
            }
        },
        crate::ParameterType::Object => serde_json::from_str::<Value>(raw)
            .ok()
            .filter(Value::is_object)
            .ok_or_else(|| {
                (
                    400,
                    format!("parameter `{}` must be a JSON object", parameter.name),
                )
            })?,
        crate::ParameterType::Array => serde_json::from_str::<Value>(raw)
            .ok()
            .filter(Value::is_array)
            .ok_or_else(|| {
                (
                    400,
                    format!("parameter `{}` must be a JSON array", parameter.name),
                )
            })?,
    };
    Ok(value)
}

fn check_range(parameter: &crate::Parameter, value: f64) -> Result<(), (u16, String)> {
    if parameter.minimum.is_some_and(|minimum| value < minimum)
        || parameter.maximum.is_some_and(|maximum| value > maximum)
    {
        return Err((
            400,
            format!(
                "parameter `{}` is outside its allowed range",
                parameter.name
            ),
        ));
    }
    Ok(())
}

fn url_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let high = (bytes[index + 1] as char).to_digit(16)? as u8;
                let low = (bytes[index + 2] as char).to_digit(16)? as u8;
                decoded.push((high << 4) | low);
                index += 3;
            }
            b'%' => return None,
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(decoded).ok()
}

fn write_response(stream: &mut TcpStream, response: ControlResponse) -> std::io::Result<()> {
    let reason = match response.status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Content Too Large",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Error",
    };
    let body = if response.status == 204 {
        String::new()
    } else {
        response.body.to_string()
    };
    // CORS is opt-in. A wildcard would let any web page in the browser control
    // the local launcher, so the default is "no cross-origin access"; local CLI
    // and MCP clients are unaffected. Set FANPLAY_API_CORS=1 for a trusted
    // browser-based local tool.
    let cors = if std::env::var("FANPLAY_API_CORS").as_deref() == Ok("1") {
        "Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, PUT, PATCH, DELETE, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\n"
    } else {
        ""
    };
    write!(
        stream,
        "HTTP/1.1 {} {}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\n{cors}Connection: close\r\n\r\n{}",
        response.status,
        reason,
        body.len(),
        body
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Catalog, Endpoint, Scope};
    use serde_json::json;
    use std::sync::Mutex;

    struct Echo {
        seen: Mutex<Vec<Invocation>>,
    }

    impl ControlHandler for Echo {
        fn handle(&self, request: ControlRequest) -> ControlResponse {
            let invocation = request.invocation;
            self.seen.lock().unwrap().push(invocation.clone());
            if invocation.endpoint == "launcher.launch" {
                return ControlResponse::ok(json!({"queued": true}));
            }
            ControlResponse::ok(json!({
                "endpoint": invocation.endpoint,
                "parameters": invocation.parameters,
                "body": invocation.body,
            }))
        }

        fn dynamic_endpoints(&self) -> Vec<Endpoint> {
            vec![Endpoint::new(
                "example.dynamic",
                "POST",
                "/v1/example/{id}",
                "Dynamic example",
                "A route supplied at runtime.",
                Scope::Extensions,
            )
            .parameter(crate::Parameter::path(
                "id",
                crate::ParameterType::Integer,
                "Example id",
            ))
            .parameter(crate::Parameter::query(
                "flag",
                crate::ParameterType::Boolean,
                false,
                "Example flag",
            ))]
        }

        fn status(&self) -> Value {
            json!({"game_running": false})
        }
    }

    fn handler() -> Arc<Echo> {
        Arc::new(Echo {
            seen: Mutex::new(Vec::new()),
        })
    }

    fn request(port: u16, method: &str, target: &str, body: &str) -> (String, String) {
        use std::io::{Read, Write};
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let body = body.as_bytes();
        write!(
            stream,
            "{method} {target} HTTP/1.1\r\nHost: test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(body).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let (headers, body) = response.split_once("\r\n\r\n").unwrap();
        let status = headers.lines().next().unwrap().to_owned();
        (status, body.to_owned())
    }

    #[test]
    fn discovery_status_and_dynamic_routes_are_served() {
        let handler = handler();
        let server = start_with_catalog(0, Catalog::standard(), handler.clone()).unwrap();
        let port = server.port();
        let (status, body) = request(port, "GET", "/v1/discovery", "");
        assert!(status.contains(" 200 "));
        let discovery: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(discovery["server"], "fanplay-control");
        assert!(discovery["endpoints"]
            .as_array()
            .unwrap()
            .iter()
            .any(|endpoint| endpoint["id"] == "launcher.launch"));
        assert!(discovery["endpoints"]
            .as_array()
            .unwrap()
            .iter()
            .any(|endpoint| endpoint["id"] == "example.dynamic"));
        let (_, body) = request(port, "GET", "/v1/status", "");
        let status: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(status["mode"], "launcher");
        assert_eq!(status["game_running"], false);
        assert!(status["base_url"]
            .as_str()
            .unwrap()
            .contains(&port.to_string()));
    }

    #[test]
    fn parses_path_query_and_json_body_types() {
        let handler = handler();
        let server = start_with_catalog(0, Catalog::standard(), handler.clone()).unwrap();
        let port = server.port();
        let (status, body) = request(
            port,
            "POST",
            "/v1/example/7?flag=true",
            r#"{"nested":{"ok":true}}"#,
        );
        assert!(status.contains(" 200 "), "{body}");
        let seen = handler.seen.lock().unwrap();
        let invocation = seen.last().unwrap();
        assert_eq!(invocation.endpoint, "example.dynamic");
        assert_eq!(invocation.parameters["id"], json!(7));
        assert_eq!(invocation.parameters["flag"], json!(true));
        assert_eq!(invocation.body["nested"]["ok"], json!(true));
    }

    #[test]
    fn body_parameters_are_coerced_and_required_parameters_rejected() {
        let handler = handler();
        let server = start_with_catalog(0, Catalog::standard(), handler.clone()).unwrap();
        let port = server.port();
        let (status, body) = request(port, "POST", "/v1/launcher/pause", r#"{"paused":"true"}"#);
        assert!(status.contains(" 200 "), "{body}");
        assert_eq!(
            handler.seen.lock().unwrap().last().unwrap().parameters["paused"],
            json!(true)
        );
        let (status, body) = request(port, "POST", "/v1/launcher/pause", "{}");
        assert!(status.contains(" 400 "), "{body}");
        assert!(body.contains("paused"));
    }

    #[test]
    fn invalid_methods_and_paths_return_helpful_errors() {
        let handler = handler();
        let server = start_with_catalog(0, Catalog::standard(), handler.clone()).unwrap();
        let port = server.port();
        let (status, body) = request(port, "GET", "/v1/launcher/pause", "");
        assert!(status.contains(" 405 "), "{body}");
        assert!(body.contains("allowed"));
        let (status, body) = request(port, "GET", "/v1/nope", "");
        assert!(status.contains(" 404 "), "{body}");
        assert!(body.contains("/v1/discovery"));
    }

    #[test]
    fn accepts_expect_continue_for_bounded_json_bodies() {
        let handler = handler();
        let server = start_with_catalog(0, Catalog::standard(), handler.clone()).unwrap();
        let mut stream = TcpStream::connect(("127.0.0.1", server.port())).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let body = r#"{"paused":true}"#;
        write!(
            stream,
            "POST /v1/launcher/pause HTTP/1.1\r\nHost: test\r\nExpect: 100-continue\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        let mut interim = [0u8; 64];
        let read = stream.read(&mut interim).unwrap();
        assert!(String::from_utf8_lossy(&interim[..read]).contains("100 Continue"));
        stream.write_all(body.as_bytes()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.contains(" 200 "), "{response}");
        assert_eq!(
            handler.seen.lock().unwrap().last().unwrap().parameters["paused"],
            json!(true)
        );
    }

    #[test]
    fn every_standard_endpoint_has_a_matching_route() {
        let catalog = Catalog::standard();
        for endpoint in catalog.endpoints() {
            let path = endpoint
                .path
                .split('/')
                .map(|part| {
                    part.strip_prefix('{')
                        .and_then(|value| value.strip_suffix('}'))
                        .map(|_| "sample")
                        .unwrap_or(part)
                })
                .collect::<Vec<_>>()
                .join("/");
            assert!(
                catalog.match_path(&endpoint.method, &path).is_some(),
                "{} {}",
                endpoint.method,
                endpoint.path
            );
        }
    }
}
