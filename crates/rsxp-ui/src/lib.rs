// SPDX-License-Identifier: GPL-2.0-or-later
//! Shared eframe application and egui frontend for native and browser runners.
use base64::Engine as _;
use rsxp::{
    graphics::Bitmap,
    ui::{Workspace, WorkspaceInfo, WorkspaceOutput},
};
use rsxp_extension_api::{OperationRequest, OperationResponse, UiDescription};
use rsxp_extensions::{ExtensionError, ExtensionHost, ExtensionPackage};
#[cfg(not(target_arch = "wasm32"))]
use std::{env, fs, path::PathBuf};

/// Common frontend state. Platform runners update the frame and [`WorkspaceInfo`],
/// and execute the platform-neutral actions returned by the workspace.
pub struct SharedApp {
    pub workspace: Workspace,
    info: WorkspaceInfo,
    frame: Option<egui::TextureHandle>,
    frame_size: Option<[usize; 2]>,
    frame_revision: Option<u64>,
    context: Option<egui::Context>,
    output: WorkspaceOutput,
    rgba: Vec<u8>,
    pub extension_ui: rsxp_extension_ui::ExtensionUi,
    extensions: ExtensionHost,
    extension_error: Option<String>,
    extension_import_requested: bool,
    plugin_paste_open: bool,
    plugin_paste_id: String,
    plugin_paste_source: String,
    plugin_paste_error: Option<String>,
}

impl Default for SharedApp {
    fn default() -> Self {
        let mut plugins = ExtensionHost::new();
        let package = ExtensionPackage::from_rhai(
            "pokemon-essentials-debug",
            include_str!("../../../extensions/essentials-debug/main.rhai").into(),
        )
        .expect("bundled Essentials debug plugin must be valid");
        plugins
            .load(package)
            .expect("bundled Essentials debug plugin must load");
        let package = ExtensionPackage::from_rhai_with_ruby(
            "essentials-game-api",
            include_str!("../../../extensions/essentials-game-api/main.rhai").into(),
            Some(include_str!("../../../extensions/essentials-game-api/game_api.rb").into()),
        )
        .expect("bundled Essentials game API plugin must be valid");
        plugins
            .load(package)
            .expect("bundled Essentials game API plugin must load");
        let package = ExtensionPackage::from_rhai(
            "essentials-runtime-patches",
            include_str!("../../../extensions/essentials-runtime-patches.rhai").into(),
        )
        .expect("bundled Essentials runtime patches plugin must be valid");
        plugins
            .load(package)
            .expect("bundled Essentials runtime patches plugin must load");
        let package = ExtensionPackage::from_rhai(
            "pokeapi",
            include_str!("../../../extensions/pokeapi.rhai").into(),
        )
        .expect("bundled PokeAPI plugin must be valid");
        plugins
            .load(package)
            .expect("bundled PokeAPI plugin must load");
        let package = ExtensionPackage::from_rhai(
            "essentials-world-map",
            include_str!("../../../extensions/essentials-world-map.rhai").into(),
        )
        .expect("bundled Essentials World Map plugin must be valid");
        plugins
            .load(package)
            .expect("bundled Essentials World Map plugin must load");
        let package = ExtensionPackage::from_rhai(
            "essentials-movedex",
            include_str!("../../../extensions/essentials-movedex.rhai").into(),
        )
        .expect("bundled Essentials Movedex plugin must be valid");
        plugins
            .load(package)
            .expect("bundled Essentials Movedex plugin must load");
        let package = ExtensionPackage::from_rhai(
            "essentials-itemdex",
            include_str!("../../../extensions/essentials-itemdex.rhai").into(),
        )
        .expect("bundled Essentials Itemdex plugin must be valid");
        plugins
            .load(package)
            .expect("bundled Essentials Itemdex plugin must load");
        let package = ExtensionPackage::from_rhai(
            "essentials-pokedex",
            include_str!("../../../extensions/essentials-pokedex.rhai").into(),
        )
        .expect("bundled Essentials Pokedex plugin must be valid");
        plugins
            .load(package)
            .expect("bundled Essentials Pokedex plugin must load");
        let package = ExtensionPackage::from_rhai(
            "essentials-encounters",
            include_str!("../../../extensions/essentials-encounters.rhai").into(),
        )
        .expect("bundled Essentials encounters plugin must be valid");
        plugins
            .load(package)
            .expect("bundled Essentials encounters plugin must load");
        let package = ExtensionPackage::from_rhai(
            "essentials-inventory",
            include_str!("../../../extensions/essentials-inventory/main.rhai").into(),
        )
        .expect("bundled Essentials inventory plugin must be valid");
        plugins
            .load(package)
            .expect("bundled Essentials inventory plugin must load");
        #[cfg(not(target_arch = "wasm32"))]
        {
            let package = ExtensionPackage::from_rhai(
                "essentials-online-play",
                include_str!("../../../extensions/essentials-online-play.rhai").into(),
            )
            .expect("bundled Essentials online-play plugin must be valid");
            plugins
                .load(package)
                .expect("bundled Essentials online-play plugin must load");
        }
        #[cfg(not(target_arch = "wasm32"))]
        load_external_plugins(&mut plugins);
        rsxp::api_bridge::register_extension_routes(plugins.route_aliases());
        Self::new(
            Workspace::new(),
            WorkspaceInfo {
                core_id: "rgss",
                logical_size: (640, 480),
                fps: 0.0,
                frames: 0,
                global_fullscreen: false,
                show_fullscreen: true,
                show_save_buttons: false,
            },
        )
        .with_plugins(plugins)
    }
}

/// Loads native Rhai plugins from `RSXP_EXTENSIONS_DIR`. The variable
/// may point at one package directory or at a directory containing immediate
/// package subdirectories. Embedded bundles win when an external package uses
/// the same ID; unrelated packages are loaded through the same manifest,
/// capability, Rhai and Ruby-source paths as bundled plugins.
#[cfg(not(target_arch = "wasm32"))]
fn load_external_plugins(plugins: &mut ExtensionHost) {
    let Some(root) = env::var_os("RSXP_EXTENSIONS_DIR").map(PathBuf::from) else {
        return;
    };
    let candidates = if root.is_file() {
        vec![root]
    } else {
        let Ok(entries) = fs::read_dir(&root) else {
            eprintln!("[Plugins] cannot read {}", root.display());
            return;
        };
        entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_file() && path.extension().and_then(|value| value.to_str()) == Some("rhai")
            })
            .collect()
    };
    for path in candidates {
        match ExtensionPackage::load(&path) {
            Ok(package) if plugins.contains(&package.manifest.id) => {
                eprintln!(
                    "[Plugins] skipped {} because {} is already embedded",
                    path.display(),
                    package.manifest.id
                );
            }
            Ok(package) => {
                let id = package.manifest.id.clone();
                match plugins.load(package) {
                    Ok(()) => eprintln!("[Plugins] loaded {id} from {}", path.display()),
                    Err(error) => eprintln!("[Plugins] failed to load {id}: {error}"),
                }
            }
            Err(error) => eprintln!("[Plugins] failed to read {}: {error}", path.display()),
        }
    }
}

impl SharedApp {
    pub fn new(workspace: Workspace, info: WorkspaceInfo) -> Self {
        Self {
            workspace,
            info,
            frame: None,
            frame_size: None,
            frame_revision: None,
            context: None,
            output: WorkspaceOutput::default(),
            rgba: Vec::new(),
            extension_ui: rsxp_extension_ui::ExtensionUi::default(),
            extensions: ExtensionHost::new(),
            extension_error: None,
            extension_import_requested: false,
            plugin_paste_open: false,
            plugin_paste_id: "pasted-plugin".into(),
            plugin_paste_source: String::new(),
            plugin_paste_error: None,
        }
    }

    fn with_plugins(mut self, plugins: ExtensionHost) -> Self {
        self.extensions = plugins;
        self
    }

    /// Loads a self-contained Rhai plugin from pasted text. If the host has
    /// already activated its plugins, the new script is activated
    /// immediately; otherwise it participates in the next session activation.
    pub fn paste_plugin(&mut self, id: impl Into<String>, source: String) -> Result<(), String> {
        self.extensions
            .load_rhai(id, source)
            .map_err(|error| self.record_extension_error(error))?;
        rsxp::api_bridge::register_extension_routes(self.extensions.route_aliases());
        Ok(())
    }

    /// Loads a single `.rhai` plugin file from disk on native
    /// hosts. The importer uses exactly the same package validation as startup
    /// discovery and accepts embedded-Ruby source inside the Rhai script.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn import_plugin(&mut self, path: impl AsRef<std::path::Path>) -> Result<(), String> {
        let package =
            ExtensionPackage::load(path).map_err(|error| self.record_extension_error(error))?;
        self.extensions
            .load(package)
            .map_err(|error| self.record_extension_error(error))?;
        rsxp::api_bridge::register_extension_routes(self.extensions.route_aliases());
        Ok(())
    }

    pub fn set_info(&mut self, info: WorkspaceInfo) {
        self.info = info;
    }

    /// Enables the toolbar's save export/import and snapshot actions for hosts
    /// that provide their own save adapter (the browser page does).
    pub fn set_save_buttons(&mut self, enabled: bool) {
        self.info.show_save_buttons = enabled;
    }

    /// Updates the live performance figures shown by the workspace.
    ///
    /// Runners that don't have a complete [`WorkspaceInfo`] still need to
    /// report the FPS counters; keeping this separate avoids overwriting the
    /// frontend-specific flags when a frame arrives.
    pub fn set_performance(&mut self, fps: f64, frames: u64) {
        self.info.fps = fps;
        self.info.frames = frames;
    }

    /// True while an egui text field or other widget owns the keyboard.
    pub fn wants_keyboard(&self) -> bool {
        self.context
            .as_ref()
            .is_some_and(egui::Context::egui_wants_keyboard_input)
    }

    pub fn set_game_paint_callback(&mut self, callback: Option<egui::PaintCallback>) {
        self.workspace.set_game_paint_callback(callback);
    }

    pub fn set_logical_size(&mut self, size: (u32, u32)) {
        self.info.logical_size = size;
    }

    pub fn draw(&mut self, ui: &mut egui::Ui) {
        self.context = Some(ui.ctx().clone());
        let mut descriptions: Vec<_> = self.extensions.ui().cloned().collect();
        if let Some(error) = &self.extension_error {
            for description in &mut descriptions {
                description.error = Some(error.clone());
            }
        }
        self.extension_ui.set_descriptions(descriptions);
        self.workspace
            .set_frame_texture(self.frame.as_ref().map(egui::TextureHandle::id));
        let extension_ui = &mut self.extension_ui;
        self.output = self
            .workspace
            .draw_with_plugin_ui(ui, &self.info, &mut |plugin_ui| {
                extension_ui.draw(plugin_ui)
            });
        if let Some(fullscreen) = self.output.fullscreen_request {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Fullscreen(fullscreen));
        }
        for event in self.extension_ui.take_events() {
            if event.panel_id == "__host" && event.widget_id == "import-plugin" {
                self.extension_import_requested = true;
            } else if event.panel_id == "__host" && event.widget_id == "paste-plugin" {
                self.plugin_paste_open = true;
                self.plugin_paste_error = None;
            } else if let Err(error) = self.extensions.dispatch_ui_event(event) {
                self.extension_error = Some(error.to_string());
            }
        }
        self.draw_plugin_paste_window(ui.ctx());
    }

    /// Returns and clears a native plugin-import request emitted by the
    /// Runtime tools panel.
    pub fn take_plugin_import_request(&mut self) -> bool {
        std::mem::take(&mut self.extension_import_requested)
    }

    /// Compatibility alias for frontends that still use the old terminology.
    pub fn take_extension_import_request(&mut self) -> bool {
        self.take_plugin_import_request()
    }

    /// Compatibility alias for callers migrating from extensions to plugins.
    pub fn paste_extension(&mut self, id: impl Into<String>, source: String) -> Result<(), String> {
        self.paste_plugin(id, source)
    }

    /// Compatibility alias for callers migrating from extensions to plugins.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn import_extension(&mut self, path: impl AsRef<std::path::Path>) -> Result<(), String> {
        self.import_plugin(path)
    }

    fn draw_plugin_paste_window(&mut self, context: &egui::Context) {
        if !self.plugin_paste_open {
            return;
        }
        let mut open = true;
        let mut submit = false;
        let mut cancel = false;
        egui::Window::new("Paste Rhai plugin")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size(egui::vec2(720.0, 520.0))
            .show(context, |ui| {
                ui.label("Paste a complete self-contained .rhai plugin. Metadata comments can set its id, name, routes, and panels.");
                ui.horizontal(|ui| {
                    ui.label("Fallback id");
                    ui.text_edit_singleline(&mut self.plugin_paste_id);
                });
                egui::ScrollArea::vertical()
                    .id_salt("plugin-paste-source")
                    .max_height(360.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut self.plugin_paste_source)
                                .code_editor()
                                .desired_rows(20)
                                .desired_width(f32::INFINITY),
                        );
                    });
                if let Some(error) = &self.plugin_paste_error {
                    ui.colored_label(egui::Color32::RED, error);
                }
                ui.horizontal(|ui| {
                    if ui.button("Load plugin").clicked() {
                        submit = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if submit {
            let id = self.plugin_paste_id.trim();
            if id.is_empty() {
                self.plugin_paste_error = Some("Fallback id cannot be empty".into());
            } else if self.plugin_paste_source.trim().is_empty() {
                self.plugin_paste_error = Some("Plugin text cannot be empty".into());
            } else {
                match self.paste_plugin(id.to_owned(), self.plugin_paste_source.clone()) {
                    Ok(()) => {
                        self.plugin_paste_open = false;
                        self.plugin_paste_source.clear();
                        self.plugin_paste_error = None;
                    }
                    Err(error) => self.plugin_paste_error = Some(error),
                }
            }
        }
        self.plugin_paste_open = open && !cancel && self.plugin_paste_open;
    }

    /// Publishes cached plugin descriptions for the next draw pass.
    pub fn set_plugin_descriptions(
        &mut self,
        descriptions: impl IntoIterator<Item = UiDescription>,
    ) {
        self.extension_ui.set_descriptions(descriptions);
    }

    pub fn set_extension_descriptions(
        &mut self,
        descriptions: impl IntoIterator<Item = UiDescription>,
    ) {
        self.set_plugin_descriptions(descriptions);
    }

    /// Activates all loaded plugins after a game session is ready.
    pub fn activate_plugins(&mut self) -> Result<(), String> {
        self.extensions
            .activate_all()
            .map_err(|error| self.record_extension_error(error))
    }

    pub fn activate_extensions(&mut self) -> Result<(), String> {
        self.activate_plugins()
    }

    /// Invalidates pending plugin work when a game is reset or replaced.
    pub fn reset_plugins(&mut self) {
        self.extensions.reset_generation();
        self.extension_error = None;
    }

    pub fn reset_extensions(&mut self) {
        self.reset_plugins();
    }

    /// Drains plugin requests for the host adapter (desktop HTTP/RGSS, browser
    /// JS, or a headless executor) without executing script code in layout.
    pub fn take_plugin_requests(&mut self) -> Vec<OperationRequest> {
        self.extensions.take_requests()
    }

    pub fn take_extension_requests(&mut self) -> Vec<OperationRequest> {
        self.take_plugin_requests()
    }

    /// Delivers a completed operation to the owning plugin.
    pub fn dispatch_plugin_response(&mut self, response: OperationResponse) -> Result<(), String> {
        self.extensions
            .dispatch_response(response)
            .map_err(|error| self.record_extension_error(error))
    }

    pub fn dispatch_extension_response(
        &mut self,
        response: OperationResponse,
    ) -> Result<(), String> {
        self.dispatch_plugin_response(response)
    }

    /// Reads one game image for a plugin webview through the active game's
    /// backend and returns a one-shot PNG data URL. Sprite sheets are cropped
    /// for Pokémon artwork; tilesets and autotiles retain their full bitmap
    /// because map plugins need the complete sheet. The response is not
    /// retained by the plugin host.
    pub fn plugin_asset_data_url(&self, category: &str, name: &str) -> Option<String> {
        let image = self.workspace.backend.asset_image(category, name)?;
        let bytes = if matches!(category, "Tilesets" | "Autotiles") {
            rsxp::ui::png_bytes(image.as_ref())?
        } else {
            rsxp::ui::first_pokemon_sprite_png(image.as_ref())?
        };
        Some(format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ))
    }

    fn record_extension_error(&mut self, error: ExtensionError) -> String {
        let message = error.to_string();
        self.extension_error = Some(message.clone());
        message
    }

    pub fn set_bitmap(&mut self, context: &egui::Context, bitmap: &Bitmap) {
        let revision = bitmap.revision();
        let size = [bitmap.width() as usize, bitmap.height() as usize];
        // The submitted frame is authoritative. Hosts may receive a frame
        // before their session metadata update (notably cartridge sessions),
        // so derive the layout aspect from the texture dimensions as well.
        self.info.logical_size = (bitmap.width(), bitmap.height());
        if self.frame_revision == Some(revision) && self.frame_size == Some(size) {
            return;
        }
        bitmap.write_rgba(&mut self.rgba);
        let image = egui::ColorImage::from_rgba_unmultiplied(size, &self.rgba);
        if self.frame_size != Some(size) {
            self.frame =
                Some(context.load_texture("rsxp-game-frame", image, egui::TextureOptions::NEAREST));
            self.frame_size = Some(size);
        } else if let Some(frame) = &mut self.frame {
            frame.set(image, egui::TextureOptions::NEAREST);
        }
        self.frame_revision = Some(revision);
        self.workspace
            .set_frame_texture(self.frame.as_ref().map(egui::TextureHandle::id));
        context.request_repaint();
    }

    pub fn update_bitmap(&mut self, bitmap: &Bitmap) {
        if let Some(context) = self.context.clone() {
            self.set_bitmap(&context, bitmap);
        }
    }

    pub fn game_rect(&self) -> egui::Rect {
        self.output.game_rect
    }

    pub fn request_repaint(&self) {
        if let Some(context) = &self.context {
            context.request_repaint();
        }
    }

    pub fn take_output(&mut self) -> WorkspaceOutput {
        std::mem::take(&mut self.output)
    }

    /// Consumes only the save-related intents from the last workspace frame,
    /// leaving `game_rect` intact for the browser's pointer mapping.
    pub fn take_save_actions(&mut self) -> (bool, bool, bool, bool) {
        let actions = (
            self.output.export_saves,
            self.output.import_saves,
            self.output.create_backup,
            self.output.restore_latest_backup,
        );
        self.output.export_saves = false;
        self.output.import_saves = false;
        self.output.create_backup = false;
        self.output.restore_latest_backup = false;
        actions
    }

    /// Consumes folder-open intents from the last workspace frame.
    pub fn take_folder_actions(&mut self) -> (bool, bool, bool) {
        let actions = (
            self.output.open_game_folder,
            self.output.open_save_folder,
            self.output.open_patches_folder,
        );
        self.output.open_game_folder = false;
        self.output.open_save_folder = false;
        self.output.open_patches_folder = false;
        actions
    }
}

impl eframe::App for SharedApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }

    #[cfg(target_arch = "wasm32")]
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub fn android_main(app: winit::platform::android::activity::AndroidApp) {
    let options = eframe::NativeOptions {
        android_app: Some(app),
        ..Default::default()
    };
    eframe::run_native(
        "RSXP",
        options,
        Box::new(|_| {
            let mut app = SharedApp::default();
            let mut panel = rsxp::ui::session::SessionPanel::default();
            panel.capabilities = rsxp::ui::session::SessionCapabilities::MOBILE;
            app.workspace.session = Some(panel);
            Ok(Box::new(app))
        }),
    )
    .unwrap();
}
