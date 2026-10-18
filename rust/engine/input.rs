// SPDX-License-Identifier: GPL-2.0-or-later
use std::collections::{HashMap, HashSet};

/// SDL scancodes are independent of the user's keyboard layout.
#[derive(Clone, Default)]
pub struct Input {
    pub mouse_x: i32,
    pub mouse_y: i32,
    pub mouse_inside: bool,
    pub scroll_v: i32,
    /// Text typed while OS text input is enabled (see `Input.gets`).
    pub text: String,
    held: HashSet<i32>,
    triggered: HashSet<i32>,
    released: HashSet<i32>,
    held_frames: HashMap<i32, u32>,
    repeated: HashSet<i32>,
}

impl Input {
    pub fn begin_frame(&mut self) {
        self.scroll_v = 0;
        self.triggered.clear();
        self.released.clear();
    }
    pub fn push_text(&mut self, text: &str) {
        self.text.push_str(text);
    }
    /// Copies the per-frame button state from another Input, preserving this
    /// instance's repeat counters (which are not reset by `begin_frame`).
    pub fn copy_state_from(&mut self, other: &Input) {
        self.mouse_x = other.mouse_x;
        self.mouse_y = other.mouse_y;
        self.mouse_inside = other.mouse_inside;
        self.scroll_v = other.scroll_v;
        self.held = other.held.clone();
        self.triggered = other.triggered.clone();
        self.released = other.released.clone();
    }
    /// Advances key-repeat tracking by one update. `repeat_start` is the number
    /// of held updates before repeating, `repeat_delay` the period after that.
    pub fn advance(&mut self, repeat_start: u32, repeat_delay: u32) {
        self.repeated.clear();
        let held: Vec<i32> = self.held.iter().copied().collect();
        self.held_frames.retain(|key, _| held.contains(key));
        for key in held {
            let count = self.held_frames.entry(key).or_insert(0);
            *count += 1;
            if *count >= repeat_start && (*count + 1).is_multiple_of(repeat_delay) {
                self.repeated.insert(key);
            }
        }
    }
    pub fn key_down(&mut self, key: i32) {
        if self.held.insert(key) {
            self.triggered.insert(key);
        }
    }
    pub fn key_up(&mut self, key: i32) {
        if self.held.remove(&key) {
            self.released.insert(key);
        }
    }
    pub fn release_all(&mut self) {
        self.released.extend(self.held.drain());
    }
    pub fn pressed(&self, key: i32) -> bool {
        self.held.contains(&key)
    }
    pub fn triggered(&self, key: i32) -> bool {
        self.triggered.contains(&key)
    }
    pub fn released(&self, key: i32) -> bool {
        self.released.contains(&key)
    }
    pub fn repeated(&self, key: i32) -> bool {
        self.repeated.contains(&key)
    }
    /// Sparse button snapshot: signed scancode followed by press/trigger/
    /// release/repeat flags. One bridge call replaces per-button queries.
    pub fn snapshot(&self) -> Vec<u8> {
        let keys: HashSet<_> = self
            .held
            .iter()
            .chain(&self.triggered)
            .chain(&self.released)
            .chain(&self.repeated)
            .copied()
            .collect();
        let mut bytes = Vec::with_capacity(keys.len() * 5);
        for key in keys {
            bytes.extend_from_slice(&key.to_le_bytes());
            bytes.push(
                u8::from(self.pressed(key))
                    | (u8::from(self.triggered(key)) << 1)
                    | (u8::from(self.released(key)) << 2)
                    | (u8::from(self.repeated(key)) << 3),
            );
        }
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edges_repeats_and_focus_loss() {
        let mut input = Input::default();
        input.key_down(4);
        assert!(input.triggered(4));
        input.begin_frame();
        input.key_down(4);
        assert!(input.pressed(4));
        assert!(!input.triggered(4));
        input.release_all();
        assert!(!input.pressed(4));
        assert!(input.released(4));
        input.begin_frame();
        assert!(!input.released(4));
    }
}
