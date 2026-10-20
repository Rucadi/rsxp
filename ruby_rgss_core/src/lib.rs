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
    startup_status: Result<i64, String>,
}

impl RubyCore {
    /// Create an RGSS VM, install bindings, then evaluate the embedded RGSS core.
    pub fn new() -> Result<Self, String> {
        // SAFETY: one VM is initialized on the desktop main thread per process.
        let ruby = unsafe { magnus::embed::init() };
        // The Windows dashboard uses stdout for framed runner IPC. Keep Ruby's
        // normal `puts`/`print` output on stderr so game scripts cannot corrupt
        // that byte stream; the dashboard already captures stderr as runtime logs.
        if std::env::var_os("RSXP_SESSION_STDIO").is_some() {
            Self::eval::<Value>(&ruby, "$stdout = $stderr", "<rsxp-session-stdio>")
                .map_err(|error| error.to_string())?;
        }
        let _ = Self::eval::<Value>(
            &ruby,
            "if ENV.fetch('RSXP_YJIT', '1') != '0'; if defined?(RubyVM::ZJIT); RubyVM::ZJIT.enable; elsif defined?(RubyVM::YJIT); RubyVM::YJIT.enable; end; end",
            "<rsxp-jit>",
        );
        bindings::bind(&ruby).map_err(|error| error.to_string())?;
        ruby.define_global_function("__rsxp_native", function!(bridge::native_call, 1))
            .map_err(|error| error.to_string())?;
        ruby.define_global_function("__mkxp_native", function!(bridge::native_call, 1))
            .map_err(|error| error.to_string())?;
        let startup_status = Self::eval::<i64>(&ruby, rsxp::ruby::RGSS_SOURCE, "rgss.rb")
            .map_err(|error| error.to_string());
        Ok(Self {
            ruby,
            startup_status,
        })
    }

    /// Result code returned by the embedded RGSS game scripts.
    pub fn startup_status(&self) -> Result<i64, String> {
        self.startup_status.clone()
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
