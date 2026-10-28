// SPDX-License-Identifier: GPL-2.0-or-later
//! Host-registered native calls exposed to Ruby as `FANPLAY.native`.
//!
//! The registry lives in the platform-independent engine, not in the Ruby
//! extension. Native and wasm builds therefore install the same handlers and
//! Ruby reaches them through the existing `__fanplay_native` bridge:
//!
//! ```ruby
//! FANPLAY.native("example.echo", "payload")
//! ```
//!
//! A handler receives the raw request payload (after the command header) and
//! returns raw bytes or a human-readable error. Call [`register`] once at
//! startup; [`install_defaults`] is idempotently called by
//! [`Runtime::new`](crate::engine::Runtime::new) for the built-in diagnostics.
//!
//! Handlers run while the engine is already mutably borrowed by
//! [`native_call_bytes`](crate::engine::native_call_bytes). Do not call
//! `engine::with_runtime`/`with_runtime_mut` from a handler; keep host calls
//! self-contained and return their result through the payload.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// A host function callable from Ruby. Handlers run on the game thread with no
/// Ruby values borrowed, so they must not retain or re-enter Ruby.
pub type NativeHandler = Arc<dyn Fn(&[u8]) -> Result<Vec<u8>, String> + Send + Sync + 'static>;

fn registry() -> &'static Mutex<HashMap<String, NativeHandler>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, NativeHandler>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Registers one named native call.
///
/// Names are the stable part of the Ruby API: they must be non-empty ASCII
/// without tabs or newlines so they fit the tab-separated bridge protocol.
/// Registering the same name twice is an error; use a single startup path.
pub fn register(
    name: impl Into<String>,
    handler: impl Fn(&[u8]) -> Result<Vec<u8>, String> + Send + Sync + 'static,
) -> Result<(), String> {
    let name = name.into();
    if name.is_empty()
        || name
            .bytes()
            .any(|byte| !byte.is_ascii() || byte == b'\t' || byte == b'\n')
    {
        return Err("native call names must be non-empty ASCII without tabs or newlines".into());
    }
    let mut handlers = registry()
        .lock()
        .map_err(|_| "native call registry is unavailable".to_string())?;
    if handlers.contains_key(&name) {
        return Err(format!("native call is already registered: {name}"));
    }
    handlers.insert(name, Arc::new(handler));
    Ok(())
}

/// Looks up and runs a registered native call. The registry lock is released
/// before the handler runs so a handler may call back into the engine.
pub fn dispatch(name: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
    let handler = registry()
        .lock()
        .map_err(|_| "native call registry is unavailable".to_string())?
        .get(name)
        .cloned()
        .ok_or_else(|| format!("unknown native call: {name}"))?;
    handler(payload)
}

/// Installs the built-in diagnostics shared by every frontend. Registration is
/// idempotent, so repeated runtime construction (tests, reset, worker restart)
/// leaves the existing handlers untouched.
pub fn install_defaults() {
    let _ = register("fanplay.echo", |payload| Ok(payload.to_vec()));
    let _ = register("fanplay.version", |_| {
        Ok(env!("CARGO_PKG_VERSION").as_bytes().to_vec())
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_calls_receive_the_request_payload() {
        register("test.native.echo", |payload| Ok(payload.to_vec())).unwrap();
        assert_eq!(
            dispatch("test.native.echo", b"hello").unwrap(),
            b"hello".to_vec()
        );
    }

    #[test]
    fn unknown_calls_name_the_missing_handler() {
        let error = dispatch("test.native.missing", b"").unwrap_err();
        assert!(error.contains("test.native.missing"), "{error}");
    }

    #[test]
    fn bridge_unsafe_names_are_rejected() {
        assert!(register("bad\tname", |payload| Ok(payload.to_vec())).is_err());
        assert!(register("", |payload| Ok(payload.to_vec())).is_err());
    }

    #[test]
    fn duplicate_names_are_rejected() {
        register("test.native.duplicate", |payload| Ok(payload.to_vec())).unwrap();
        assert!(register("test.native.duplicate", |payload| Ok(payload.to_vec())).is_err());
    }
}
