// SPDX-License-Identifier: GPL-2.0-or-later
//! The RGSS-only embedded MRI runtime and its native engine bindings.
//!
//! Magnus is an implementation detail of this crate. Desktop code interacts
//! through [`RubyCore`].

use magnus::{embed::Cleanup, function, Ruby, Value};

mod bindings;
mod bridge;

/// An initialized MRI VM with all RGSS native bindings and scripts installed.
pub struct RubyCore {
    ruby: Cleanup,
    startup_status: Option<Result<i64, String>>,
}

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
        // The Windows dashboard uses stdout for framed runner IPC. Keep Ruby's
        // normal `puts`/`print` output on stderr so game scripts cannot corrupt
        // that byte stream; the dashboard already captures stderr as runtime logs.
        if std::env::var_os("RSXP_SESSION_STDIO").is_some() {
            Self::eval::<Value>(&ruby, "$stdout = $stderr", "<rsxp-session-stdio>")
                .map_err(|error| error.to_string())?;
        }
        if let Err(error) = Self::eval::<Value>(&ruby, include_str!("jit.rb"), "<rsxp-jit>") {
            eprintln!("Could not configure Ruby JIT: {error}");
        }
        bindings::bind(&ruby).map_err(|error| error.to_string())?;
        ruby.define_global_function("__rsxp_native", function!(bridge::native_call, 1))
            .map_err(|error| error.to_string())?;
        ruby.define_global_function("__mkxp_native", function!(bridge::native_call, 1))
            .map_err(|error| error.to_string())?;
        Ok(Self {
            ruby,
            startup_status: None,
        })
    }

    /// Evaluate the embedded RGSS shim and, for a game bundle, run the game.
    ///
    /// Returns the game's exit status. Calling this twice returns the first
    /// result without evaluating the scripts again.
    pub fn start(&mut self) -> Result<i64, String> {
        if let Some(status) = &self.startup_status {
            return status.clone();
        }
        let status = Self::eval::<i64>(&self.ruby, rsxp::ruby::RGSS_SOURCE, "rgss.rb")
            .map_err(|error| error.to_string());
        self.startup_status = Some(status.clone());
        status
    }

    /// Result code returned by the embedded RGSS game scripts.
    pub fn startup_status(&self) -> Result<i64, String> {
        match &self.startup_status {
            Some(status) => status.clone(),
            None => Err("Ruby core has not been started".into()),
        }
    }

    /// Evaluate source in the VM. The result is discarded at this boundary.
    pub fn inject_eval(&self, source: &str, name: &str) -> Result<(), String> {
        Self::eval::<Value>(&self.ruby, source, name)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    /// Evaluate an expression and extract an integer without exposing Magnus.
    pub fn extract_i64(&self, expression: &str) -> Result<i64, String> {
        self.eval_named(expression, "<rsxp-extract>")
    }

    /// Evaluate an expression and extract a UTF-8 string without exposing Magnus.
    pub fn extract_string(&self, expression: &str) -> Result<String, String> {
        self.eval_named(expression, "<rsxp-extract>")
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

fn ruby_string_literal(value: &str) -> String {
    let mut hex = String::with_capacity(value.len() * 2);
    for byte in value.as_bytes() {
        use std::fmt::Write as _;
        write!(hex, "{byte:02x}").expect("writing to a String cannot fail");
    }
    format!("[\"{hex}\"].pack(\"H*\").force_encoding(Encoding::UTF_8)")
}
