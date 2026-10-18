// SPDX-License-Identifier: GPL-2.0-or-later
//! Cross-platform media decoders shared by every frontend.
//!
//! [`gif`] decodes animated GIFs and [`movie`] decodes Theora (`.ogv`) movies,
//! both in pure Rust, so the same behavior is available on desktop, Android,
//! iOS and WebAssembly without system codecs. The engine only wires these
//! decoders to its bitmap resources and frame loop.
pub mod gif;
pub mod movie;
