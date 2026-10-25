// SPDX-License-Identifier: GPL-2.0-or-later
//! `rsxp-mcp`: a Model Context Protocol (MCP) stdio server that bridges MCP
//! clients to the live game HTTP API of a running desktop `rsxp`.
//!
//! The binary speaks newline-delimited JSON-RPC 2.0 on stdin/stdout and answers
//! the `initialize`, `ping`, `tools/list` and `tools/call` methods. Request
//! parsing (`Parser`, `Json`) and response encoding are implemented here so the
//! server needs no serialization dependency. `tools/call` maps the built-in
//! tools and dynamically discovered extension operations to REST requests;
//! `call_game` opens a short-lived loopback connection and unwraps the JSON
//! body (or its `error` field) into the MCP text result. Extension validation
//! and response construction stay in the extension's injected Ruby code.
//!
//! Environment: `RSXP_API_HOST` / `RSXP_API_PORT` select the game endpoint
//! (defaults `127.0.0.1` / `47821`).
//!
//! This binary is only built with the `api-docs` feature (see `Cargo.toml`).
//! With that feature, `--openapi` prints the shared control catalog as OpenAPI
//! 3.1 instead of starting the stdio loop.
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    net::TcpStream,
    time::Duration,
};

#[derive(Clone, Debug)]
enum Json {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Self::Object(fields) => fields.iter().find(|(name, _)| name == key).map(|(_, v)| v),
            _ => None,
        }
    }
    fn string(&self) -> Option<&str> {
        if let Self::String(value) = self {
            Some(value)
        } else {
            None
        }
    }
    #[allow(dead_code)]
    fn integer(&self) -> Option<i64> {
        match self {
            Self::Number(v) => v.parse().ok(),
            _ => None,
        }
    }
    fn encode(&self, out: &mut String) {
        match self {
            Self::Null => out.push_str("null"),
            Self::Bool(v) => out.push_str(if *v { "true" } else { "false" }),
            Self::Number(v) => out.push_str(v),
            Self::String(v) => quote(v, out),
            Self::Array(values) => {
                out.push('[');
                for (i, value) in values.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    value.encode(out);
                }
                out.push(']');
            }
            Self::Object(fields) => {
                out.push('{');
                for (i, (key, value)) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    quote(key, out);
                    out.push(':');
                    value.encode(out);
                }
                out.push('}');
            }
        }
    }
}

fn quote(value: &str, out: &mut String) {
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

struct Parser<'a> {
    input: &'a [u8],
    offset: usize,
}
impl<'a> Parser<'a> {
    fn parse(input: &'a str) -> Result<Json, String> {
        let mut parser = Self {
            input: input.as_bytes(),
            offset: 0,
        };
        let value = parser.value()?;
        parser.space();
        if parser.offset != parser.input.len() {
            return Err("trailing JSON data".into());
        }
        Ok(value)
    }
    fn space(&mut self) {
        while self
            .input
            .get(self.offset)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.offset += 1;
        }
    }
    fn value(&mut self) -> Result<Json, String> {
        self.space();
        match self.input.get(self.offset).copied() {
            Some(b'n') => {
                self.word(b"null")?;
                Ok(Json::Null)
            }
            Some(b't') => {
                self.word(b"true")?;
                Ok(Json::Bool(true))
            }
            Some(b'f') => {
                self.word(b"false")?;
                Ok(Json::Bool(false))
            }
            Some(b'"') => self.string().map(Json::String),
            Some(b'[') => self.array(),
            Some(b'{') => self.object(),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err("invalid JSON value".into()),
        }
    }
    fn word(&mut self, word: &[u8]) -> Result<(), String> {
        if self.input.get(self.offset..self.offset + word.len()) == Some(word) {
            self.offset += word.len();
            Ok(())
        } else {
            Err("invalid JSON token".into())
        }
    }
    fn string(&mut self) -> Result<String, String> {
        self.offset += 1;
        let mut output = String::new();
        while let Some(byte) = self.input.get(self.offset).copied() {
            self.offset += 1;
            match byte {
                b'"' => return Ok(output),
                b'\\' => {
                    let escaped = *self
                        .input
                        .get(self.offset)
                        .ok_or("incomplete JSON escape")?;
                    self.offset += 1;
                    match escaped {
                        b'"' => output.push('"'),
                        b'\\' => output.push('\\'),
                        b'/' => output.push('/'),
                        b'b' => output.push('\u{8}'),
                        b'f' => output.push('\u{c}'),
                        b'n' => output.push('\n'),
                        b'r' => output.push('\r'),
                        b't' => output.push('\t'),
                        b'u' => {
                            let end = self.offset + 4;
                            let digits = std::str::from_utf8(
                                self.input
                                    .get(self.offset..end)
                                    .ok_or("incomplete unicode escape")?,
                            )
                            .map_err(|_| "invalid unicode escape")?;
                            let code = u16::from_str_radix(digits, 16)
                                .map_err(|_| "invalid unicode escape")?;
                            self.offset = end;
                            let scalar = if (0xd800..=0xdbff).contains(&code) {
                                if self.input.get(self.offset..self.offset + 2) != Some(b"\\u") {
                                    return Err("incomplete unicode surrogate pair".into());
                                }
                                self.offset += 2;
                                let end = self.offset + 4;
                                let digits = std::str::from_utf8(
                                    self.input
                                        .get(self.offset..end)
                                        .ok_or("incomplete unicode escape")?,
                                )
                                .map_err(|_| "invalid unicode escape")?;
                                let low = u16::from_str_radix(digits, 16)
                                    .map_err(|_| "invalid unicode escape")?;
                                if !(0xdc00..=0xdfff).contains(&low) {
                                    return Err("invalid unicode surrogate pair".into());
                                }
                                self.offset = end;
                                let value = 0x10000
                                    + (((code as u32 - 0xd800) << 10) | (low as u32 - 0xdc00));
                                char::from_u32(value).ok_or("invalid unicode scalar")?
                            } else {
                                char::from_u32(code as u32).ok_or("invalid unicode scalar")?
                            };
                            output.push(scalar);
                        }
                        _ => return Err("invalid JSON escape".into()),
                    }
                }
                b if b < 0x20 => return Err("control character in JSON string".into()),
                b if b < 0x80 => output.push(b as char),
                _ => {
                    self.offset -= 1;
                    let remainder = std::str::from_utf8(&self.input[self.offset..])
                        .map_err(|_| "invalid UTF-8")?;
                    let c = remainder.chars().next().ok_or("incomplete UTF-8")?;
                    self.offset += c.len_utf8();
                    output.push(c);
                }
            }
        }
        Err("unterminated JSON string".into())
    }
    fn number(&mut self) -> Result<Json, String> {
        let start = self.offset;
        if self.input.get(self.offset) == Some(&b'-') {
            self.offset += 1;
        }
        match self.input.get(self.offset) {
            Some(b'0') => {
                self.offset += 1;
                if self.input.get(self.offset).is_some_and(u8::is_ascii_digit) {
                    return Err("leading zero in JSON number".into());
                }
            }
            Some(b'1'..=b'9') => {
                self.offset += 1;
                while self.input.get(self.offset).is_some_and(u8::is_ascii_digit) {
                    self.offset += 1;
                }
            }
            _ => return Err("invalid JSON number".into()),
        }
        if self.input.get(self.offset) == Some(&b'.') {
            self.offset += 1;
            let fraction = self.offset;
            while self.input.get(self.offset).is_some_and(u8::is_ascii_digit) {
                self.offset += 1;
            }
            if self.offset == fraction {
                return Err("missing JSON fraction digits".into());
            }
        }
        if self
            .input
            .get(self.offset)
            .is_some_and(|b| matches!(b, b'e' | b'E'))
        {
            self.offset += 1;
            if self
                .input
                .get(self.offset)
                .is_some_and(|b| matches!(b, b'+' | b'-'))
            {
                self.offset += 1;
            }
            let exponent = self.offset;
            while self.input.get(self.offset).is_some_and(u8::is_ascii_digit) {
                self.offset += 1;
            }
            if self.offset == exponent {
                return Err("missing JSON exponent digits".into());
            }
        }
        let number =
            std::str::from_utf8(&self.input[start..self.offset]).map_err(|_| "invalid number")?;
        Ok(Json::Number(number.to_string()))
    }
    fn array(&mut self) -> Result<Json, String> {
        self.offset += 1;
        self.space();
        let mut values = Vec::new();
        if self.input.get(self.offset) == Some(&b']') {
            self.offset += 1;
            return Ok(Json::Array(values));
        }
        loop {
            values.push(self.value()?);
            self.space();
            match self.input.get(self.offset) {
                Some(b',') => self.offset += 1,
                Some(b']') => {
                    self.offset += 1;
                    break;
                }
                _ => return Err("expected , or ]".into()),
            }
        }
        Ok(Json::Array(values))
    }
    fn object(&mut self) -> Result<Json, String> {
        self.offset += 1;
        self.space();
        let mut fields = Vec::new();
        if self.input.get(self.offset) == Some(&b'}') {
            self.offset += 1;
            return Ok(Json::Object(fields));
        }
        loop {
            self.space();
            if self.input.get(self.offset) != Some(&b'"') {
                return Err("expected JSON object key".into());
            }
            let key = self.string()?;
            self.space();
            if self.input.get(self.offset) != Some(&b':') {
                return Err("expected :".into());
            }
            self.offset += 1;
            let value = self.value()?;
            fields.push((key, value));
            self.space();
            match self.input.get(self.offset) {
                Some(b',') => self.offset += 1,
                Some(b'}') => {
                    self.offset += 1;
                    break;
                }
                _ => return Err("expected , or }".into()),
            }
        }
        Ok(Json::Object(fields))
    }
}

fn json(value: &Json) -> String {
    let mut out = String::new();
    value.encode(&mut out);
    out
}
fn response(id: &Json, result: &str) -> String {
    let mut out = format!("{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":", json(id));
    out.push_str(result);
    out.push('}');
    out
}
fn error(id: &Json, code: i64, message: &str) -> String {
    let mut out = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{},\"error\":{{\"code\":{code},\"message\":",
        json(id)
    );
    quote(message, &mut out);
    out.push_str("}}");
    out
}
fn text_result(text: &str, is_error: bool) -> String {
    let mut out = String::from("{\"content\":[{\"type\":\"text\",\"text\":");
    quote(text, &mut out);
    out.push_str("}]");
    if is_error {
        out.push_str(",\"isError\":true");
    }
    out.push('}');
    out
}

#[derive(Clone, Debug)]
struct ExtensionRoute {
    method: String,
    path: String,
    operation: String,
    arguments: Vec<String>,
}

#[derive(Clone, Debug)]
struct ApiParameter {
    name: String,
    location: String,
    kind: String,
    required: bool,
    description: String,
    choices: Vec<String>,
}

#[derive(Clone, Debug)]
struct ApiEndpoint {
    id: String,
    method: String,
    path: String,
    summary: String,
    description: String,
    parameters: Vec<ApiParameter>,
}

fn discovery_endpoints() -> Result<Vec<ApiEndpoint>, String> {
    let value = Parser::parse(&call_game("/v1/discovery")?)?;
    let endpoints = value
        .get("endpoints")
        .and_then(|value| match value {
            Json::Array(endpoints) => Some(endpoints),
            _ => None,
        })
        .ok_or("control discovery returned no endpoints")?;
    endpoints
        .iter()
        .map(|endpoint| {
            let parameters = endpoint
                .get("parameters")
                .and_then(|value| match value {
                    Json::Array(parameters) => Some(parameters),
                    _ => None,
                })
                .map_or(&[][..], Vec::as_slice)
                .iter()
                .filter_map(|parameter| {
                    Some(ApiParameter {
                        name: parameter.get("name")?.string()?.to_owned(),
                        location: parameter
                            .get("in")
                            .and_then(Json::string)
                            .unwrap_or("query")
                            .to_owned(),
                        kind: parameter
                            .get("type")
                            .and_then(Json::string)
                            .unwrap_or("string")
                            .to_owned(),
                        required: matches!(parameter.get("required"), Some(Json::Bool(true))),
                        description: parameter
                            .get("description")
                            .and_then(Json::string)
                            .unwrap_or("")
                            .to_owned(),
                        choices: parameter
                            .get("enum")
                            .and_then(|value| match value {
                                Json::Array(values) => Some(values),
                                _ => None,
                            })
                            .map_or(&[][..], Vec::as_slice)
                            .iter()
                            .filter_map(Json::string)
                            .map(str::to_owned)
                            .collect(),
                    })
                })
                .collect();
            Ok(ApiEndpoint {
                id: endpoint
                    .get("id")
                    .and_then(Json::string)
                    .ok_or("endpoint has no id")?
                    .to_owned(),
                method: endpoint
                    .get("method")
                    .and_then(Json::string)
                    .ok_or("endpoint has no method")?
                    .to_owned(),
                path: endpoint
                    .get("path")
                    .and_then(Json::string)
                    .ok_or("endpoint has no path")?
                    .to_owned(),
                summary: endpoint
                    .get("summary")
                    .and_then(Json::string)
                    .unwrap_or("")
                    .to_owned(),
                description: endpoint
                    .get("description")
                    .and_then(Json::string)
                    .unwrap_or("")
                    .to_owned(),
                parameters,
            })
        })
        .collect()
}

fn tool_name(id: &str) -> String {
    id.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn tools_from_discovery(endpoints: &[ApiEndpoint]) -> String {
    let mut tools = Vec::new();
    for endpoint in endpoints {
        let mut properties = Vec::new();
        let mut required = Vec::new();
        for parameter in &endpoint.parameters {
            let mut schema = vec![("type".into(), Json::String(parameter.kind.clone()))];
            if !parameter.description.is_empty() {
                schema.push((
                    "description".into(),
                    Json::String(parameter.description.clone()),
                ));
            }
            if !parameter.choices.is_empty() {
                schema.push((
                    "enum".into(),
                    Json::Array(
                        parameter
                            .choices
                            .iter()
                            .map(|choice| Json::String(choice.clone()))
                            .collect(),
                    ),
                ));
            }
            properties.push((parameter.name.clone(), Json::Object(schema)));
            if parameter.required {
                required.push(Json::String(parameter.name.clone()));
            }
        }
        let description = if endpoint.description.is_empty() {
            endpoint.summary.clone()
        } else {
            format!("{} {}", endpoint.summary, endpoint.description)
        };
        tools.push(Json::Object(vec![
            ("name".into(), Json::String(tool_name(&endpoint.id))),
            ("description".into(), Json::String(description)),
            (
                "inputSchema".into(),
                Json::Object(vec![
                    ("type".into(), Json::String("object".into())),
                    ("properties".into(), Json::Object(properties)),
                    ("required".into(), Json::Array(required)),
                    ("additionalProperties".into(), Json::Bool(false)),
                ]),
            ),
        ]));
    }
    json(&Json::Object(vec![("tools".into(), Json::Array(tools))]))
}

fn call_discovered_tool(name: &str, args: &Json) -> Result<String, String> {
    let endpoints = discovery_endpoints()?;
    let endpoint = endpoints
        .iter()
        .find(|endpoint| endpoint.id == name || tool_name(&endpoint.id) == name)
        .ok_or_else(|| format!("unknown control operation: {name}"))?;
    let mut path = String::new();
    for (index, segment) in endpoint.path.split('/').enumerate() {
        if index > 0 {
            path.push('/');
        }
        if let Some(parameter) = segment
            .strip_prefix('{')
            .and_then(|value| value.strip_suffix('}'))
        {
            let value = args
                .get(parameter)
                .ok_or_else(|| format!("missing required argument: {parameter}"))?;
            path.push_str(&url_encode(&json_argument(value)?));
        } else {
            path.push_str(segment);
        }
    }
    let mut first = true;
    let mut body = Vec::new();
    for parameter in &endpoint.parameters {
        let Some(value) = args.get(&parameter.name) else {
            if parameter.required && parameter.location == "body" {
                return Err(format!("missing required argument: {}", parameter.name));
            }
            continue;
        };
        match parameter.location.as_str() {
            "path" => {}
            "body" => body.push((parameter.name.clone(), value.clone())),
            _ => {
                path.push(if first { '?' } else { '&' });
                first = false;
                path.push_str(&url_encode(&parameter.name));
                path.push('=');
                path.push_str(&url_encode(&json_argument(value)?));
            }
        }
    }
    let body = if body.is_empty() {
        None
    } else {
        Some(Json::Object(body))
    };
    call_game_json(&endpoint.method, &path, body.as_ref())
}

fn extension_routes() -> Result<Vec<ExtensionRoute>, String> {
    let value = Parser::parse(&call_game("/v1/extensions/routes")?)?;
    let routes = value
        .get("routes")
        .and_then(|value| match value {
            Json::Array(routes) => Some(routes),
            _ => None,
        })
        .ok_or("extension route discovery returned no routes")?;
    routes
        .iter()
        .map(|route| {
            let arguments = route
                .get("arguments")
                .and_then(|value| match value {
                    Json::Array(arguments) => Some(arguments),
                    _ => None,
                })
                .map_or(&[][..], Vec::as_slice)
                .iter()
                .filter_map(Json::string)
                .map(str::to_owned)
                .collect();
            Ok(ExtensionRoute {
                method: route
                    .get("method")
                    .and_then(Json::string)
                    .ok_or("extension route has no method")?
                    .to_owned(),
                path: route
                    .get("path")
                    .and_then(Json::string)
                    .ok_or("extension route has no path")?
                    .to_owned(),
                operation: route
                    .get("operation")
                    .and_then(Json::string)
                    .ok_or("extension route has no operation")?
                    .to_owned(),
                arguments,
            })
        })
        .collect()
}

fn tools_from_routes(routes: impl IntoIterator<Item = ExtensionRoute>) -> String {
    let mut listing = Json::Object(vec![("tools".into(), Json::Array(Vec::new()))]);
    let Json::Object(fields) = &mut listing else {
        return "{\"tools\":[]}".into();
    };
    let Some((_, Json::Array(tools))) = fields.iter_mut().find(|(key, _)| key == "tools") else {
        return "{\"tools\":[]}".into();
    };
    let mut seen = std::collections::HashSet::new();
    for route in routes {
        if !seen.insert(route.operation.clone())
            || tools.iter().any(|tool| {
                tool.get("name").and_then(Json::string) == Some(route.operation.as_str())
            })
        {
            continue;
        }
        let properties = route
            .arguments
            .iter()
            .map(|argument| {
                (
                    argument.clone(),
                    Json::Object(vec![("type".into(), Json::String("string".into()))]),
                )
            })
            .collect();
        tools.push(Json::Object(vec![
            ("name".into(), Json::String(route.operation.clone())),
            (
                "description".into(),
                Json::String(format!("Extension operation {}.", route.operation)),
            ),
            (
                "inputSchema".into(),
                Json::Object(vec![
                    ("type".into(), Json::String("object".into())),
                    ("properties".into(), Json::Object(properties)),
                    ("additionalProperties".into(), Json::Bool(false)),
                ]),
            ),
        ]));
    }
    json(&listing)
}

fn tools() -> String {
    match discovery_endpoints() {
        Ok(endpoints) if !endpoints.is_empty() => tools_from_discovery(&endpoints),
        _ => tools_from_routes(extension_routes().unwrap_or_default()),
    }
}

fn url_encode(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push(HEX[(byte >> 4) as usize] as char);
            encoded.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    encoded
}

fn extension_route_target(route: &ExtensionRoute, args: &Json) -> Result<String, String> {
    let mut path = String::new();
    for (index, segment) in route.path.split('/').enumerate() {
        if index > 0 {
            path.push('/');
        }
        if let Some(name) = segment
            .strip_prefix('{')
            .and_then(|value| value.strip_suffix('}'))
        {
            let value = args
                .get(name)
                .ok_or_else(|| format!("missing extension argument: {name}"))?;
            path.push_str(&url_encode(&json_argument(value)?));
        } else {
            path.push_str(segment);
        }
    }
    let mut first = true;
    for argument in &route.arguments {
        if route.path.contains(&format!("{{{argument}}}")) {
            continue;
        }
        let Some(value) = args.get(argument) else {
            continue;
        };
        path.push(if first { '?' } else { '&' });
        first = false;
        path.push_str(&url_encode(argument));
        path.push('=');
        path.push_str(&url_encode(&json_argument(value)?));
    }
    Ok(path)
}

fn json_argument(value: &Json) -> Result<String, String> {
    match value {
        Json::String(value) => Ok(value.clone()),
        Json::Bool(value) => Ok(value.to_string()),
        Json::Number(value) => Ok(value.clone()),
        _ => Err("extension arguments must be strings, booleans, or numbers".into()),
    }
}

fn call_extension_tool(name: &str, args: &Json) -> Result<String, String> {
    let routes = extension_routes()?;
    let mut candidates = routes.into_iter().filter(|route| route.operation == name);
    let mut selected = None;
    while let Some(route) = candidates.next() {
        if route
            .path
            .split('/')
            .filter_map(|segment| segment.strip_prefix('{').and_then(|v| v.strip_suffix('}')))
            .any(|name| args.get(name).is_none())
        {
            continue;
        }
        let score = route
            .arguments
            .iter()
            .filter(|argument| args.get(argument).is_some())
            .count();
        if selected
            .as_ref()
            .is_none_or(|(_, previous_score): &(ExtensionRoute, usize)| score > *previous_score)
        {
            selected = Some((route, score));
        }
    }
    let (route, _) = selected.ok_or_else(|| format!("unknown extension tool: {name}"))?;
    let target = extension_route_target(&route, args)?;
    call_game_with_method(&route.method, &target)
}

fn call_game(path: &str) -> Result<String, String> {
    call_game_with_method("GET", path)
}

fn call_game_with_method(method: &str, path: &str) -> Result<String, String> {
    call_game_json(method, path, None)
}

fn call_game_json(method: &str, path: &str, body: Option<&Json>) -> Result<String, String> {
    let host = std::env::var("RSXP_API_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port = std::env::var("RSXP_API_PORT").unwrap_or_else(|_| "47821".into());
    let mut stream = TcpStream::connect(format!("{host}:{port}"))
        .map_err(|e| format!("cannot connect to game API at {host}:{port}: {e}"))?;
    let timeout = Some(Duration::from_secs(10));
    stream
        .set_read_timeout(timeout)
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(timeout)
        .map_err(|e| e.to_string())?;
    let body = body.map(json).unwrap_or_default();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .map_err(|e| e.to_string())?;
    let mut response = String::new();
    BufReader::new(stream)
        .read_to_string(&mut response)
        .map_err(|e| format!("game API read failed: {e}"))?;
    let (headers, body) = response
        .split_once("\r\n\r\n")
        .ok_or("invalid HTTP response from game API")?;
    let status = headers.lines().next().unwrap_or_default();
    if !status.contains(" 200 ") && !status.contains(" 204 ") {
        let value = Parser::parse(body)?;
        return Err(value
            .get("error")
            .and_then(Json::string)
            .unwrap_or(status)
            .to_string());
    }
    let value = Parser::parse(body)?;
    if let Some(message) = value.get("error").and_then(Json::string) {
        return Err(message.to_string());
    }
    Ok(body.to_string())
}

fn handle(message: Json) -> Option<String> {
    let id = message.get("id")?;
    let method = message.get("method").and_then(Json::string).unwrap_or("");
    let params = message.get("params").unwrap_or(&Json::Null);
    match method {
        "initialize" => {
            let requested = params
                .get("protocolVersion")
                .and_then(Json::string)
                .unwrap_or("");
            let version = if ["2024-11-05", "2025-03-26", "2025-06-18"].contains(&requested) {
                requested
            } else {
                "2025-06-18"
            };
            let result = format!(
                "{{\"protocolVersion\":{},\"capabilities\":{{\"tools\":{{\"listChanged\":false}}}},\"serverInfo\":{{\"name\":\"rsxp-game-api\",\"version\":\"0.1.0\"}}}}",
                { let mut encoded = String::new(); quote(version, &mut encoded); encoded }
            );
            Some(response(id, &result))
        }
        "ping" => Some(response(id, "{}")),
        "tools/list" => {
            let listing = tools();
            Some(response(id, &listing))
        }
        "tools/call" => {
            let name = params.get("name").and_then(Json::string).unwrap_or("");
            let args_obj = params.get("arguments").unwrap_or(&Json::Null);
            let result = call_discovered_tool(name, args_obj)
                .or_else(|_| call_extension_tool(name, args_obj));
            let (content, failed) = match result {
                Ok(raw) => match Parser::parse(&raw) {
                    Ok(value) => (json(&value), false),
                    Err(_) => (raw, false),
                },
                Err(message) => (message, true),
            };
            Some(response(id, &text_result(&content, failed)))
        }
        _ => Some(error(id, -32601, &format!("Method not found: {method}"))),
    }
}

fn main() -> io::Result<()> {
    #[cfg(feature = "api-docs")]
    if std::env::args().any(|arg| arg == "--openapi") {
        let status = serde_json::json!({
            "base_url": "http://127.0.0.1:47821",
            "mode": "launcher",
            "game_running": false,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&rsxp_control::Catalog::standard().openapi(status))
                .map_err(io::Error::other)?
        );
        return Ok(());
    }
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match Parser::parse(&line) {
            Ok(message) => handle(message),
            Err(message) => Some(error(
                &Json::Null,
                -32700,
                &format!("Parse error: {message}"),
            )),
        };
        if let Some(reply) = reply {
            writeln!(stdout, "{reply}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_tool_names_are_mcp_safe() {
        assert_eq!(tool_name("launcher.launch"), "launcher_launch");
        assert_eq!(tool_name("game.press"), "game_press");
        assert_eq!(tool_name("press_key"), "press_key");
    }

    #[test]
    fn parses_nested_requests_and_supplementary_unicode() {
        let value = Parser::parse(
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"press_key","arguments":{"key":"up","label":"\ud83d\ude80"}}}"#,
        )
        .unwrap();
        assert_eq!(value.get("id").and_then(Json::integer), Some(7));
        assert_eq!(
            value
                .get("params")
                .and_then(|v| v.get("arguments"))
                .and_then(|v| v.get("label"))
                .and_then(Json::string),
            Some("🚀")
        );
    }

    #[test]
    fn rejects_malformed_numbers_and_unpaired_surrogates() {
        assert!(Parser::parse(r#"{"id":01}"#).is_err());
        assert!(Parser::parse(r#"{"label":"\ud83d"}"#).is_err());
    }

    #[test]
    fn extension_tools_are_advertised_from_plugin_routes() {
        let listing = Parser::parse(&tools_from_routes([
            ExtensionRoute {
                method: "GET".into(),
                path: "/v1/game/status".into(),
                operation: "game_status".into(),
                arguments: vec![],
            },
            ExtensionRoute {
                method: "POST".into(),
                path: "/v1/game/teleport".into(),
                operation: "teleport".into(),
                arguments: vec!["map_id".into(), "x".into(), "y".into()],
            },
        ]))
        .unwrap();
        let names = listing
            .get("tools")
            .and_then(|value| match value {
                Json::Array(values) => Some(values),
                _ => None,
            })
            .unwrap();
        assert!(names
            .iter()
            .any(|tool| tool.get("name").and_then(Json::string) == Some("game_status")));
        assert!(names
            .iter()
            .any(|tool| tool.get("name").and_then(Json::string) == Some("teleport")));
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn extension_route_targets_only_encode_and_forward_arguments() {
        let route = ExtensionRoute {
            method: "POST".into(),
            path: "/v1/example/{species}".into(),
            operation: "example".into(),
            arguments: vec!["species".into(), "level".into(), "shiny".into()],
        };
        let args = Parser::parse(r#"{"species":"PIKA CHU","level":30,"shiny":true}"#).unwrap();
        assert_eq!(
            extension_route_target(&route, &args).unwrap(),
            "/v1/example/PIKA%20CHU?level=30&shiny=true"
        );
    }

    #[cfg(feature = "api-docs")]
    #[test]
    fn generated_openapi_lists_the_unified_control_catalog() {
        let spec = rsxp_control::Catalog::standard()
            .openapi(serde_json::json!({"base_url": "http://127.0.0.1:47821"}))
            .to_string();
        Parser::parse(&spec).expect("generated OpenAPI should be valid JSON");
        assert!(spec.contains("/v1/launcher/launch"));
        assert!(spec.contains("/v1/saves/backup"));
        assert!(spec.contains("/v1/game/status"));
        assert!(spec.contains("/v1/extensions/routes"));
    }
}
