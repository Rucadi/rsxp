// SPDX-License-Identifier: GPL-2.0-or-later
//! Workspace root: the single egui UI shared by every frontend.
//!
//! The desktop window and the browser canvas both render this module: the
//! toolbar, the runtime-patches panel, the output log, the performance panel, the game
//! view placement, and — behind `game-api` — the API console and the
//! Essentials inspector windows. A frontend supplies per-frame
//! [`WorkspaceInfo`] plus an injected [`backend::Backend`] for runtime services,
//! then reads back a [`WorkspaceOutput`] describing where the game view landed
//! and which global actions (fullscreen, save import/export) were requested.
//!
//! ## Main types
//! * [`Workspace`] — persistent UI state: panel visibility, [`ViewMode`], and
//!   the session panel. Game-specific explorers are supplied by Rhai plugins
//!   and open in their HTML webviews. [`Workspace::draw`] is the per-frame
//!   entry point; it applies the theme, handles keyboard shortcuts (F1 toggles
//!   all chrome, F2/F3/F10 toggle the left/output/performance panels, F4 requests
//!   frontend fullscreen), and lays out the panels around the game view.
//! * [`WorkspaceInfo`] — the frame's frontend-provided facts (logical size,
//!   FPS/frame count, global fullscreen, and which
//!   controls the frontend supports).
//! * [`WorkspaceOutput`] — the frame's results: the `game_rect`, an optional
//!   `fullscreen_request`, and `export_saves` / `import_saves` intents.
//! * [`ViewMode`] — how the game view is placed (docked, workspace-fullscreen,
//!   or floating).
//! * `Tool` (private) — the active native tool tab (runtime, Essentials,
//!   online play); game explorers are supplied by plugins.
//!
//! ## Backend use
//! All runtime interaction goes through `Workspace::backend`; the workspace
//! never handles VM or window types directly. The session panels ([`session`])
//! present the launcher/save UI and emit platform-neutral [`session::SessionActions`]
//! intents for the frontend to execute, and the `game-api` tool windows queue
//! named operations through the backend.
//!
//! ## Game data
//! Plugin webviews receive game data and assets through the generic API and
//! asset bridges. The Rust workspace only retains the small sprite-frame
//! helper used by other plugins.
//!
//! The panel implementations live in the sibling modules declared below
//! (toolbar, logs, patches, performance, game_view, session, and the `game-api`
//! explorers).
pub mod backend;
mod connections;
mod game_view;
mod inspector;
mod library;
mod logs;
mod patches;
mod performance;
mod saves;
pub mod session;
mod theme;
mod toolbar;

#[cfg(feature = "game-api")]
pub fn first_pokemon_sprite_frame(image: &image::RgbaImage) -> image::RgbaImage {
    let width = image.width();
    let height = image.height();
    // Some games store icons as exactly two horizontal frames. Treat the
    // boundary as an animated sheet too; otherwise the webview displays both
    // copies in the small list thumbnail.
    if height > 0 && width >= height.saturating_mul(2) {
        image::imageops::crop_imm(image, 0, 0, height, height).to_image()
    } else {
        image.clone()
    }
}

#[cfg(feature = "game-api")]
pub fn png_bytes(image: &image::RgbaImage) -> Option<Vec<u8>> {
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).ok()?;
    Some(bytes.into_inner())
}

/// Encodes the first square frame of a Pokémon sprite sheet for a plugin
/// webview without retaining the encoded bytes.
#[cfg(feature = "game-api")]
pub fn first_pokemon_sprite_png(image: &image::RgbaImage) -> Option<Vec<u8>> {
    let frame = first_pokemon_sprite_frame(image);
    png_bytes(&frame)
}

/// How the game view is placed within the workspace.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewMode {
    #[default]
    Docked,
    WorkspaceFullscreen,
    Floating,
}

impl ViewMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Docked => "Docked",
            Self::WorkspaceFullscreen => "Workspace fullscreen",
            Self::Floating => "Floating window",
        }
    }
}

/// What the frontend tells the workspace for the current frame.
pub struct WorkspaceInfo {
    pub logical_size: (u32, u32),
    pub fps: f64,
    pub frames: u64,
    pub global_fullscreen: bool,
    /// Show the OS/browser fullscreen button (the desktop window has one).
    pub show_fullscreen: bool,
    /// Show the save export/import buttons (the browser provides the dialogs).
    pub show_save_buttons: bool,
}

/// What the workspace asked the frontend to do, plus the game view rect.
pub struct WorkspaceOutput {
    pub game_rect: egui::Rect,
    pub fullscreen_request: Option<bool>,
    pub export_saves: bool,
    pub import_saves: bool,
}

impl Default for WorkspaceOutput {
    fn default() -> Self {
        Self {
            game_rect: egui::Rect::ZERO,
            fullscreen_request: None,
            export_saves: false,
            import_saves: false,
        }
    }
}

/// Persistent workspace UI state.
#[derive(Default, PartialEq, Eq, Clone, Copy)]
enum Tool {
    #[default]
    Runtime,
    Essentials,
    #[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
    OnlinePlay,
}

pub struct Workspace {
    pub(crate) frame_texture: Option<egui::TextureId>,
    pub(crate) game_paint_callback: Option<egui::PaintCallback>,
    pub logs: Vec<String>,
    tool: Tool,
    pub backend: Box<dyn backend::Backend>,
    pub session: Option<session::SessionPanel>,
    pub session_actions: session::SessionActions,
    panels_top: bool,
    panels_left: bool,
    panels_performance: bool,
    panels_output: bool,
    view_mode: ViewMode,
    controls_visible: bool,
    controller_settings_open: bool,
    was_global_fullscreen: bool,
    reset_confirm: bool,
    #[cfg(feature = "game-api")]
    api_error: Option<String>,
    #[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
    cable_bind: String,
    #[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
    cable_port: String,
    #[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
    cable_client_host: String,
    #[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
    cable_client_port: String,
    #[cfg(feature = "game-api")]
    cable_error: Option<String>,
    #[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
    cable_saved: Option<String>,
    #[cfg(feature = "game-api")]
    api_pending: Option<u64>,
    #[cfg(feature = "game-api")]
    api_pending_operation: Option<String>,
    #[cfg(feature = "game-api")]
    api_response: String,
    #[cfg(feature = "game-api")]
    api_response_json: Option<serde_json::Value>,
    #[cfg(feature = "game-api")]
    essentials_debug: Option<bool>,
    #[cfg(feature = "game-api")]
    essentials_debug_pending: Option<u64>,
    #[cfg(feature = "game-api")]
    essentials_debug_queried: bool,
    #[cfg(feature = "game-api")]
    essentials_debug_notice: Option<(bool, String)>,
    /// `Some((done, total))` shows a download progress screen instead of the
    /// workspace (used while the browser preloads the game).
    loading: Option<(u32, u32)>,
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            frame_texture: None,
            game_paint_callback: None,
            logs: Vec::new(),
            tool: Tool::default(),
            backend: Box::new(crate::workspace_services::LocalServices::default()),
            session: None,
            session_actions: Default::default(),
            panels_top: false,
            panels_left: true,
            panels_performance: false,
            panels_output: false,
            view_mode: ViewMode::default(),
            controls_visible: false,
            controller_settings_open: false,
            was_global_fullscreen: false,
            reset_confirm: false,
            #[cfg(feature = "game-api")]
            api_error: None,
            #[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
            cable_bind: "0.0.0.0".into(),
            #[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
            cable_port: "25565".into(),
            #[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
            cable_client_host: "127.0.0.1".into(),
            #[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
            cable_client_port: "25565".into(),
            #[cfg(feature = "game-api")]
            cable_error: None,
            #[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
            cable_saved: None,
            #[cfg(feature = "game-api")]
            api_pending: None,
            #[cfg(feature = "game-api")]
            api_pending_operation: None,
            #[cfg(feature = "game-api")]
            api_response: String::new(),
            #[cfg(feature = "game-api")]
            api_response_json: None,
            #[cfg(feature = "game-api")]
            essentials_debug: None,
            #[cfg(feature = "game-api")]
            essentials_debug_pending: None,
            #[cfg(feature = "game-api")]
            essentials_debug_queried: false,
            #[cfg(feature = "game-api")]
            essentials_debug_notice: None,
            loading: None,
        }
    }
}

impl Workspace {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reveal diagnostics and controls when a cooperative game stops.
    pub fn game_stopped(&mut self, message: &str) {
        self.loading = None;
        self.controls_visible = true;
        self.view_mode = ViewMode::Docked;
        self.panels_top = true;
        self.panels_output = true;
        self.logs.push(message.into());
    }

    pub fn performance_visible(&self) -> bool {
        self.controls_visible && !self.is_workspace_fullscreen() && self.panels_performance
    }

    pub fn is_workspace_fullscreen(&self) -> bool {
        self.view_mode == ViewMode::WorkspaceFullscreen
    }

    /// Shows or clears the download progress screen (`total == 0` clears it).
    pub fn set_loading(&mut self, done: u32, total: u32) {
        self.loading = if total == 0 {
            None
        } else {
            Some((done, total))
        };
    }

    pub fn set_frame_texture(&mut self, texture: Option<egui::TextureId>) {
        self.frame_texture = texture;
    }

    /// Set a platform GPU callback for drawing the live game surface.
    pub fn set_game_paint_callback(&mut self, callback: Option<egui::PaintCallback>) {
        self.game_paint_callback = callback;
    }

    pub fn draw(&mut self, ui: &mut egui::Ui, info: &WorkspaceInfo) -> WorkspaceOutput {
        self.draw_with_plugin_ui(ui, info, &mut |_| {})
    }

    /// Draws the workspace and lets the frontend add its plugin controls to
    /// the Runtime tools panel. Keeping the callback here makes plugins live
    /// beside runtime patches instead of creating a second top-level panel.
    pub fn draw_with_plugin_ui(
        &mut self,
        ui: &mut egui::Ui,
        info: &WorkspaceInfo,
        draw_plugin_ui: &mut dyn FnMut(&mut egui::Ui),
    ) -> WorkspaceOutput {
        let context = ui.ctx().clone();
        theme::apply(&context);
        let mut output = WorkspaceOutput::default();
        let key_pressed = |wanted: egui::Key| {
            context.input(|input| {
                input.events.iter().any(|event| {
                    matches!(event, egui::Event::Key {
                        key, pressed: true, repeat: false, ..
                    } if *key == wanted)
                })
            })
        };
        if info.global_fullscreen && !self.was_global_fullscreen {
            self.controls_visible = false;
        }
        self.was_global_fullscreen = info.global_fullscreen;
        if key_pressed(egui::Key::F1) {
            self.controls_visible = !self.controls_visible;
            if self.controls_visible && self.is_workspace_fullscreen() {
                self.view_mode = ViewMode::Docked;
            }
        }
        for (key, panel) in [
            (egui::Key::F2, &mut self.panels_left),
            (egui::Key::F3, &mut self.panels_output),
            (egui::Key::F10, &mut self.panels_performance),
        ] {
            if key_pressed(key) {
                *panel = !self.controls_visible || !*panel;
                self.controls_visible = true;
                self.view_mode = ViewMode::Docked;
            }
        }
        if info.show_fullscreen && key_pressed(egui::Key::F4) {
            output.fullscreen_request = Some(!info.global_fullscreen);
            if !info.global_fullscreen {
                self.controls_visible = false;
            }
        }

        // Stopped sessions retain diagnostics even when controls were hidden.
        // Allocate output before the session's central library panel.
        if self.session.as_ref().is_some_and(|session| !session.active) && !self.logs.is_empty() {
            self.draw_logs(ui);
        }
        if let Some(session) = &mut self.session {
            if self.controls_visible || !session.active {
                self.session_actions = session.draw(ui);
            }
            if !session.active {
                return output;
            }
        }
        if let Some((done, total)) = self.loading {
            draw_loading(ui, done, total);
            return output;
        }

        #[cfg(feature = "game-api")]
        self.poll_api_response();
        #[cfg(feature = "game-api")]

        if !self.controls_visible {
            self.draw_game_view(ui, &mut output, info.logical_size);
            return output;
        }
        self.draw_toolbar(ui, info, &mut output);

        let fullscreen = self.is_workspace_fullscreen();
        if !fullscreen && self.panels_top {
            egui::Panel::top("workspace-top-panel")
                .resizable(false)
                .show(ui, |ui| {
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(egui::RichText::new("Dashboard").size(24.0).strong());
                        ui.separator();
                        ui.weak(format!(
                            "{} × {} native resolution",
                            info.logical_size.0, info.logical_size.1
                        ));
                        if info.fps > 0.0 {
                            ui.label(
                                egui::RichText::new(format!("{:.0} FPS", info.fps))
                                    .color(egui::Color32::from_rgb(127, 231, 200)),
                            );
                        }
                    });
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.toggle_value(&mut self.panels_left, "Game tools · F2");
                        ui.toggle_value(&mut self.panels_output, "Activity · F3");
                        ui.toggle_value(&mut self.panels_performance, "Performance · F10");
                        if ui.button("Controller settings").clicked() {
                            self.controller_settings_open = true;
                        }
                    });
                    ui.add_space(8.0);
                });
        }
        if !fullscreen && self.panels_output {
            self.draw_logs(ui);
        }
        if !fullscreen && self.panels_left {
            self.draw_patches_panel(ui, info, draw_plugin_ui);
        }
        if !fullscreen && self.panels_performance {
            self.draw_performance_panel(ui, info);
        }
        self.draw_game_view(ui, &mut output, info.logical_size);
        self.draw_controller_settings(&context);

        if self.reset_confirm {
            egui::Window::new("Reset game?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(&context, |ui| {
                    ui.label("Restart the game from the beginning? Unsaved progress is lost.");
                    ui.horizontal(|ui| {
                        if ui.button("Reset").clicked() {
                            self.backend.request_reset();
                            self.reset_confirm = false;
                        }
                        if ui.button("Cancel").clicked() {
                            self.reset_confirm = false;
                        }
                    });
                });
        }

        output
    }
}

fn draw_loading(ui: &mut egui::Ui, done: u32, total: u32) {
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.35);
                ui.heading("Loading game…");
                ui.add_space(8.0);
                let fraction = if total == 0 {
                    0.0
                } else {
                    done as f32 / total as f32
                };
                ui.add(
                    egui::ProgressBar::new(fraction)
                        .desired_width(360.0)
                        .text(format!("{done} / {total}")),
                );
            });
        });
}

#[cfg(all(test, feature = "game-api"))]
mod sprite_tests {
    use super::first_pokemon_sprite_frame;

    #[test]
    fn animated_pokemon_sprite_sheet_uses_one_square_frame() {
        let mut strip = image::RgbaImage::new(120, 40);
        for (x, _, pixel) in strip.enumerate_pixels_mut() {
            *pixel = if x < 40 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 255, 255])
            };
        }
        let frame = first_pokemon_sprite_frame(&strip);
        assert_eq!((frame.width(), frame.height()), (40, 40));
        assert_eq!(frame.get_pixel(20, 20), &image::Rgba([255, 0, 0, 255]));
    }

    #[test]
    fn two_frame_icon_sheet_uses_one_square_frame_at_boundary() {
        let mut strip = image::RgbaImage::new(64, 32);
        for (x, _, pixel) in strip.enumerate_pixels_mut() {
            *pixel = if x < 32 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 255, 255])
            };
        }
        let frame = first_pokemon_sprite_frame(&strip);
        assert_eq!((frame.width(), frame.height()), (32, 32));
        assert_eq!(frame.get_pixel(16, 16), &image::Rgba([255, 0, 0, 255]));
    }
}

#[cfg(all(test, feature = "game-api"))]
mod workspace_tests {
    use super::*;
    fn draw_frame(
        workspace: &mut Workspace,
        key: Option<egui::Key>,
        fullscreen: bool,
    ) -> WorkspaceOutput {
        let context = egui::Context::default();
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1100.0, 740.0),
            )),
            ..Default::default()
        };
        if let Some(key) = key {
            input.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
        }
        let info = WorkspaceInfo {
            logical_size: (640, 480),
            fps: 0.0,
            frames: 0,
            global_fullscreen: fullscreen,
            show_fullscreen: true,
            show_save_buttons: false,
        };
        let mut result = WorkspaceOutput::default();
        context
            .run_ui(input, |ui| result = workspace.draw(ui, &info))
            .drop_without_applying_deltas();
        result
    }

    #[test]
    fn fullscreen_workspace_keeps_controller_settings_and_dashboard_visible() {
        let mut workspace = Workspace::new();
        workspace.controls_visible = true;
        workspace.view_mode = ViewMode::WorkspaceFullscreen;
        workspace.controller_settings_open = true;
        for _ in 0..3 {
            draw_frame(&mut workspace, None, false);
            assert!(workspace.controls_visible);
            assert!(workspace.controller_settings_open);
            assert_eq!(workspace.view_mode, ViewMode::WorkspaceFullscreen);
        }
    }

    #[test]
    fn gameplay_starts_without_chrome_and_f1_toggles_all_controls() {
        let mut workspace = Workspace::new();
        let mut session = session::SessionPanel::default();
        session.active = true;
        workspace.session = Some(session);
        let game_only = draw_frame(&mut workspace, None, false).game_rect;
        assert_eq!(game_only.min, egui::Pos2::ZERO);
        assert_eq!(game_only.size(), egui::vec2(1100.0, 740.0));
        let with_controls = draw_frame(&mut workspace, Some(egui::Key::F1), false).game_rect;
        assert!(workspace.controls_visible);
        assert!(with_controls.height() < game_only.height());
        assert_eq!(
            draw_frame(&mut workspace, Some(egui::Key::F1), false).game_rect,
            game_only
        );
        assert!(!workspace.controls_visible);
    }

    #[test]
    fn fullscreen_hides_controls_and_f1_can_reveal_them() {
        let mut workspace = Workspace::new();
        draw_frame(&mut workspace, Some(egui::Key::F1), false);
        assert!(workspace.controls_visible);
        let request = draw_frame(&mut workspace, Some(egui::Key::F4), false);
        assert_eq!(request.fullscreen_request, Some(true));
        assert!(!workspace.controls_visible);
        workspace.controls_visible = true;
        draw_frame(&mut workspace, None, true);
        assert!(!workspace.controls_visible);
        draw_frame(&mut workspace, Some(egui::Key::F1), true);
        assert!(workspace.controls_visible);
        assert_eq!(
            draw_frame(&mut workspace, Some(egui::Key::F4), true).fullscreen_request,
            Some(false)
        );
    }

    #[test]
    fn panel_shortcuts_reveal_ui_and_hidden_floating_view_has_no_chrome() {
        let mut workspace = Workspace::new();
        workspace.view_mode = ViewMode::Floating;
        assert_eq!(
            draw_frame(&mut workspace, None, false).game_rect.size(),
            egui::vec2(1100.0, 740.0)
        );
        for key in [egui::Key::F2, egui::Key::F3, egui::Key::F10] {
            workspace.controls_visible = false;
            draw_frame(&mut workspace, Some(key), false);
            assert!(workspace.controls_visible);
            assert_eq!(workspace.view_mode, ViewMode::Docked);
        }
        assert!(workspace.performance_visible());
        draw_frame(&mut workspace, Some(egui::Key::F1), false);
        assert!(!workspace.performance_visible());
    }

    #[test]
    fn cooperative_game_crash_reveals_controls_and_clears_loading() {
        let mut workspace = Workspace::new();
        workspace.set_loading(1, 10);
        workspace.game_stopped("Ruby crash marker");
        assert!(workspace.controls_visible && workspace.panels_top && workspace.panels_output);
        assert!(workspace.loading.is_none());
        assert_eq!(
            workspace.logs.last().map(String::as_str),
            Some("Ruby crash marker")
        );
        draw_frame(&mut workspace, None, false);
        draw_frame(&mut workspace, Some(egui::Key::F1), false);
        assert!(!workspace.controls_visible);
    }

    #[test]
    fn stopped_session_keeps_crash_output_visible_when_output_was_hidden() {
        let mut workspace = Workspace::new();
        workspace.session = Some(session::SessionPanel::default());
        workspace.panels_output = false;
        workspace.logs = vec!["script.rb:7: crash marker (RuntimeError)".into()];
        let context = egui::Context::default();
        let info = WorkspaceInfo {
            logical_size: (640, 480),
            fps: 0.0,
            frames: 0,
            global_fullscreen: false,
            show_fullscreen: true,
            show_save_buttons: false,
        };
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            workspace.draw(ui, &info);
        });
        output.textures_delta.clear();
        fn contains_crash(shape: &egui::Shape) -> bool {
            match shape {
                egui::Shape::Text(text) => text.galley.text().contains("crash marker"),
                egui::Shape::Vec(shapes) => shapes.iter().any(contains_crash),
                _ => false,
            }
        }
        assert!(output
            .shapes
            .iter()
            .any(|shape| contains_crash(&shape.shape)));
    }

    #[test]
    fn drawing_before_the_runner_is_ready_does_not_override_its_rate() {
        crate::runtime_settings::reset();
        let mut workspace = Workspace::new();
        let mut session = session::SessionPanel::default();
        session.active = true;
        workspace.session = Some(session);
        let context = egui::Context::default();
        let info = WorkspaceInfo {
            logical_size: (640, 480),
            fps: 0.0,
            frames: 0,
            global_fullscreen: false,
            show_fullscreen: true,
            show_save_buttons: false,
        };
        for _ in 0..3 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100.0, 740.0),
                )),
                ..Default::default()
            };
            context
                .run_ui(input, |ui| {
                    workspace.draw(ui, &info);
                })
                .drop_without_applying_deltas();
            assert_eq!(crate::runtime_settings::rate_override(), None);
        }
    }
}
