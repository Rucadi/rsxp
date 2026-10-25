//! Desktop library and repository management. Blocking work stays off the UI thread.
use rsxp_provision::{self as provision, Repository};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
};
const ACCENT: egui::Color32 = egui::Color32::from_rgb(53, 220, 195);
#[derive(PartialEq)]
enum Page {
    Library,
    Repositories,
    Storage,
}
enum Completed {
    Catalog(Vec<Repository>, Vec<String>),
    Play(PathBuf),
    Imported(String),
    CoverAssigned(String, PathBuf, String),
}
enum Cover { Loading(Receiver<Result<egui::ColorImage, String>>), Ready(egui::TextureHandle), Failed(String) }
pub struct Launcher {
    covers: std::collections::HashMap<String, Cover>,
    preferences: std::collections::HashMap<String, provision::settings::Preferences>,
    editor: Option<crate::launcher_settings::Editor>,
    root: PathBuf,
    sources: Vec<String>,
    catalogs: Vec<Repository>,
    source: String,
    query: String,
    platform: String,
    page: Page,
    task: Option<Receiver<Result<Completed, String>>>,
    status: String,
    error: Option<String>,
}
impl Default for Launcher {
    fn default() -> Self {
        let root = provision::root(None);
        let sources = std::fs::read(root.join("repositories/sources.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let catalogs = std::fs::read(root.join("repositories/catalog.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Vec<Repository>>(&b).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.validate().is_ok())
            .collect();
        Self {
            covers: Default::default(), preferences: Default::default(), editor: None,
            root,
            sources,
            catalogs,
            source: String::new(),
            query: String::new(),
            platform: "All platforms".into(),
            page: Page::Library,
            task: None,
            status: "Your games. One home.".into(),
            error: None,
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
        let sources = self.sources.clone();
        self.job("Refreshing repositories…", move || {
            let mut repos = Vec::new();
            let mut errors = Vec::new();
            for source in sources {
                match provision::load_repository(&source) {
                    Ok(repo) if !repos.iter().any(|r: &Repository| r.id == repo.id) => {
                        repos.push(repo)
                    }
                    Ok(repo) => errors.push(format!("Duplicate repository id: {}", repo.id)),
                    Err(e) => errors.push(format!("{source}: {e}")),
                }
            }
            Ok(Completed::Catalog(repos, errors))
        });
    }
    fn persist(&mut self) {
        let result = (|| -> provision::Result<()> {
            provision::initialize(&self.root)?;
            for (name, bytes) in [
                ("sources", serde_json::to_vec_pretty(&self.sources)?),
                ("catalog", serde_json::to_vec_pretty(&self.catalogs)?),
            ] {
                let path = self.root.join(format!("repositories/{name}.json"));
                let temp = path.with_extension("tmp");
                std::fs::write(&temp, bytes)?;
                std::fs::rename(temp, path)?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            self.error = Some(e.to_string());
        }
    }
    pub fn draw(
        &mut self,
        ui: &mut egui::Ui,
        recent: &[String],
        runtime_error: Option<&str>,
    ) -> Option<PathBuf> {
        let mut launch = None;
        if let Some(task) = &self.task {
            match task.try_recv() {
                Ok(result) => {
                    self.task = None;
                    match result {
                        Ok(Completed::Catalog(repos, errors)) => {
                            // Keep the last working catalog for unavailable sources.
                            for repo in repos {
                                self.catalogs.retain(|r| r.id != repo.id);
                                self.catalogs.push(repo);
                            }
                            self.error = (!errors.is_empty()).then(|| errors.join("\n"));
                            self.status = "Repositories refreshed".into();
                            self.persist();
                        }
                        Ok(Completed::Play(path)) => {
                            self.status = "Ready to play".into();
                            launch = Some(path);
                        }
                        Ok(Completed::CoverAssigned(key, root, hash)) => {
                            let mut prefs = self.preferences.get(&key).cloned().unwrap_or_default();
                            prefs.cover_sha256 = Some(hash);
                            match provision::settings::save(&root, &key, &prefs) {
                                Ok(()) => { self.preferences.insert(key.clone(), prefs.clone()); if let Some(editor) = &mut self.editor { if editor.key == key { editor.prefs = prefs; } } self.status = "Cover image saved".into(); },
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
        ui.ctx().set_visuals(egui::Visuals::dark());
        egui::Panel::left("launcher-nav")
            .exact_size(190.0)
            .resizable(false)
            .show(ui, |ui| {
                ui.add_space(25.0);
                ui.label(
                    egui::RichText::new("RSXP")
                        .size(34.0)
                        .strong()
                        .color(ACCENT),
                );
                ui.weak("THE FAN GAME LIBRARY");
                ui.add_space(36.0);
                for (page, title) in [
                    (Page::Library, "Library"),
                    (Page::Repositories, "Repositories"),
                    (Page::Storage, "Storage & saves"),
                ] {
                    ui.selectable_value(
                        &mut self.page,
                        page,
                        egui::RichText::new(title).size(17.0),
                    );
                    ui.add_space(12.0);
                }
                ui.separator();
                ui.add_space(12.0);
                ui.label("RPG Maker · GBA · NDS");
                ui.weak(format!("{} repositories", self.catalogs.len()));
                ui.add_space(20.0);
                if self.task.is_some() {
                    ui.spinner();
                }
                ui.small(&self.status);
            });
        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(16.0);
            if let Some(error) = self.error.as_deref().or(runtime_error) {
                egui::Frame::new().fill(egui::Color32::from_rgb(64, 31, 37)).inner_margin(12.0).show(ui, |ui| { ui.colored_label(egui::Color32::LIGHT_RED, error); });
                ui.add_space(12.0);
            }
            match self.page {
                Page::Library => {
                    ui.horizontal(|ui| {
                        ui.heading("Library");
                        ui.add(egui::TextEdit::singleline(&mut self.query).hint_text("Search games…").desired_width(220.0));
                        egui::ComboBox::from_id_salt("platform-filter").selected_text(&self.platform).show_ui(ui, |ui| {
                            for p in ["All platforms", "RPG Maker", "GBA", "NDS"] { ui.selectable_value(&mut self.platform, p.into(), p); }
                        });
                    });
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.add_enabled(self.task.is_none(), egui::Button::new("+ Add game file")).clicked() {
                            launch = rfd::FileDialog::new().add_filter("Games", &["ini", "gba", "nds"]).pick_file();
                        }
                        if ui.add_enabled(self.task.is_none(), egui::Button::new("+ Add game folder")).clicked() { launch = rfd::FileDialog::new().pick_folder(); }
                        if ui.add_enabled(self.task.is_none(), egui::Button::new("Refresh repositories")).clicked() { self.refresh(); }
                    });
                    ui.add_space(20.0);
                    let query = self.query.to_lowercase();
                    let mut entries = Vec::new();
                    for (ri, repo) in self.catalogs.iter().enumerate() {
                        for (gi, game) in repo.games.iter().enumerate() {
                            let platform = match game.platform { provision::Platform::RpgMaker => "RPG Maker", provision::Platform::Gba => "GBA", provision::Platform::Nds => "NDS" };
                            if game.title.to_lowercase().contains(&query) && (self.platform == "All platforms" || self.platform == platform) {
                                entries.push((game.title.clone(), platform.to_string(), game.description.clone(), Some((ri, gi)), provision::installed(&self.root, repo, game)));
                            }
                        }
                    }
                    for path in recent {
                        let p = PathBuf::from(path);
                        if entries.iter().any(|e| e.4.as_ref() == Some(&p)) { continue; }
                        let name = p.file_stem().unwrap_or_default().to_string_lossy().into_owned();
                        let platform = match p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase().as_str() { "gba" => "GBA", "nds" => "NDS", _ => "RPG Maker" };
                        if name.to_lowercase().contains(&query) && (self.platform == "All platforms" || self.platform == platform) {
                            entries.push((name, platform.into(), path.clone(), None, Some(p)));
                        }
                    }
                    entries.sort_by_key(|e| e.0.to_lowercase());
                    ui.weak(format!("{} games", entries.len()));
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        if entries.is_empty() {
                            ui.add_space(60.0); ui.heading("Your next adventure starts here");
                            ui.label("Add a local game or connect a JSON repository to discover games.");
                            if ui.button("Connect a repository").clicked() { self.page = Page::Repositories; }
                        }
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = egui::vec2(18.0, 18.0);
                            for (name, platform, description, index, path) in entries {
                                let (key, defaults, artwork, prefs_root) = if let Some((ri, gi)) = index {
                                    let repo = &self.catalogs[ri]; let game = &repo.games[gi];
                                    (format!("{}/{}",repo.id,game.id), game.config.clone(),game.image.clone(),self.root.clone())
                                } else {
                                    let path = path.as_ref().unwrap(); let (key, defaults) = provision::settings::key_for_path(path);
                                    (key, defaults, None, provision::root(Some(path)))
                                };
                                let prefs = self.preferences.entry(key.clone()).or_insert_with(|| provision::settings::load(&prefs_root, &key).unwrap_or_default()).clone();
                                let artwork = prefs.cover_sha256.as_ref().map(|hash| provision::Artwork { url: prefs_root.join("cache/artwork").join(hash).to_string_lossy().into_owned(), sha256: hash.clone() }).or(artwork);
                                let texture = artwork.as_ref().and_then(|art| self.cover_texture(ui.ctx(), &prefs_root, art));
                                game_card(ui, |ui| {
                                    let (rect, _) = ui.allocate_exact_size(egui::vec2(175.0, 160.0), egui::Sense::hover());
                                    let color = match platform.as_str() { "GBA" => egui::Color32::from_rgb(45, 75, 90), "NDS" => egui::Color32::from_rgb(75, 55, 100), _ => egui::Color32::from_rgb(35, 83, 69) };
                                    if let Some(texture) = texture {
                                        ui.painter().image(texture, rect, egui::Rect::from_min_max(egui::Pos2::ZERO,egui::pos2(1.0,1.0)),egui::Color32::WHITE);
                                    } else {
                                        ui.painter().rect_filled(rect, 8, color);
                                        ui.painter().circle_stroke(rect.center(), 48.0, egui::Stroke::new(2.0, ACCENT));
                                        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, &platform, egui::FontId::proportional(20.0), egui::Color32::WHITE);
                                        if let Some(art) = &artwork { if let Some(Cover::Failed(error)) = self.covers.get(&art.sha256) { ui.interact(rect, ui.id().with(&key), egui::Sense::hover()).on_hover_text(error); } }
                                    }
                                    ui.add_space(8.0);
                                    ui.add(egui::Label::new(egui::RichText::new(&name).strong().size(16.0)).truncate()).on_hover_text(&description);
                                    ui.horizontal(|ui| {
                                        ui.weak(if path.is_some() { "Installed" } else { "Available" });
                                        if ui.small_button("Settings").clicked() {
                                            let settings = prefs.settings.clone().unwrap_or(defaults.clone());
                                            self.editor = Some(crate::launcher_settings::Editor { key: key.clone(), title: name.clone(), root: prefs_root.clone(), prefs: prefs.clone(), patches: settings.patches.join(", "), settings, error: None });
                                        }
                                    });
                                    if let Some((ri, gi)) = index {
                                        let game = &self.catalogs[ri].games[gi];
                                        if let Some(base) = &game.base { ui.small("IPS · Base ROM required").on_hover_text(format!("{}\nSHA-256 {}", base.title, base.sha256)); }
                                    }
                                    // Align every play button to the bottom of its tile.
                                    let bottom = ui.max_rect().top() + 272.0;
                                    ui.add_space((bottom - 36.0 - ui.cursor().top()).max(0.0));
                                    if ui.add_enabled_ui(self.task.is_none(), |ui| ui.add_sized([175.0, 32.0], egui::Button::new(if path.is_some() { "▶ Play" } else { "↓ Download & play" }).fill(color))).inner.clicked() {
                                        if let Some((ri, gi)) = index {
                                            let repo = self.catalogs[ri].clone(); let game = repo.games[gi].clone(); let root = self.root.clone();
                                            self.job("Downloading and verifying game…", move || provision::install(&root, &repo, &game).map(Completed::Play).map_err(|e| e.to_string()));
                                        } else { launch = path; }
                                    }
                                });
                            }
                        });
                    });
                }
                Page::Repositories => {
                    ui.heading("Software repositories");
                    ui.label("Connect one or more JSON catalogs by HTTPS URL or local path."); ui.add_space(16.0);
                    ui.add(egui::TextEdit::singleline(&mut self.source).desired_width(f32::INFINITY).hint_text("https://example.org/games.json"));
                    ui.horizontal(|ui| {
                        if ui.add_enabled(self.task.is_none(), egui::Button::new("Choose JSON files…")).clicked() {
                            if let Some(paths) = rfd::FileDialog::new().add_filter("Repository", &["json"]).pick_files() {
                                for p in paths { let p = p.to_string_lossy().into_owned(); if !self.sources.contains(&p) { self.sources.push(p); } }
                                self.persist(); if self.task.is_none() { self.refresh(); }
                            }
                        }
                        if ui.add_enabled(self.task.is_none() && !self.source.trim().is_empty(), egui::Button::new("Connect")).clicked() {
                            let source = self.source.trim().to_owned();
                            if !self.sources.contains(&source) { self.sources.push(source); }
                            self.source.clear(); self.persist(); self.refresh();
                        }
                    });
                    ui.add_space(20.0);
                    let mut remove = None;
                    for (i, source) in self.sources.iter().enumerate() {
                        ui.horizontal_wrapped(|ui| { ui.label(source); if ui.add_enabled(self.task.is_none(), egui::Button::new("Remove")).clicked() { remove = Some(i); } });
                    }
                    if let Some(i) = remove { self.sources.remove(i); self.catalogs.clear(); self.persist(); self.refresh(); }
                    ui.add_space(20.0); ui.weak("Catalogs are cached for offline use. Downloads must match their SHA-256 hashes.");
                }
                Page::Storage => {
                    ui.heading("One folder for your adventures"); ui.label(self.root.display().to_string());
                    ui.add_space(16.0);
                    for (dir, detail) in [("games", "Installed games, including RPG Maker project-local saves"), ("roms", "Your imported base ROMs"), ("saves", "Battery saves, save states and RPG Maker user data"), ("repositories", "Connected sources and offline catalogs")] { ui.label(format!("{dir}/   {detail}")); }
                    ui.add_space(20.0);
                    ui.label("Portable mode: create an RSXP folder beside the executable or game. Use RSXP_HOME to choose another location.");
                    if ui.add_enabled(self.task.is_none(), egui::Button::new("Import base ROM…")).clicked() {
                        if let Some(path) = rfd::FileDialog::new().add_filter("Base ROM", &["gba", "nds"]).pick_file() {
                            let root = self.root.clone(); self.job("Importing and hashing base ROM…", move || provision::import_base(&root, &path).map(Completed::Imported).map_err(|e| e.to_string()));
                        }
                    }
                    ui.weak("IPS installation matches ROM contents by SHA-256. Original ROMs are never patched in place.");
                }
            }
        });
        self.draw_editor(ui.ctx());
        launch
    }
}

/// A wrapping row's horizontal layout must never leak into the card body.
/// Reserve the complete tile before laying out its contents vertically.
fn game_card(ui: &mut egui::Ui, contents: impl FnOnce(&mut egui::Ui)) {
    ui.allocate_ui_with_layout(
        egui::vec2(203.0, 300.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgb(25, 33, 40))
                .corner_radius(12)
                .inner_margin(14.0)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(8.0, 4.0);
                    ui.set_width(175.0);
                    ui.set_min_height(272.0);
                    contents(ui);
                });
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn library_cards_stay_bounded_and_wrap_without_overlap() {
        for width in [700.0, 1100.0, 1897.0] {
            let context = egui::Context::default();
            let mut launcher = Launcher {
                covers: Default::default(), preferences: Default::default(), editor: None,
                root: PathBuf::from("/tmp/rsxp-ui-test-not-created"),
                sources: vec![], catalogs: vec![], source: String::new(), query: String::new(),
                platform: "All platforms".into(), page: Page::Library, task: None,
                status: String::new(), error: None,
            };
            let recent: Vec<_> = (0..12).map(|i| format!("/games/Game {i:02}.gba")).collect();
            for _ in 0..2 {
                let output = context.run_ui(egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 1908.0))),
                    ..Default::default()
                }, |ui| { launcher.draw(ui, &recent, None); });
                let mut cards = Vec::new();
                fn collect(shape: &egui::epaint::Shape, cards: &mut Vec<egui::Rect>) {
                    match shape {
                        egui::epaint::Shape::Rect(r) if r.fill == egui::Color32::from_rgb(25, 33, 40) => cards.push(r.rect),
                        egui::epaint::Shape::Vec(shapes) => for shape in shapes { collect(shape, cards); },
                        _ => {}
                    }
                }
                for shape in &output.shapes { collect(&shape.shape, &mut cards); }
                output.drop_without_applying_deltas();
                assert!(cards.len() >= 4);
                for (i, card) in cards.iter().enumerate() {
                    assert!(card.height() <= 301.0, "stretched card at {width}: {card:?}");
                    assert!((card.width() - 203.0).abs() < 1.0, "wrong card width: {card:?}");
                    assert!(card.right() <= width, "horizontal overflow: {card:?}");
                    for other in &cards[..i] { assert!(!card.intersects(*other), "overlapping cards"); }
                }
                assert!((cards[0].top() - cards[1].top()).abs() < 1.0);
                assert!(cards.last().unwrap().top() > cards[0].top(), "cards failed to wrap");
            }
        }
    }
    #[test]
    fn pages_render_without_a_display() {
        for page in [Page::Library, Page::Repositories, Page::Storage] {
            let context = egui::Context::default();
            let mut launcher = Launcher {
                covers: Default::default(), preferences: Default::default(), editor: None,
                root: PathBuf::from("/tmp/rsxp-ui-test-not-created"),
                sources: vec![],
                catalogs: vec![],
                source: String::new(),
                query: String::new(),
                platform: "All platforms".into(),
                page,
                task: None,
                status: String::new(),
                error: None,
            };
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
    let mut reader = image::ImageReader::open(path).map_err(|e| e.to_string())?.with_guessed_format().map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096); limits.max_image_height = Some(4096); limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().map_err(|e| e.to_string())?.thumbnail(350, 320).to_rgba8();
    Ok(egui::ColorImage::from_rgba_unmultiplied([image.width() as usize,image.height() as usize],image.as_raw()))
}
impl Launcher {
    fn cover_texture(&mut self, ctx: &egui::Context, root: &std::path::Path, art: &provision::Artwork) -> Option<egui::TextureId> {
        if !self.covers.contains_key(&art.sha256) {
            if self.covers.values().filter(|c| matches!(c,Cover::Loading(_))).count() >= 4 { return None; }
            let (tx,rx) = mpsc::channel(); let root = root.to_owned(); let art = art.clone(); let ctx = ctx.clone();
            self.covers.insert(art.sha256.clone(),Cover::Loading(rx));
            std::thread::spawn(move || {
                let result = provision::cache_artwork(&root,&art).map_err(|e| e.to_string()).and_then(|path| decode_cover(&path));
                let _ = tx.send(result); ctx.request_repaint();
            });
        }
        let cover = self.covers.get_mut(&art.sha256)?;
        if let Cover::Loading(rx) = cover {
            match rx.try_recv() {
                Ok(Ok(image)) => *cover = Cover::Ready(ctx.load_texture(&art.sha256,image,egui::TextureOptions::LINEAR)),
                Ok(Err(error)) => *cover = Cover::Failed(format!("Cover unavailable: {error}")),
                Err(mpsc::TryRecvError::Disconnected) => *cover = Cover::Failed("Cover worker stopped".into()),
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
            }
        }
        match cover { Cover::Ready(texture) => Some(texture.id()), _ => None }
    }
    fn draw_editor(&mut self, ctx: &egui::Context) {
        use crate::launcher_settings::Action;
        let Some(mut editor) = self.editor.take() else { return; };
        let mut open = true;
        if let Some(action) = editor.draw(ctx,&mut open) {
            match action {
                Action::Cover if self.task.is_none() => {
                    if let Some(path) = rfd::FileDialog::new().add_filter("Cover images", &["png","jpg","jpeg","gif","bmp"]).pick_file() {
                        let key = editor.key.clone(); let root = editor.root.clone();
                        self.job("Caching cover image…", move || {
                            decode_cover(&path)?;
                            let hash = provision::import_artwork(&root,&path).map_err(|e| e.to_string())?;
                            Ok(Completed::CoverAssigned(key,root,hash))
                        });
                    }
                }
                Action::Cover => editor.error = Some("Wait for the current operation to finish".into()),
                action => {
                    match action {
                        Action::Save => editor.prefs.settings = Some(editor.settings.clone()),
                        Action::Reset => editor.prefs.settings = None,
                        Action::ClearCover => editor.prefs.cover_sha256 = None,
                        _ => {}
                    }
                    match provision::settings::save(&editor.root,&editor.key,&editor.prefs) {
                        Ok(()) => { self.preferences.insert(editor.key.clone(),editor.prefs.clone()); self.status = "Game preferences saved".into(); open = false; },
                        Err(error) => editor.error = Some(error.to_string()),
                    }
                }
            }
        }
        if open { self.editor = Some(editor); }
    }
}
