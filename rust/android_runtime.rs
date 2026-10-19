// SPDX-License-Identifier: GPL-2.0-or-later
//! Minimal counterpart to the desktop typed-binding timing wrapper.
use magnus::{Error, Ruby};
use std::panic::{catch_unwind, AssertUnwindSafe};

pub(crate) fn direct_call<T>(
    _operation: &'static str,
    call: impl FnOnce() -> Result<T, Error>,
) -> Result<T, Error> {
    match catch_unwind(AssertUnwindSafe(call)) {
        Ok(result) => result,
        Err(payload) => {
            let message = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("unknown Rust panic");
            Err(Error::new(
                Ruby::get()
                    .expect("Android binding panic must be caught on the Ruby thread")
                    .exception_runtime_error(),
                format!("Android native engine panic: {message}"),
            ))
        }
    }
}
