// SPDX-License-Identifier: GPL-2.0-or-later
//! Generic operation discovery and routing shared by GUI, HTTP and MCP.
//!
//! This crate deliberately does not open sockets or know about an emulator.
//! Native transports resolve aliases and submit [`OperationRequest`] values;
//! the extension host or a core adapter performs the actual work.

use rsxp_extension_api::{
    ExtensionManifest, OperationDeclaration, OperationRequest, OperationResponse,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RegisteredOperation {
    pub name: String,
    pub extension_id: String,
    pub declaration: OperationDeclaration,
}

#[derive(Clone, Debug, Default)]
pub struct OperationRegistry {
    operations: BTreeMap<String, RegisteredOperation>,
    aliases: BTreeMap<String, String>,
}

impl OperationRegistry {
    pub fn register_manifest(&mut self, manifest: &ExtensionManifest) -> Result<(), ApiError> {
        manifest
            .validate()
            .map_err(|error| ApiError::InvalidManifest(error.to_string()))?;
        self.remove_extension(&manifest.id);
        for declaration in &manifest.operations {
            let name = format!("{}.{}", manifest.id, declaration.id);
            if self.operations.contains_key(&name) {
                return Err(ApiError::DuplicateOperation(name));
            }
            for alias in &declaration.route_aliases {
                let key = route_key(&alias.method, &alias.path);
                if self.aliases.contains_key(&key) {
                    return Err(ApiError::DuplicateRoute(key));
                }
                self.aliases.insert(key, name.clone());
            }
            self.operations.insert(
                name.clone(),
                RegisteredOperation {
                    name,
                    extension_id: manifest.id.clone(),
                    declaration: declaration.clone(),
                },
            );
        }
        Ok(())
    }

    pub fn remove_extension(&mut self, extension_id: &str) {
        let names: Vec<_> = self
            .operations
            .iter()
            .filter(|(_, operation)| operation.extension_id == extension_id)
            .map(|(name, _)| name.clone())
            .collect();
        for name in names {
            self.operations.remove(&name);
            self.aliases.retain(|_, operation| operation != &name);
        }
    }

    pub fn get(&self, name: &str) -> Option<&RegisteredOperation> {
        self.operations.get(name)
    }

    pub fn resolve_route(&self, method: &str, path: &str) -> Option<&RegisteredOperation> {
        let name = self.aliases.get(&route_key(method, path))?;
        self.operations.get(name)
    }

    pub fn iter(&self) -> impl Iterator<Item = &RegisteredOperation> {
        self.operations.values()
    }

    pub fn discover(&self) -> Vec<RegisteredOperation> {
        self.operations.values().cloned().collect()
    }
}

fn route_key(method: &str, path: &str) -> String {
    format!("{} {}", method.to_ascii_uppercase(), path)
}

/// External invocation accepted by any transport adapter.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OperationInvocation {
    pub operation: String,
    #[serde(default)]
    pub arguments: Value,
}

/// Small request/status router for HTTP and MCP adapters. It preserves the
/// same generation and response semantics as the script host.
pub struct ApiRouter {
    pub registry: OperationRegistry,
    generation: u64,
    next_id: u64,
    max_pending: usize,
    pending: HashMap<u64, OperationRequest>,
    completed: HashMap<u64, OperationResponse>,
}

impl Default for ApiRouter {
    fn default() -> Self {
        Self {
            registry: OperationRegistry::default(),
            generation: 1,
            next_id: 0,
            max_pending: 256,
            pending: HashMap::new(),
            completed: HashMap::new(),
        }
    }
}

impl ApiRouter {
    pub fn new(registry: OperationRegistry) -> Self {
        Self {
            registry,
            generation: 1,
            max_pending: 256,
            ..Default::default()
        }
    }

    pub fn with_max_pending(mut self, max_pending: usize) -> Self {
        self.max_pending = max_pending;
        self
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn invoke(
        &mut self,
        invocation: OperationInvocation,
    ) -> Result<OperationRequest, ApiError> {
        let operation = self
            .registry
            .get(&invocation.operation)
            .ok_or_else(|| ApiError::UnknownOperation(invocation.operation.clone()))?;
        if self.pending.len() >= self.max_pending {
            return Err(ApiError::QueueFull);
        }
        if operation.declaration.mutates_state && !invocation.arguments.is_object() {
            return Err(ApiError::InvalidArguments(
                "operation arguments must be an object".into(),
            ));
        }
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let request = OperationRequest {
            id: self.next_id,
            generation: self.generation,
            operation: invocation.operation,
            arguments: invocation.arguments,
        };
        self.pending.insert(request.id, request.clone());
        Ok(request)
    }

    pub fn complete(&mut self, response: OperationResponse) -> Result<(), ApiError> {
        if response.generation != self.generation {
            return Ok(());
        }
        if self.pending.remove(&response.id).is_none() {
            return Err(ApiError::UnknownRequest(response.id));
        }
        self.completed.insert(response.id, response);
        Ok(())
    }

    pub fn status(&mut self, id: u64) -> Option<OperationResponse> {
        self.completed.remove(&id)
    }

    pub fn reset_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1).max(1);
        self.pending.clear();
        self.completed.clear();
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ApiError {
    InvalidManifest(String),
    DuplicateOperation(String),
    DuplicateRoute(String),
    UnknownOperation(String),
    InvalidArguments(String),
    QueueFull,
    UnknownRequest(u64),
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidManifest(message) => write!(f, "invalid extension manifest: {message}"),
            Self::DuplicateOperation(name) => write!(f, "duplicate operation {name:?}"),
            Self::DuplicateRoute(route) => write!(f, "duplicate route {route:?}"),
            Self::UnknownOperation(name) => write!(f, "unknown operation {name:?}"),
            Self::InvalidArguments(message) => f.write_str(message),
            Self::QueueFull => f.write_str("operation request queue is full"),
            Self::UnknownRequest(id) => write!(f, "unknown operation request {id}"),
        }
    }
}

impl std::error::Error for ApiError {}

#[cfg(test)]
mod tests {
    use super::*;
    use rsxp_extension_api::{EntryPoints, ExtensionManifest, OperationDeclaration, RouteAlias};

    fn manifest() -> ExtensionManifest {
        ExtensionManifest {
            api_version: 1,
            id: "example".into(),
            name: "Example".into(),
            version: "1.0.0".into(),
            entry: EntryPoints::default(),
            compatibility: Default::default(),
            capabilities: Vec::new(),
            operations: vec![OperationDeclaration {
                id: "toggle".into(),
                route_aliases: vec![RouteAlias {
                    method: "POST".into(),
                    path: "/v1/example/toggle".into(),
                    ..Default::default()
                }],
                mutates_state: true,
                ..Default::default()
            }],
            panels: Vec::new(),
        }
    }

    #[test]
    fn discovers_and_resolves_namespaced_operations() {
        let mut registry = OperationRegistry::default();
        registry.register_manifest(&manifest()).unwrap();
        assert_eq!(registry.discover().len(), 1);
        assert_eq!(
            registry
                .resolve_route("post", "/v1/example/toggle")
                .unwrap()
                .name,
            "example.toggle"
        );
    }

    #[test]
    fn router_tracks_completion_and_ignores_stale_generations() {
        let mut registry = OperationRegistry::default();
        registry.register_manifest(&manifest()).unwrap();
        let mut router = ApiRouter::new(registry);
        let request = router
            .invoke(OperationInvocation {
                operation: "example.toggle".into(),
                arguments: serde_json::json!({}),
            })
            .unwrap();
        router
            .complete(OperationResponse {
                id: request.id,
                generation: request.generation,
                result: Ok(serde_json::json!({"enabled": true})),
            })
            .unwrap();
        assert!(router.status(request.id).is_some());
        router.reset_generation();
        assert!(router.status(request.id).is_none());
    }
}
