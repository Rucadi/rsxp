// SPDX-License-Identifier: GPL-2.0-or-later
//! Live engine settings a host UI can change while the game runs.
//!
//! These cells are local to the runtime's owning thread. The cooperative browser
//! adapter uses them directly. The native dashboard has its own settings mirror
//! and relays changes across the isolated session transport; it never accesses
//! the game thread's cells directly.

use std::cell::Cell;

thread_local! {
    /// Frame-rate cap chosen in the UI; `None` keeps the game's own rate.
    static RATE_OVERRIDE: Cell<Option<u32>> = const { Cell::new(None) };
    /// Effective frame rate, written by the engine for the UI to display.
    static CURRENT_RATE: Cell<u32> = const { Cell::new(0) };
    /// Frame pacing disabled ("uncapped"/fast-forward).
    static FAST_FORWARD: Cell<bool> = const { Cell::new(false) };
    /// A reset (restart) of the whole game has been requested.
    static RESET: Cell<bool> = const { Cell::new(false) };
}

/// The frame-rate cap requested by the UI, if any.
pub fn rate_override() -> Option<u32> {
    RATE_OVERRIDE.with(Cell::get)
}

/// Sets the frame-rate cap (1..=1000) requested by the UI.
pub fn set_rate_override(rate: u32) {
    RATE_OVERRIDE.with(|cell| cell.set(Some(rate.clamp(1, 1000))));
}

/// Clears the UI rate override, returning to the game's own rate.
pub fn clear_rate_override() {
    RATE_OVERRIDE.with(|cell| cell.set(None));
}

/// The frame rate currently in effect, as last reported by the engine.
pub fn current_rate() -> u32 {
    CURRENT_RATE.with(Cell::get)
}

/// Records the effective frame rate for the UI. Called by the engine.
pub fn set_current_rate(rate: u32) {
    CURRENT_RATE.with(|cell| cell.set(rate));
}

/// Whether frame pacing is disabled.
pub fn fast_forward() -> bool {
    FAST_FORWARD.with(Cell::get)
}

/// Enables or disables frame pacing.
pub fn set_fast_forward(enabled: bool) {
    FAST_FORWARD.with(|cell| cell.set(enabled));
}

/// Flips frame pacing and returns the new value.
pub fn toggle_fast_forward() -> bool {
    let value = !fast_forward();
    set_fast_forward(value);
    value
}

/// Asks the host to restart the game.
pub fn request_reset() {
    RESET.with(|cell| cell.set(true));
}

/// Whether a reset was requested.
pub fn reset_requested() -> bool {
    RESET.with(Cell::get)
}

/// Clears a pending reset request.
pub fn clear_reset() {
    RESET.with(|cell| cell.set(false));
}

/// Resets every setting to its startup default. Used when a run begins.
pub fn reset() {
    clear_rate_override();
    set_current_rate(0);
    set_fast_forward(false);
    clear_reset();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_are_clamped_and_reset() {
        reset();
        assert_eq!(rate_override(), None);
        assert_eq!(current_rate(), 0);
        assert!(!fast_forward());
        assert!(!reset_requested());
        set_rate_override(5000);
        assert_eq!(rate_override(), Some(1000));
        set_current_rate(60);
        assert_eq!(current_rate(), 60);
        assert!(toggle_fast_forward());
        assert!(fast_forward());
        request_reset();
        assert!(reset_requested());
        reset();
        assert_eq!(rate_override(), None);
        assert_eq!(current_rate(), 0);
        assert!(!fast_forward());
        assert!(!reset_requested());
    }
}
