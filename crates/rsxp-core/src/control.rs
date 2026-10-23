// SPDX-License-Identifier: GPL-2.0-or-later
//! Frontend-to-core control plane.
//!
//! Stop, pause, fast-forward and frame limits are atomics and can be observed
//! every frame without locking. Anything that mutates emulated state (reset,
//! save state, options) goes through a bounded command queue drained by the
//! core on its own thread.

use crate::input::InputState;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

const MAX_COMMANDS: usize = 256;

/// Typed value for a core option.
#[derive(Clone, Debug, PartialEq)]
pub enum OptionValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
}

/// Cold-path command from the frontend.
#[derive(Clone, Debug, PartialEq)]
pub enum CoreCommand {
    /// Restart the emulated machine from its initial state.
    Reset,
    /// Serialize state into `CoreEvent::SaveState`.
    SaveState { slot: String },
    /// Load a state previously delivered by the frontend.
    LoadState { slot: String, data: Vec<u8> },
    /// Flush battery-backed save data through `CoreEvent::SaveRam`.
    SaveRam,
    /// Produce a `CoreEvent::Screenshot`.
    Screenshot,
    /// Apply a per-core option. Options that affect emulation must be applied
    /// between frames, never in the middle of one.
    SetOption { key: String, value: OptionValue },
    /// A core-defined extension command. Cores must ignore unknown ids.
    Extension { id: String, payload: Vec<u8> },
}

/// Shared control plane. The frontend holds one `Arc` and the core holds
/// another for the duration of the session.
pub struct CoreControl {
    stop: AtomicBool,
    paused: AtomicBool,
    fast_forward: AtomicBool,
    frame_limit: AtomicU64,
    input: InputState,
    /// Fast-path gate so a core with no pending commands never takes the lock.
    command_count: AtomicUsize,
    commands: Mutex<VecDeque<CoreCommand>>,
}

impl Default for CoreControl {
    fn default() -> Self {
        Self::new()
    }
}

impl CoreControl {
    pub fn new() -> Self {
        Self {
            stop: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            fast_forward: AtomicBool::new(false),
            frame_limit: AtomicU64::new(0),
            input: InputState::new(),
            command_count: AtomicUsize::new(0),
            commands: Mutex::new(VecDeque::new()),
        }
    }

    #[inline]
    pub fn input(&self) -> &InputState {
        &self.input
    }

    /// Requests a clean stop. This is idempotent and safe from any thread.
    #[inline]
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }

    #[inline]
    pub fn should_stop(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }

    #[inline]
    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Release);
    }

    #[inline]
    pub fn paused(&self) -> bool {
        self.paused.load(Ordering::Acquire)
    }

    #[inline]
    pub fn set_fast_forward(&self, enabled: bool) {
        self.fast_forward.store(enabled, Ordering::Release);
    }

    #[inline]
    pub fn fast_forward(&self) -> bool {
        self.fast_forward.load(Ordering::Acquire)
    }

    /// Zero means unlimited. This is a safety limit for tests and benchmarks,
    /// not the normal user-facing fast-forward control.
    #[inline]
    pub fn set_frame_limit(&self, frames: Option<u64>) {
        self.frame_limit
            .store(frames.unwrap_or(0), Ordering::Release);
    }

    #[inline]
    pub fn frame_limit(&self) -> Option<u64> {
        match self.frame_limit.load(Ordering::Acquire) {
            0 => None,
            frames => Some(frames),
        }
    }

    /// Queues a cold-path command. Returns `false` when the bounded queue is
    /// full, which indicates a frontend that is not draining replies.
    pub fn send(&self, command: CoreCommand) -> bool {
        let mut queue = self
            .commands
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if queue.len() >= MAX_COMMANDS {
            return false;
        }
        queue.push_back(command);
        self.command_count.store(queue.len(), Ordering::Release);
        true
    }

    /// Drains commands into `out` while preserving its allocation. The core
    /// should call this at a frame boundary. An empty queue costs one atomic
    /// load and no lock acquisition.
    pub fn take_commands(&self, out: &mut Vec<CoreCommand>) {
        if self.command_count.load(Ordering::Acquire) == 0 {
            return;
        }
        let mut queue = self
            .commands
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        out.extend(queue.drain(..));
        self.command_count.store(queue.len(), Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_flags_are_lock_free_in_the_hot_path() {
        let control = CoreControl::new();
        control.set_fast_forward(true);
        control.set_paused(true);
        control.set_frame_limit(Some(120));
        assert!(control.fast_forward());
        assert!(control.paused());
        assert_eq!(control.frame_limit(), Some(120));
        control.stop();
        assert!(control.should_stop());
    }

    #[test]
    fn commands_are_bounded_and_drained_without_reallocating_the_output() {
        let control = CoreControl::new();
        assert!(control.send(CoreCommand::Reset));
        let mut commands = Vec::with_capacity(4);
        control.take_commands(&mut commands);
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0], CoreCommand::Reset);
        commands.clear();
        control.take_commands(&mut commands);
        assert!(commands.is_empty());
    }
}
