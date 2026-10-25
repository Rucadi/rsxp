// SPDX-License-Identifier: GPL-2.0-or-later
//! One transport-neutral control contract for the launcher and the player.
//!
//! RSXP used to expose two unrelated loopback services: the desktop workspace
//! started a game API in the dashboard process, while a headless game could
//! start its own API in the runner process. Their discovery documents and
//! endpoint sets differed, so an external tool had to know which kind of
//! process answered on the port.
//!
//! This crate defines the stable contract instead:
//!
//! * [`Catalog`] is a typed description of every host operation (method, path,
//!   summary, parameter names/locations/types/required/default/enum, request
//!   and response schemas). [`Catalog::standard`] is the built-in contract.
//! * [`Catalog::discovery`] returns machine-readable endpoint discovery suitable
//!   for an AI agent or a human with `curl`.
//! * [`Catalog::openapi`] renders the same catalog as OpenAPI 3.1.
//! * [`server`] (native only) serves the catalog and dispatches normalized
//!   [`ControlRequest`] values to a [`ControlHandler`].
//!
//! The launcher and the player both own the same catalog. A desktop dashboard
//! additionally merges the game runner's extension routes into the catalog at
//! runtime. The browser cannot bind a TCP socket, so its host exposes the same
//! catalog through a JavaScript bridge instead of pretending to be a server.
//!
//! Keeping metadata separate from execution is deliberate: the HTTP, MCP and
//! browser adapters can all describe an operation even when a particular host
//! cannot execute it. Unavailable launcher operations, for example, still
//! appear in a headless game's discovery document with an explicit
//! `availability` marker, so a client never has to guess which URL is live.

mod catalog;
#[cfg(not(target_arch = "wasm32"))]
pub mod server;

pub use catalog::{
    Catalog, Endpoint, EndpointMatch, Parameter, ParameterLocation, ParameterType, Scope,
    API_VERSION, DEFAULT_PORT,
};
#[cfg(not(target_arch = "wasm32"))]
pub use server::{
    start, start_on, start_with_catalog, start_with_catalog_on, ControlHandler, ControlRequest,
    ControlResponse, ServerHandle,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A normalized request handed to a host adapter after path/query/body parsing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Invocation {
    /// Stable endpoint id, for example `launcher.launch` or `game.press`.
    pub endpoint: String,
    /// HTTP method used by the caller.
    pub method: String,
    /// Original request target, including the query string.
    pub target: String,
    /// Path and query parameters named by the catalog.
    #[serde(default)]
    pub parameters: std::collections::BTreeMap<String, Value>,
    /// JSON request body. Always an object (possibly empty) for mutations.
    #[serde(default)]
    pub body: Value,
}

impl Invocation {
    pub fn parameter(&self, name: &str) -> Option<&Value> {
        self.parameters.get(name)
    }

    pub fn string(&self, name: &str) -> Option<&str> {
        self.parameters.get(name).and_then(Value::as_str)
    }

    pub fn integer(&self, name: &str) -> Option<i64> {
        self.parameters.get(name).and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        })
    }

    pub fn boolean(&self, name: &str) -> Option<bool> {
        self.parameters.get(name).and_then(|value| match value {
            Value::Bool(value) => Some(*value),
            Value::String(text) => match text.as_str() {
                "true" | "1" | "yes" | "on" => Some(true),
                "false" | "0" | "no" | "off" => Some(false),
                _ => None,
            },
            _ => None,
        })
    }

    /// Reads a body field, falling back to a query parameter of the same name.
    pub fn field(&self, name: &str) -> Option<&Value> {
        self.body.get(name).or_else(|| self.parameters.get(name))
    }
}
