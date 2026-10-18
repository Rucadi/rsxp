// SPDX-License-Identifier: GPL-2.0-or-later
//! Magnus command callbacks and desktop-side bridge/frame timing.
//!
//! [`native_call`] is exposed to Ruby as `__rsxp_native` (and the legacy
//! `__mkxp_native`); it decodes a tab/newline-separated byte request, dispatches it
//! through [`engine::native_call_bytes`], and returns the reply as a Ruby string,
//! raising a `RuntimeError` on failure. After a successful `graphics_update` it also
//! advances the Ruby sampler via [`crate::profiling::frame`].
//!
//! Timing is opt-in. [`start`] reads `RSXP_PROFILE=1` into a global flag, while the
//! engine's own counter exposes a profiling flag and generation through
//! [`engine::profile_counter_state`]. [`direct_call`] wraps typed Magnus entry points
//! and attributes their whole duration to the native API category.
//!
//! Measurements feed two thread-local accumulators: `PROFILE`, drained by the shutdown
//! [`report`], and `FRAME_PROFILE`, which reconstructs ruby/bridge/native durations
//! across `graphics_update` boundaries into per-frame [`ProfileStats`] and publishes
//! the top eight hotspots over rolling five-second windows. [`profile_stats`] returns
//! the latest frame stats (or `None` when profiling is off) and [`report`] prints up to
//! twenty native operations ranked by total time.
//!
//! Ruby-side sampling itself lives in `crate::profiling`.
use magnus::{RString, Ruby};
use rsxp::{
    engine,
    host::{ProfileHotspot, ProfileStats},
};
use std::{
    cell::RefCell,
    collections::HashMap,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

static PROFILING: AtomicBool = AtomicBool::new(false);
#[derive(Default)]
struct FrameProfile {
    generation: u64,
    initialized: bool,
    last_graphics_start: Option<Instant>,
    callback_time: Duration,
    rust_time: Duration,
    graphics_time: Duration,
    calls: u32,
    latest: Option<ProfileStats>,
    hotspot_window_start: Option<Instant>,
    hotspot_published_at: Option<Instant>,
    hotspot_totals: HashMap<String, (u64, Duration)>,
    published_hotspots: Vec<ProfileHotspot>,
}

thread_local! {
    static PROFILE: RefCell<HashMap<String, (u64, Duration)>> = RefCell::new(HashMap::new());
    static FRAME_PROFILE: RefCell<FrameProfile> = RefCell::new(FrameProfile::default());
}

pub fn profile_stats() -> Option<ProfileStats> {
    if !engine::profile_counter_state().0 {
        return None;
    }
    FRAME_PROFILE.with(|profile| {
        let profile = profile.borrow();
        let mut latest = profile.latest.clone()?;
        latest.hotspot_age_secs = profile
            .hotspot_published_at
            .map_or(0.0, |at| at.elapsed().as_secs_f64());
        Some(latest)
    })
}

fn record_hotspot(operation: &[u8], elapsed: Duration) {
    let operation = std::str::from_utf8(operation).unwrap_or("<invalid>");
    FRAME_PROFILE.with(|slot| {
        let mut profile = slot.borrow_mut();
        if let Some(entry) = profile.hotspot_totals.get_mut(operation) {
            entry.0 += 1;
            entry.1 += elapsed;
        } else {
            profile
                .hotspot_totals
                .insert(operation.to_owned(), (1, elapsed));
        }
    });
}

fn record_native_profile(operation: &str, elapsed: Duration) {
    PROFILE.with(|slot| {
        let mut stats = slot.borrow_mut();
        if let Some(entry) = stats.get_mut(operation) {
            entry.0 += 1;
            entry.1 += elapsed;
        } else {
            stats.insert(operation.to_owned(), (1, elapsed));
        }
    });
}

/// Account for typed Magnus calls alongside the serialized bridge. All time
/// spent in the typed entry point is attributed to the native API category.
pub fn direct_call<T>(
    operation: &str,
    call: impl FnOnce() -> Result<T, magnus::Error>,
) -> Result<T, magnus::Error> {
    let (enabled, generation) = engine::profile_counter_state();
    let native = PROFILING.load(Ordering::Relaxed);
    let started = (enabled || native).then(Instant::now);
    let result = call();
    if let Some(started) = started {
        let elapsed = started.elapsed();
        if native {
            record_native_profile(operation, elapsed);
        }
        if enabled {
            record_frame_profile(operation.as_bytes(), generation, started, elapsed, elapsed);
            record_hotspot(operation.as_bytes(), elapsed);
        }
    }
    result
}

fn record_frame_profile(
    operation: &[u8],
    generation: u64,
    started: Instant,
    callback_time: Duration,
    rust_time: Duration,
) {
    FRAME_PROFILE.with(|slot| {
        let mut profile = slot.borrow_mut();
        if !profile.initialized || profile.generation != generation {
            *profile = FrameProfile {
                generation,
                initialized: true,
                ..FrameProfile::default()
            };
        }
        if operation == b"graphics_update" {
            let now = Instant::now();
            let window_start = profile.hotspot_window_start.get_or_insert(now);
            if now.duration_since(*window_start) >= Duration::from_secs(5) {
                let mut entries: Vec<_> = profile
                    .hotspot_totals
                    .drain()
                    .map(|(operation, (calls, elapsed))| ProfileHotspot {
                        operation,
                        calls,
                        total_ms: elapsed.as_secs_f64() * 1000.0,
                    })
                    .collect();
                entries.sort_by(|a, b| b.total_ms.total_cmp(&a.total_ms));
                entries.truncate(8);
                profile.published_hotspots = entries;
                profile.hotspot_published_at = Some(now);
                profile.hotspot_window_start = Some(now);
            }
            if let Some(last_start) = profile.last_graphics_start {
                let frame_time = started.saturating_duration_since(last_start);
                let ruby_time = frame_time.saturating_sub(profile.callback_time);
                profile.latest = Some(ProfileStats {
                    ruby_ms: ruby_time.as_secs_f64() * 1000.0,
                    bridge_ms: profile
                        .callback_time
                        .saturating_sub(profile.rust_time)
                        .as_secs_f64()
                        * 1000.0,
                    native_ms: profile
                        .rust_time
                        .saturating_sub(profile.graphics_time)
                        .as_secs_f64()
                        * 1000.0,
                    calls: profile.calls,
                    hotspots: profile.published_hotspots.clone(),
                    ..ProfileStats::default()
                });
            }
            profile.last_graphics_start = Some(started);
            profile.callback_time = Duration::ZERO;
            profile.rust_time = Duration::ZERO;
            profile.graphics_time = Duration::ZERO;
            profile.calls = 0;
        }
        profile.callback_time += callback_time;
        profile.rust_time += rust_time;
        if operation == b"graphics_update" {
            profile.graphics_time += rust_time;
        }
        profile.calls += 1;
    });
}

pub(super) fn native_call(ruby: &Ruby, request: RString) -> Result<Option<RString>, magnus::Error> {
    let (profile_enabled, generation) = engine::profile_counter_state();
    let frame_started = profile_enabled.then(Instant::now);
    // SAFETY: copy before calling back into Rust; the borrowed slice is never
    // used after this point.
    let bytes = unsafe { request.as_slice() }.to_vec();
    let start = PROFILING.load(Ordering::Relaxed).then(Instant::now);
    let rust_started = frame_started.map(|_| Instant::now());
    let result = engine::native_call_bytes(&bytes);
    let rust_time = rust_started.map(|started| started.elapsed());
    if let Some(start) = start {
        let elapsed = start.elapsed();
        let op = bytes
            .split(|b| *b == b'\t' || *b == b'\n')
            .next()
            .unwrap_or_default();
        record_native_profile(std::str::from_utf8(op).unwrap_or("<invalid>"), elapsed);
    }
    let operation = bytes
        .split(|byte| *byte == b'\t' || *byte == b'\n')
        .next()
        .unwrap_or_default();
    if operation == b"graphics_update" && matches!(&result, Ok(Some(_))) {
        crate::profiling::frame(ruby)?;
    }
    let ruby_result = match result {
        Ok(Some(bytes)) => Ok(Some(ruby.str_from_slice(&bytes))),
        Ok(None) => Ok(None),
        Err(message) => Err(magnus::Error::new(ruby.exception_runtime_error(), message)),
    };
    if let (Some(started), Some(rust_time)) = (frame_started, rust_time) {
        record_frame_profile(operation, generation, started, started.elapsed(), rust_time);
        // This command wraps the complete frame, including render and pacing;
        // ranking it beside leaf API commands would hide the actual hotspots.
        if operation != b"graphics_update" {
            record_hotspot(operation, rust_time);
        }
    }
    ruby_result
}

pub(super) fn start() {
    PROFILING.store(
        std::env::var("RSXP_PROFILE").is_ok_and(|v| v == "1"),
        Ordering::Relaxed,
    );
}

pub(super) fn enabled() -> bool {
    PROFILING.load(Ordering::Relaxed)
}

pub(super) fn report(elapsed: Duration) {
    if enabled() {
        let elapsed = elapsed.as_secs_f64();
        PROFILE.with(|profile| {
            let mut entries: Vec<_> = profile.borrow_mut().drain().collect();
            entries.sort_by_key(|(_, (_, time))| std::cmp::Reverse(*time));
            eprintln!("Native profile: {elapsed:.3}s including Ruby startup and scripts (not a GPU timer)");
            for (op, (calls, time)) in entries.iter().take(20) {
                eprintln!("  {op:24} {calls:9} calls {:10.3}ms", time.as_secs_f64()*1000.0);
            }
        });
    }
}
