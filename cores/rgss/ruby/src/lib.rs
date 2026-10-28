// SPDX-License-Identifier: GPL-2.0-or-later
//! The RGSS-only embedded MRI runtime and its native engine bindings.
//!
//! Native builds use Magnus to embed MRI. The browser build keeps the Rust
//! bindings and engine in WASM and delegates CRuby execution to ruby.wasm.

#[cfg(all(target_os = "wasi", feature = "ruby"))]
use magnus::Ruby;
#[cfg(not(target_arch = "wasm32"))]
use magnus::{embed::Cleanup, function, Ruby, Value};

#[cfg(any(not(target_arch = "wasm32"), all(target_os = "wasi", feature = "ruby")))]
mod bindings;
#[cfg(not(target_arch = "wasm32"))]
mod bridge;

#[cfg(not(target_arch = "wasm32"))]
/// An initialized MRI VM with all RGSS native bindings and scripts installed.
pub struct RubyCore {
    ruby: Cleanup,
    startup_status: Option<Result<i64, String>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl RubyCore {
    /// Create an RGSS VM, install bindings, then evaluate the embedded RGSS core.
    pub fn new() -> Result<Self, String> {
        let mut core = Self::prepare()?;
        core.start()?;
        Ok(core)
    }

    /// Initialize MRI and install bindings without starting the game scripts.
    ///
    /// Core adapters use this so content loading and the actual game loop can
    /// happen in separate `Core::load` and `Core::run` phases.
    pub fn prepare() -> Result<Self, String> {
        // SAFETY: one VM is initialized per process. The caller must not create
        // concurrent Ruby VMs; the desktop runner and RGSS core both do this on
        // their dedicated game thread.
        let ruby = unsafe { magnus::embed::init() };
        if std::env::var_os("FANPLAY_SESSION_STDIO").is_some() {
            Self::eval::<Value>(&ruby, "$stdout = $stderr", "<fanplay-session-stdio>")
                .map_err(|error| error.to_string())?;
        }
        if let Err(error) = Self::eval::<Value>(&ruby, include_str!("jit.rb"), "<fanplay-jit>") {
            eprintln!("Could not configure Ruby JIT: {error}");
        }
        bindings::bind(&ruby).map_err(|error| error.to_string())?;
        ruby.define_global_function("__fanplay_native", function!(bridge::native_call, 1))
            .map_err(|error| error.to_string())?;
        ruby.define_global_function("__mkxp_native", function!(bridge::native_call, 1))
            .map_err(|error| error.to_string())?;
        Ok(Self {
            ruby,
            startup_status: None,
        })
    }

    /// Evaluate the embedded RGSS shim and, for a game bundle, run the game.
    pub fn start(&mut self) -> Result<i64, String> {
        if let Some(status) = &self.startup_status {
            return status.clone();
        }
        let status = Self::eval::<i64>(&self.ruby, fanplay_rgss::ruby::RGSS_SOURCE, "rgss.rb")
            .map_err(|error| error.to_string());
        self.startup_status = Some(status.clone());
        status
    }

    pub fn startup_status(&self) -> Result<i64, String> {
        match &self.startup_status {
            Some(status) => status.clone(),
            None => Err("Ruby core has not been started".into()),
        }
    }

    pub fn inject_eval(&self, source: &str, name: &str) -> Result<(), String> {
        Self::eval::<Value>(&self.ruby, source, name)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub fn extract_i64(&self, expression: &str) -> Result<i64, String> {
        self.eval_named(expression, "<fanplay-extract>")
    }

    pub fn extract_string(&self, expression: &str) -> Result<String, String> {
        self.eval_named(expression, "<fanplay-extract>")
    }

    fn eval_named<T: magnus::TryConvert>(&self, source: &str, name: &str) -> Result<T, String> {
        Self::eval(&self.ruby, source, name).map_err(|error| error.to_string())
    }

    fn eval<T: magnus::TryConvert>(
        ruby: &Ruby,
        source: &str,
        name: &str,
    ) -> Result<T, magnus::Error> {
        let wrapped = format!(
            "eval({}, TOPLEVEL_BINDING, {}, 1)",
            ruby_string_literal(source),
            ruby_string_literal(name)
        );
        ruby.eval(&wrapped)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn ruby_string_literal(value: &str) -> String {
    let mut hex = String::with_capacity(value.len() * 2);
    for byte in value.as_bytes() {
        use std::fmt::Write as _;
        write!(hex, "{byte:02x}").expect("writing to a String cannot fail");
    }
    format!("[\"{hex}\"].pack(\"H*\").force_encoding(Encoding::UTF_8)")
}

/// Entry point linked into a ruby.wasm interpreter by the companion gem.
///
/// This deliberately uses the same Magnus extension archive as the desktop
/// embed, with only the browser-safe, self-contained bindings installed.
/// `rbwasm` supplies the Ruby headers and statically links the archive; there
/// is no second Ruby-facing extension API for the browser build.
#[cfg(all(target_os = "wasi", feature = "ruby"))]
#[magnus::init(name = "fanplay_native")]
fn fanplay_native_init(ruby: &Ruby) -> Result<(), magnus::Error> {
    bindings::bind_wasm(ruby)?;
    Ok(())
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod web;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub use web::RubyCore;
