// SPDX-License-Identifier: GPL-2.0-or-later
//! Aggregate physical keys and independent touch/mouse owners. Releasing one
//! source must not release a button still held by another source.
use rsxp::host::InputEvent;
use std::collections::{BTreeSet, HashMap};

#[derive(Default)]
pub struct Controls {
    keyboard: BTreeSet<i32>,
    pointers: HashMap<u64, i32>,
    sent: BTreeSet<i32>,
    pub regions: Vec<(egui::Rect, i32)>,
}
impl Controls {
    pub fn key(&mut self, code: i32, down: bool) {
        if down {
            self.keyboard.insert(code);
        } else {
            self.keyboard.remove(&code);
        }
    }
    pub fn pointer(&mut self, id: u64, position: Option<egui::Pos2>) {
        self.pointers.remove(&id);
        if let Some(code) = position.and_then(|p| {
            self.regions
                .iter()
                .find(|(rect, _)| rect.contains(p))
                .map(|(_, code)| *code)
        }) {
            self.pointers.insert(id, code);
        }
    }
    pub fn held(&self, code: i32) -> bool {
        self.keyboard.contains(&code) || self.pointers.values().any(|held| *held == code)
    }
    pub fn clear(&mut self) {
        self.keyboard.clear();
        self.pointers.clear();
    }
    pub fn drain(&mut self) -> Vec<InputEvent> {
        let held: BTreeSet<_> = self
            .keyboard
            .iter()
            .copied()
            .chain(self.pointers.values().copied())
            .collect();
        let mut events = Vec::new();
        events.extend(self.sent.difference(&held).copied().map(InputEvent::KeyUp));
        events.extend(
            held.difference(&self.sent)
                .copied()
                .map(InputEvent::KeyDown),
        );
        self.sent = held;
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multiple_sources_slide_cancel_and_focus_loss() {
        let mut input = Controls::default();
        input.regions = vec![
            (
                egui::Rect::from_min_size(egui::pos2(0., 0.), egui::vec2(40., 40.)),
                79,
            ),
            (
                egui::Rect::from_min_size(egui::pos2(40., 0.), egui::vec2(40., 40.)),
                40,
            ),
        ];
        input.pointer(1, Some(egui::pos2(10., 10.)));
        input.pointer(2, Some(egui::pos2(10., 10.)));
        assert!(matches!(
            input.drain().as_slice(),
            [InputEvent::KeyDown(79)]
        ));
        input.key(79, true);
        input.pointer(1, None);
        assert!(input.drain().is_empty());
        input.pointer(2, Some(egui::pos2(50., 10.)));
        assert!(matches!(
            input.drain().as_slice(),
            [InputEvent::KeyDown(40)]
        ));
        input.key(79, false);
        assert!(matches!(input.drain().as_slice(), [InputEvent::KeyUp(79)]));
        input.clear();
        assert!(matches!(input.drain().as_slice(), [InputEvent::KeyUp(40)]));
        assert!(input.drain().is_empty());
    }
}
