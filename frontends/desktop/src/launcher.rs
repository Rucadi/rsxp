//! Desktop library, permanent catalogs, common/per-game settings, and saves.
//!
//! Blocking work stays off the UI thread. Catalog sources are stored in
//! `config/config.json`; clicking refresh updates their offline caches and the
//! UI reads only the cache, so an unreachable server never empties the library.
//!
//! The presentation follows the Adventure Journal design system: a calm
//! library for discovering and returning to RPG adventures.
use fanplay_provision::catalog::CatalogEntry;
use fanplay_provision::settings::Settings;
use fanplay_provision::{self as provision, Repository};
use fanplay_ui::theme::{self, ButtonKind, ButtonSize, Tone};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
};

const CARD_WIDTH: f32 = 203.0;
const CARD_HEIGHT: f32 = 344.0;
const CARD_MARGIN: f32 = 14.0;
const ART_WIDTH: f32 = CARD_WIDTH - CARD_MARGIN * 2.0;
const ART_HEIGHT: f32 = 150.0;

#[derive(PartialEq, Clone, Copy)]
enum Page {
    Library,
    Repositories,
    Settings,
    Storage,
}
enum Completed {
    Catalog(Vec<Repository>, Vec<String>, Vec<CatalogEntry>),
    Play(PathBuf),
    Imported(String),
    CoverAssigned(String, PathBuf, String),
}
enum Cover {
    Loading(Receiver<Result<egui::ColorImage, String>>),
    Ready(egui::TextureHandle),
    Failed(String),
}
pub struct Launcher {
    covers: std::collections::HashMap<String, Cover>,
    preferences: std::collections::HashMap<String, provision::settings::Preferences>,
    editor: Option<crate::launcher_settings::Editor>,
    root: PathBuf,
    entries: Vec<CatalogEntry>,
    catalogs: Vec<Repository>,
    common: Settings,
    source: String,
    query: String,
    platform: String,
    page: Page,
    task: Option<Receiver<Result<Completed, String>>>,
    status: String,
    error: Option<String>,
    theme_name: String,
    themes: Vec<theme::ThemeDefinition>,
}
impl Default for Launcher {
    fn default() -> Self {
        let root = provision::root(None);
        let entries = provision::catalog::load_entries(&root);
        let (catalogs, warnings) = provision::catalog::selected(&root);
        let common = provision::settings::common(&root).unwrap_or_default();
        let themes = theme::load_themes(&root.join("config/themes.json"));
        let status = if !entries.is_empty() && catalogs.is_empty() {
            "Refresh repositories to load your permanent catalogs."
        } else {
            "Your adventures, ready to continue."
        };
        Self {
            covers: Default::default(),
            preferences: Default::default(),
            editor: None,
            root,
            entries,
            catalogs,
            common,
            source: String::new(),
            query: String::new(),
            platform: "All platforms".into(),
            page: Page::Library,
            task: None,
            status: status.into(),
            error: (!warnings.is_empty()).then(|| warnings.join("\n")),
            theme_name: "Adventure Journal".into(),
            themes,
        }
    }
}
impl Launcher {
    fn job(
        &mut self,
        status: &str,
        job: impl FnOnce() -> Result<Completed, String> + Send + 'static,
    ) {
        let (tx, rx) = mpsc::channel();
        self.task = Some(rx);
        self.status = status.into();
        self.error = None;
        std::thread::spawn(move || {
            let _ = tx.send(job());
        });
    }
    fn refresh(&mut self) {
        let root = self.root.clone();
        self.job("Refreshing repositories…", move || {
            let (entries, mut warnings) = provision::catalog::refresh(&root, None);
            let (repos, selected_warnings) = provision::catalog::selected(&root);
            warnings.extend(selected_warnings);
            Ok(Completed::Catalog(repos, warnings, entries))
        });
    }
    fn reload_catalogs(&mut self) {
        let (repos, warnings) = provision::catalog::selected(&self.root);
        self.catalogs = repos;
        if !warnings.is_empty() {
            self.error = Some(warnings.join("\n"));
        }
    }
    fn open_common_editor(&mut self) {
        self.editor = Some(crate::launcher_settings::Editor {
            key: "common".into(),
            common: true,
            game_path: None,
            title: "All games".into(),
            root: self.root.clone(),
            prefs: Default::default(),
            patches: self.common.patches.join(", "),
            settings: self.common.clone(),
            error: None,
            capturing_key: None,
        });
    }

    pub fn draw(
        &mut self,
        ui: &mut egui::Ui,
        recent: &[String],
        runtime_error: Option<&str>,
    ) -> Option<PathBuf> {
        if let Some(selected) = self.themes.iter().find(|t| t.name == self.theme_name) {
            theme::set_active_theme(selected.clone());
        }
        theme::apply(ui.ctx());
        let mut launch = None;
        keyboard_navigation(ui.ctx(), &mut self.page);

        // Drops are launch shortcuts: a folder, Game.ini, or a cartridge ROM.
        if let Some(path) = ui.ctx().input(|input| {
            input
                .raw
                .dropped_files
                .first()
                .map(|file| file.path().to_owned())
        }) {
            let supported = path.is_dir()
                || path.extension().is_some_and(|extension| {
                    ["gb", "gbc", "gba", "nds", "ini"]
                        .iter()
                        .any(|wanted| extension.to_string_lossy().eq_ignore_ascii_case(wanted))
                });
            if supported {
                launch = Some(path);
            }
        }

        if let Some(task) = &self.task {
            match task.try_recv() {
                Ok(result) => {
                    self.task = None;
                    match result {
                        Ok(Completed::Catalog(repos, errors, entries)) => {
                            self.entries = entries;
                            self.catalogs = repos;
                            self.error = (!errors.is_empty()).then(|| errors.join("\n"));
                            self.status = "Repositories refreshed".into();
                        }
                        Ok(Completed::Play(path)) => {
                            self.status = "Ready to play".into();
                            launch = Some(path);
                        }
                        Ok(Completed::CoverAssigned(key, root, hash)) => {
                            let mut prefs = self.preferences.get(&key).cloned().unwrap_or_default();
                            prefs.cover_sha256 = Some(hash);
                            match provision::settings::save(&root, &key, &prefs) {
                                Ok(()) => {
                                    self.preferences.insert(key.clone(), prefs.clone());
                                    if let Some(editor) = &mut self.editor
                                        && editor.key == key
                                    {
                                        editor.prefs = prefs;
                                    }
                                    self.status = "Cover image saved".into();
                                }
                                Err(error) => self.error = Some(error.to_string()),
                            }
                        }
                        Ok(Completed::Imported(hash)) => {
                            self.status = format!("Base ROM imported · SHA-256 {hash}")
                        }
                        Err(e) => {
                            self.error = Some(e);
                            self.status = "Action failed".into();
                        }
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.task = None;
                    self.error = Some("Background task stopped unexpectedly".into());
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(100));
                }
            }
        }

        egui::Panel::left("launcher-nav")
            .exact_size(214.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(theme::surface())
                    .inner_margin(egui::Margin::symmetric(16, 18)),
            )
            .show(ui, |ui| self.draw_nav(ui, recent));

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::background())
                    .inner_margin(egui::Margin::symmetric(26, 20)),
            )
            .show(ui, |ui| {
                if let Some(error) = self.error.as_deref().or(runtime_error) {
                    launcher_error(ui, error);
                    ui.add_space(14.0);
                }
                match self.page {
                    Page::Library => self.draw_library_page(ui, recent, &mut launch),
                    Page::Repositories => self.draw_repositories_page(ui),
                    Page::Settings => self.draw_settings_page(ui),
                    Page::Storage => self.draw_storage_page(ui),
                }
            });
        self.draw_editor(ui.ctx());
        launch
    }

    fn draw_nav(&mut self, ui: &mut egui::Ui, recent: &[String]) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(104.0, 46.0), egui::Sense::hover());
        theme::glow(ui.painter(), rect, 10, theme::PINK, 0.55);
        theme::centered_text(ui, rect, "FANPLAY", theme::display_font(32.0), theme::PINK);
        ui.label(theme::eyebrow("RPG adventure library"));
        ui.add_space(24.0);
        for (index, (page, label)) in [
            (Page::Library, "Library"),
            (Page::Repositories, "Repositories"),
            (Page::Settings, "Settings"),
            (Page::Storage, "Storage & saves"),
        ]
        .into_iter()
        .enumerate()
        {
            if nav_item(ui, index + 1, label, self.page == page).clicked() {
                self.page = page;
            }
            ui.add_space(4.0);
        }
        ui.add_space(18.0);
        theme::divider(ui);

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            egui::Frame::new()
                .fill(theme::surface())
                .stroke(egui::Stroke::new(1.0, theme::BORDER))
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(egui::Margin::same(12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let games: usize = self.catalogs.iter().map(|repo| repo.games.len()).sum();
                    theme::section_label(ui, "Status", theme::LIME);
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(format!(
                            "{games} catalog games · {} local · {} sources",
                            recent.len(),
                            self.entries.len()
                        ))
                        .font(theme::mono_font(11.0))
                        .color(theme::TEXT_MUTED),
                    );
                    if self.task.is_some() {
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(
                                egui::RichText::new("Working…")
                                    .font(theme::body_font(12.0))
                                    .color(theme::CYAN),
                            );
                        });
                    } else {
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new(&self.status)
                                .font(theme::body_font(12.0))
                                .color(theme::TEXT_MUTED),
                        );
                    }
                });
            ui.add_space(10.0);
            ui.label(
                egui::RichText::new("RPG MAKER · GB / GBC · GBA · NDS")
                    .font(theme::mono_font(9.0))
                    .color(theme::TEXT_FAINT),
            );
            ui.add_space(4.0);
        });
    }

    fn page_header(&self, ui: &mut egui::Ui, eyebrow: &str, title: &str, subtitle: &str) {
        theme::section_label(ui, eyebrow, theme::PINK);
        ui.add_space(4.0);
        ui.label(theme::display(title, 36.0));
        ui.add_space(2.0);
        let _ = self;
        ui.label(
            egui::RichText::new(subtitle)
                .font(theme::body_font(14.0))
                .color(theme::TEXT_MUTED),
        );
        ui.add_space(18.0);
    }

    fn draw_library_page(
        &mut self,
        ui: &mut egui::Ui,
        recent: &[String],
        launch: &mut Option<PathBuf>,
    ) {
        egui::Frame::new()
            .fill(theme::surface())
            .stroke(egui::Stroke::new(1.0, theme::BORDER))
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(egui::Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.query)
                            .hint_text("Search games…")
                            .desired_width((ui.available_width() * 0.28).clamp(150.0, 260.0)),
                    );
                    for platform in ["All platforms", "RPG Maker", "GB / GBC", "GBA", "NDS"] {
                        let tone = match platform {
                            "GB / GBC" => Tone::Warning,
                            "GBA" => Tone::Cyan,
                            "NDS" => Tone::Pink,
                            "RPG Maker" => Tone::Lime,
                            _ => Tone::Pink,
                        };
                        if theme::chip(ui, platform, tone, self.platform == platform).clicked() {
                            self.platform = platform.into();
                        }
                    }
                    ui.separator();
                    if theme::button_sized(ui, "+ Game file", ButtonKind::Primary, ButtonSize::Sm)
                        .clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("Games", &["ini", "gb", "gbc", "gba", "nds"])
                            .pick_file()
                    {
                        *launch = Some(path);
                    }
                    if theme::button_sized(ui, "+ Folder", ButtonKind::Secondary, ButtonSize::Sm)
                        .clicked()
                        && let Some(path) = rfd::FileDialog::new().pick_folder()
                    {
                        *launch = Some(path);
                    }
                    if theme::button_sized(ui, "Refresh", ButtonKind::Ghost, ButtonSize::Sm)
                        .clicked()
                    {
                        self.refresh();
                    }
                    if theme::button_sized(ui, "Common settings", ButtonKind::Ghost, ButtonSize::Sm)
                        .clicked()
                    {
                        self.open_common_editor();
                    }
                });
            });
        ui.add_space(14.0);

        let query = self.query.to_lowercase();
        let mut entries = Vec::new();
        for (ri, repo) in self.catalogs.iter().enumerate() {
            for (gi, game) in repo.games.iter().enumerate() {
                let platform = match game.platform {
                    provision::Platform::RpgMaker => "RPG Maker",
                    provision::Platform::Gbc => "GB / GBC",
                    provision::Platform::Gba => "GBA",
                    provision::Platform::Nds => "NDS",
                };
                if game.title.to_lowercase().contains(&query)
                    && (self.platform == "All platforms" || self.platform == platform)
                {
                    entries.push((
                        game.title.clone(),
                        platform.to_string(),
                        game.description.clone(),
                        Some((ri, gi)),
                        provision::installed(&self.root, repo, game),
                    ));
                }
            }
        }
        for path in recent {
            let p = PathBuf::from(path);
            if entries.iter().any(|entry| entry.4.as_ref() == Some(&p)) {
                continue;
            }
            let name = p
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let platform = match p
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or("")
                .to_ascii_lowercase()
                .as_str()
            {
                "gb" | "gbc" => "GB / GBC",
                "gba" => "GBA",
                "nds" => "NDS",
                _ => "RPG Maker",
            };
            if name.to_lowercase().contains(&query)
                && (self.platform == "All platforms" || self.platform == platform)
            {
                entries.push((name, platform.into(), path.clone(), None, Some(p)));
            }
        }
        entries.sort_by_key(|entry| entry.0.to_lowercase());
        ui.horizontal_wrapped(|ui| {
            theme::tag(ui, &format_count(entries.len()), Tone::Neutral);
            if !self.catalogs.is_empty() {
                theme::tag(
                    ui,
                    &format!("{} repositories", self.catalogs.len()),
                    Tone::Cyan,
                );
            }
            ui.label(
                egui::RichText::new("Click a card to configure · Use play to launch")
                    .font(theme::body_font(11.0))
                    .color(theme::TEXT_FAINT),
            );
        });
        ui.add_space(10.0);

        if entries.is_empty() {
            theme::empty_state(
                ui,
                "✦",
                "Your next adventure starts here",
                "Add a local game or connect a JSON repository to discover games.",
            );
            if theme::button(ui, "Connect a repository", ButtonKind::Secondary).clicked() {
                self.page = Page::Repositories;
            }
            return;
        }

        egui::ScrollArea::vertical()
            .id_salt("launcher-library-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(18.0, 18.0);
                    for (name, platform, description, index, path) in entries {
                        let (key, defaults, artwork, prefs_root) = if let Some((ri, gi)) = index {
                            let repo = &self.catalogs[ri];
                            let game = &repo.games[gi];
                            (
                                format!("{}/{}", repo.id, game.id),
                                game.config.clone(),
                                game.image.clone(),
                                self.root.clone(),
                            )
                        } else {
                            let path = path.as_ref().unwrap();
                            let (key, defaults) = provision::settings::key_for_path(path);
                            (key, defaults, None, provision::root(Some(path)))
                        };
                        let prefs = self
                            .preferences
                            .entry(key.clone())
                            .or_insert_with(|| {
                                provision::settings::load(&prefs_root, &key).unwrap_or_default()
                            })
                            .clone();
                        let effective = {
                            let mut effective =
                                provision::settings::merge(defaults.clone(), self.common.clone());
                            if let Some(overrides) = &prefs.settings {
                                effective =
                                    provision::settings::merge(effective, overrides.clone());
                            }
                            effective
                        };
                        let artwork = prefs
                            .cover_sha256
                            .as_ref()
                            .map(|hash| provision::Artwork {
                                url: prefs_root
                                    .join("cache/artwork")
                                    .join(hash)
                                    .to_string_lossy()
                                    .into_owned(),
                                sha256: hash.clone(),
                            })
                            .or(artwork);
                        let texture = artwork
                            .as_ref()
                            .and_then(|art| self.cover_texture(ui.ctx(), &prefs_root, art));
                        let (tone, accent) = match platform.as_str() {
                            "GB / GBC" => (Tone::Warning, theme::WARNING),
                            "GBA" => (Tone::Cyan, theme::CYAN),
                            "NDS" => (Tone::Pink, theme::PINK),
                            _ => (Tone::Lime, theme::LIME),
                        };
                        game_card(ui, |ui| {
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(ART_WIDTH, ART_HEIGHT),
                                egui::Sense::click(),
                            );
                            let hovered = response.hovered();
                            if let Some(texture) = texture {
                                ui.painter().image(
                                    texture,
                                    rect,
                                    egui::Rect::from_min_max(
                                        egui::Pos2::ZERO,
                                        egui::pos2(1.0, 1.0),
                                    ),
                                    egui::Color32::WHITE,
                                );
                                ui.painter().rect_stroke(
                                    rect,
                                    egui::CornerRadius::same(8),
                                    egui::Stroke::new(1.0, theme::BORDER),
                                    egui::StrokeKind::Inside,
                                );
                            } else {
                                cover_placeholder(ui, rect, &platform, tone, accent);
                                if let Some(art) = &artwork
                                    && let Some(Cover::Failed(error)) = self.covers.get(&art.sha256)
                                {
                                    response.clone().on_hover_text(error);
                                }
                            }
                            if hovered {
                                theme::glow(ui.painter(), rect, 8, accent, 0.55);
                                ui.painter().rect_stroke(
                                    rect,
                                    egui::CornerRadius::same(8),
                                    egui::Stroke::new(2.0, accent),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            ui.add_space(2.0);
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(&name)
                                        .font(theme::body_font(15.0))
                                        .color(theme::TEXT),
                                )
                                .truncate(),
                            )
                            .on_hover_text(&description);
                            ui.horizontal_wrapped(|ui| {
                                theme::tag(ui, &platform, tone);
                                theme::tag(
                                    ui,
                                    if path.is_some() {
                                        "installed"
                                    } else {
                                        "available"
                                    },
                                    if path.is_some() {
                                        Tone::Lime
                                    } else {
                                        Tone::Neutral
                                    },
                                );
                                if let Some((ri, gi)) = index {
                                    let game = &self.catalogs[ri].games[gi];
                                    if game.base.is_some() {
                                        theme::tag(ui, "IPS", Tone::Warning);
                                    }
                                }
                            });
                            ui.horizontal(|ui| {
                                if theme::button_sized(
                                    ui,
                                    "Settings",
                                    ButtonKind::Ghost,
                                    ButtonSize::Sm,
                                )
                                .clicked()
                                {
                                    self.editor = Some(crate::launcher_settings::Editor {
                                        key: key.clone(),
                                        common: false,
                                        game_path: path.clone(),
                                        title: name.clone(),
                                        root: prefs_root.clone(),
                                        prefs: prefs.clone(),
                                        patches: effective.patches.join(", "),
                                        settings: effective,
                                        error: None,
                                        capturing_key: None,
                                    });
                                }
                            });
                            let bottom = ui.max_rect().top() + CARD_HEIGHT - CARD_MARGIN * 2.0;
                            ui.add_space((bottom - 36.0 - ui.cursor().top()).max(0.0));
                            let label = if path.is_some() {
                                "▶  Play"
                            } else {
                                "↓  Download & play"
                            };
                            if theme::button_width(
                                ui,
                                label,
                                ButtonKind::Primary,
                                ButtonSize::Sm,
                                ART_WIDTH,
                            )
                            .clicked()
                            {
                                if let Some((ri, gi)) = index {
                                    let repo = self.catalogs[ri].clone();
                                    let game = repo.games[gi].clone();
                                    let root = self.root.clone();
                                    self.job("Downloading and verifying game…", move || {
                                        provision::install(&root, &repo, &game)
                                            .map(Completed::Play)
                                            .map_err(|e| e.to_string())
                                    });
                                } else {
                                    *launch = path;
                                }
                            }
                        });
                    }
                });
                ui.add_space(12.0);
                theme::shortcut_hint(
                    ui,
                    &[
                        ("Ctrl+1", "Library"),
                        ("Ctrl+2", "Repositories"),
                        ("Ctrl+3", "Settings"),
                        ("Ctrl+4", "Storage"),
                    ],
                );
            });
    }

    fn draw_repositories_page(&mut self, ui: &mut egui::Ui) {
        self.page_header(
            ui,
            "Software repositories",
            "Connect the catalogs",
            "Catalogs are stored permanently in config/config.json. Selected catalogs contribute games to the library; unselected ones stay available offline.",
        );
        egui::Frame::new()
            .fill(theme::surface())
            .stroke(egui::Stroke::new(1.0, theme::BORDER))
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(egui::Margin::same(14))
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.source)
                        .desired_width(f32::INFINITY)
                        .hint_text("https://example.org/games.json or /path/to/games.json"),
                );
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    if theme::button_sized(
                        ui,
                        "Choose JSON files…",
                        ButtonKind::Ghost,
                        ButtonSize::Sm,
                    )
                    .clicked()
                        && let Some(paths) = rfd::FileDialog::new()
                            .add_filter("Repository", &["json"])
                            .pick_files()
                    {
                        for path in paths {
                            let source = path.to_string_lossy().into_owned();
                            if let Err(error) =
                                provision::catalog::add(&self.root, &source, None, true)
                            {
                                self.error = Some(error.to_string());
                            }
                        }
                        self.entries = provision::catalog::load_entries(&self.root);
                        self.refresh();
                    }
                    if theme::button_sized(ui, "Add & refresh", ButtonKind::Primary, ButtonSize::Sm)
                        .clicked()
                    {
                        let source = self.source.trim().to_owned();
                        match provision::catalog::add(&self.root, &source, None, true) {
                            Ok(_) => {
                                self.source.clear();
                                self.entries = provision::catalog::load_entries(&self.root);
                                self.refresh();
                            }
                            Err(error) => self.error = Some(error.to_string()),
                        }
                    }
                    if theme::button_sized(ui, "Refresh all", ButtonKind::Secondary, ButtonSize::Sm)
                        .clicked()
                    {
                        self.refresh();
                    }
                });
            });
        ui.add_space(16.0);

        let mut changed = false;
        let mut remove = None;
        let mut refresh_one = None;
        egui::ScrollArea::vertical()
            .id_salt("launcher-repositories-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for entry in &mut self.entries {
                    egui::Frame::new()
                        .fill(theme::surface_inset())
                        .stroke(egui::Stroke::new(1.0, theme::BORDER))
                        .corner_radius(egui::CornerRadius::same(8))
                        .inner_margin(egui::Margin::same(14))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal_wrapped(|ui| {
                                let mut selected = entry.selected;
                                if ui
                                    .checkbox(&mut selected, "")
                                    .on_hover_text("Use this catalog in the library")
                                    .changed()
                                {
                                    entry.selected = selected;
                                    changed = true;
                                }
                                ui.label(theme::display(&entry.name, 18.0));
                                let cache_label = match (entry.game_count, entry.last_refreshed) {
                                    (Some(count), Some(_)) => format!("{count} games · cached"),
                                    (Some(count), None) => format!("{count} games"),
                                    _ => "not cached yet".into(),
                                };
                                theme::tag(
                                    ui,
                                    &cache_label,
                                    if entry.last_refreshed.is_some() {
                                        Tone::Lime
                                    } else {
                                        Tone::Warning
                                    },
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if theme::button_sized(
                                            ui,
                                            "Remove",
                                            ButtonKind::Ghost,
                                            ButtonSize::Sm,
                                        )
                                        .clicked()
                                        {
                                            remove = Some(entry.id.clone());
                                        }
                                        if theme::button_sized(
                                            ui,
                                            "Refresh",
                                            ButtonKind::Ghost,
                                            ButtonSize::Sm,
                                        )
                                        .clicked()
                                        {
                                            refresh_one = Some(entry.id.clone());
                                        }
                                    },
                                );
                            });
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(&entry.source)
                                    .font(theme::mono_font(11.0))
                                    .color(theme::TEXT_MUTED),
                            );
                            if let Some(error) = &entry.last_error {
                                ui.add_space(4.0);
                                ui.label(
                                    egui::RichText::new(error)
                                        .font(theme::body_font(11.0))
                                        .color(theme::PINK),
                                );
                            }
                        });
                    ui.add_space(8.0);
                }
                if self.entries.is_empty() {
                    theme::empty_state(
                        ui,
                        "◈",
                        "No catalogs yet",
                        "Add a local JSON file or HTTPS URL above.",
                    );
                }
            });
        if let Some(id) = refresh_one {
            let root = self.root.clone();
            self.job("Refreshing catalog…", move || {
                let (entries, warnings) = provision::catalog::refresh(&root, Some(&id));
                let (repos, _) = provision::catalog::selected(&root);
                Ok(Completed::Catalog(repos, warnings, entries))
            });
        }
        if let Some(id) = remove {
            if let Err(error) = provision::catalog::remove(&self.root, &id) {
                self.error = Some(error.to_string());
            }
            self.entries = provision::catalog::load_entries(&self.root);
            self.reload_catalogs();
        } else if changed {
            for entry in &self.entries {
                let _ = provision::catalog::select(&self.root, &entry.id, entry.selected);
            }
            self.reload_catalogs();
        }
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new(
                "Downloads must match their SHA-256 hashes. Removing a catalog keeps its offline cache.",
            )
            .font(theme::body_font(11.0))
            .color(theme::TEXT_FAINT),
        );
    }

    fn draw_settings_page(&mut self, ui: &mut egui::Ui) {
        self.page_header(
            ui,
            "Settings",
            "Common settings",
            "These settings apply to every game unless a per-game override says otherwise.",
        );
        theme::card(false, None).show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::section_label(ui, "Defaults", theme::CYAN);
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(format!(
                    "Renderer: {}   ·   JIT: {}   ·   RGSS: {}",
                    self.common.renderer.as_deref().unwrap_or("automatic"),
                    self.common.jit.as_deref().unwrap_or("automatic"),
                    self.common
                        .rgss
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "automatic".into()),
                ))
                .font(theme::mono_font(12.0))
                .color(theme::TEXT_MUTED),
            );
            ui.label(
                egui::RichText::new(format!(
                    "FPS: {}   ·   Fast-forward: {}",
                    self.common
                        .fps
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "game".into()),
                    if self.common.fast_forward {
                        "on"
                    } else {
                        "off"
                    },
                ))
                .font(theme::mono_font(12.0))
                .color(theme::TEXT_MUTED),
            );
            if !self.common.patches.is_empty() {
                ui.label(
                    egui::RichText::new(format!(
                        "Common patches: {}",
                        self.common.patches.join(", ")
                    ))
                    .font(theme::body_font(12.0))
                    .color(theme::TEXT_MUTED),
                );
            }
            ui.add_space(12.0);
            ui.horizontal_wrapped(|ui| {
                if theme::button(ui, "Edit common settings", ButtonKind::Primary).clicked() {
                    self.open_common_editor();
                }
                if theme::button(ui, "Open config folder", ButtonKind::Ghost).clicked() {
                    let _ = provision::folders::open(&self.root.join("config"));
                }
                if theme::button(ui, "Open backups folder", ButtonKind::Ghost).clicked() {
                    let _ = provision::folders::open(&self.root.join("backups"));
                }
            });
        });
        ui.add_space(14.0);
        theme::card(false, None).show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::section_label(ui, "Appearance", theme::CYAN);
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Theme");
                egui::ComboBox::from_id_salt("launcher-theme")
                    .selected_text(&self.theme_name)
                    .show_ui(ui, |ui| {
                        for definition in &self.themes {
                            ui.selectable_value(&mut self.theme_name, definition.name.clone(), &definition.name);
                        }
                    });
                if theme::button_sized(ui, "Save themes.json", ButtonKind::Secondary, ButtonSize::Sm).clicked() {
                    let _ = theme::save_themes(&self.root.join("config/themes.json"), &self.themes);
                }
            });
            ui.label("Built-in themes can be extended or edited in config/themes.json. Colors use #RRGGBB values.");
        });
        ui.add_space(14.0);
        theme::card(false, None).show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::section_label(ui, "Control API", theme::PINK);
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(
                    "The control API is started by the launcher/player and exposes discovery at /v1/discovery. Every option above is also writable through the API.",
                )
                .font(theme::body_font(13.0))
                .color(theme::TEXT_MUTED),
            );
        });
        ui.add_space(14.0);
        ui.label(
            egui::RichText::new(format!(
                "Configuration file: {}",
                provision::config::path(&self.root).display()
            ))
            .font(theme::mono_font(11.0))
            .color(theme::TEXT_FAINT),
        );
        ui.label(
            egui::RichText::new(format!("FANPLAY home: {}", self.root.display()))
                .font(theme::mono_font(11.0))
                .color(theme::TEXT_FAINT),
        );
    }

    fn draw_storage_page(&mut self, ui: &mut egui::Ui) {
        self.page_header(
            ui,
            "Storage & saves",
            "One folder for your adventures",
            "Everything FANPLAY writes lives under this portable home. Copy it between machines and your library, saves and snapshots come along.",
        );
        egui::Frame::new()
            .fill(theme::surface())
            .stroke(egui::Stroke::new(1.0, theme::BORDER))
            .corner_radius(egui::CornerRadius::same(8))
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(self.root.display().to_string())
                        .font(theme::mono_font(13.0))
                        .color(theme::CYAN),
                );
                if theme::button_sized(ui, "Copy path", ButtonKind::Ghost, ButtonSize::Sm).clicked()
                {
                    ui.ctx().copy_text(self.root.display().to_string());
                }
            });
        ui.add_space(14.0);
        let folders = [
            (
                "games",
                "Installed games, including RPG Maker project-local saves",
                Tone::Lime,
            ),
            ("roms", "Imported base ROMs", Tone::Cyan),
            (
                "saves",
                "Battery saves, save states and RPG Maker user data",
                Tone::Pink,
            ),
            (
                "backups",
                "Named save snapshots, safe to copy between machines",
                Tone::Lime,
            ),
            (
                "repositories",
                "Connected sources and offline catalogs",
                Tone::Cyan,
            ),
            (
                "config",
                "config.json, common settings, per-game overrides and the library",
                Tone::Pink,
            ),
        ];
        egui::Grid::new("storage-folders")
            .num_columns(2)
            .spacing([16.0, 12.0])
            .show(ui, |ui| {
                for (index, (dir, detail, tone)) in folders.iter().enumerate() {
                    theme::card(false, Some(tone.color())).show(ui, |ui| {
                        ui.set_width(260.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(theme::display(format!("{dir}/"), 18.0));
                            theme::tag(ui, if index % 2 == 0 { "data" } else { "saves" }, *tone);
                        });
                        ui.add_space(2.0);
                        ui.label(
                            egui::RichText::new(*detail)
                                .font(theme::body_font(12.0))
                                .color(theme::TEXT_MUTED),
                        );
                        ui.add_space(8.0);
                        if theme::button_sized(ui, "Open folder", ButtonKind::Ghost, ButtonSize::Sm)
                            .clicked()
                        {
                            let _ = provision::folders::open(&self.root.join(dir));
                        }
                    });
                    if index % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
        ui.add_space(16.0);
        ui.horizontal_wrapped(|ui| {
            if theme::button_sized(ui, "Import base ROM…", ButtonKind::Primary, ButtonSize::Md)
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Base ROM", &["gb", "gbc", "gba", "nds"])
                    .pick_file()
            {
                let root = self.root.clone();
                self.job("Importing and hashing base ROM…", move || {
                    provision::import_base(&root, &path)
                        .map(Completed::Imported)
                        .map_err(|e| e.to_string())
                });
            }
            ui.label(
                egui::RichText::new(
                    "Portable mode: create an FANPLAY folder beside the executable or use FANPLAY_HOME.",
                )
                .font(theme::body_font(11.0))
                .color(theme::TEXT_FAINT),
            );
        });
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(
                "IPS installation matches ROM contents by SHA-256. Original ROMs are never patched in place.",
            )
            .font(theme::body_font(11.0))
            .color(theme::TEXT_FAINT),
        );
    }
}

fn keyboard_navigation(context: &egui::Context, page: &mut Page) {
    if context.egui_wants_keyboard_input() {
        return;
    }
    let command = context.input(|input| input.modifiers.command || input.modifiers.ctrl);
    if !command {
        return;
    }
    for (key, target) in [
        (egui::Key::Num1, Page::Library),
        (egui::Key::Num2, Page::Repositories),
        (egui::Key::Num3, Page::Settings),
        (egui::Key::Num4, Page::Storage),
    ] {
        if context.input(|input| input.key_pressed(key)) {
            *page = target;
        }
    }
}

fn format_count(count: usize) -> String {
    match count {
        1 => "1 game".into(),
        _ => format!("{count} games"),
    }
}

fn launcher_error(ui: &mut egui::Ui, message: &str) {
    egui::Frame::new()
        .fill(theme::PINK.gamma_multiply(0.10))
        .stroke(egui::Stroke::new(1.0, theme::PINK.gamma_multiply(0.55)))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                theme::status_dot(ui, theme::PINK, 0.8);
                ui.label(
                    egui::RichText::new(message)
                        .font(theme::body_font(13.0))
                        .color(theme::TEXT),
                );
            });
        });
}

fn nav_item(ui: &mut egui::Ui, number: usize, label: &str, selected: bool) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 38.0), egui::Sense::click());
    let hovered = response.hovered();
    if selected || hovered {
        let fill = if selected {
            theme::PINK.gamma_multiply(0.16)
        } else {
            theme::SURFACE_RAISED
        };
        if selected {
            theme::glow(ui.painter(), rect, 8, theme::PINK, 0.35);
        }
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(8), fill);
    }
    if selected {
        ui.painter().rect_filled(
            egui::Rect::from_min_size(
                rect.left_top() + egui::vec2(0.0, 7.0),
                egui::vec2(3.0, 24.0),
            ),
            egui::CornerRadius::same(2),
            theme::PINK,
        );
    }
    ui.painter().text(
        egui::pos2(rect.left() + 16.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        format!("{number:02}"),
        theme::mono_font(11.0),
        if selected {
            theme::PINK
        } else {
            theme::TEXT_FAINT
        },
    );
    ui.painter().text(
        egui::pos2(rect.left() + 44.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        theme::body_font(15.0),
        if selected {
            theme::TEXT
        } else {
            theme::TEXT_MUTED
        },
    );
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, label));
    response
}

fn cover_placeholder(
    ui: &egui::Ui,
    rect: egui::Rect,
    platform: &str,
    _tone: Tone,
    accent: egui::Color32,
) {
    let painter = ui.painter_at(rect);
    let base = match platform {
        "GBA" => egui::Color32::from_rgb(79, 57, 43),
        "NDS" => egui::Color32::from_rgb(38, 14, 44),
        _ => egui::Color32::from_rgb(12, 40, 30),
    };
    painter.rect_filled(rect, egui::CornerRadius::same(8), base);
    let center = rect.center();
    for index in (1..=3).rev() {
        painter.circle_stroke(
            center,
            28.0 + index as f32 * 22.0,
            egui::Stroke::new(1.0, accent.gamma_multiply(0.18)),
        );
    }
    painter.text(
        rect.left_top() + egui::vec2(10.0, 10.0),
        egui::Align2::LEFT_TOP,
        platform.to_uppercase(),
        theme::mono_font(9.0),
        accent,
    );
    theme::centered_text(
        ui,
        rect,
        platform,
        theme::display_font(28.0),
        accent.gamma_multiply(0.85),
    );
}

/// A wrapping row's horizontal layout must never leak into the card body.
/// Reserve the complete tile before laying out its contents vertically.
fn game_card(ui: &mut egui::Ui, contents: impl FnOnce(&mut egui::Ui)) {
    ui.allocate_ui_with_layout(
        egui::vec2(CARD_WIDTH, CARD_HEIGHT),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            egui::Frame::new()
                .fill(theme::surface_inset())
                .stroke(egui::Stroke::new(1.0, theme::BORDER))
                .corner_radius(egui::CornerRadius::same(12))
                .inner_margin(egui::Margin::same(CARD_MARGIN as i8))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(8.0, 4.0);
                    ui.set_width(ART_WIDTH);
                    ui.set_min_height(CARD_HEIGHT - CARD_MARGIN * 2.0);
                    contents(ui);
                });
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    fn launcher() -> Launcher {
        Launcher {
            covers: Default::default(),
            preferences: Default::default(),
            editor: None,
            root: PathBuf::from("/tmp/fanplay-ui-test-not-created"),
            entries: vec![],
            catalogs: vec![],
            common: Settings::default(),
            source: String::new(),
            query: String::new(),
            platform: "All platforms".into(),
            page: Page::Library,
            task: None,
            status: String::new(),
            error: None,
            theme_name: "Adventure Journal".into(),
            themes: theme::builtin_themes(),
        }
    }
    #[test]
    fn library_cards_stay_bounded_and_wrap_without_overlap() {
        for width in [700.0, 1100.0, 1897.0] {
            let context = egui::Context::default();
            let mut launcher = launcher();
            let recent: Vec<_> = (0..12).map(|i| format!("/games/Game {i:02}.gba")).collect();
            for _ in 0..2 {
                let output = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 1908.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        launcher.draw(ui, &recent, None);
                    },
                );
                let mut cards = Vec::new();
                fn collect(shape: &egui::epaint::Shape, cards: &mut Vec<egui::Rect>) {
                    match shape {
                        egui::epaint::Shape::Rect(r)
                            if r.fill == theme::SURFACE_RAISED
                                && (r.rect.width() - CARD_WIDTH - 2.0).abs() < 1.0 =>
                        {
                            cards.push(r.rect)
                        }
                        egui::epaint::Shape::Vec(shapes) => {
                            for shape in shapes {
                                collect(shape, cards);
                            }
                        }
                        _ => {}
                    }
                }
                for shape in &output.shapes {
                    collect(&shape.shape, &mut cards);
                }
                output.drop_without_applying_deltas();
                assert!(cards.len() >= 4);
                for (i, card) in cards.iter().enumerate() {
                    assert!(
                        card.height() <= CARD_HEIGHT + 2.0,
                        "stretched card at {width}: {card:?}"
                    );
                    assert!(
                        (card.width() - CARD_WIDTH - 2.0).abs() < 1.0,
                        "wrong card width: {card:?}"
                    );
                    assert!(card.right() <= width, "horizontal overflow: {card:?}");
                    for other in &cards[..i] {
                        assert!(!card.intersects(*other), "overlapping cards");
                    }
                }
                assert!((cards[0].top() - cards[1].top()).abs() < 2.0);
                assert!(
                    cards.last().unwrap().top() > cards[0].top(),
                    "cards failed to wrap"
                );
            }
        }
    }
    #[test]
    fn launcher_play_button_opens_recent_games() {
        let context = egui::Context::default();
        let mut launcher = launcher();
        let recent = vec!["/games/Alpha".to_string()];
        let mut launch = None;
        let mut frame = |events: Vec<egui::Event>| {
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100.0, 740.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    launch = launcher.draw(ui, &recent, None);
                },
            );
            output.textures_delta.clear();
            output
        };
        frame(vec![]);
        let output = frame(vec![]);
        let mut position = None;
        fn find(shape: &egui::epaint::Shape, position: &mut Option<egui::Pos2>) {
            match shape {
                egui::epaint::Shape::Text(text) if text.galley.text().contains("Play") => {
                    *position = Some(text.pos + text.galley.size() * 0.5);
                }
                egui::epaint::Shape::Vec(shapes) => {
                    for shape in shapes {
                        find(shape, position);
                    }
                }
                _ => {}
            }
        }
        for shape in &output.shapes {
            find(&shape.shape, &mut position);
        }
        let position = position.expect("play button label");
        frame(vec![
            egui::Event::PointerMoved(position),
            egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        frame(vec![egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert_eq!(
            launch.as_deref(),
            Some(std::path::Path::new("/games/Alpha"))
        );
    }

    #[test]
    fn pages_render_without_a_display() {
        for page in [
            Page::Library,
            Page::Repositories,
            Page::Settings,
            Page::Storage,
        ] {
            let context = egui::Context::default();
            let mut launcher = launcher();
            launcher.page = page;
            context
                .run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1100.0, 740.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        assert!(launcher
                            .draw(ui, &["/games/Homebrew.gba".into()], None)
                            .is_none());
                    },
                )
                .drop_without_applying_deltas();
        }
    }
}

fn decode_cover(path: &std::path::Path) -> Result<egui::ColorImage, String> {
    let mut reader = image::ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|e| e.to_string())?
        .thumbnail(350, 320)
        .to_rgba8();
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    ))
}
impl Launcher {
    fn cover_texture(
        &mut self,
        ctx: &egui::Context,
        root: &std::path::Path,
        art: &provision::Artwork,
    ) -> Option<egui::TextureId> {
        if !self.covers.contains_key(&art.sha256) {
            if self
                .covers
                .values()
                .filter(|c| matches!(c, Cover::Loading(_)))
                .count()
                >= 4
            {
                return None;
            }
            let (tx, rx) = mpsc::channel();
            let root = root.to_owned();
            let art = art.clone();
            let ctx = ctx.clone();
            self.covers.insert(art.sha256.clone(), Cover::Loading(rx));
            std::thread::spawn(move || {
                let result = provision::cache_artwork(&root, &art)
                    .map_err(|e| e.to_string())
                    .and_then(|path| decode_cover(&path));
                let _ = tx.send(result);
                ctx.request_repaint();
            });
        }
        let cover = self.covers.get_mut(&art.sha256)?;
        if let Cover::Loading(rx) = cover {
            match rx.try_recv() {
                Ok(Ok(image)) => {
                    *cover = Cover::Ready(ctx.load_texture(
                        &art.sha256,
                        image,
                        egui::TextureOptions::LINEAR,
                    ))
                }
                Ok(Err(error)) => *cover = Cover::Failed(format!("Cover unavailable: {error}")),
                Err(mpsc::TryRecvError::Disconnected) => {
                    *cover = Cover::Failed("Cover worker stopped".into())
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100))
                }
            }
        }
        match cover {
            Cover::Ready(texture) => Some(texture.id()),
            _ => None,
        }
    }
    fn draw_editor(&mut self, ctx: &egui::Context) {
        use crate::launcher_settings::Action;
        let Some(mut editor) = self.editor.take() else {
            return;
        };
        let mut open = true;
        if let Some(action) = editor.draw(ctx, &mut open) {
            match action {
                Action::Cover if !editor.common && self.task.is_none() => {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Cover images", &["png", "jpg", "jpeg", "gif", "bmp"])
                        .pick_file()
                    {
                        let key = editor.key.clone();
                        let root = editor.root.clone();
                        self.job("Caching cover image…", move || {
                            decode_cover(&path)?;
                            let hash = provision::import_artwork(&root, &path)
                                .map_err(|e| e.to_string())?;
                            Ok(Completed::CoverAssigned(key, root, hash))
                        });
                    }
                }
                Action::Cover => {
                    if !editor.common {
                        editor.error = Some("Wait for the current operation to finish".into())
                    }
                }
                action => {
                    if editor.common {
                        match action {
                            Action::Save => self.common = editor.settings.clone(),
                            Action::Reset => {
                                editor.settings = Settings::default();
                                editor.patches.clear();
                                self.common = Settings::default();
                            }
                            _ => {}
                        }
                        editor.settings.patches = editor
                            .patches
                            .split(',')
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned)
                            .collect();
                        match provision::settings::save_common(&editor.root, &editor.settings) {
                            Ok(()) => {
                                self.common = editor.settings.clone();
                                self.status = "Common settings saved".into();
                                open = false;
                            }
                            Err(error) => editor.error = Some(error.to_string()),
                        }
                    } else {
                        match action {
                            Action::Save => editor.prefs.settings = Some(editor.settings.clone()),
                            Action::Reset => editor.prefs.settings = None,
                            Action::ClearCover => editor.prefs.cover_sha256 = None,
                            _ => {}
                        }
                        match provision::settings::save(&editor.root, &editor.key, &editor.prefs) {
                            Ok(()) => {
                                self.preferences
                                    .insert(editor.key.clone(), editor.prefs.clone());
                                self.status = "Game preferences saved".into();
                                open = false;
                            }
                            Err(error) => editor.error = Some(error.to_string()),
                        }
                    }
                }
            }
        }
        if open {
            self.editor = Some(editor);
        }
    }
}
