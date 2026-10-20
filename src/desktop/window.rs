// SPDX-License-Identifier: GPL-2.0-or-later
//! Native desktop window and input handling built on `winit`.
//!
//! [`Window`] owns the `winit` `EventLoop`, the created OS window, a [`WindowApp`]
//! `ApplicationHandler`, and the controller backend. [`Window::new`] creates and
//! centers the window, pumping events once so creation errors surface eagerly.
//! [`Window::poll`] pumps pending events, applies them to `Input`, and returns
//! `false` once the window is closed; `escape_quits` makes Escape (scan code 41) close
//! the window. Sizing honors a logical size scaled by [`Window::scale`], and the
//! remaining setters cover title, fullscreen, cursor visibility and IME/text input.
//!
//! Window events are queued as [`PendingInput`] values and only applied in `poll`, so
//! input is mutated on a single path. Physical keys are translated to SDL-compatible
//! scan codes by [`scancode`] and mouse buttons to negative codes by [`mouse_button`],
//! matching the RGSS bindings. Focus loss releases all held inputs. With the `gpu`
//! feature, `egui` sees events first and [`WindowApp`] only forwards keyboard, mouse
//! and scroll events `egui` did not consume (or that target the game's pointer rect
//! set via [`Window::set_game_pointer_rect`]). Text entry is gated on
//! [`Window::text_input`] and deduplicated between ordinary key text and IME commits
//! via [`keyboard_text`].
use rsxp::input::Input;
use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
    time::Duration,
};
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalPosition, LogicalSize, PhysicalPosition},
    event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    platform::pump_events::EventLoopExtPumpEvents,
    window::{Fullscreen, Window as NativeWindow, WindowAttributes, WindowId},
};

enum PendingInput {
    KeyDown(i32),
    KeyUp(i32),
    MouseMove(i32, i32),
    MouseInside(bool),
    Scroll(i32),
    Text(String),
    ReleaseAll,
}

#[derive(Default)]
struct WindowApp {
    attributes: Option<WindowAttributes>,
    window: Option<Arc<NativeWindow>>,
    window_id: Option<WindowId>,
    events: VecDeque<PendingInput>,
    forwarded_keys: HashSet<i32>,
    forwarded_mouse_buttons: HashSet<i32>,
    cursor_position: Option<(f64, f64)>,
    open: bool,
    focused: bool,
    text_input: bool,
    last_key_text: Option<String>,
    error: Option<String>,
    #[cfg(feature = "gpu")]
    egui_context: egui::Context,
    #[cfg(feature = "gpu")]
    egui_platform: Option<egui_winit::State>,
    #[cfg(feature = "gpu")]
    game_pointer_rect: Option<egui::Rect>,
}

impl WindowApp {
    fn pointer_over_game(&self, x: f64, y: f64) -> bool {
        #[cfg(feature = "gpu")]
        {
            self.game_pointer_rect
                .is_some_and(|rect| rect.contains(egui::pos2(x as f32, y as f32)))
        }
        #[cfg(not(feature = "gpu"))]
        {
            let _ = (x, y);
            false
        }
    }
}

fn keyboard_text(text_input: bool, text: Option<&str>) -> Option<String> {
    if !text_input {
        return None;
    }
    let text = text?;
    (!text.is_empty() && text.chars().any(|character| !character.is_control()))
        .then(|| text.to_owned())
}

impl ApplicationHandler for WindowApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let Some(attributes) = self.attributes.take() else {
            self.error = Some("window attributes were not initialized".into());
            return;
        };
        match event_loop.create_window(attributes) {
            Ok(window) => {
                self.window_id = Some(window.id());
                let window = Arc::new(window);
                #[cfg(feature = "gpu")]
                {
                    self.egui_platform = Some(egui_winit::State::new(
                        self.egui_context.clone(),
                        egui::ViewportId::ROOT,
                        window.as_ref(),
                        Some(window.scale_factor() as f32),
                        window.theme(),
                        None,
                    ));
                }
                self.window = Some(window);
                self.open = true;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if Some(window_id) != self.window_id {
            return;
        }
        // Winit may report both KeyEvent::text and an IME commit for the same
        // ordinary key. Keep the text only across adjacent key/commit events.
        if !matches!(&event, WindowEvent::Ime(_)) {
            self.last_key_text = None;
        }
        #[cfg(feature = "gpu")]
        let consumed = match (&mut self.egui_platform, &self.window) {
            (Some(platform), Some(window)) => platform.on_window_event(window, &event).consumed,
            _ => false,
        };
        #[cfg(not(feature = "gpu"))]
        let consumed = false;
        match event {
            WindowEvent::CloseRequested => self.open = false,
            WindowEvent::Focused(true) => self.focused = true,
            WindowEvent::Focused(false) => {
                self.focused = false;
                self.last_key_text = None;
                self.forwarded_keys.clear();
                self.forwarded_mouse_buttons.clear();
                self.events.push_back(PendingInput::ReleaseAll);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if self.text_input
                    && event.state == ElementState::Pressed
                    && let Some(text) = keyboard_text(true, event.text.as_deref())
                {
                    self.last_key_text = Some(text.clone());
                    self.events.push_back(PendingInput::Text(text));
                }
                if let PhysicalKey::Code(code) = event.physical_key
                    && let Some(scancode) = scancode(code)
                {
                    match event.state {
                        ElementState::Pressed
                            if !consumed || self.text_input || code == KeyCode::F10 =>
                        {
                            self.forwarded_keys.insert(scancode);
                            self.events.push_back(PendingInput::KeyDown(scancode));
                        }
                        ElementState::Released if self.forwarded_keys.remove(&scancode) => {
                            self.events.push_back(PendingInput::KeyUp(scancode));
                        }
                        _ => {}
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let scale = self
                    .window
                    .as_ref()
                    .map(|window| window.scale_factor())
                    .unwrap_or(1.0);
                let LogicalPosition { x, y } = position.to_logical::<f64>(scale);
                self.cursor_position = Some((x, y));
                if !consumed || self.pointer_over_game(x, y) {
                    self.events
                        .push_back(PendingInput::MouseMove(x.round() as i32, y.round() as i32));
                }
            }
            WindowEvent::CursorEntered { .. } => {
                self.events.push_back(PendingInput::MouseInside(true));
            }
            WindowEvent::CursorLeft { .. } => {
                self.cursor_position = None;
                self.events.push_back(PendingInput::MouseInside(false));
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let amount = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y.round() as i32,
                    MouseScrollDelta::PixelDelta(position) => position.y.signum() as i32,
                };
                let over_game = self
                    .cursor_position
                    .is_some_and(|(x, y)| self.pointer_over_game(x, y));
                if amount != 0 && (!consumed || over_game) {
                    self.events.push_back(PendingInput::Scroll(amount));
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let over_game = self
                    .cursor_position
                    .is_some_and(|(x, y)| self.pointer_over_game(x, y));
                if let Some(button) = mouse_button(button) {
                    match state {
                        ElementState::Pressed if !consumed || over_game => {
                            self.forwarded_mouse_buttons.insert(button);
                            self.events.push_back(PendingInput::KeyDown(-button));
                        }
                        ElementState::Released if self.forwarded_mouse_buttons.remove(&button) => {
                            self.events.push_back(PendingInput::KeyUp(-button));
                        }
                        _ => {}
                    }
                }
            }
            WindowEvent::Ime(Ime::Commit(text)) => {
                if self.text_input && self.last_key_text.as_deref() != Some(text.as_str()) {
                    self.events.push_back(PendingInput::Text(text));
                }
                self.last_key_text = None;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::keyboard_text;

    #[test]
    fn regular_unicode_key_text_is_forwarded() {
        assert_eq!(keyboard_text(true, Some("ñ🙂")), Some("ñ🙂".into()));
        assert_eq!(keyboard_text(true, Some("\r")), None);
    }

    #[test]
    fn text_is_only_forwarded_in_text_mode() {
        assert_eq!(keyboard_text(true, Some("a")), Some("a".into()));
        assert_eq!(keyboard_text(false, Some("a")), None);
    }
}

fn mouse_button(button: MouseButton) -> Option<i32> {
    match button {
        MouseButton::Left => Some(1),
        MouseButton::Right => Some(3),
        MouseButton::Middle => Some(2),
        MouseButton::Back => Some(4),
        MouseButton::Forward => Some(5),
        MouseButton::Other(button) => Some(i32::from(button)),
    }
}

/// Maps physical keys to the SDL-compatible scan codes consumed by the RGSS
/// bindings. The game therefore keeps the same key mapping without SDL.
fn scancode(code: KeyCode) -> Option<i32> {
    let code = match code {
        KeyCode::KeyA => 4,
        KeyCode::KeyB => 5,
        KeyCode::KeyC => 6,
        KeyCode::KeyD => 7,
        KeyCode::KeyE => 8,
        KeyCode::KeyF => 9,
        KeyCode::KeyG => 10,
        KeyCode::KeyH => 11,
        KeyCode::KeyI => 12,
        KeyCode::KeyJ => 13,
        KeyCode::KeyK => 14,
        KeyCode::KeyL => 15,
        KeyCode::KeyM => 16,
        KeyCode::KeyN => 17,
        KeyCode::KeyO => 18,
        KeyCode::KeyP => 19,
        KeyCode::KeyQ => 20,
        KeyCode::KeyR => 21,
        KeyCode::KeyS => 22,
        KeyCode::KeyT => 23,
        KeyCode::KeyU => 24,
        KeyCode::KeyV => 25,
        KeyCode::KeyW => 26,
        KeyCode::KeyX => 27,
        KeyCode::KeyY => 28,
        KeyCode::KeyZ => 29,
        KeyCode::Digit1 => 30,
        KeyCode::Digit2 => 31,
        KeyCode::Digit3 => 32,
        KeyCode::Digit4 => 33,
        KeyCode::Digit5 => 34,
        KeyCode::Digit6 => 35,
        KeyCode::Digit7 => 36,
        KeyCode::Digit8 => 37,
        KeyCode::Digit9 => 38,
        KeyCode::Digit0 => 39,
        KeyCode::Enter => 40,
        KeyCode::Escape => 41,
        KeyCode::Backspace => 42,
        KeyCode::Tab => 43,
        KeyCode::Space => 44,
        KeyCode::Minus => 45,
        KeyCode::Equal => 46,
        KeyCode::BracketLeft => 47,
        KeyCode::BracketRight => 48,
        KeyCode::Backslash => 49,
        KeyCode::Semicolon => 51,
        KeyCode::Quote => 52,
        KeyCode::Backquote => 53,
        KeyCode::Comma => 54,
        KeyCode::Period => 55,
        KeyCode::Slash => 56,
        KeyCode::CapsLock => 57,
        KeyCode::F1 => 58,
        KeyCode::F2 => 59,
        KeyCode::F3 => 60,
        KeyCode::F4 => 61,
        KeyCode::F5 => 62,
        KeyCode::F6 => 63,
        KeyCode::F7 => 64,
        KeyCode::F8 => 65,
        KeyCode::F9 => 66,
        KeyCode::F10 => 67,
        KeyCode::F11 => 68,
        KeyCode::F12 => 69,
        KeyCode::PrintScreen => 70,
        KeyCode::ScrollLock => 71,
        KeyCode::Pause => 72,
        KeyCode::Insert => 73,
        KeyCode::Home => 74,
        KeyCode::PageUp => 75,
        KeyCode::Delete => 76,
        KeyCode::End => 77,
        KeyCode::PageDown => 78,
        KeyCode::ArrowRight => 79,
        KeyCode::ArrowLeft => 80,
        KeyCode::ArrowDown => 81,
        KeyCode::ArrowUp => 82,
        KeyCode::NumLock => 83,
        KeyCode::NumpadDivide => 84,
        KeyCode::NumpadMultiply => 85,
        KeyCode::NumpadSubtract => 86,
        KeyCode::NumpadAdd => 87,
        KeyCode::NumpadEnter => 88,
        KeyCode::Numpad1 => 89,
        KeyCode::Numpad2 => 90,
        KeyCode::Numpad3 => 91,
        KeyCode::Numpad4 => 92,
        KeyCode::Numpad5 => 93,
        KeyCode::Numpad6 => 94,
        KeyCode::Numpad7 => 95,
        KeyCode::Numpad8 => 96,
        KeyCode::Numpad9 => 97,
        KeyCode::Numpad0 => 98,
        KeyCode::NumpadDecimal => 99,
        KeyCode::ShiftLeft => 225,
        KeyCode::ControlLeft => 224,
        KeyCode::AltLeft => 226,
        KeyCode::SuperLeft => 227,
        KeyCode::ControlRight => 228,
        KeyCode::ShiftRight => 229,
        KeyCode::AltRight => 230,
        KeyCode::SuperRight => 231,
        _ => return None,
    };
    Some(code)
}

pub struct Window {
    controllers: crate::controller::Controllers,
    event_loop: EventLoop<()>,
    app: WindowApp,
    window: Arc<NativeWindow>,
    width: u32,
    height: u32,
    scale: f64,
}

impl Window {
    pub fn new(width: u32, height: u32) -> Result<Self, String> {
        if width == 0 || height == 0 {
            return Err("invalid window dimensions".into());
        }
        let mut event_loop = EventLoop::new().map_err(|error| error.to_string())?;
        let mut app = WindowApp {
            attributes: Some(
                NativeWindow::default_attributes()
                    .with_title("rsxp Rust engine")
                    .with_inner_size(LogicalSize::new(width as f64, height as f64)),
            ),
            ..WindowApp::default()
        };
        event_loop.pump_app_events(Some(Duration::ZERO), &mut app);
        if let Some(error) = app.error.take() {
            return Err(error);
        }
        let window = app
            .window
            .clone()
            .ok_or_else(|| "winit did not create a window".to_owned())?;
        center_window(&window);
        Ok(Self {
            controllers: crate::controller::Controllers::new(),
            event_loop,
            app,
            window,
            width,
            height,
            scale: 1.0,
        })
    }

    #[cfg(feature = "gpu")]
    pub fn native(&self) -> Arc<NativeWindow> {
        Arc::clone(&self.window)
    }

    #[cfg(feature = "gpu")]
    pub fn egui_context(&self) -> &egui::Context {
        &self.app.egui_context
    }

    #[cfg(feature = "gpu")]
    pub fn take_egui_input(&mut self) -> egui::RawInput {
        let context = self.app.egui_context.clone();
        let mut input = self
            .app
            .egui_platform
            .as_mut()
            .expect("winit resumed before egui input")
            .take_egui_input(&self.window);
        let viewport = input.viewports.entry(egui::ViewportId::ROOT).or_default();
        egui_winit::update_viewport_info(viewport, &context, &self.window, false);
        input
    }

    #[cfg(feature = "gpu")]
    pub fn handle_egui_platform_output(&mut self, output: egui::PlatformOutput) {
        self.app
            .egui_platform
            .as_mut()
            .expect("winit resumed before egui platform output")
            .handle_platform_output(&self.window, output);
    }

    #[cfg(feature = "gpu")]
    pub fn set_game_pointer_rect(&mut self, rect: egui::Rect) {
        self.app.game_pointer_rect = Some(rect);
    }

    #[cfg(feature = "gpu")]
    pub fn physical_size(&self) -> (u32, u32) {
        let size = self.window.inner_size();
        (size.width.max(1), size.height.max(1))
    }

    pub fn poll(&mut self, input: &mut Input, escape_quits: bool) -> bool {
        self.event_loop
            .pump_app_events(Some(Duration::ZERO), &mut self.app);
        while let Some(event) = self.app.events.pop_front() {
            match event {
                PendingInput::KeyDown(key) => {
                    if escape_quits && key == 41 {
                        self.app.open = false;
                    } else {
                        input.key_down(key);
                    }
                }
                PendingInput::KeyUp(key) => input.key_up(key),
                PendingInput::MouseMove(x, y) => {
                    input.mouse_x = x;
                    input.mouse_y = y;
                }
                PendingInput::MouseInside(inside) => input.mouse_inside = inside,
                PendingInput::Scroll(delta) => input.scroll_v += delta,
                PendingInput::Text(text) => input.push_text(&text),
                PendingInput::ReleaseAll => input.release_all(),
            }
        }
        self.controllers
            .poll(input, self.app.focused && !self.app.text_input);
        self.app.open
    }

    pub fn set_title(&mut self, title: &str) -> Result<(), String> {
        self.window.set_title(title);
        Ok(())
    }

    pub fn fullscreen(&mut self, enabled: bool) -> Result<(), String> {
        self.window
            .set_fullscreen(enabled.then(|| Fullscreen::Borderless(self.window.current_monitor())));
        Ok(())
    }

    pub fn scale(&mut self, scale: f64) {
        self.scale = if scale.is_finite() {
            scale.max(0.1)
        } else {
            1.0
        };
        self.request_size();
    }

    pub fn resize_logical(&mut self, width: u32, height: u32) {
        self.width = width.max(1);
        self.height = height.max(1);
        self.request_size();
    }

    fn request_size(&self) {
        let _ = self.window.request_inner_size(LogicalSize::new(
            self.width as f64 * self.scale,
            self.height as f64 * self.scale,
        ));
    }

    pub fn center(&mut self) {
        center_window(&self.window);
    }

    pub fn cursor(&mut self, shown: bool) {
        self.window.set_cursor_visible(shown);
    }

    pub fn text_input(&mut self, enabled: bool) {
        self.app.text_input = enabled;
        self.window.set_ime_allowed(enabled);
    }
}

fn center_window(window: &NativeWindow) {
    if let Some(monitor) = window.current_monitor() {
        let monitor_position = monitor.position();
        let monitor_size = monitor.size();
        let window_size = window.outer_size();
        window.set_outer_position(PhysicalPosition::new(
            monitor_position.x + (monitor_size.width as i32 - window_size.width as i32) / 2,
            monitor_position.y + (monitor_size.height as i32 - window_size.height as i32) / 2,
        ));
    }
}
