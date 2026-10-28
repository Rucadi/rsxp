#[path = "../src/game_input.rs"]
mod game_input;
use egui::{Event, Key, Modifiers};
use fanplay_core::{CoreControl, InputEvent};
use game_input::GameKeyboard;
fn key(key: Key, pressed: bool) -> Event {
    Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}
fn drain(control: &CoreControl) -> Vec<InputEvent> {
    let mut events = Vec::new();
    control.input().take_events(&mut events);
    events
}
#[test]
fn game_name_entry_forwards_unicode_editing_and_confirmation() {
    let control = CoreControl::new();
    let mut input = GameKeyboard::default();
    input.route(
        &control,
        &[
            key(Key::A, true),
            Event::Text("Añil".into()),
            key(Key::A, false),
            key(Key::Backspace, true),
            key(Key::Backspace, false),
            key(Key::Enter, true),
            key(Key::Enter, false),
        ],
        Modifiers::NONE,
        true,
        true,
    );
    let events = drain(&control);
    assert_eq!(
        events[0],
        InputEvent::Key {
            code: 4,
            down: true,
            repeat: false
        }
    );
    assert_eq!(events[1], InputEvent::Text("Añil".into()));
    assert!(events.contains(&InputEvent::Key {
        code: 42,
        down: true,
        repeat: false
    }));
    assert_eq!(
        events.last(),
        Some(&InputEvent::Key {
            code: 40,
            down: false,
            repeat: false
        })
    );
}
#[test]
fn ui_capture_releases_game_keys_and_does_not_leak_text() {
    let control = CoreControl::new();
    let mut input = GameKeyboard::default();
    input.route(
        &control,
        &[key(Key::ArrowRight, true)],
        Modifiers::SHIFT,
        true,
        false,
    );
    drain(&control);
    input.route(
        &control,
        &[Event::Text("UI search".into())],
        Modifiers::NONE,
        false,
        true,
    );
    let events = drain(&control);
    assert_eq!(events.len(), 2);
    assert!(events.contains(&InputEvent::Key {
        code: 79,
        down: false,
        repeat: false
    }));
    assert!(events.contains(&InputEvent::Key {
        code: 225,
        down: false,
        repeat: false
    }));
}
#[test]
fn gameplay_escape_space_and_focus_loss_preserve_key_edges() {
    let control = CoreControl::new();
    let mut input = GameKeyboard::default();
    input.route(
        &control,
        &[
            key(Key::Space, true),
            key(Key::Space, false),
            key(Key::Escape, true),
            Event::WindowFocused(false),
        ],
        Modifiers::NONE,
        true,
        false,
    );
    assert_eq!(
        drain(&control),
        vec![
            InputEvent::Key {
                code: 44,
                down: true,
                repeat: false
            },
            InputEvent::Key {
                code: 44,
                down: false,
                repeat: false
            },
            InputEvent::Key {
                code: 41,
                down: true,
                repeat: false
            },
            InputEvent::Key {
                code: 41,
                down: false,
                repeat: false
            }
        ]
    );
    assert!(!game_input::game_event(&key(Key::F1, true)));
    assert!(game_input::game_event(&key(Key::ArrowDown, true)));
}
