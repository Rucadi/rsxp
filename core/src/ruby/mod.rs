// SPDX-License-Identifier: GPL-2.0-or-later
//! Embedded MRI Ruby integration for the desktop runtime.
//!
//! This module owns the native boundary between the embedded Ruby VM and the
//! engine: the RGSS-facing Magnus bindings ([`bind`]), the serialized
//! `__rsxp_native` bridge and its profiling counters, and the optional Ruby
//! sampling profiler. Frontends that do not embed MRI (the browser, mobile) do
//! not enable the `ruby` feature.
mod bindings;
mod bridge;
pub mod profiling;

pub use bindings::bind;
pub use bridge::{
    direct_call, enabled, native_call, profile_stats, report, start,
};
