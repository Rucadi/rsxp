// SPDX-License-Identifier: GPL-2.0-or-later
//! RGSS keyboard routing, independent of cartridge controller bindings.
use egui::{Event, ImeEvent, Key, Modifiers};
use fanplay_core::{CoreControl, InputEvent};
use std::collections::BTreeSet;

#[derive(Default)]
pub struct GameKeyboard {
    held: BTreeSet<u32>,
}
impl GameKeyboard {
    fn key(&mut self, control: &CoreControl, code: u32, down: bool) {
        let changed = if down {
            self.held.insert(code)
        } else {
            self.held.remove(&code)
        };
        if changed {
            let _ = control.input().push_event(InputEvent::Key {
                code,
                down,
                repeat: false,
            });
        }
    }
    fn modifiers(&mut self, control: &CoreControl, modifiers: Modifiers) {
        for (code, down) in [
            (225, modifiers.shift),
            (224, modifiers.ctrl),
            (226, modifiers.alt),
        ] {
            self.key(control, code, down);
        }
    }
    pub fn release(&mut self, control: &CoreControl) {
        for code in std::mem::take(&mut self.held) {
            let _ = control.input().push_event(InputEvent::Key {
                code,
                down: false,
                repeat: false,
            });
        }
    }
    pub fn route(
        &mut self,
        control: &CoreControl,
        events: &[Event],
        modifiers: Modifiers,
        allowed: bool,
        text_input: bool,
    ) {
        if !allowed {
            self.release(control);
            return;
        }
        for event in events {
            match event {
                Event::Key {
                    key,
                    physical_key,
                    pressed,
                    repeat,
                    modifiers,
                } => {
                    self.modifiers(control, *modifiers);
                    if !repeat && let Some(code) = scancode(physical_key.unwrap_or(*key)) {
                        self.key(control, code, *pressed);
                    }
                }
                Event::Text(text) | Event::Paste(text) | Event::Ime(ImeEvent::Commit(text))
                    if text_input =>
                {
                    let _ = control.input().push_event(InputEvent::Text(text.clone()));
                }
                Event::WindowFocused(false) => {
                    self.release(control);
                    return;
                }
                _ => {}
            }
        }
        self.modifiers(control, modifiers);
    }
}

pub fn game_event(event: &Event) -> bool {
    match event {
        Event::Key {
            key, physical_key, ..
        } => scancode(physical_key.unwrap_or(*key)).is_some(),
        Event::Text(_) | Event::Paste(_) | Event::Ime(_) => true,
        _ => false,
    }
}

pub fn scancode(key: Key) -> Option<u32> {
    use Key::*;
    Some(match key {
        A => 4,
        B => 5,
        C => 6,
        D => 7,
        E => 8,
        F => 9,
        G => 10,
        H => 11,
        I => 12,
        J => 13,
        K => 14,
        L => 15,
        M => 16,
        N => 17,
        O => 18,
        P => 19,
        Q => 20,
        R => 21,
        S => 22,
        T => 23,
        U => 24,
        V => 25,
        W => 26,
        X => 27,
        Y => 28,
        Z => 29,
        Num1 => 30,
        Num2 => 31,
        Num3 => 32,
        Num4 => 33,
        Num5 => 34,
        Num6 => 35,
        Num7 => 36,
        Num8 => 37,
        Num9 => 38,
        Num0 => 39,
        Enter => 40,
        Escape => 41,
        Backspace => 42,
        Tab => 43,
        Space => 44,
        Minus => 45,
        Equals => 46,
        OpenBracket => 47,
        CloseBracket => 48,
        Backslash => 49,
        Semicolon => 51,
        Quote => 52,
        Backtick => 53,
        Comma => 54,
        Period => 55,
        Slash => 56,
        F5 => 62,
        F6 => 63,
        F7 => 64,
        F8 => 65,
        F9 => 66,
        F12 => 69,
        Insert => 73,
        Home => 74,
        PageUp => 75,
        Delete => 76,
        End => 77,
        PageDown => 78,
        ArrowRight => 79,
        ArrowLeft => 80,
        ArrowDown => 81,
        ArrowUp => 82,
        // F1/F2/F3/F4/F10/F11 belong to the shared workspace.
        _ => return None,
    })
}
