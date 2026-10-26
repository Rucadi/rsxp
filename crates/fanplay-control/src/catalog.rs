// SPDX-License-Identifier: GPL-2.0-or-later
//! Typed endpoint catalog shared by every FANPLAY control transport.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Major/minor version of the control contract. Additive changes keep the
/// major version; removing or repurposing an endpoint increments it.
pub const API_VERSION: &str = "1.0";
/// Stable default loopback port used by both the launcher and the player.
pub const DEFAULT_PORT: u16 = 47_821;

/// Which part of the application owns an endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Server,
    Settings,
    Launcher,
    Catalogs,
    Saves,
    Game,
    Extensions,
}

impl Scope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::Settings => "settings",
            Self::Launcher => "launcher",
            Self::Catalogs => "catalogs",
            Self::Saves => "saves",
            Self::Game => "game",
            Self::Extensions => "extensions",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParameterType {
    String,
    Integer,
    Number,
    Boolean,
    Object,
    Array,
}

impl ParameterType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Integer => "integer",
            Self::Number => "number",
            Self::Boolean => "boolean",
            Self::Object => "object",
            Self::Array => "array",
        }
    }

    fn schema(self) -> Value {
        json!({ "type": self.as_str() })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParameterLocation {
    Path,
    Query,
    Body,
}

impl ParameterLocation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Path => "path",
            Self::Query => "query",
            Self::Body => "body",
        }
    }
}

/// One named operation argument. `type` and `in` are serialized with the names
/// used by OpenAPI and by the discovery document, so a client does not need an
/// adapter-specific decoder.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    #[serde(rename = "in")]
    pub location: ParameterLocation,
    #[serde(rename = "type")]
    pub kind: ParameterType,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, rename = "enum", skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum: Option<f64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub example: String,
}

impl Parameter {
    pub fn new(
        name: impl Into<String>,
        location: ParameterLocation,
        kind: ParameterType,
        required: bool,
        description: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            location,
            kind,
            required,
            default: None,
            description: description.into(),
            choices: Vec::new(),
            minimum: None,
            maximum: None,
            example: String::new(),
        }
    }

    pub fn path(
        name: impl Into<String>,
        kind: ParameterType,
        description: impl Into<String>,
    ) -> Self {
        Self::new(name, ParameterLocation::Path, kind, true, description)
    }

    pub fn query(
        name: impl Into<String>,
        kind: ParameterType,
        required: bool,
        description: impl Into<String>,
    ) -> Self {
        Self::new(name, ParameterLocation::Query, kind, required, description)
    }

    pub fn body(
        name: impl Into<String>,
        kind: ParameterType,
        required: bool,
        description: impl Into<String>,
    ) -> Self {
        Self::new(name, ParameterLocation::Body, kind, required, description)
    }

    pub fn default(mut self, value: impl Into<Value>) -> Self {
        self.default = Some(value.into());
        self
    }

    pub fn choices(mut self, values: &[&str]) -> Self {
        self.choices = values.iter().map(|value| (*value).to_owned()).collect();
        self
    }

    pub fn range(mut self, minimum: f64, maximum: f64) -> Self {
        self.minimum = Some(minimum);
        self.maximum = Some(maximum);
        self
    }

    pub fn example(mut self, value: impl Into<String>) -> Self {
        self.example = value.into();
        self
    }

    fn schema(&self) -> Value {
        let mut schema = self.kind.schema();
        if let Some(object) = schema.as_object_mut() {
            if !self.choices.is_empty() {
                object.insert("enum".into(), json!(self.choices));
            }
            if let Some(minimum) = self.minimum {
                object.insert("minimum".into(), json!(minimum));
            }
            if let Some(maximum) = self.maximum {
                object.insert("maximum".into(), json!(maximum));
            }
            if let Some(default) = &self.default {
                object.insert("default".into(), default.clone());
            }
            if !self.description.is_empty() {
                object.insert("description".into(), json!(self.description));
            }
        }
        schema
    }
}

/// One method/path operation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Endpoint {
    pub id: String,
    pub method: String,
    pub path: String,
    pub summary: String,
    pub description: String,
    pub scope: Scope,
    #[serde(default)]
    pub mutates_state: bool,
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    #[serde(default = "empty_object")]
    pub request: Value,
    #[serde(default = "empty_object")]
    pub response: Value,
}

fn empty_object() -> Value {
    json!({ "type": "object" })
}

impl Endpoint {
    pub fn new(
        id: impl Into<String>,
        method: impl Into<String>,
        path: impl Into<String>,
        summary: impl Into<String>,
        description: impl Into<String>,
        scope: Scope,
    ) -> Self {
        Self {
            id: id.into(),
            method: method.into().to_ascii_uppercase(),
            path: path.into(),
            summary: summary.into(),
            description: description.into(),
            scope,
            mutates_state: false,
            parameters: Vec::new(),
            request: empty_object(),
            response: empty_object(),
        }
    }

    pub fn parameter(mut self, parameter: Parameter) -> Self {
        self.parameters.push(parameter);
        self
    }

    pub fn mutates(mut self) -> Self {
        self.mutates_state = true;
        self
    }

    pub fn request(mut self, schema: Value) -> Self {
        self.request = schema;
        self
    }

    pub fn response(mut self, schema: Value) -> Self {
        self.response = schema;
        self
    }

    pub fn path_parameters(&self) -> impl Iterator<Item = &Parameter> {
        self.parameters
            .iter()
            .filter(|parameter| parameter.location == ParameterLocation::Path)
    }

    pub fn query_parameters(&self) -> impl Iterator<Item = &Parameter> {
        self.parameters
            .iter()
            .filter(|parameter| parameter.location == ParameterLocation::Query)
    }

    /// A copy/pasteable `curl` command for this endpoint.
    pub fn curl(&self, base_url: &str) -> String {
        let mut path = self.path.clone();
        let mut query = Vec::new();
        for parameter in &self.parameters {
            match parameter.location {
                ParameterLocation::Path => {
                    let value = if parameter.example.is_empty() {
                        placeholder(parameter)
                    } else {
                        parameter.example.clone()
                    };
                    path = path.replace(&format!("{{{}}}", parameter.name), &value);
                }
                ParameterLocation::Query if parameter.required => {
                    let value = if parameter.example.is_empty() {
                        placeholder(parameter)
                    } else {
                        parameter.example.clone()
                    };
                    query.push(format!("{}={}", parameter.name, value));
                }
                _ => {}
            }
        }
        if !query.is_empty() {
            path.push('?');
            path.push_str(&query.join("&"));
        }
        if self.mutates_state || matches!(self.method.as_str(), "POST" | "PUT" | "PATCH" | "DELETE")
        {
            let example = self
                .parameters
                .iter()
                .filter(|parameter| parameter.location == ParameterLocation::Body)
                .map(|parameter| {
                    (
                        parameter.name.clone(),
                        parameter
                            .default
                            .clone()
                            .unwrap_or_else(|| placeholder_value(parameter)),
                    )
                })
                .collect::<serde_json::Map<_, _>>();
            format!(
                "curl -s -X {} '{}{}' -H 'content-type: application/json' -d '{}'",
                self.method,
                base_url,
                path,
                Value::Object(example)
            )
        } else {
            format!("curl -s '{}{}'", base_url, path)
        }
    }

    fn openapi_operation(&self, base_url: &str) -> Value {
        let mut parameters = Vec::new();
        for parameter in &self.parameters {
            if parameter.location == ParameterLocation::Body {
                continue;
            }
            let mut value = json!({
                "name": parameter.name,
                "in": parameter.location.as_str(),
                "required": parameter.required,
                "schema": parameter.schema(),
            });
            if !parameter.description.is_empty()
                && let Some(object) = value.as_object_mut()
            {
                object.insert("description".into(), json!(parameter.description));
            }
            parameters.push(value);
        }
        let mut operation = json!({
            "operationId": self.id.replace('.', "_"),
            "summary": self.summary,
            "description": self.description,
            "tags": [self.scope.as_str()],
            "responses": {
                "200": {
                    "description": "Successful response",
                    "content": { "application/json": { "schema": self.response } }
                },
                "400": { "description": "Invalid request" },
                "404": { "description": "Unknown endpoint" },
                "503": { "description": "Operation unavailable in the current host state" }
            },
            "x-fanplay-curl": self.curl(base_url),
        });
        if !parameters.is_empty()
            && let Some(object) = operation.as_object_mut()
        {
            object.insert("parameters".into(), Value::Array(parameters));
        }
        let body_parameters = self
            .parameters
            .iter()
            .filter(|parameter| parameter.location == ParameterLocation::Body)
            .collect::<Vec<_>>();
        if !body_parameters.is_empty()
            && let Some(object) = operation.as_object_mut()
        {
            let properties = body_parameters
                .iter()
                .map(|parameter| (parameter.name.clone(), parameter.schema()))
                .collect::<serde_json::Map<_, _>>();
            let required = body_parameters
                .iter()
                .filter(|parameter| parameter.required)
                .map(|parameter| json!(parameter.name))
                .collect::<Vec<_>>();
            object.insert(
                "requestBody".into(),
                json!({
                    "required": !required.is_empty(),
                    "content": {
                        "application/json": {
                            "schema": {
                                "type": "object",
                                "properties": properties,
                                "required": required,
                            }
                        }
                    }
                }),
            );
        } else if self.request != empty_object()
            && let Some(object) = operation.as_object_mut()
        {
            object.insert(
                "requestBody".into(),
                json!({
                    "required": self.mutates_state,
                    "content": { "application/json": { "schema": self.request } }
                }),
            );
        }
        operation
    }
}

fn placeholder(parameter: &Parameter) -> String {
    match parameter.kind {
        ParameterType::Integer | ParameterType::Number => "1".into(),
        ParameterType::Boolean => "true".into(),
        ParameterType::String => format!("<{}>", parameter.name),
        ParameterType::Object | ParameterType::Array => "{}".into(),
    }
}

fn placeholder_value(parameter: &Parameter) -> Value {
    match parameter.kind {
        ParameterType::Integer => json!(1),
        ParameterType::Number => json!(1.0),
        ParameterType::Boolean => json!(true),
        ParameterType::String => json!(format!("<{}>", parameter.name)),
        ParameterType::Object => json!({}),
        ParameterType::Array => json!([]),
    }
}

/// Deterministic path match, including decoded path parameters.
#[derive(Clone, Debug, PartialEq)]
pub struct EndpointMatch {
    pub endpoint: Endpoint,
    pub path_parameters: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    endpoints: Vec<Endpoint>,
}

impl Catalog {
    pub fn new() -> Self {
        Self::default()
    }

    /// The built-in launcher, player, settings, catalog and save contract.
    pub fn standard() -> Self {
        let mut catalog = Self::new();
        catalog.extend(server_endpoints());
        catalog.extend(settings_endpoints());
        catalog.extend(launcher_endpoints());
        catalog.extend(catalog_endpoints());
        catalog.extend(save_endpoints());
        catalog.extend(game_endpoints());
        catalog
    }

    pub fn add(&mut self, endpoint: Endpoint) {
        self.endpoints.push(endpoint);
    }

    pub fn extend(&mut self, endpoints: impl IntoIterator<Item = Endpoint>) {
        self.endpoints.extend(endpoints);
    }

    /// Merges dynamic endpoints (for example Rhai route aliases). A dynamic
    /// endpoint replaces a built-in with the same method and path so a plugin
    /// can document a route more precisely than the generic contract.
    pub fn merge_dynamic(&mut self, endpoints: impl IntoIterator<Item = Endpoint>) {
        for endpoint in endpoints {
            let path = normalize_path(&endpoint.path);
            if let Some(existing) = self.endpoints.iter_mut().find(|candidate| {
                candidate.method.eq_ignore_ascii_case(&endpoint.method)
                    && normalize_path(&candidate.path) == path
            }) {
                *existing = endpoint;
            } else {
                self.endpoints.push(endpoint);
            }
        }
    }

    pub fn endpoints(&self) -> &[Endpoint] {
        &self.endpoints
    }

    pub fn get(&self, id: &str) -> Option<&Endpoint> {
        self.endpoints.iter().find(|endpoint| endpoint.id == id)
    }

    pub fn remove_scope(&mut self, scope: Scope) {
        self.endpoints.retain(|endpoint| endpoint.scope != scope);
    }

    pub fn retain(&mut self, mut keep: impl FnMut(&Endpoint) -> bool) {
        self.endpoints.retain(|endpoint| keep(endpoint));
    }

    /// Matches a method and URL path against the catalog.
    ///
    /// Path matching is case-sensitive and treats `{name}` segments as single
    /// non-empty path components. The returned path parameters are already
    /// percent-decoded by the server before this is called.
    pub fn match_path(&self, method: &str, path: &str) -> Option<EndpointMatch> {
        let path = normalize_path(path);
        self.endpoints.iter().find_map(|endpoint| {
            if !endpoint.method.eq_ignore_ascii_case(method) {
                return None;
            }
            match_template(&endpoint.path, &path).map(|parameters| EndpointMatch {
                endpoint: endpoint.clone(),
                path_parameters: parameters,
            })
        })
    }

    /// Discovery document consumed by CLI tools, MCP clients, and AI agents.
    pub fn discovery(&self, status: Value) -> Value {
        let base_url = status
            .get("base_url")
            .and_then(Value::as_str)
            .unwrap_or("http://127.0.0.1:47821");
        let endpoints = self
            .endpoints
            .iter()
            .map(|endpoint| {
                json!({
                    "id": endpoint.id,
                    "method": endpoint.method,
                    "path": endpoint.path,
                    "scope": endpoint.scope.as_str(),
                    "summary": endpoint.summary,
                    "description": endpoint.description,
                    "mutates_state": endpoint.mutates_state,
                    "parameters": endpoint.parameters,
                    "request": endpoint.request,
                    "response": endpoint.response,
                    "curl": endpoint.curl(base_url),
                })
            })
            .collect::<Vec<_>>();
        json!({
            "server": "fanplay-control",
            "api_version": API_VERSION,
            "description": "FANPLAY launcher and player control API. One loopback endpoint serves the launcher at all times and the active game while one is running. Discover every operation, its parameters and their types here before calling it.",
            "status": status,
            "endpoint_count": endpoints.len(),
            "endpoints": endpoints,
            "notes": [
                "Mutating requests accept parameters in the JSON body or in the query string.",
                "Operations under /v1/game and /v1/input are unavailable with HTTP 409 when no game is running.",
                "Extension routes are merged into this catalog while their plugin is loaded.",
                "The browser host exposes the same catalog through its JavaScript bridge because browsers cannot bind a loopback TCP listener."
            ]
        })
    }

    /// OpenAPI 3.1 rendering of the catalog.
    pub fn openapi(&self, status: Value) -> Value {
        let base_url = status
            .get("base_url")
            .and_then(Value::as_str)
            .unwrap_or("http://127.0.0.1:47821");
        let mut paths: BTreeMap<String, BTreeMap<String, Value>> = BTreeMap::new();
        for endpoint in &self.endpoints {
            paths.entry(endpoint.path.clone()).or_default().insert(
                endpoint.method.to_ascii_lowercase(),
                endpoint.openapi_operation(base_url),
            );
        }
        let paths = paths
            .into_iter()
            .map(|(path, operations)| (path, Value::Object(operations.into_iter().collect())))
            .collect::<serde_json::Map<_, _>>();
        json!({
            "openapi": "3.1.0",
            "info": {
                "title": "FANPLAY Control API",
                "version": API_VERSION,
                "description": "One loopback API for the FANPLAY launcher and the active game. Use /v1/discovery for a flat, typed operation catalog.",
            },
            "servers": [{ "url": base_url, "description": "FANPLAY control endpoint" }],
            "paths": paths,
        })
    }
}

fn normalize_path(path: &str) -> String {
    let path = path.split('?').next().unwrap_or(path);
    if path.len() > 1 && path.ends_with('/') {
        path.trim_end_matches('/').to_owned()
    } else if path.is_empty() {
        "/".into()
    } else {
        path.to_owned()
    }
}

fn match_template(template: &str, path: &str) -> Option<BTreeMap<String, String>> {
    let template = normalize_path(template);
    let template_parts = template.split('/').collect::<Vec<_>>();
    let path_parts = path.split('/').collect::<Vec<_>>();
    if template_parts.len() != path_parts.len() {
        return None;
    }
    let mut values = BTreeMap::new();
    for (template, value) in template_parts.iter().zip(path_parts) {
        if let Some(name) = template
            .strip_prefix('{')
            .and_then(|value| value.strip_suffix('}'))
        {
            if name.is_empty() || value.is_empty() {
                return None;
            }
            values.insert(name.to_owned(), value.to_owned());
        } else if *template != value {
            return None;
        }
    }
    Some(values)
}

fn server_endpoints() -> Vec<Endpoint> {
    vec![
        Endpoint::new(
            "server.info",
            "GET",
            "/v1",
            "Server identity",
            "Returns the API version and a short endpoint index. Use /v1/discovery for the complete typed catalog.",
            Scope::Server,
        )
        .response(json!({
            "type": "object",
            "properties": {
                "server": { "type": "string" },
                "api_version": { "type": "string" },
                "discovery": { "type": "string" },
                "openapi": { "type": "string" }
            }
        })),
        Endpoint::new(
            "server.discovery",
            "GET",
            "/v1/discovery",
            "Discover every operation",
            "Machine-readable discovery: stable operation id, HTTP method and path, summary, description, parameter names/locations/types, required flags, defaults, enums, request/response JSON schemas, and a ready-to-run curl example. This is the primary document for AI agents and external tools.",
            Scope::Server,
        ),
        Endpoint::new(
            "server.openapi",
            "GET",
            "/v1/openapi.json",
            "OpenAPI document",
            "OpenAPI 3.1 rendering of the same operation catalog.",
            Scope::Server,
        ),
        Endpoint::new(
            "server.status",
            "GET",
            "/v1/status",
            "Control server status",
            "Reports process mode, PID, uptime, active game, and whether launcher operations are available.",
            Scope::Server,
        )
        .response(json!({
            "type": "object",
            "properties": {
                "server": { "type": "string" },
                "mode": { "type": "string", "enum": ["launcher", "game"] },
                "pid": { "type": "integer" },
                "uptime_seconds": { "type": "integer" },
                "game_running": { "type": "boolean" },
                "api_url": { "type": "string" }
            }
        })),
        Endpoint::new(
            "extensions.routes",
            "GET",
            "/v1/extensions/routes",
            "List plugin route aliases",
            "Returns the route aliases currently registered by loaded Rhai plugins. The same routes are merged into /v1/discovery with their parameter names.",
            Scope::Extensions,
        )
        .response(json!({
            "type": "object",
            "properties": {
                "routes": { "type": "array", "items": { "type": "object" } }
            }
        })),
    ]
}

fn settings_schema() -> Value {
    json!({
        "type": "object",
        "description": "FANPLAY settings. Unset fields inherit the previous layer.",
        "additionalProperties": false,
        "properties": {
            "renderer": { "type": "string", "enum": ["auto", "wgpu", "software"] },
            "rgss": { "type": "integer", "minimum": 1, "maximum": 3 },
            "jit": { "type": "string", "enum": ["yjit", "zjit", "off"] },
            "fps": { "type": "integer", "minimum": 1, "maximum": 240 },
            "fast_forward": { "type": "boolean" },
            "patches": { "type": "array", "items": { "type": "string" } },
            "patch_values": { "type": "object", "additionalProperties": { "type": "number" } },
            "keyboard": { "type": "object", "additionalProperties": { "type": "integer" } },
            "gamepad": { "type": "string", "description": "Deadzone;20 button mappings" },
            "nds_renderer": { "type": "string", "enum": ["wgpu", "software"] },
            "gba_bios": { "type": "string" },
            "nds_bios7": { "type": "string" },
            "nds_bios9": { "type": "string" },
            "nds_firmware": { "type": "string" }
        }
    })
}

fn settings_endpoints() -> Vec<Endpoint> {
    vec![
        Endpoint::new(
            "settings.common.get",
            "GET",
            "/v1/settings/common",
            "Read common settings",
            "Returns the settings shared by every game. Per-game settings override these values at launch.",
            Scope::Settings,
        )
        .response(settings_schema()),
        Endpoint::new(
            "settings.common.update",
            "PUT",
            "/v1/settings/common",
            "Replace common settings",
            "Validates and stores the settings shared by every game in <FANPLAY_HOME>/config/config.json. Unknown fields are rejected.",
            Scope::Settings,
        )
        .mutates()
        .request(settings_schema()),
        Endpoint::new(
            "settings.game.get",
            "GET",
            "/v1/settings/games/{key}",
            "Read per-game settings",
            "Returns the per-game override and the effective settings after merging with common settings. The key is the launcher's stable game key (repository/id or local:absolute-path).",
            Scope::Settings,
        )
        .parameter(Parameter::path("key", ParameterType::String, "Stable game key"))
        .parameter(Parameter::query(
            "path",
            ParameterType::String,
            false,
            "Optional game path used to derive the key and defaults when the key is unknown.",
        )),
        Endpoint::new(
            "settings.game.update",
            "PUT",
            "/v1/settings/games/{key}",
            "Replace per-game override",
            "Stores a per-game settings override. Send `{ \"settings\": null }` to clear the override and inherit common settings.",
            Scope::Settings,
        )
        .mutates()
        .parameter(Parameter::path("key", ParameterType::String, "Stable game key"))
        .parameter(Parameter::body("settings", ParameterType::Object, false, "Per-game settings object, or null to clear"))
        .parameter(Parameter::body("cover_sha256", ParameterType::String, false, "Optional cached cover hash")),
        Endpoint::new(
            "settings.effective",
            "GET",
            "/v1/settings/effective",
            "Effective settings for a game",
            "Merges common settings with a per-game override and returns the values a launch would use.",
            Scope::Settings,
        )
        .parameter(Parameter::query(
            "game",
            ParameterType::String,
            true,
            "Game directory, Game.ini, or cartridge path",
        )),
    ]
}

fn launcher_endpoints() -> Vec<Endpoint> {
    vec![
        Endpoint::new(
            "launcher.status",
            "GET",
            "/v1/launcher/status",
            "Launcher and session status",
            "Reports the FANPLAY home, active game, pause state, save/patches folders, library size, and whether this host can execute launcher actions.",
            Scope::Launcher,
        ),
        Endpoint::new(
            "launcher.library",
            "GET",
            "/v1/launcher/library",
            "List the game library",
            "Returns recent local games and games from every selected JSON catalog, including installation state.",
            Scope::Launcher,
        )
        .parameter(Parameter::query(
            "query",
            ParameterType::String,
            false,
            "Optional case-insensitive title filter",
        ))
        .parameter(Parameter::query(
            "platform",
            ParameterType::String,
            false,
            "Optional platform filter: rpg-maker, gba, nds",
        ))
        .parameter(Parameter::query(
            "catalog",
            ParameterType::String,
            false,
            "Optional catalog id; omit for every selected catalog",
        )),
        Endpoint::new(
            "launcher.library.add",
            "POST",
            "/v1/launcher/library",
            "Add a local game to the library",
            "Remembers a local game directory, Game.ini, or cartridge path in the persistent library.",
            Scope::Launcher,
        )
        .mutates()
        .parameter(Parameter::body("path", ParameterType::String, true, "Game directory, Game.ini, or .gba/.nds file"))
        .parameter(Parameter::body("launch", ParameterType::Boolean, false, "Launch immediately after adding")),
        Endpoint::new(
            "launcher.launch",
            "POST",
            "/v1/launcher/launch",
            "Launch a game",
            "Launches a local path or a game from a selected catalog. Catalog launches use the existing hash-verified installer. The response is returned after the launch is queued.",
            Scope::Launcher,
        )
        .mutates()
        .parameter(Parameter::body("path", ParameterType::String, false, "Local game directory, Game.ini, or ROM path"))
        .parameter(Parameter::body("catalog", ParameterType::String, false, "Catalog id for a catalog game"))
        .parameter(Parameter::body("game", ParameterType::String, false, "Game id inside the catalog"))
        .parameter(Parameter::body("headless", ParameterType::Boolean, false, "Launch without opening the game window")),
        Endpoint::new(
            "launcher.stop",
            "POST",
            "/v1/launcher/stop",
            "Stop the active game",
            "Stops the active game while keeping the launcher and this API server alive.",
            Scope::Launcher,
        )
        .mutates(),
        Endpoint::new(
            "launcher.restart",
            "POST",
            "/v1/launcher/restart",
            "Restart the active game",
            "Restarts the active game with its current source and effective settings.",
            Scope::Launcher,
        )
        .mutates(),
        Endpoint::new(
            "launcher.pause",
            "POST",
            "/v1/launcher/pause",
            "Pause or resume the active game",
            "Pauses game updates and audio while leaving the launcher responsive.",
            Scope::Launcher,
        )
        .mutates()
        .parameter(Parameter::body("paused", ParameterType::Boolean, true, "True pauses, false resumes")),
        Endpoint::new(
            "launcher.folders",
            "GET",
            "/v1/launcher/games/{key}/folders",
            "Resolve a game's folders",
            "Returns the game installation, save, patches, and backup directories without opening them.",
            Scope::Launcher,
        )
        .parameter(Parameter::path("key", ParameterType::String, "Stable game key or a local path"))
        .parameter(Parameter::query("path", ParameterType::String, false, "Game path when key is local:unknown")),
        Endpoint::new(
            "launcher.open_folder",
            "POST",
            "/v1/launcher/games/{key}/open-folder",
            "Open a game folder",
            "Opens the game installation, save, patches, or backup folder in the host file manager. On hosts without a file manager, returns the resolved path with opened=false.",
            Scope::Launcher,
        )
        .mutates()
        .parameter(Parameter::path("key", ParameterType::String, "Stable game key or a local path"))
        .parameter(Parameter::body(
            "kind",
            ParameterType::String,
            true,
            "Folder to open",
        )
        .choices(&["game", "save", "patches", "backups"]))
        .parameter(Parameter::body("path", ParameterType::String, false, "Game path when key is local:unknown")),
        Endpoint::new(
            "launcher.live",
            "GET",
            "/v1/launcher/live",
            "Read live runtime settings",
            "Returns fast-forward, frame-rate override, pause state, and effective launch settings for the active game.",
            Scope::Launcher,
        ),
        Endpoint::new(
            "launcher.live.update",
            "PATCH",
            "/v1/launcher/live",
            "Change live runtime settings",
            "Changes settings that do not require a restart: fast-forward, frame-rate override, and pause. Renderer, JIT, RGSS and BIOS changes are launch settings and must be saved through /v1/settings and applied with a restart.",
            Scope::Launcher,
        )
        .mutates()
        .parameter(Parameter::body("fast_forward", ParameterType::Boolean, false, "Toggle uncapped execution"))
        .parameter(Parameter::body("fps", ParameterType::Integer, false, "Frame-rate override; 0 restores the game's own pacing").range(0.0, 1000.0))
        .parameter(Parameter::body("paused", ParameterType::Boolean, false, "Pause or resume")),
    ]
}

fn catalog_endpoints() -> Vec<Endpoint> {
    vec![
        Endpoint::new(
            "catalogs.list",
            "GET",
            "/v1/catalogs",
            "List JSON catalogs",
            "Returns every permanently stored catalog source, whether it is selected, its offline cache state, and game count.",
            Scope::Catalogs,
        ),
        Endpoint::new(
            "catalogs.add",
            "POST",
            "/v1/catalogs",
            "Add a JSON catalog",
            "Stores a local path or HTTPS catalog URL permanently in <FANPLAY_HOME>/config/config.json. Optionally refreshes and selects it.",
            Scope::Catalogs,
        )
        .mutates()
        .parameter(Parameter::body("source", ParameterType::String, true, "HTTPS URL or local JSON path"))
        .parameter(Parameter::body("name", ParameterType::String, false, "Display name; defaults to the path/file name"))
        .parameter(Parameter::body("select", ParameterType::Boolean, false, "Select the catalog for the library; defaults to true"))
        .parameter(Parameter::body("refresh", ParameterType::Boolean, false, "Download and cache immediately; defaults to true")),
        Endpoint::new(
            "catalogs.remove",
            "DELETE",
            "/v1/catalogs/{id}",
            "Remove a JSON catalog",
            "Removes a catalog from the selection list. Its offline cache is retained so re-adding the source is instant.",
            Scope::Catalogs,
        )
        .mutates()
        .parameter(Parameter::path("id", ParameterType::String, "Catalog id")),
        Endpoint::new(
            "catalogs.select",
            "POST",
            "/v1/catalogs/{id}/select",
            "Select or deselect a catalog",
            "Controls whether a permanently stored catalog contributes games to the library.",
            Scope::Catalogs,
        )
        .mutates()
        .parameter(Parameter::path("id", ParameterType::String, "Catalog id"))
        .parameter(Parameter::body("selected", ParameterType::Boolean, true, "New selection state")),
        Endpoint::new(
            "catalogs.refresh",
            "POST",
            "/v1/catalogs/refresh",
            "Refresh catalogs",
            "Re-downloads one or every stored catalog. A failing source keeps its last good offline cache.",
            Scope::Catalogs,
        )
        .mutates()
        .parameter(Parameter::body("id", ParameterType::String, false, "Catalog id; omit for all")),
        Endpoint::new(
            "catalogs.games",
            "GET",
            "/v1/catalogs/{id}/games",
            "List games in a catalog",
            "Returns the cached games of one catalog, including download and installation metadata.",
            Scope::Catalogs,
        )
        .parameter(Parameter::path("id", ParameterType::String, "Catalog id")),
    ]
}

fn save_endpoints() -> Vec<Endpoint> {
    vec![
        Endpoint::new(
            "saves.info",
            "GET",
            "/v1/saves",
            "Resolve save folders and backups",
            "Returns the save directory, default backup directory, named backups, and whether backup/restore is possible while a game is running.",
            Scope::Saves,
        )
        .parameter(Parameter::query(
            "game",
            ParameterType::String,
            false,
            "Game key, directory, Game.ini, or ROM path; defaults to the active game",
        )),
        Endpoint::new(
            "saves.backup",
            "POST",
            "/v1/saves/backup",
            "Create a save backup",
            "Copies a game's save directory into a named, timestamped backup under <FANPLAY_HOME>/backups. The source and destination must not overlap.",
            Scope::Saves,
        )
        .mutates()
        .parameter(Parameter::body("game", ParameterType::String, false, "Game key or path; defaults to the active game"))
        .parameter(Parameter::body("source", ParameterType::String, false, "Explicit save directory"))
        .parameter(Parameter::body("name", ParameterType::String, false, "Backup name; defaults to a UTC timestamp")),
        Endpoint::new(
            "saves.restore",
            "POST",
            "/v1/saves/restore",
            "Restore a save backup",
            "Atomically restores matching files from a named backup. Existing files are preserved in a before-restore safety directory. The game must be stopped.",
            Scope::Saves,
        )
        .mutates()
        .parameter(Parameter::body("game", ParameterType::String, false, "Game key or path; defaults to the active game"))
        .parameter(Parameter::body("backup", ParameterType::String, true, "Backup name from /v1/saves"))
        .parameter(Parameter::body("destination", ParameterType::String, false, "Explicit destination save directory")),
        Endpoint::new(
            "saves.delete",
            "DELETE",
            "/v1/saves/backups/{name}",
            "Delete a save backup",
            "Deletes one named backup. This never touches live saves.",
            Scope::Saves,
        )
        .mutates()
        .parameter(Parameter::path("name", ParameterType::String, "Backup name"))
        .parameter(Parameter::query("game", ParameterType::String, false, "Game key or path; defaults to the active game")),
    ]
}

fn game_endpoints() -> Vec<Endpoint> {
    vec![
        Endpoint::new(
            "game.status",
            "GET",
            "/v1/game/status",
            "Read game status",
            "Returns the running RGSS game's current map, player position, frame counters, and generic runtime facts. Produced by the loaded game API extension.",
            Scope::Game,
        ),
        Endpoint::new(
            "game.world_map",
            "GET",
            "/v1/game/world-map",
            "Read the connected world map",
            "Returns maps, connections, and transfer links from the active Essentials world-map plugin.",
            Scope::Game,
        ),
        Endpoint::new(
            "game.map_tiles",
            "GET",
            "/v1/game/maps/{map_id}/tiles",
            "Read actual map tile layers",
            "Returns decoded tile layers for one map.",
            Scope::Game,
        )
        .parameter(Parameter::path("map_id", ParameterType::Integer, "RPG Maker map id")),
        Endpoint::new(
            "game.surroundings",
            "GET",
            "/v1/game/surroundings",
            "Read nearby map contents",
            "Returns events, NPCs, and tiles around the player.",
            Scope::Game,
        )
        .parameter(Parameter::query(
            "radius",
            ParameterType::Integer,
            false,
            "Tile radius around the player; defaults to the game API extension's limit",
        ).range(1.0, 64.0)),
        Endpoint::new(
            "game.teleport",
            "POST",
            "/v1/game/teleport",
            "Teleport the player",
            "Queues a transfer through the game's own transfer helper so map events and transitions stay authoritative.",
            Scope::Game,
        )
        .mutates()
        .parameter(Parameter::body("map_id", ParameterType::Integer, true, "Destination map id"))
        .parameter(Parameter::body("x", ParameterType::Integer, true, "Destination tile x"))
        .parameter(Parameter::body("y", ParameterType::Integer, true, "Destination tile y")),
        Endpoint::new(
            "game.press",
            "POST",
            "/v1/input/press",
            "Press a game key",
            "Feeds a normal RGSS input press for a bounded number of frames, preserving the game's own input and collision behavior.",
            Scope::Game,
        )
        .mutates()
        .parameter(Parameter::body("key", ParameterType::String, true, "Key name: up, down, left, right, action, cancel, menu, run, etc."))
        .parameter(Parameter::body("frames", ParameterType::Integer, false, "Press duration in frames").range(1.0, 600.0)),
        Endpoint::new(
            "game.release_keys",
            "DELETE",
            "/v1/input/held",
            "Release all game keys",
            "Clears held API-injected keys.",
            Scope::Game,
        )
        .mutates(),
        Endpoint::new(
            "game.settings",
            "GET",
            "/v1/game/settings",
            "Read effective game settings",
            "Returns common, per-game, and live settings for the active game. This endpoint is available from both the launcher and the player.",
            Scope::Game,
        )
        .parameter(Parameter::query("game", ParameterType::String, false, "Game path; defaults to the active game")),
        Endpoint::new(
            "game.reset",
            "POST",
            "/v1/game/reset",
            "Reset the active game",
            "Requests a full restart from the beginning. Unsaved progress is lost.",
            Scope::Game,
        )
        .mutates(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_catalog_is_typed_and_unique() {
        let catalog = Catalog::standard();
        let mut seen = std::collections::HashSet::new();
        for endpoint in catalog.endpoints() {
            assert!(
                seen.insert(format!("{} {}", endpoint.method, endpoint.path)),
                "duplicate route {} {}",
                endpoint.method,
                endpoint.path
            );
            assert!(!endpoint.summary.is_empty());
            assert!(!endpoint.description.is_empty());
            for parameter in &endpoint.parameters {
                assert!(!parameter.name.is_empty());
                assert!(!parameter.description.is_empty());
            }
        }
        assert!(catalog.get("launcher.launch").is_some());
        assert!(catalog.get("settings.common.update").unwrap().mutates_state);
    }

    #[test]
    fn discovery_exposes_names_types_and_curl() {
        let catalog = Catalog::standard();
        let document = catalog.discovery(json!({"base_url": "http://127.0.0.1:47821"}));
        let launch = document["endpoints"]
            .as_array()
            .unwrap()
            .iter()
            .find(|endpoint| endpoint["id"] == "launcher.launch")
            .unwrap();
        assert_eq!(launch["method"], "POST");
        let path = launch["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|parameter| parameter["name"] == "path")
            .unwrap();
        assert_eq!(path["type"], "string");
        assert_eq!(path["in"], "body");
        assert!(launch["curl"].as_str().unwrap().contains("curl -s -X POST"));
        let kind = document["endpoints"]
            .as_array()
            .unwrap()
            .iter()
            .find(|endpoint| endpoint["id"] == "launcher.open_folder")
            .unwrap()["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|parameter| parameter["name"] == "kind")
            .unwrap();
        assert_eq!(kind["enum"], json!(["game", "save", "patches", "backups"]));
    }

    #[test]
    fn path_templates_match_and_reject_trailing_segments() {
        let catalog = Catalog::standard();
        let matched = catalog.match_path("GET", "/v1/game/maps/25/tiles").unwrap();
        assert_eq!(matched.endpoint.id, "game.map_tiles");
        assert_eq!(matched.path_parameters["map_id"], "25");
        assert!(catalog
            .match_path("GET", "/v1/game/maps/25/tiles/extra")
            .is_none());
        assert!(catalog.match_path("GET", "/v1/game/maps//tiles").is_none());
    }

    #[test]
    fn dynamic_routes_replace_builtin_metadata() {
        let mut catalog = Catalog::standard();
        catalog.merge_dynamic([Endpoint::new(
            "extension.game_status",
            "GET",
            "/v1/game/status",
            "Game status from plugin",
            "Plugin-provided schema.",
            Scope::Extensions,
        )]);
        assert!(catalog.get("extension.game_status").is_some());
        assert_eq!(
            catalog.get("extension.game_status").unwrap().summary,
            "Game status from plugin"
        );
        assert_eq!(
            catalog
                .match_path("GET", "/v1/game/status")
                .unwrap()
                .endpoint
                .id,
            "extension.game_status"
        );
    }

    #[test]
    fn openapi_contains_parameters_and_operation_ids() {
        let catalog = Catalog::standard();
        let document = catalog.openapi(json!({"base_url": "http://127.0.0.1:47821"}));
        assert_eq!(document["openapi"], "3.1.0");
        assert_eq!(
            document["paths"]["/v1/settings/common"]["put"]["operationId"],
            "settings_common_update"
        );
        assert_eq!(
            document["paths"]["/v1/launcher/pause"]["post"]["requestBody"]["content"]
                ["application/json"]["schema"]["properties"]["paused"]["type"],
            "boolean"
        );
        assert_eq!(
            document["paths"]["/v1/settings/common"]["put"]["requestBody"]["content"]
                ["application/json"]["schema"]["properties"]["renderer"]["type"],
            "string"
        );
    }

    #[test]
    fn trailing_slash_is_tolerated_but_unknown_paths_are_not() {
        let catalog = Catalog::standard();
        assert!(catalog.match_path("GET", "/v1/status/").is_some());
        assert!(catalog.match_path("GET", "/v1/not-real").is_none());
    }
}
