// SPDX-License-Identifier: GPL-2.0-or-later
//! Serialized native callbacks exposed to RGSS Ruby as `__rsxp_native` and
//! the legacy `__mkxp_native` alias.
use magnus::{RString, Ruby};
use rsxp::engine;

pub(crate) fn native_call(ruby: &Ruby, request: RString) -> Result<Option<RString>, magnus::Error> {
    // SAFETY: copy the Ruby bytes before calling back into Rust; the borrowed
    // slice is not used after the native dispatcher runs.
    let bytes = unsafe { request.as_slice() }.to_vec();
    match engine::native_call_bytes(&bytes) {
        Ok(Some(bytes)) => Ok(Some(ruby.str_from_slice(&bytes))),
        Ok(None) => Ok(None),
        Err(message) => Err(magnus::Error::new(ruby.exception_runtime_error(), message)),
    }
}
