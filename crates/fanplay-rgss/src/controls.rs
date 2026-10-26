// SPDX-License-Identifier: GPL-2.0-or-later
//! Standard gamepad mapping shared by both frontends.
//!
//! The engine models controller input as virtual scancodes so actions stay
//! independent of physical keyboard state. [`ACTIONS`] is the 13-entry RGSS
//! action list (index 0 is `Unbound`); [`BUTTONS`] is the 20 physical gamepad
//! inputs the UI offers. [`Mapping`] assigns each button an action index, plus a
//! stick deadzone and a connected-device count.
//!
//! [`keys`] turns a pressed-button array into a set of virtual scancodes
//! (`599 + action`) that the engine merges with keyboard input. [`encode`] /
//! [`decode`] persist a mapping as `deadzone;b0,b1,…,b19`; `decode` rejects a
//! non-finite or out-of-range deadzone and any malformed button list, leaving
//! the existing value unchanged. The active mapping lives in a thread-local.
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashSet},
};
pub const ACTIONS: [&str; 13] = [
    "Unbound",
    "Down",
    "Left",
    "Right",
    "Up",
    "A / Run",
    "B / Cancel",
    "C / Confirm",
    "X",
    "Y",
    "Z",
    "L",
    "R",
];
pub const BUTTONS: [&str; 20] = [
    "South (A / Cross)",
    "East (B / Circle)",
    "West (X / Square)",
    "North (Y / Triangle)",
    "Left shoulder",
    "Right shoulder",
    "Left trigger",
    "Right trigger",
    "Select",
    "Start",
    "Left stick click",
    "Right stick click",
    "D-pad up",
    "D-pad down",
    "D-pad left",
    "D-pad right",
    "Stick left",
    "Stick right",
    "Stick up",
    "Stick down",
];
#[derive(Clone)]
pub struct Mapping {
    pub buttons: [usize; 20],
    pub deadzone: f32,
    pub connected: usize,
}
impl Default for Mapping {
    fn default() -> Self {
        Self {
            buttons: [
                7, 6, 8, 9, 11, 12, 5, 10, 6, 7, 0, 0, 4, 1, 2, 3, 2, 3, 4, 1,
            ],
            deadzone: 0.25,
            connected: 0,
        }
    }
}
thread_local! { static MAPPING: RefCell<Mapping> = RefCell::new(Mapping::default()); }
thread_local! { static KEYBOARD: RefCell<BTreeMap<i32, i32>> = RefCell::new(BTreeMap::new()); }
pub fn get() -> Mapping {
    MAPPING.with(|m| m.borrow().clone())
}
pub fn set(mapping: Mapping) {
    MAPPING.with(|m| *m.borrow_mut() = mapping);
}
pub fn keyboard_binding(action: i32) -> i32 {
    KEYBOARD.with(|mapping| mapping.borrow().get(&action).copied().unwrap_or(action))
}
pub fn set_keyboard_binding(action: i32, physical: i32) {
    KEYBOARD.with(|mapping| {
        if action == physical {
            mapping.borrow_mut().remove(&action);
        } else {
            mapping.borrow_mut().insert(action, physical);
        }
    });
}
pub fn load_keyboard(bindings: &BTreeMap<i32, i32>) {
    KEYBOARD.with(|mapping| *mapping.borrow_mut() = bindings.clone());
}
pub fn keys(buttons: &[bool]) -> HashSet<i32> {
    let mapping = get();
    buttons
        .iter()
        .zip(mapping.buttons)
        .filter_map(|(&pressed, action)| {
            (pressed && action > 0 && action < ACTIONS.len()).then_some(599 + action as i32)
        })
        .collect()
}
pub fn encode() -> String {
    let m = get();
    format!(
        "{};{}",
        m.deadzone,
        m.buttons.map(|b| b.to_string()).join(",")
    )
}
pub fn decode(text: &str) -> bool {
    let Some((zone, buttons)) = text.split_once(';') else {
        return false;
    };
    let Ok(deadzone) = zone.parse::<f32>() else {
        return false;
    };
    let Ok(buttons) = buttons
        .split(',')
        .map(str::parse::<usize>)
        .collect::<Result<Vec<_>, _>>()
    else {
        return false;
    };
    if !deadzone.is_finite()
        || !(0.1..=0.9).contains(&deadzone)
        || buttons.len() != 20
        || buttons.iter().any(|&b| b >= ACTIONS.len())
    {
        return false;
    }
    set(Mapping {
        buttons: buttons.try_into().unwrap(),
        deadzone,
        connected: get().connected,
    });
    true
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mapping_roundtrip_and_validation() {
        set(Mapping::default());
        let original = encode();
        assert!(!decode("NaN;0"));
        assert_eq!(encode(), original);
        assert!(decode(&original));
        let mut buttons = [false; 20];
        buttons[0] = true;
        buttons[9] = true;
        assert_eq!(keys(&buttons), HashSet::from([606]));
        let mut mapping = get();
        mapping.buttons[0] = 6;
        set(mapping);
        assert_eq!(keys(&buttons), HashSet::from([605, 606]));
        assert!(!decode(&original.replace("0.25", "1.0")));
    }
}
