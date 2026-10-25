// SPDX-License-Identifier: GPL-2.0-or-later
//! Workspace root: the single egui UI shared by every frontend.
//!
//! The desktop window and the browser canvas both render this module: the
//! toolbar, the runtime-patches panel, the output log, the performance panel, the game
//! view placement, and — behind `game-api` — the native host controls. A frontend supplies
//! per-frame
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
//!   `fullscreen_request`, an optional `fit_aspect` window-fit hint, and
//!   `export_saves` / `import_saves` intents.
//! * [`ViewMode`] — how the game view is placed (docked, workspace-fullscreen,
//!   or floating).
//! * `Tool` (private) — the active native tool tab (runtime or online play);
//!   game explorers are supplied by plugins.
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
mod game_view;
mod library;
mod logs;
mod patches;
mod performance;
mod saves;
pub mod session;
pub mod theme;
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

pub use rsxp_nds::ui::DsLayout;

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
    /// Explicit core identity used to gate core-specific UI (for example DS
    /// screen layouts). Frame dimensions alone are not a safe discriminator.
    pub core_id: &'static str,
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
    pub touch_rect: egui::Rect,
    pub fullscreen_request: Option<bool>,
    /// The native aspect ratio to enforce on the host window while the game is
    /// shown without panels. `None` when the UI is visible, fullscreen, or no
    /// game is running. Browsers ignore this; native hosts resize the window.
    pub fit_aspect: Option<f32>,
    pub export_saves: bool,
    pub import_saves: bool,
    pub create_backup: bool,
    pub restore_latest_backup: bool,
    pub open_game_folder: bool,
    pub open_save_folder: bool,
    pub open_patches_folder: bool,
}

impl Default for WorkspaceOutput {
    fn default() -> Self {
        Self {
            game_rect: egui::Rect::ZERO,
            touch_rect: egui::Rect::ZERO,
            fullscreen_request: None,
            fit_aspect: None,
            export_saves: false,
            import_saves: false,
            create_backup: false,
            restore_latest_backup: false,
            open_game_folder: false,
            open_save_folder: false,
            open_patches_folder: false,
        }
    }
}

/// Persistent workspace UI state.
#[derive(Default, PartialEq, Eq, Clone, Copy)]
enum Tool {
    #[default]
    Runtime,
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
    logs_filter: String,
    logs_follow: bool,
    shortcuts_open: bool,
    fps_history: std::collections::VecDeque<f32>,
    view_mode: ViewMode,
    ds_layout: DsLayout,
    controls_visible: bool,
    controller_settings_open: bool,
    keyboard_capture: Option<i32>,
    keyboard_add_action: i32,
    was_global_fullscreen: bool,
    reset_confirm: bool,
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
            logs_filter: String::new(),
            logs_follow: true,
            shortcuts_open: false,
            fps_history: std::collections::VecDeque::new(),
            view_mode: ViewMode::default(),
            ds_layout: DsLayout::default(),
            controls_visible: false,
            controller_settings_open: false,
            keyboard_capture: None,
            keyboard_add_action: 0,
            was_global_fullscreen: false,
            reset_confirm: false,
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
        if info.fps > 0.0 {
            self.fps_history.push_back(info.fps as f32);
            while self.fps_history.len() > 120 {
                self.fps_history.pop_front();
            }
        }
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
        if key_pressed(egui::Key::F11) {
            let fast_forward = self.backend.fast_forward();
            self.backend.set_fast_forward(!fast_forward);
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

        if !self.controls_visible {
            if !info.global_fullscreen && output.fullscreen_request != Some(true) {
                output.fit_aspect = if info.core_id == "nds" {
                    Some(self.ds_layout.aspect())
                } else {
                    native_aspect(info.logical_size)
                };
            }
            self.draw_game_view(ui, &mut output, info.core_id, info.logical_size);
            return output;
        }
        self.draw_toolbar(ui, info, &mut output);

        let fullscreen = self.is_workspace_fullscreen();
        if !fullscreen && self.panels_top {
            egui::Panel::top("workspace-top-panel")
                .resizable(false)
                .frame(
                    egui::Frame::new()
                        .fill(theme::SURFACE)
                        .inner_margin(egui::Margin::symmetric(18, 12)),
                )
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        theme::section_label(ui, "Session overview", theme::PINK);
                        let title = self
                            .session
                            .as_ref()
                            .filter(|session| session.active && !session.title.is_empty())
                            .map(|session| session.title.as_str())
                            .unwrap_or("Workspace");
                        ui.label(theme::display(title, 24.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            theme::tag(
                                ui,
                                &format!("{} × {}", info.logical_size.0, info.logical_size.1),
                                theme::Tone::Neutral,
                            );
                            if info.fps > 0.0 {
                                theme::tag(ui, &format!("{:.0} fps", info.fps), theme::Tone::Cyan);
                            }
                            theme::tag(
                                ui,
                                &format!("{} frames", info.frames),
                                theme::Tone::Neutral,
                            );
                        });
                    });
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        if theme::chip(ui, "Game tools · F2", theme::Tone::Cyan, self.panels_left)
                            .clicked()
                        {
                            self.panels_left = !self.panels_left;
                        }
                        if theme::chip(ui, "Activity · F3", theme::Tone::Pink, self.panels_output)
                            .clicked()
                        {
                            self.panels_output = !self.panels_output;
                        }
                        if theme::chip(
                            ui,
                            "Performance · F10",
                            theme::Tone::Lime,
                            self.panels_performance,
                        )
                        .clicked()
                        {
                            self.panels_performance = !self.panels_performance;
                        }
                        if theme::button_sized(
                            ui,
                            "Controller settings",
                            theme::ButtonKind::Ghost,
                            theme::ButtonSize::Sm,
                        )
                        .clicked()
                        {
                            self.controller_settings_open = true;
                        }
                    });
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
        self.draw_game_view(ui, &mut output, info.core_id, info.logical_size);
        self.draw_controller_settings(&context);
        self.draw_shortcuts(&context);

        if self.reset_confirm {
            egui::Window::new("Reset game?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(&context, |ui| {
                    ui.horizontal(|ui| {
                        theme::status_dot(ui, theme::WARNING, 1.0);
                        ui.label(theme::display("Restart from the beginning?", 20.0));
                    });
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(
                            "Unsaved progress is lost. This applies to the running session.",
                        )
                        .font(theme::body_font(13.0))
                        .color(theme::TEXT_MUTED),
                    );
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if theme::button(ui, "Reset game", theme::ButtonKind::Destructive).clicked()
                        {
                            self.backend.request_reset();
                            self.reset_confirm = false;
                        }
                        if theme::button(ui, "Cancel", theme::ButtonKind::Secondary).clicked() {
                            self.reset_confirm = false;
                        }
                    });
                });
        }

        output
    }

    fn draw_shortcuts(&mut self, context: &egui::Context) {
        if !self.shortcuts_open {
            return;
        }
        let mut open = self.shortcuts_open;
        egui::Window::new("Shortcuts")
            .id(egui::Id::unique("workspace-shortcuts"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(440.0)
            .show(context, |ui| {
                theme::section_label(ui, "Keyboard", theme::PINK);
                egui::Grid::new("shortcut-grid")
                    .num_columns(2)
                    .spacing([18.0, 8.0])
                    .show(ui, |ui| {
                        for (key, action) in [
                            ("F1", "Show or hide the interface"),
                            ("F2", "Game tools and runtime patches"),
                            ("F3", "Runtime output / activity"),
                            ("F4", "Toggle fullscreen"),
                            ("F10", "Performance panel"),
                            ("F11", "Toggle uncapped execution"),
                            ("Enter", "Play the selected library game"),
                            ("Arrows", "Move through the library"),
                        ] {
                            theme::keycap(ui, key);
                            ui.label(
                                egui::RichText::new(action)
                                    .font(theme::body_font(13.0))
                                    .color(theme::TEXT_MUTED),
                            );
                            ui.end_row();
                        }
                    });
                ui.add_space(12.0);
                theme::divider(ui);
                theme::section_label(ui, "Controller", theme::CYAN);
                ui.label(
                    egui::RichText::new(
                        "D-pad or left stick moves · South confirms · East cancels. Remap every button under Controls.",
                    )
                    .font(theme::body_font(13.0))
                    .color(theme::TEXT_MUTED),
                );
            });
        self.shortcuts_open = open;
    }
}

fn native_aspect(size: (u32, u32)) -> Option<f32> {
    let (width, height) = size;
    (width > 0 && height > 0).then(|| width as f32 / height as f32)
}

fn draw_loading(ui: &mut egui::Ui, done: u32, total: u32) {
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(theme::BG))
        .show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.30);
                ui.label(
                    egui::RichText::new("NOW LOADING")
                        .font(theme::mono_font(11.0))
                        .color(theme::PINK),
                );
                ui.add_space(8.0);
                ui.label(theme::display_color("RSXP", 48.0, theme::PINK));
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("Preparing your game")
                        .font(theme::body_font(16.0))
                        .color(theme::TEXT_MUTED),
                );
                ui.add_space(22.0);
                let fraction = if total == 0 {
                    0.0
                } else {
                    done as f32 / total as f32
                };
                let progress_width = ui.available_width().clamp(160.0, 380.0);
                theme::neon_progress(ui, fraction, progress_width);
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new(format!(
                        "{} / {}  ·  {:.0}%",
                        done,
                        total,
                        fraction * 100.0
                    ))
                    .font(theme::mono_font(13.0))
                    .color(theme::CYAN),
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
        draw_frame_sized(workspace, key, fullscreen, (640, 480))
    }

    fn draw_frame_sized(
        workspace: &mut Workspace,
        key: Option<egui::Key>,
        fullscreen: bool,
        logical_size: (u32, u32),
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
            core_id: "rgss",
            logical_size,
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
        let game_only = draw_frame(&mut workspace, None, false);
        assert!(
            (game_only.game_rect.width() / game_only.game_rect.height() - 4.0 / 3.0).abs() < 0.01
        );
        assert!((game_only.game_rect.height() - 740.0).abs() < 0.5);
        assert_eq!(game_only.fit_aspect, Some(4.0 / 3.0));
        let with_controls = draw_frame(&mut workspace, Some(egui::Key::F1), false);
        assert!(workspace.controls_visible);
        assert_eq!(with_controls.fit_aspect, None);
        assert!(with_controls.game_rect.height() < game_only.game_rect.height());
        let hidden = draw_frame(&mut workspace, Some(egui::Key::F1), false);
        assert_eq!(hidden.game_rect, game_only.game_rect);
        assert_eq!(hidden.fit_aspect, Some(4.0 / 3.0));
        assert!(!workspace.controls_visible);
    }

    #[test]
    fn ds_aspect_is_preserved_and_reported_for_window_fitting() {
        let mut workspace = Workspace::new();
        let mut session = session::SessionPanel::default();
        session.active = true;
        workspace.session = Some(session);
        let output = draw_frame_sized(&mut workspace, None, false, (256, 384));
        assert!(
            (output.game_rect.width() / output.game_rect.height() - 256.0 / 384.0).abs() < 0.01
        );
        assert!((output.game_rect.height() - 740.0).abs() < 0.5);
        assert!((output.fit_aspect.unwrap() - 2.0 / 3.0).abs() < 0.001);
    }

    #[test]
    fn fullscreen_hides_controls_and_f1_can_reveal_them() {
        let mut workspace = Workspace::new();
        draw_frame(&mut workspace, Some(egui::Key::F1), false);
        assert!(workspace.controls_visible);
        let request = draw_frame(&mut workspace, Some(egui::Key::F4), false);
        assert_eq!(request.fullscreen_request, Some(true));
        assert_eq!(request.fit_aspect, None);
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
        let hidden = draw_frame(&mut workspace, None, false);
        assert!((hidden.game_rect.width() / hidden.game_rect.height() - 4.0 / 3.0).abs() < 0.01);
        assert_eq!(hidden.fit_aspect, Some(4.0 / 3.0));
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
            core_id: "rgss",
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
        rsxp::runtime_settings::reset();
        let mut workspace = Workspace::new();
        let mut session = session::SessionPanel::default();
        session.active = true;
        workspace.session = Some(session);
        let context = egui::Context::default();
        let info = WorkspaceInfo {
            core_id: "rgss",
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
            assert_eq!(rsxp::runtime_settings::rate_override(), None);
        }
    }
}
