// SPDX-License-Identifier: GPL-2.0-or-later
//! Lock-free controller state and a bounded queue for discrete input events.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicUsize, Ordering};
use std::sync::Mutex;

/// Number of standard controller buttons exposed by the contract.
pub const BUTTON_COUNT: usize = 32;
/// Number of analog axes exposed by the contract (X/Y for four sticks).
pub const AXIS_COUNT: usize = 8;
/// Axis values are fixed-point in `[-32768, 32767]`.
pub const AXIS_MIN: i32 = i16::MIN as i32;
pub const AXIS_MAX: i32 = i16::MAX as i32;
const MAX_EVENTS: usize = 4096;

/// Standard button indices shared by every frontend and core.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Button {
    Up = 0,
    Down = 1,
    Left = 2,
    Right = 3,
    South = 4,
    East = 5,
    West = 6,
    North = 7,
    Start = 8,
    Select = 9,
    LeftShoulder = 10,
    RightShoulder = 11,
    LeftStick = 12,
    RightStick = 13,
    LeftTrigger = 14,
    RightTrigger = 15,
    /// Additional platform buttons start here. Higher values are core-defined.
    Extra0 = 16,
    Extra1 = 17,
    Extra2 = 18,
    Extra3 = 19,
}

impl Button {
    pub const fn mask(self) -> u32 {
        1 << (self as u32)
    }
}

/// Small, copyable button set used to submit a whole frame of input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ButtonSet(u32);

impl ButtonSet {
    pub const fn empty() -> Self {
        Self(0)
    }
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }
    pub const fn bits(self) -> u32 {
        self.0
    }
    pub fn set(&mut self, button: Button, pressed: bool) {
        self.set_bits(button.mask(), pressed);
    }
    pub fn set_bits(&mut self, mask: u32, pressed: bool) {
        if pressed {
            self.0 |= mask;
        } else {
            self.0 &= !mask;
        }
    }
    pub const fn contains(self, button: Button) -> bool {
        self.0 & button.mask() != 0
    }
    pub const fn any(self, mask: u32) -> bool {
        self.0 & mask != 0
    }
}

/// One discrete input change. These are low-frequency events; button and axis
/// state is read directly from [`InputState`] without allocating.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputEvent {
    Key {
        /// Platform-independent key code. Frontends and cores agree on the
        /// mapping; RGSS uses its existing scancode values here.
        code: u32,
        down: bool,
        repeat: bool,
    },
    Text(String),
    PointerMove {
        x: i32,
        y: i32,
        inside: bool,
    },
    PointerButton {
        button: u8,
        down: bool,
    },
    Wheel {
        x: i32,
        y: i32,
    },
    Focus(bool),
}

/// Copyable snapshot for cores that want a consistent view for one frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputSnapshot {
    pub buttons: ButtonSet,
    pub axes: [i32; AXIS_COUNT],
    pub pointer_x: i32,
    pub pointer_y: i32,
    pub pointer_inside: bool,
}

/// Input state written by the frontend and read by the core.
///
/// State fields are atomics: no lock is taken in the steady-state frame loop.
/// Only the discrete event queue uses a mutex, and it is drained once per
/// frame into a caller-owned buffer.
pub struct InputState {
    buttons: AtomicU32,
    axes: [AtomicI32; AXIS_COUNT],
    pointer_x: AtomicI32,
    pointer_y: AtomicI32,
    pointer_inside: AtomicBool,
    /// Fast-path gate so an idle frame never acquires the event mutex.
    event_count: AtomicUsize,
    events: Mutex<VecDeque<InputEvent>>,
}

impl Default for InputState {
    fn default() -> Self {
        Self::new()
    }
}

impl InputState {
    pub fn new() -> Self {
        Self {
            buttons: AtomicU32::new(0),
            axes: std::array::from_fn(|_| AtomicI32::new(0)),
            pointer_x: AtomicI32::new(0),
            pointer_y: AtomicI32::new(0),
            pointer_inside: AtomicBool::new(false),
            event_count: AtomicUsize::new(0),
            events: Mutex::new(VecDeque::new()),
        }
    }

    #[inline]
    pub fn set_button(&self, button: Button, pressed: bool) {
        self.set_button_bits(button.mask(), pressed);
    }

    #[inline]
    pub fn set_button_bits(&self, mask: u32, pressed: bool) {
        if pressed {
            self.buttons.fetch_or(mask, Ordering::Release);
        } else {
            self.buttons.fetch_and(!mask, Ordering::Release);
        }
    }

    #[inline]
    pub fn buttons(&self) -> ButtonSet {
        ButtonSet::from_bits(self.buttons.load(Ordering::Acquire))
    }

    #[inline]
    pub fn set_axis(&self, axis: usize, value: i32) {
        if let Some(slot) = self.axes.get(axis) {
            slot.store(value.clamp(AXIS_MIN, AXIS_MAX), Ordering::Release);
        }
    }

    #[inline]
    pub fn axis(&self, axis: usize) -> i32 {
        self.axes
            .get(axis)
            .map_or(0, |slot| slot.load(Ordering::Acquire))
    }

    #[inline]
    pub fn set_pointer(&self, x: i32, y: i32, inside: bool) {
        self.pointer_x.store(x, Ordering::Release);
        self.pointer_y.store(y, Ordering::Release);
        self.pointer_inside.store(inside, Ordering::Release);
    }

    #[inline]
    pub fn snapshot(&self) -> InputSnapshot {
        InputSnapshot {
            buttons: self.buttons(),
            axes: std::array::from_fn(|axis| self.axis(axis)),
            pointer_x: self.pointer_x.load(Ordering::Acquire),
            pointer_y: self.pointer_y.load(Ordering::Acquire),
            pointer_inside: self.pointer_inside.load(Ordering::Acquire),
        }
    }

    /// Queues a discrete event. Returns `false` if the bounded queue is full,
    /// which signals a stuck or runaway event producer.
    pub fn push_event(&self, event: InputEvent) -> bool {
        let mut events = self
            .events
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if events.len() >= MAX_EVENTS {
            return false;
        }
        events.push_back(event);
        self.event_count.store(events.len(), Ordering::Release);
        true
    }

    /// Drains queued events into `out`, preserving its allocation. The vector
    /// is not cleared first: callers may batch several sources into it.
    ///
    /// An empty queue costs one atomic load and no lock acquisition, so the
    /// steady-state frame loop stays lock-free.
    pub fn take_events(&self, out: &mut Vec<InputEvent>) {
        if self.event_count.load(Ordering::Acquire) == 0 {
            return;
        }
        let mut events = self
            .events
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        out.extend(events.drain(..));
        self.event_count.store(events.len(), Ordering::Release);
    }

    pub fn clear(&self) {
        self.buttons.store(0, Ordering::Release);
        for axis in &self.axes {
            axis.store(0, Ordering::Release);
        }
        let mut events = self
            .events
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        events.clear();
        self.event_count.store(0, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_and_axes_round_trip() {
        let input = InputState::new();
        input.set_button(Button::South, true);
        input.set_button(Button::Left, true);
        input.set_button(Button::Left, false);
        input.set_axis(0, 40000);
        let snapshot = input.snapshot();
        assert!(snapshot.buttons.contains(Button::South));
        assert!(!snapshot.buttons.contains(Button::Left));
        assert_eq!(snapshot.axes[0], AXIS_MAX);
    }

    #[test]
    fn events_are_bounded_and_drained() {
        let input = InputState::new();
        assert!(input.push_event(InputEvent::Focus(true)));
        let mut events = Vec::new();
        input.take_events(&mut events);
        assert_eq!(events.len(), 1);
        events.clear();
        input.take_events(&mut events);
        assert!(events.is_empty());
    }
}
