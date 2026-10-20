// SPDX-License-Identifier: GPL-2.0-or-later
//! Native RSXP runtime for cargo-mobile2.
//!
//! `cargo-mobile2` generates and builds the iOS and Android projects. All app
//! behavior lives in Rust: a tao window hosts a system WebView, the WebView
//! sends requests over wry IPC, and the existing RSXP engine plus native MRI
//! Ruby render straight into the shared UI.
mod app;
pub mod engine;
mod platform;

pub use app::run;
pub use engine::{configure, request};

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub fn main() {
    app::run();
}
