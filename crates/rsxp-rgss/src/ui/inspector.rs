// SPDX-License-Identifier: GPL-2.0-or-later
//! The Essentials inspector/tools and response plumbing for a running game.
//!
//! Every backend operation is queued with `Workspace::queue_api_request_tagged`, which remembers the returned id in
//! `api_pending` and is consumed by [`Workspace::poll_api_response`]. The
//! PokeAPI and game explorers are supplied by Rhai plugins; this module keeps
//! only generic response plumbing and debug compatibility controls.
//!
//! ## Main functions
//! * [`Workspace::poll_api_response`] — drains the pending foreground response
//!   and routes it by operation. Notable results: `pokemon_essentials_status`
//!   updates the cached `$DEBUG` flag.
//! * [`Workspace::queue_api_request_tagged`] — send an operation through the
//!   backend, tracking the response against a possibly different operation tag.
//! * `draw_essentials_tools` and the debug helper — the remaining native
//!   Essentials compatibility UI.
//! * `draw_debug_toggle` / `poll_essentials_debug` / `request_essentials_debug`
//!   — query and set the game's `$DEBUG` through `pokemon_essentials_debug`
//!   (empty argument queries; `"true"`/`"false"` sets).
//!
//! ## Backend and JSON use
//! `backend.send_request(operation, args)` returns a request id and
//! `backend.take_response(id)` yields `Ok(json_string)` or `Err(message)`.
//! Responses are parsed into `serde_json::Value` (`api_response_json`):
//! `pokemon_essentials_status` returns `{ "map_id", "debug": bool }`;
//! `pokemon_essentials_debug` returns `{ "debug": bool }` or
//! `{ "error": "..." }`.
use super::*;
impl Workspace {
    #[cfg(feature = "game-api")]
    pub(super) fn poll_api_response(&mut self) {
        if let Some(id) = self.api_pending
            && let Some(result) = self.backend.take_response(id)
        {
            self.api_pending = None;
            let operation = self.api_pending_operation.take();
            match result {
                Ok(response) => {
                    if operation.as_deref() == Some("cable_club_launch") {
                        self.cable_error = None;
                    }
                    let response_json = serde_json::from_str::<serde_json::Value>(&response).ok();
                    self.api_response_json = response_json.clone();
                    if operation.as_deref() == Some("pokemon_essentials_status")
                        && let Some(debug) = response_json
                            .as_ref()
                            .and_then(|value| value.get("debug"))
                            .and_then(serde_json::Value::as_bool)
                    {
                        self.essentials_debug = Some(debug);
                    }
                    self.api_response = response;
                    self.api_error = None;
                }
                Err(error) => {
                    if operation.as_deref() == Some("cable_club_launch") {
                        self.cable_error = Some(error);
                    } else {
                        self.api_error = Some(error);
                    }
                }
            }
        }
    }

    #[cfg(feature = "game-api")]
    pub(super) fn queue_api_request_tagged(
        &mut self,
        operation: &str,
        response_operation: &str,
        args: Vec<String>,
    ) {
        match self.backend.send_request(operation, args) {
            Ok(id) => {
                self.api_pending = Some(id);
                self.api_pending_operation = Some(response_operation.to_owned());
                self.api_error = None;
            }
            Err(error) => self.api_error = Some(error),
        }
    }

    #[cfg(feature = "game-api")]
    pub(super) fn draw_essentials_tools(&mut self, ui: &mut egui::Ui) {
        self.poll_essentials_debug();
        self.draw_debug_toggle(ui);
    }

    #[cfg(feature = "game-api")]
    fn draw_debug_toggle(&mut self, ui: &mut egui::Ui) {
        // Ask the game for its current `$DEBUG` value the first time the panel
        // is shown, then keep the checkbox in sync with the game.
        if !self.essentials_debug_queried && self.essentials_debug_pending.is_none() {
            self.essentials_debug_queried = true;
            self.request_essentials_debug("");
        }
        ui.horizontal_wrapped(|ui| {
            let busy = self.essentials_debug_pending.is_some();
            let known = self.essentials_debug.is_some();
            let mut enabled = self.essentials_debug.unwrap_or(false);
            let response = ui.add_enabled(
                !busy && known,
                egui::Checkbox::new(&mut enabled, "Debug mode"),
            );
            if response
                .on_hover_text("Enables the game's own debug menus, Ctrl-modifiers and shortcuts.")
                .changed()
            {
                self.request_essentials_debug(if enabled { "true" } else { "false" });
            }
            if busy {
                ui.spinner();
            } else if !known && ui.button("Query debug state").clicked() {
                self.request_essentials_debug("");
            }
        });
        if let Some((error, message)) = &self.essentials_debug_notice {
            ui.colored_label(
                if *error {
                    ui.visuals().error_fg_color
                } else {
                    egui::Color32::from_rgb(100, 205, 168)
                },
                message,
            );
        }
    }

    #[cfg(feature = "game-api")]
    fn poll_essentials_debug(&mut self) {
        let Some(id) = self.essentials_debug_pending else {
            return;
        };
        let Some(result) = self.backend.take_response(id) else {
            return;
        };
        self.essentials_debug_pending = None;
        match result.and_then(|body| {
            serde_json::from_str::<serde_json::Value>(&body).map_err(|error| error.to_string())
        }) {
            Ok(value) if value["error"].is_string() => {
                self.essentials_debug_notice = Some((
                    true,
                    value["error"]
                        .as_str()
                        .unwrap_or("debug request failed")
                        .to_owned(),
                ));
            }
            Ok(value) => {
                self.essentials_debug = value["debug"].as_bool();
                self.essentials_debug_queried = true;
                let state = match self.essentials_debug {
                    Some(true) => "enabled",
                    Some(false) => "disabled",
                    None => "unknown",
                };
                self.essentials_debug_notice = Some((false, format!("Debug mode {state}.")));
            }
            Err(error) => self.essentials_debug_notice = Some((true, error)),
        }
    }

    #[cfg(feature = "game-api")]
    fn request_essentials_debug(&mut self, mode: &str) {
        if self.essentials_debug_pending.is_some() {
            return;
        }
        match self
            .backend
            .send_request("pokemon_essentials_debug", vec![mode.to_owned()])
        {
            Ok(id) => {
                self.essentials_debug_pending = Some(id);
                self.essentials_debug_notice = None;
            }
            Err(error) => self.essentials_debug_notice = Some((true, error)),
        }
    }
}

#[cfg(all(test, feature = "game-api"))]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::HashMap, rc::Rc};

    #[derive(Clone, Default)]
    struct TestBackend {
        requests: Rc<RefCell<Vec<String>>>,
        request_args: Rc<RefCell<Vec<Vec<String>>>>,
        responses: Rc<RefCell<HashMap<u64, Result<String, String>>>>,
    }

    impl backend::Backend for TestBackend {
        fn fast_forward(&self) -> bool {
            false
        }
        fn set_fast_forward(&self, _: bool) {}
        fn current_rate(&self) -> u32 {
            40
        }
        fn set_rate_override(&self, _: u32) {}
        fn request_reset(&self) {}
        fn patch_enabled(&self, _: &str) -> bool {
            false
        }
        fn set_patch_enabled(&self, _: &str, _: bool) -> bool {
            false
        }
        fn patch_value(&self, _: &str) -> Option<f64> {
            None
        }
        fn set_patch_value(&self, _: &str, _: f64) -> bool {
            false
        }
        fn listening_port(&self) -> Option<u16> {
            None
        }
        fn start_api(&self, _: u16) -> Result<(), String> {
            Ok(())
        }
        fn take_response(&self, id: u64) -> Option<Result<String, String>> {
            self.responses.borrow_mut().remove(&id)
        }
        fn send_request(&self, operation: &str, args: Vec<String>) -> Result<u64, String> {
            let mut requests = self.requests.borrow_mut();
            requests.push(operation.into());
            self.request_args.borrow_mut().push(args);
            Ok(requests.len() as u64)
        }
        fn asset_image(&self, _: &str, _: &str) -> Option<std::sync::Arc<image::RgbaImage>> {
            None
        }
        #[cfg(not(target_arch = "wasm32"))]
        fn cable_status(&self) -> crate::cable_club::ServerStatus {
            Default::default()
        }
        #[cfg(not(target_arch = "wasm32"))]
        fn stop_cable(&self) {}
        #[cfg(not(target_arch = "wasm32"))]
        fn start_cable(&self, _: &str, _: u16) -> Result<(), String> {
            Ok(())
        }
        #[cfg(not(target_arch = "wasm32"))]
        fn save_endpoint(&self, _: &str, _: u16) -> Result<std::path::PathBuf, String> {
            Ok(Default::default())
        }
    }

    #[test]
    fn debug_toggle_queries_the_game_and_tracks_its_state() {
        let backend = TestBackend::default();
        let mut workspace = Workspace::new();
        workspace.backend = Box::new(backend.clone());
        // Asking for the current value uses an empty mode argument.
        workspace.request_essentials_debug("");
        assert_eq!(backend.requests.borrow().len(), 1);
        assert_eq!(
            backend.requests.borrow()[0].as_str(),
            "pokemon_essentials_debug"
        );
        assert_eq!(backend.request_args.borrow()[0], vec![String::new()]);
        // The reply feeds the cached checkbox state.
        backend
            .responses
            .borrow_mut()
            .insert(1, Ok(r#"{"debug":true}"#.into()));
        workspace.poll_essentials_debug();
        assert_eq!(workspace.essentials_debug, Some(true));
        assert!(workspace.essentials_debug_notice.is_some());
        // A toggle sends the desired value explicitly rather than flipping blindly.
        workspace.request_essentials_debug("false");
        assert_eq!(backend.request_args.borrow()[1], vec!["false".to_string()]);
        // Errors surface as a notice without clearing the known state.
        backend
            .responses
            .borrow_mut()
            .insert(2, Err("game is not ready".into()));
        workspace.poll_essentials_debug();
        assert_eq!(workspace.essentials_debug, Some(true));
        assert!(workspace.essentials_debug_notice.as_ref().unwrap().0);
    }
}
