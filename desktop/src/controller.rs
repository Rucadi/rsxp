// SPDX-License-Identifier: GPL-2.0-or-later
//! Native gamepad input via `gilrs`, translated into the engine's SDL-style
//! key codes and merged into [`rsxp_core::input::Input`].
//!
//! [`Controllers`] owns the optional `gilrs` backend, the set of key codes it
//! currently holds down, and the on-disk control mapping. The mapping is
//! provided by the engine's `rsxp_core::controls` module: [`Controllers::new`]
//! decodes any saved layout (from `rsxp/controls` under `%APPDATA%` on Windows
//! or `$XDG_CONFIG_HOME`/`~/.config` elsewhere) and records an `encode()`d
//! snapshot for change detection. If no backend can be created it logs and
//! continues with keyboard-only input.
//!
//! Each [`Controllers::poll`] drains pending `gilrs` events, counts connected
//! gamepads, and (only while the window is focused) samples the standard
//! buttons plus the left stick against the mapping's deadzone. Button state is
//! passed to `controls::keys` and extended with `620 + i` pad keys, then
//! diffed against the previously held set: released keys get `key_up`, and all
//! held keys are re-asserted with `key_down` so gameplay input survives a
//! frame in which the UI suppressed input. When the encoded mapping changes it
//! is persisted back to the config file.
use gilrs::{Axis, Button, Gilrs};
use rsxp_core::{controls, input::Input};
use std::{collections::HashSet, path::PathBuf};

pub struct Controllers {
    backend: Option<Gilrs>,
    held: HashSet<i32>,
    path: Option<PathBuf>,
    saved: String,
}
impl Controllers {
    pub fn new() -> Self {
        let path = if cfg!(target_os = "windows") {
            std::env::var_os("APPDATA").map(PathBuf::from)
        } else {
            std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
        }
        .map(|p| p.join("rsxp/controls"));
        if let Some(path) = &path {
            if let Ok(text) = std::fs::read_to_string(path) {
                controls::decode(&text);
            }
        }
        let backend = Gilrs::new()
            .map_err(|error| eprintln!("Controller backend unavailable: {error}"))
            .ok();
        Self {
            backend,
            held: HashSet::new(),
            path,
            saved: controls::encode(),
        }
    }
    pub fn poll(&mut self, input: &mut Input, focused: bool) {
        let mut buttons = [false; 20];
        let mut mapping = controls::get();
        mapping.connected = 0;
        if let Some(backend) = &mut self.backend {
            while backend.next_event().is_some() {}
            for (_, pad) in backend.gamepads() {
                mapping.connected += 1;
                if !focused {
                    continue;
                }
                for (i, button) in [
                    Button::South,
                    Button::East,
                    Button::West,
                    Button::North,
                    Button::LeftTrigger,
                    Button::RightTrigger,
                    Button::LeftTrigger2,
                    Button::RightTrigger2,
                    Button::Select,
                    Button::Start,
                    Button::LeftThumb,
                    Button::RightThumb,
                    Button::DPadUp,
                    Button::DPadDown,
                    Button::DPadLeft,
                    Button::DPadRight,
                ]
                .iter()
                .enumerate()
                {
                    buttons[i] |= pad.is_pressed(*button);
                }
                let x = pad.value(Axis::LeftStickX);
                let y = pad.value(Axis::LeftStickY);
                buttons[16] |= x < -mapping.deadzone;
                buttons[17] |= x > mapping.deadzone;
                buttons[18] |= y > mapping.deadzone;
                buttons[19] |= y < -mapping.deadzone;
            }
        }
        let count = mapping.connected.min(16);
        controls::set(mapping);
        let mut keys = controls::keys(&buttons);
        keys.extend((0..count).map(|i| 620 + i as i32));
        for &key in self.held.difference(&keys) {
            input.key_up(key);
        }
        // Reassert held buttons after the workspace suppresses gameplay input.
        for &key in &keys {
            input.key_down(key);
        }
        self.held = keys;
        let encoded = controls::encode();
        if encoded != self.saved {
            if let Some(path) = &self.path {
                let result = std::fs::create_dir_all(path.parent().unwrap())
                    .and_then(|()| std::fs::write(path, &encoded));
                if let Err(error) = result {
                    eprintln!("Could not save controller mapping: {error}");
                }
            }
            self.saved = encoded;
        }
    }
}
