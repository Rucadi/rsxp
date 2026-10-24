// SPDX-License-Identifier: GPL-2.0-or-later
//! Frontend-to-core control plane.
//!
//! Stop, pause, fast-forward and frame limits are atomics and can be observed
//! every frame without locking. Anything that mutates emulated state (reset,
//! save state, options) goes through a bounded command queue drained by the
//! core on its own thread.
//!
//! Every queued command carries a request id so the core can report a
//! [`CoreEvent::CommandResult`](crate::CoreEvent::CommandResult) even when it
//! does not implement the command. The outcome is a cold-path event; it never
//! enters the frame loop.

use crate::events::CoreEvent;
use crate::input::InputState;
use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

const MAX_COMMANDS: usize = 256;
/// Upper bound for a command payload crossing the control plane. State loads
/// and extension commands larger than this are rejected before queueing.
pub const MAX_COMMAND_PAYLOAD: usize = 64 * 1024 * 1024;

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

impl CoreCommand {
    /// Number of payload bytes carried by the command.
    pub fn payload_len(&self) -> usize {
        match self {
            Self::LoadState { data, .. } => data.len(),
            Self::Extension { payload, .. } => payload.len(),
            Self::SetOption { key, value } => {
                key.len()
                    + match value {
                        OptionValue::String(value) => value.len(),
                        _ => 0,
                    }
            }
            _ => 0,
        }
    }
}

/// A queued command and its frontend-assigned request id.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandRequest {
    pub id: u64,
    /// Session generation at the time the frontend queued the request.
    pub generation: u64,
    pub command: CoreCommand,
}

/// Why a command could not be queued.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandError {
    QueueFull,
    PayloadTooLarge { bytes: usize, max: usize },
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QueueFull => f.write_str("core command queue is full"),
            Self::PayloadTooLarge { bytes, max } => {
                write!(f, "core command payload is {bytes} bytes (max {max})")
            }
        }
    }
}

impl std::error::Error for CommandError {}

/// Result a core reports for one command request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandOutcome {
    Succeeded,
    Unsupported,
    Failed(String),
}

impl CommandOutcome {
    pub fn succeeded(&self) -> bool {
        matches!(self, Self::Succeeded)
    }
}

/// Shared control plane. The frontend holds one `Arc` and the core holds
/// another for the duration of the session.
pub struct CoreControl {
    stop: AtomicBool,
    paused: AtomicBool,
    fast_forward: AtomicBool,
    frame_limit: AtomicU64,
    input: InputState,
    next_request: AtomicU64,
    generation: AtomicU64,
    /// Fast-path gate so a core with no pending commands never takes the lock.
    command_count: AtomicUsize,
    commands: Mutex<VecDeque<CommandRequest>>,
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
            next_request: AtomicU64::new(0),
            generation: AtomicU64::new(1),
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

    /// Queues a command, returning a request id that the core echoes in
    /// [`CoreEvent::CommandResult`].
    pub fn send_request(&self, command: CoreCommand) -> Result<u64, CommandError> {
        let bytes = command.payload_len();
        if bytes > MAX_COMMAND_PAYLOAD {
            return Err(CommandError::PayloadTooLarge {
                bytes,
                max: MAX_COMMAND_PAYLOAD,
            });
        }
        let mut queue = self
            .commands
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if queue.len() >= MAX_COMMANDS {
            return Err(CommandError::QueueFull);
        }
        let id = self.next_request.fetch_add(1, Ordering::Relaxed) + 1;
        let generation = self.generation.load(Ordering::Acquire);
        queue.push_back(CommandRequest {
            id,
            generation,
            command,
        });
        self.command_count.store(queue.len(), Ordering::Release);
        Ok(id)
    }

    /// Returns the generation attached to newly queued requests.
    pub fn session_generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Starts a new logical game session and drops commands queued for the old
    /// session. Replies from the old generation remain distinguishable at the
    /// event boundary and can be safely ignored by adapters.
    pub fn begin_session(&self) -> u64 {
        let generation = self
            .generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1)
            .max(1);
        let mut queue = self
            .commands
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        queue.clear();
        self.command_count.store(0, Ordering::Release);
        generation
    }

    /// Queues a command, returning `false` when it is rejected. Prefer
    /// [`send_request`](Self::send_request) when the caller needs the id or the
    /// reason for rejection.
    pub fn send(&self, command: CoreCommand) -> bool {
        self.send_request(command).is_ok()
    }

    /// Emits a command outcome through `events` without requiring the core to
    /// know the frontend's event sink type.
    pub fn report(
        &self,
        events: &mut dyn crate::events::EventSink,
        id: u64,
        outcome: CommandOutcome,
    ) {
        self.report_with_response(events, id, outcome, None);
    }

    /// Emits a command outcome and an optional bounded response payload.
    pub fn report_with_response(
        &self,
        events: &mut dyn crate::events::EventSink,
        id: u64,
        outcome: CommandOutcome,
        response: Option<Vec<u8>>,
    ) {
        if response
            .as_ref()
            .is_some_and(|payload| payload.len() > MAX_COMMAND_PAYLOAD)
        {
            events.emit(CoreEvent::CommandResult {
                id,
                generation: self.session_generation(),
                outcome: CommandOutcome::Failed("command response payload is too large".into()),
                response: None,
            });
            return;
        }
        events.emit(CoreEvent::CommandResult {
            id,
            generation: self.session_generation(),
            outcome,
            response,
        });
    }

    /// Drains commands with their request ids while preserving `out`'s
    /// allocation. An empty queue costs one atomic load and no lock.
    pub fn take_requests(&self, out: &mut Vec<CommandRequest>) {
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

    /// Drains commands without request ids. Cores that can report outcomes
    /// should use [`take_requests`](Self::take_requests) instead.
    pub fn take_commands(&self, out: &mut Vec<CoreCommand>) {
        if self.command_count.load(Ordering::Acquire) == 0 {
            return;
        }
        let mut queue = self
            .commands
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        out.extend(queue.drain(..).map(|request| request.command));
        self.command_count.store(queue.len(), Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventSink;

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
        let id = control.send_request(CoreCommand::Reset).unwrap();
        assert_eq!(id, 1);
        let mut requests = Vec::with_capacity(4);
        control.take_requests(&mut requests);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].id, 1);
        assert_eq!(requests[0].generation, control.session_generation());
        assert_eq!(requests[0].command, CoreCommand::Reset);
        requests.clear();
        control.take_requests(&mut requests);
        assert!(requests.is_empty());
    }

    #[test]
    fn oversized_payloads_are_rejected_before_queueing() {
        let control = CoreControl::new();
        let error = control
            .send_request(CoreCommand::LoadState {
                slot: "1".into(),
                data: vec![0; MAX_COMMAND_PAYLOAD + 1],
            })
            .unwrap_err();
        assert!(matches!(error, CommandError::PayloadTooLarge { .. }));
        let mut commands = Vec::new();
        control.take_commands(&mut commands);
        assert!(commands.is_empty());
    }

    #[test]
    fn command_responses_are_bounded_and_optional() {
        let control = CoreControl::new();
        let mut events = Vec::new();
        struct Sink<'a>(&'a mut Vec<CoreEvent>);
        impl EventSink for Sink<'_> {
            fn emit(&mut self, event: CoreEvent) {
                self.0.push(event);
            }
        }
        control.report_with_response(
            &mut Sink(&mut events),
            7,
            CommandOutcome::Succeeded,
            Some(b"{\"ok\":true}".to_vec()),
        );
        assert!(matches!(
            events.pop(),
            Some(CoreEvent::CommandResult {
                id: 7,
                response: Some(_),
                ..
            })
        ));
    }
}
