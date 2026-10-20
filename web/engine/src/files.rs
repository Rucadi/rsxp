// SPDX-License-Identifier: GPL-2.0-or-later
//! Browser virtual filesystem, on-demand page reader and startup archives.
//!
//! Game files live in a thread-local `HashMap` keyed by lower-cased,
//! forward-slash-normalized paths. JavaScript populates it with `web_add_file`
//! (or `web_add_archive` for encrypted `Game.rgssad`/`2a`/`3a` images, which
//! `web_init` drains via `take_archives`). For paths that are not cached,
//! `read_via_host` falls back to a registered reader function (`web_set_reader`,
//! signature `(path) => Uint8Array | null`) so the host page can fetch assets on
//! demand. `web_file_count` and `web_read_file` are introspection/debugging
//! helpers.
use js_sys::Uint8Array;
use std::{cell::RefCell, collections::HashMap};
use wasm_bindgen::{prelude::*, JsCast};

thread_local! {
    static FILES: RefCell<HashMap<String, Vec<u8>>> = RefCell::new(HashMap::new());
    static ARCHIVES: RefCell<Vec<Vec<u8>>> = RefCell::new(Vec::new());
    // JS function `(path) => Uint8Array | null` for on-demand asset loading.
    static READER: RefCell<Option<js_sys::Function>> = const { RefCell::new(None) };
}

fn normalize(path: &str) -> String {
    path.replace('\\', "/").to_lowercase()
}

/// Looks a file up in the in-memory cache, then asks the host page to fetch it.
pub(crate) fn read_via_host(path: &str) -> Option<Vec<u8>> {
    let normalized = path.replace('\\', "/");
    if let Some(bytes) = FILES.with(|files| files.borrow().get(&normalize(&normalized)).cloned()) {
        return Some(bytes);
    }
    READER.with(|slot| {
        let slot = slot.borrow();
        let reader = slot.as_ref()?;
        let value = reader
            .call1(&JsValue::NULL, &JsValue::from_str(&normalized))
            .ok()?;
        if value.is_null() || value.is_undefined() {
            return None;
        }
        value
            .dyn_into::<Uint8Array>()
            .ok()
            .map(|array| array.to_vec())
    })
}

/// Adds (or replaces) a game file in the virtual filesystem.
#[wasm_bindgen]
pub fn web_add_file(path: &str, bytes: &[u8]) {
    FILES.with(|files| {
        files.borrow_mut().insert(normalize(path), bytes.to_vec());
    });
}

/// Registers the host page's on-demand reader: `(path) => Uint8Array | null`.
/// It is called (synchronously) for files not already in memory.
#[wasm_bindgen]
pub fn web_set_reader(reader: js_sys::Function) {
    READER.with(|slot| *slot.borrow_mut() = Some(reader));
}

/// Adds an encrypted archive (`Game.rgssad`/`2a`/`3a`) to search before init.
#[wasm_bindgen]
pub fn web_add_archive(bytes: &[u8]) {
    ARCHIVES.with(|archives| archives.borrow_mut().push(bytes.to_vec()));
}

/// Number of files currently in the virtual filesystem.
#[wasm_bindgen]
pub fn web_file_count() -> usize {
    FILES.with(|files| files.borrow().len())
}

/// Reads a virtual file back out, for debugging from JavaScript.
#[wasm_bindgen]
pub fn web_read_file(path: &str) -> Option<Uint8Array> {
    FILES.with(|files| {
        files
            .borrow()
            .get(&normalize(path))
            .map(|bytes| Uint8Array::from(bytes.as_slice()))
    })
}

pub(crate) fn take_archives() -> Vec<Vec<u8>> {
    ARCHIVES.with(|archives| std::mem::take(&mut *archives.borrow_mut()))
}
