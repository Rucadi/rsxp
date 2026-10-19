// SPDX-License-Identifier: GPL-2.0-or-later
//! Frame lifecycle: input, composition, presentation, profiling and pacing.
//!
//! `update_frame` is one RGSS `Graphics.update`. In order it: stops when the
//! frame limit is reached, applies live rate/fast-forward settings from the
//! workspace UI, flushes queued host events into the pending `Input`, advances
//! animated GIF bitmaps (`advance_animations`, host-clock driven), polls the
//! host (unless headless), injects scripted headless input, handles the F10
//! frame-counter and F11 fast-forward keys (scancodes 67/68, unmapped by RGSS),
//! renders via GPU when available and otherwise software, syncs profiler state,
//! then sleeps until the next deadline.
//!
//! Pacing budgets time from the previous frame start so Ruby work between
//! updates counts; an overloaded frame reschedules instead of accumulating lag.
//! `counter_bitmap` builds the optional FPS/profile overlay.
use super::{Runtime, PROFILE_COUNTER_ACTIVE, PROFILE_COUNTER_GENERATION};
use crate::graphics::{Bitmap, Color, Rect, SpriteFx};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

impl Runtime {
    /// Advances animated GIF bitmaps using the host clock. Static bitmaps and
    /// hosts without a clock cost nothing here.
    pub(super) fn advance_animations(&mut self) {
        let now = self.host.now();
        if now <= 0.0 {
            return;
        }
        let dt = if self.animation_time > 0.0 {
            (now - self.animation_time).clamp(0.0, 0.25)
        } else {
            0.0
        };
        self.animation_time = now;
        if dt <= 0.0 {
            return;
        }
        for bitmap in self.bitmaps.values_mut() {
            bitmap.advance_animation(dt);
        }
    }
    pub(super) fn update_frame(&mut self) -> Result<(), String> {
        let frame_start = self.host.now();
        if self.frame_limit.is_some_and(|n| self.updates >= n) {
            return Err("__RSXP_STOP__".into());
        }
        let profile_start = self.show_counter.then(Instant::now);
        if profile_start.is_some() {
            self.profile_present_ms = 0.0;
        }
        // Apply live settings from the workspace UI. The rate override is
        // optional (the game keeps its own rate until the UI changes it), while
        // fast-forward is authoritative here so the UI and F11 stay in sync.
        if let Some(rate) = crate::runtime_settings::rate_override()
            && self.rate != rate
        {
            self.rate = rate;
            self.next_frame_deadline = None;
        }
        crate::runtime_settings::set_current_rate(self.rate);
        self.fast_forward = crate::runtime_settings::fast_forward();
        self.pending.begin_frame();
        self.apply_queued();
        self.advance_animations();
        if !self.headless && !self.host.poll(&mut self.pending) {
            return Err("__RSXP_STOP__".into());
        }
        self.inject_scripted();
        // F10 toggles the on-screen frame counter; F11 toggles fast-forward
        // (disables the RGSS frame-rate cap). Neither key is mapped by RGSS.
        const COUNTER_KEY: i32 = 67;
        const FAST_FORWARD_KEY: i32 = 68;
        if self.pending.triggered(COUNTER_KEY) {
            self.show_counter = !self.show_counter;
            PROFILE_COUNTER_ACTIVE.store(self.show_counter, Ordering::Relaxed);
            PROFILE_COUNTER_GENERATION.fetch_add(1, Ordering::Relaxed);
        }
        if self.pending.triggered(FAST_FORWARD_KEY) {
            self.fast_forward = crate::runtime_settings::toggle_fast_forward();
        }
        #[cfg(feature = "gpu")]
        let gpu_drawn = self.render_gpu()?;
        #[cfg(not(feature = "gpu"))]
        let gpu_drawn = false;
        if !gpu_drawn {
            self.render_software()?;
        }
        if let Some(start) = profile_start {
            self.profile_render_ms =
                (start.elapsed().as_secs_f64() * 1000.0 - self.profile_present_ms).max(0.0);
            self.host
                .set_frame_profile(self.profile_render_ms, self.profile_present_ms);
        }
        if let Some(enabled) = self.host.profiler_enabled()
            && enabled != self.show_counter
        {
            self.show_counter = enabled;
            PROFILE_COUNTER_ACTIVE.store(enabled, Ordering::Relaxed);
            PROFILE_COUNTER_GENERATION.fetch_add(1, Ordering::Relaxed);
        }
        self.frame += 1;
        self.updates += 1;
        if !self.headless && !self.fast_forward {
            let period = 1.0 / f64::from(self.rate);
            let deadline = self.next_frame_deadline.unwrap_or(frame_start + period);
            let now = self.host.now();
            self.host
                .sleep(Duration::from_secs_f64((deadline - now).max(0.0)));
            // Include Ruby work between Graphics updates in the same budget.
            // When overloaded, reset the schedule instead of accumulating lag.
            self.next_frame_deadline = Some(deadline.max(now) + period);
        } else {
            self.next_frame_deadline = None;
        }
        Ok(())
    }
    /// Draws the optional frame counter overlay in the top-left corner.
    pub(super) fn counter_bitmap(&mut self) -> Option<Bitmap> {
        let now = self.host.now();
        if now > 0.0 {
            if self.fps_window_start == 0.0 {
                self.fps_window_start = now;
                self.fps_window_frames = 0;
            }
            self.fps_window_frames += 1;
            let elapsed = now - self.fps_window_start;
            if elapsed >= 0.5 {
                self.fps = f64::from(self.fps_window_frames) / elapsed;
                self.fps_window_start = now;
                self.fps_window_frames = 0;
            }
        }
        let profile = self.host.profile_stats();
        let label = if let Some(profile) = profile {
            format!(
                "{:.0}fps Ruby {:.1}ms Bridge {:.1}ms API {:.1}ms Render {:.1}ms Present {:.1}ms Calls {}{}",
                self.fps,
                profile.ruby_ms,
                profile.bridge_ms,
                profile.native_ms,
                self.profile_render_ms,
                self.profile_present_ms,
                profile.calls,
                if self.fast_forward { " FAST" } else { "" }
            )
        } else if now > 0.0 {
            format!(
                "{:.0} fps   frame {}{}",
                self.fps,
                self.frame,
                if self.fast_forward { "   [fast]" } else { "" }
            )
        } else {
            format!(
                "frame {}{}",
                self.frame,
                if self.fast_forward { "   [fast]" } else { "" }
            )
        };
        let rendered = self.host.text(
            None,
            10,
            false,
            false,
            &label,
            Color::rgb(255, 255, 255),
            true,
        );
        if let Ok((width, height, Some(bitmap))) = rendered {
            let mut overlay = Bitmap::new(width + 8, height + 6).ok()?;
            overlay.fill_rect(
                Rect::new(2, 2, width + 4, height + 2),
                Color::rgba(0, 0, 0, 170),
            );
            let rect = overlay.rect();
            overlay.draw_sprite(
                Rect::new(4, 3, width, height),
                &bitmap,
                bitmap.rect(),
                rect,
                &SpriteFx::default(),
            );
            return Some(overlay);
        }
        None
    }
}
