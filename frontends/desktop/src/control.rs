// SPDX-License-Identifier: GPL-2.0-or-later
//! Desktop host for the unified launcher/player control API.
//!
//! One [`fanplay_control::ServerHandle`] owns the loopback port for the lifetime of
//! the dashboard. Launcher operations are handled here directly from the
//! persistent configuration. Game operations are forwarded through the same
//! `api_bridge` queue the workspace already uses, so a client can control the
//! active game without opening a second server.
//!
//! Actions that must touch the UI/session (`launch`, `stop`, `restart`,
//! `pause`, live frame pacing) are sent to the eframe thread and answered from
//! the next UI frame. The HTTP handler waits with a bounded timeout; a stopped
//! UI therefore fails cleanly instead of hanging a client forever.

use fanplay_control::{
    Catalog, ControlHandler, ControlRequest, ControlResponse, Endpoint, Parameter, ParameterType,
    Scope, ServerHandle,
};
use fanplay_provision as provision;
use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{
    mpsc::{self, Receiver, SyncSender},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Default)]
pub struct RuntimeState {
    pub running: bool,
    pub paused: bool,
    pub title: String,
    pub source: String,
    pub game_key: String,
    pub save_directory: String,
    pub fast_forward: bool,
    pub fps: Option<u32>,
}

pub struct ControlServer {
    handle: ServerHandle,
    state: Arc<Mutex<RuntimeState>>,
    repaint: Arc<Mutex<Option<egui::Context>>>,
}

impl ControlServer {
    pub fn start(
        root: PathBuf,
        port: u16,
        actions: mpsc::Sender<HostAction>,
    ) -> Result<Self, String> {
        let state = Arc::new(Mutex::new(RuntimeState::default()));
        let repaint = Arc::new(Mutex::new(None));
        let handler = Arc::new(DesktopControl {
            root,
            state: Arc::clone(&state),
            actions,
            repaint: Arc::clone(&repaint),
        });
        let handle = fanplay_control::start_with_catalog(port, Catalog::standard(), handler)?;
        Ok(Self {
            handle,
            state,
            repaint,
        })
    }

    pub fn attach_context(&self, context: &egui::Context) {
        if let Ok(mut repaint) = self.repaint.lock()
            && repaint.is_none()
        {
            *repaint = Some(context.clone());
        }
    }

    pub fn port(&self) -> u16 {
        self.handle.port()
    }

    pub fn discovery_url(&self) -> String {
        self.handle.discovery_url()
    }

    pub fn update(&self, state: RuntimeState) {
        if let Ok(mut current) = self.state.lock() {
            *current = state;
        }
    }

    pub fn snapshot(&self) -> RuntimeState {
        self.state
            .lock()
            .map(|state| state.clone())
            .unwrap_or_default()
    }

    pub fn stop(&self) {
        self.handle.stop();
    }
}

/// Work that must run on the eframe/UI thread.
pub enum HostAction {
    Launch {
        path: String,
        reply: SyncSender<Result<Value, String>>,
    },
    Stop {
        reply: SyncSender<Result<Value, String>>,
    },
    Restart {
        reply: SyncSender<Result<Value, String>>,
    },
    Pause {
        paused: bool,
        reply: SyncSender<Result<Value, String>>,
    },
    Live {
        fast_forward: Option<bool>,
        fps: Option<u32>,
        paused: Option<bool>,
        reply: SyncSender<Result<Value, String>>,
    },
}

pub fn action_channel() -> (mpsc::Sender<HostAction>, Receiver<HostAction>) {
    mpsc::channel()
}

struct DesktopControl {
    root: PathBuf,
    state: Arc<Mutex<RuntimeState>>,
    actions: mpsc::Sender<HostAction>,
    repaint: Arc<Mutex<Option<egui::Context>>>,
}

impl DesktopControl {
    fn snapshot(&self) -> RuntimeState {
        self.state
            .lock()
            .map(|state| state.clone())
            .unwrap_or_default()
    }

    fn send(
        &self,
        make: impl FnOnce(SyncSender<Result<Value, String>>) -> HostAction,
    ) -> ControlResponse {
        let (reply, receiver) = mpsc::sync_channel(1);
        let action = make(reply);
        if let Ok(repaint) = self.repaint.lock()
            && let Some(context) = repaint.as_ref()
        {
            context.request_repaint();
        }
        if self.actions.send(action).is_err() {
            return ControlResponse::unavailable("the launcher UI is not available");
        }
        match receiver.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(value)) => ControlResponse::ok(value),
            Ok(Err(error)) => ControlResponse::error(500, error),
            Err(_) => ControlResponse::error(504, "the launcher did not answer in time"),
        }
    }

    fn proxy(&self, request: &ControlRequest) -> ControlResponse {
        if !self.snapshot().running {
            return ControlResponse::unavailable("no game is running");
        }
        let invocation = &request.invocation;
        let mut target = invocation.target.clone();
        let (path, existing) = target.split_once('?').unwrap_or((&target, ""));
        let path = path.to_owned();
        let mut query = existing.to_owned();
        for (name, value) in &invocation.parameters {
            let encoded = url_encode(&value_to_argument(value));
            if !query
                .split('&')
                .any(|pair| pair.split_once('=').is_some_and(|(key, _)| key == name))
            {
                if !query.is_empty() {
                    query.push('&');
                }
                query.push_str(&url_encode(name));
                query.push('=');
                query.push_str(&encoded);
            }
        }
        target = if query.is_empty() {
            path
        } else {
            format!("{path}?{query}")
        };
        let (operation, arguments) = match fanplay_rgss::api_bridge::route_http(&invocation.method, &target)
        {
            Ok(route) => route,
            Err((status, message)) => {
                return ControlResponse::error(
                    if status == 404 { 409 } else { status },
                    if status == 404 {
                        format!("{message} (the game or its plugin is not ready)")
                    } else {
                        message
                    },
                );
            }
        };
        let id = match fanplay_rgss::api_bridge::send_from_ui(&operation, arguments) {
            Ok(id) => id,
            Err(error) => return ControlResponse::unavailable(error),
        };
        let started = Instant::now();
        loop {
            if let Some(reply) = fanplay_rgss::api_bridge::take_ui_response(id) {
                return match reply {
                    Ok(body) => match serde_json::from_str::<Value>(&body) {
                        Ok(value) => ControlResponse::ok(value),
                        Err(_) => ControlResponse::ok(json!({ "result": body })),
                    },
                    Err(error) => ControlResponse::error(503, error),
                };
            }
            if started.elapsed() > Duration::from_secs(15) {
                return ControlResponse::error(
                    504,
                    "the game did not answer in time; it may be paused or busy",
                );
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn resolve_game(&self, request: &ControlRequest) -> Result<GameLocation, ControlResponse> {
        let state = self.snapshot();
        let body = &request.invocation.body;
        let value = |name: &str| -> Option<String> {
            request
                .invocation
                .string(name)
                .map(str::to_owned)
                .or_else(|| body.get(name).and_then(Value::as_str).map(str::to_owned))
        };
        let game = value("game").or_else(|| {
            request
                .invocation
                .parameters
                .get("key")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
        let explicit_path = value("path");
        let source = if let Some(path) = explicit_path {
            path
        } else if let Some(game) = game {
            if let Some(path) = game.strip_prefix("local:") {
                path.to_owned()
            } else if Path::new(&game).exists() {
                game
            } else if game == state.game_key && !state.source.is_empty() {
                state.source.clone()
            } else {
                // repository/game key
                match self.installed_from_key(&game) {
                    Some(path) => path.to_string_lossy().into_owned(),
                    None => {
                        return Err(ControlResponse::not_found(format!(
                            "unknown game `{game}`; pass a path or an installed catalog game"
                        )));
                    }
                }
            }
        } else if !state.source.is_empty() {
            state.source.clone()
        } else {
            return Err(ControlResponse::unavailable(
                "no active game; pass a game path or start a game first",
            ));
        };
        let mut location = GameLocation::new(&self.root, Path::new(&source))
            .map_err(ControlResponse::bad_request)?;
        if location.key == state.game_key && !state.save_directory.is_empty() {
            location.save_directory = PathBuf::from(&state.save_directory);
        }
        Ok(location)
    }

    fn installed_from_key(&self, key: &str) -> Option<PathBuf> {
        let (repository_id, game_id) = key.split_once('/')?;
        let repository = provision::catalog::cached(&self.root, repository_id)?;
        let game = repository.games.iter().find(|game| game.id == game_id)?;
        provision::installed(&self.root, &repository, game)
    }
}

struct GameLocation {
    key: String,
    path: PathBuf,
    game_directory: PathBuf,
    save_directory: PathBuf,
    patches_directory: PathBuf,
    backups_directory: PathBuf,
    title: String,
    cartridge: bool,
}

impl GameLocation {
    fn new(root: &Path, path: &Path) -> Result<Self, String> {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
        let cartridge = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                ["gb", "gbc", "gba", "nds"]
                    .iter()
                    .any(|wanted| extension.eq_ignore_ascii_case(wanted))
            });
        let game_directory = provision::folders::game_directory(&path);
        let title = provision::folders::title(&path);
        let (key, _) = provision::settings::key_for_path(&path);
        let save_directory = if cartridge {
            let id = fanplay_frontend::ContentId::of_file(&path)
                .map_err(|error| format!("cannot identify cartridge: {error}"))?;
            root.join("saves")
                .join(
                    path.extension()
                        .and_then(|extension| extension.to_str())
                        .unwrap_or("rom")
                        .to_ascii_lowercase(),
                )
                .join(id.hex())
        } else {
            provision::folders::save_directory(root, &title)
        };
        Ok(Self {
            backups_directory: provision::backup::backup_root(root, &key),
            key,
            path,
            patches_directory: provision::folders::patches_directory(&game_directory),
            game_directory,
            save_directory,
            title,
            cartridge,
        })
    }

    fn folders_json(&self) -> Value {
        json!({
            "key": self.key,
            "title": self.title,
            "path": self.path,
            "game": self.game_directory,
            "save": self.save_directory,
            "patches": self.patches_directory,
            "backups": self.backups_directory,
            "cartridge": self.cartridge,
        })
    }
}

impl ControlHandler for DesktopControl {
    fn handle(&self, request: ControlRequest) -> ControlResponse {
        let invocation = &request.invocation;
        match invocation.endpoint.as_str() {
            "server.status" | "launcher.status" => return ControlResponse::ok(self.status()),
            "extensions.routes" => {
                return match serde_json::from_str::<Value>(
                    &fanplay_rgss::api_bridge::extension_routes_json(),
                ) {
                    Ok(value) => ControlResponse::ok(value),
                    Err(error) => ControlResponse::error(500, error.to_string()),
                };
            }
            "settings.common.get" => {
                return match provision::settings::common(&self.root) {
                    Ok(settings) => ControlResponse::ok(json_value(settings)),
                    Err(error) => ControlResponse::error(500, error.to_string()),
                };
            }
            "settings.common.update" => {
                return match serde_json::from_value::<provision::settings::Settings>(
                    invocation.body.clone(),
                ) {
                    Ok(settings) => match provision::settings::save_common(&self.root, &settings) {
                        Ok(()) => ControlResponse::ok(json_value(settings)),
                        Err(error) => ControlResponse::bad_request(error.to_string()),
                    },
                    Err(error) => ControlResponse::bad_request(error.to_string()),
                };
            }
            "settings.game.get" | "settings.game.update" => {
                return self.game_settings(invocation);
            }
            "settings.effective" => {
                let Some(game) = invocation.string("game") else {
                    return ControlResponse::bad_request("missing `game`");
                };
                let path = game.strip_prefix("local:").unwrap_or(game);
                return match provision::settings::for_game(Path::new(path)) {
                    Ok(settings) => ControlResponse::ok(json_value(settings)),
                    Err(error) => ControlResponse::bad_request(error.to_string()),
                };
            }
            "launcher.library" => return self.library(invocation),
            "launcher.library.add" => return self.library_add(invocation),
            "launcher.launch" => return self.launch(invocation),
            "launcher.stop" | "game.stop" => {
                return self.send(|reply| HostAction::Stop { reply });
            }
            "launcher.restart" | "game.reset" => {
                return self.send(|reply| HostAction::Restart { reply });
            }
            "launcher.pause" => {
                let Some(paused) = invocation.field("paused").and_then(Value::as_bool) else {
                    return ControlResponse::bad_request("missing boolean `paused`");
                };
                return self.send(|reply| HostAction::Pause { paused, reply });
            }
            "launcher.live" => return ControlResponse::ok(self.live()),
            "launcher.live.update" => {
                let fast_forward = invocation.field("fast_forward").and_then(Value::as_bool);
                let fps = invocation
                    .field("fps")
                    .and_then(Value::as_u64)
                    .map(|value| value as u32);
                let paused = invocation.field("paused").and_then(Value::as_bool);
                return self.send(|reply| HostAction::Live {
                    fast_forward,
                    fps,
                    paused,
                    reply,
                });
            }
            "launcher.folders" => {
                return match self.folder_location(invocation) {
                    Ok(location) => ControlResponse::ok(location.folders_json()),
                    Err(error) => error,
                };
            }
            "launcher.open_folder" => return self.open_folder(invocation),
            "catalogs.list" => return self.catalogs(),
            "catalogs.add" => return self.catalog_add(invocation),
            "catalogs.remove" => return self.catalog_remove(invocation),
            "catalogs.select" => return self.catalog_select(invocation),
            "catalogs.refresh" => return self.catalog_refresh(invocation),
            "catalogs.games" => return self.catalog_games(invocation),
            "saves.info" => return self.saves_info(invocation),
            "saves.backup" => return self.saves_backup(invocation),
            "saves.restore" => return self.saves_restore(invocation),
            "saves.delete" => return self.saves_delete(invocation),
            "game.settings" => return self.game_settings(invocation),
            _ => {}
        }
        self.proxy(&request)
    }

    fn dynamic_endpoints(&self) -> Vec<Endpoint> {
        let Ok(value) = serde_json::from_str::<Value>(&fanplay_rgss::api_bridge::extension_routes_json())
        else {
            return Vec::new();
        };
        value
            .get("routes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|route| {
                let method = route.get("method")?.as_str()?;
                let path = route.get("path")?.as_str()?;
                let operation = route.get("operation")?.as_str()?;
                let mut endpoint = Endpoint::new(
                    operation,
                    method,
                    path,
                    format!("Plugin operation {operation}"),
                    "Route supplied by a loaded Rhai plugin. Parameters are forwarded to the game bridge; validation and the response schema belong to the plugin.",
                    Scope::Extensions,
                );
                for argument in route
                    .get("arguments")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                {
                    let in_path = path.contains(&format!("{{{argument}}}"));
                    endpoint = endpoint.parameter(if in_path {
                        Parameter::path(argument, ParameterType::String, "Plugin route argument")
                    } else {
                        Parameter::query(argument, ParameterType::String, false, "Plugin route argument")
                    });
                }
                Some(endpoint)
            })
            .collect()
    }

    fn status(&self) -> Value {
        let state = self.snapshot();
        json!({
            "game_running": state.running,
            "paused": state.paused,
            "game": {
                "title": state.title,
                "source": state.source,
                "key": state.game_key,
                "save_directory": state.save_directory,
                "fast_forward": state.fast_forward,
                "fps": state.fps,
            },
            "root": self.root,
            "config": provision::config::path(&self.root),
            "launcher_available": true,
            "game_available": state.running,
            "library": provision::config::library(&self.root).len(),
            "catalogs": provision::catalog::load_entries(&self.root).len(),
        })
    }

    fn mode(&self) -> &'static str {
        "launcher"
    }
}

impl DesktopControl {
    fn live(&self) -> Value {
        let state = self.snapshot();
        json!({
            "running": state.running,
            "paused": state.paused,
            "fast_forward": state.fast_forward,
            "fps": state.fps,
            "game": state.title,
            "source": state.source,
        })
    }

    fn game_settings(&self, invocation: &fanplay_control::Invocation) -> ControlResponse {
        let path = invocation.string("path").map(PathBuf::from).or_else(|| {
            invocation
                .string("key")
                .and_then(|key| key.strip_prefix("local:").map(PathBuf::from))
        });
        let key = invocation
            .string("key")
            .map(str::to_owned)
            .or_else(|| {
                path.as_ref()
                    .map(|path| provision::settings::key_for_path(path).0)
            })
            .or_else(|| {
                invocation.string("game").map(|game| {
                    game.strip_prefix("local:")
                        .map(|path| provision::settings::key_for_path(Path::new(path)).0)
                        .unwrap_or_else(|| game.to_owned())
                })
            })
            .or_else(|| {
                let state = self.snapshot();
                (!state.game_key.is_empty()).then_some(state.game_key)
            });
        let Some(key) = key else {
            return ControlResponse::bad_request("missing game key or path");
        };
        let defaults = if let Some(path) = &path {
            provision::settings::key_for_path(path).1
        } else if let Some((repository_id, game_id)) = key.split_once('/') {
            provision::catalog::cached(&self.root, repository_id)
                .and_then(|repository| {
                    repository
                        .games
                        .into_iter()
                        .find(|game| game.id == game_id)
                        .map(|game| game.config)
                })
                .unwrap_or_default()
        } else {
            Default::default()
        };
        if invocation.endpoint == "settings.game.update" {
            let mut preferences = match provision::settings::load(&self.root, &key) {
                Ok(preferences) => preferences,
                Err(error) => return ControlResponse::error(500, error.to_string()),
            };
            match invocation.body.get("settings") {
                Some(Value::Null) => preferences.settings = None,
                Some(value) => {
                    match serde_json::from_value::<provision::settings::Settings>(value.clone()) {
                        Ok(settings) => preferences.settings = Some(settings),
                        Err(error) => return ControlResponse::bad_request(error.to_string()),
                    }
                }
                None if invocation.body.is_object()
                    && !invocation
                        .body
                        .as_object()
                        .is_some_and(|body| body.is_empty())
                    && invocation.body.get("cover_sha256").is_none() =>
                {
                    // Accept the documented `{ "settings": … }` wrapper or a
                    // direct settings object for convenience.
                    match serde_json::from_value::<provision::settings::Settings>(
                        invocation.body.clone(),
                    ) {
                        Ok(settings) => preferences.settings = Some(settings),
                        Err(error) => return ControlResponse::bad_request(error.to_string()),
                    }
                }
                None => {}
            }
            if let Some(cover) = invocation.body.get("cover_sha256") {
                match cover {
                    Value::Null => preferences.cover_sha256 = None,
                    Value::String(hash) => preferences.cover_sha256 = Some(hash.clone()),
                    _ => {
                        return ControlResponse::bad_request(
                            "cover_sha256 must be a string or null",
                        )
                    }
                }
            }
            let effective = match provision::settings::effective(&self.root, &key, defaults) {
                Ok(settings) => json_value(settings),
                Err(error) => return ControlResponse::bad_request(error.to_string()),
            };
            return match provision::settings::save(&self.root, &key, &preferences) {
                Ok(()) => ControlResponse::ok(json!({
                    "key": key,
                    "override": preferences.settings,
                    "settings": effective,
                })),
                Err(error) => ControlResponse::bad_request(error.to_string()),
            };
        }
        match provision::settings::effective(&self.root, &key, defaults) {
            Ok(settings) => match provision::settings::preferences(&self.root, &key) {
                Ok(preferences) => ControlResponse::ok(json!({
                    "key": key,
                    "settings": settings,
                    "override": preferences.settings,
                    "cover_sha256": preferences.cover_sha256,
                })),
                Err(error) => ControlResponse::error(500, error.to_string()),
            },
            Err(error) => ControlResponse::bad_request(error.to_string()),
        }
    }

    fn library(&self, invocation: &fanplay_control::Invocation) -> ControlResponse {
        let query = invocation.string("query").unwrap_or("").to_lowercase();
        let platform = invocation.string("platform").unwrap_or("");
        let only_catalog = invocation.string("catalog");
        let mut games = Vec::new();
        if only_catalog.is_none() {
            for path in provision::config::library(&self.root) {
                let path_buf = PathBuf::from(&path);
                let title = if path_buf
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| {
                        ["gb", "gbc", "gba", "nds"]
                            .iter()
                            .any(|wanted| extension.eq_ignore_ascii_case(wanted))
                    })
                {
                    path_buf
                        .file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned())
                        .unwrap_or_else(|| provision::folders::title(&path_buf))
                } else {
                    provision::folders::title(&path_buf)
                };
                let local_platform = if path_buf
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("gba"))
                {
                    "gba"
                } else if path_buf.extension().is_some_and(|e| {
                    e.eq_ignore_ascii_case("gb") || e.eq_ignore_ascii_case("gbc")
                }) {
                    "gbc"
                } else if path_buf
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("nds"))
                {
                    "nds"
                } else {
                    "rpg-maker"
                };
                if !title.to_lowercase().contains(&query)
                    || (!platform.is_empty() && !local_platform.eq_ignore_ascii_case(platform))
                {
                    continue;
                }
                let key = provision::settings::key_for_path(&path_buf).0;
                games.push(json!({
                    "title": title,
                    "platform": local_platform,
                    "path": path,
                    "key": key,
                    "installed": true,
                    "recent": true,
                }));
            }
        }
        for entry in provision::catalog::load_entries(&self.root)
            .into_iter()
            .filter(|entry| entry.selected)
            .filter(|entry| only_catalog.is_none_or(|id| id == entry.id))
        {
            let Some(repository) = provision::catalog::cached(&self.root, &entry.id) else {
                continue;
            };
            for game in &repository.games {
                let game_platform = match game.platform {
                    provision::Platform::RpgMaker => "rpg-maker",
                    provision::Platform::Gbc => "gbc",
                    provision::Platform::Gba => "gba",
                    provision::Platform::Nds => "nds",
                };
                if !game.title.to_lowercase().contains(&query)
                    || (!platform.is_empty() && !game_platform.eq_ignore_ascii_case(platform))
                {
                    continue;
                }
                let installed = provision::installed(&self.root, &repository, game);
                games.push(json!({
                    "title": game.title,
                    "platform": game_platform,
                    "description": game.description,
                    "version": game.version,
                    "catalog": repository.id,
                    "game": game.id,
                    "key": format!("{}/{}", repository.id, game.id),
                    "installed": installed.is_some(),
                    "path": installed,
                    "base_required": game.base.is_some(),
                }));
            }
        }
        if !platform.is_empty() {
            games.retain(|game| game["platform"].as_str() == Some(platform));
        }
        ControlResponse::ok(json!({ "games": games, "count": games.len() }))
    }

    fn library_add(&self, invocation: &fanplay_control::Invocation) -> ControlResponse {
        let Some(path) = invocation.string("path") else {
            return ControlResponse::bad_request("missing `path`");
        };
        let mut recent = provision::config::library(&self.root);
        if let Err(error) = crate::library::remember(&mut recent, Path::new(path)) {
            return ControlResponse::error(500, format!("cannot save library: {error}"));
        }
        let launch = invocation
            .field("launch")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if launch {
            return self.launch(&fanplay_control::Invocation {
                endpoint: "launcher.launch".into(),
                method: "POST".into(),
                target: "/v1/launcher/launch".into(),
                parameters: Default::default(),
                body: json!({"path": path}),
            });
        }
        ControlResponse::ok(json!({ "added": path, "library": recent.len() }))
    }

    fn launch(&self, invocation: &fanplay_control::Invocation) -> ControlResponse {
        if let Some(path) = invocation.string("path") {
            return self.launch_path(path.to_owned());
        }
        let Some(catalog) = invocation.string("catalog") else {
            return ControlResponse::bad_request("pass `path`, or `catalog` and `game`");
        };
        let Some(game_id) = invocation.string("game") else {
            return ControlResponse::bad_request("pass `catalog` and `game` together");
        };
        let Some(repository) = provision::catalog::cached(&self.root, catalog) else {
            return ControlResponse::not_found(format!("catalog `{catalog}` is not cached"));
        };
        let Some(game) = repository
            .games
            .iter()
            .find(|candidate| candidate.id == game_id)
        else {
            return ControlResponse::not_found(format!("game `{game_id}` is not in `{catalog}`"));
        };
        if let Some(path) = provision::installed(&self.root, &repository, game) {
            return self.launch_path(path.to_string_lossy().into_owned());
        }
        let root = self.root.clone();
        let repository = repository.clone();
        let game = game.clone();
        let game_id = game.id.clone();
        let actions = self.actions.clone();
        std::thread::spawn(
            move || match provision::install(&root, &repository, &game) {
                Ok(path) => {
                    let (reply, _receiver) = mpsc::sync_channel(1);
                    let _ = actions.send(HostAction::Launch {
                        path: path.to_string_lossy().into_owned(),
                        reply,
                    });
                }
                Err(error) => eprintln!("[ControlAPI] install failed: {error}"),
            },
        );
        ControlResponse::ok(json!({
            "queued": true,
            "installing": true,
            "catalog": catalog,
            "game": game_id,
        }))
    }

    fn launch_path(&self, path: String) -> ControlResponse {
        if let Err(error) = crate::library::discover(&path) {
            return ControlResponse::bad_request(error);
        }
        let requested = path.clone();
        let response = self.send(|reply| HostAction::Launch { path, reply });
        match response.status {
            200 => {
                let mut body = response.body;
                if let Some(object) = body.as_object_mut() {
                    object.insert("queued".into(), json!(true));
                    object.insert("path".into(), json!(requested));
                }
                ControlResponse::ok(body)
            }
            _ => response,
        }
    }

    fn folder_location(
        &self,
        invocation: &fanplay_control::Invocation,
    ) -> Result<GameLocation, ControlResponse> {
        let source = invocation
            .string("path")
            .map(str::to_owned)
            .or_else(|| {
                invocation.string("key").map(|key| {
                    key.strip_prefix("local:")
                        .map(str::to_owned)
                        .unwrap_or_else(|| key.to_owned())
                })
            })
            .or_else(|| {
                let state = self.snapshot();
                (!state.source.is_empty()).then_some(state.source)
            })
            .ok_or_else(|| ControlResponse::bad_request("missing game key or path"))?;
        let path_buf = if Path::new(&source).exists() {
            PathBuf::from(source)
        } else if let Some(path) = self.installed_from_key(&source) {
            path
        } else {
            return Err(ControlResponse::not_found(format!(
                "unknown game `{source}`"
            )));
        };
        GameLocation::new(&self.root, &path_buf).map_err(ControlResponse::bad_request)
    }

    fn open_folder(&self, invocation: &fanplay_control::Invocation) -> ControlResponse {
        let location = match self.folder_location(invocation) {
            Ok(location) => location,
            Err(response) => return response,
        };
        let kind = invocation.string("kind").unwrap_or("game");
        let path = match kind {
            "game" => location.game_directory.clone(),
            "save" => location.save_directory.clone(),
            "patches" => location.patches_directory.clone(),
            "backups" => location.backups_directory.clone(),
            other => {
                return ControlResponse::bad_request(format!(
                    "unknown folder kind `{other}`; use game, save, patches or backups"
                ));
            }
        };
        if kind != "game"
            && let Err(error) = provision::folders::ensure(&path)
        {
            return ControlResponse::error(500, format!("cannot create folder: {error}"));
        }
        let opened = provision::folders::open(&path).is_ok();
        ControlResponse::ok(json!({
            "kind": kind,
            "path": path,
            "opened": opened,
            "game_key": location.key,
        }))
    }

    fn catalogs(&self) -> ControlResponse {
        let entries = provision::catalog::load_entries(&self.root);
        let values = entries
            .into_iter()
            .map(|entry| {
                json!({
                    "id": entry.id,
                    "name": entry.name,
                    "source": entry.source,
                    "selected": entry.selected,
                    "last_refreshed": entry.last_refreshed,
                    "game_count": entry.game_count,
                    "last_error": entry.last_error,
                    "cached": provision::catalog::cached(&self.root, &entry.id).is_some(),
                })
            })
            .collect::<Vec<_>>();
        ControlResponse::ok(json!({ "catalogs": values, "count": values.len() }))
    }

    fn catalog_add(&self, invocation: &fanplay_control::Invocation) -> ControlResponse {
        let Some(source) = invocation.string("source") else {
            return ControlResponse::bad_request("missing `source`");
        };
        let name = invocation.string("name");
        let selected = invocation
            .field("select")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let entry = match provision::catalog::add(&self.root, source, name, selected) {
            Ok(entry) => entry,
            Err(error) => return ControlResponse::bad_request(error.to_string()),
        };
        if invocation
            .field("refresh")
            .and_then(Value::as_bool)
            .unwrap_or(true)
        {
            let (entries, warnings) = provision::catalog::refresh(&self.root, Some(&entry.id));
            let entry = entries
                .into_iter()
                .find(|candidate| candidate.id == entry.id)
                .unwrap_or(entry);
            return ControlResponse::ok(json!({ "catalog": entry, "warnings": warnings }));
        }
        ControlResponse::ok(json!({ "catalog": entry }))
    }

    fn catalog_remove(&self, invocation: &fanplay_control::Invocation) -> ControlResponse {
        let Some(id) = invocation.string("id") else {
            return ControlResponse::bad_request("missing catalog id");
        };
        match provision::catalog::remove(&self.root, id) {
            Ok(removed) => ControlResponse::ok(json!({ "removed": removed, "id": id })),
            Err(error) => ControlResponse::error(500, error.to_string()),
        }
    }

    fn catalog_select(&self, invocation: &fanplay_control::Invocation) -> ControlResponse {
        let Some(id) = invocation.string("id") else {
            return ControlResponse::bad_request("missing catalog id");
        };
        let Some(selected) = invocation.field("selected").and_then(Value::as_bool) else {
            return ControlResponse::bad_request("missing boolean `selected`");
        };
        match provision::catalog::select(&self.root, id, selected) {
            Ok(()) => ControlResponse::ok(json!({ "id": id, "selected": selected })),
            Err(error) => ControlResponse::bad_request(error.to_string()),
        }
    }

    fn catalog_refresh(&self, invocation: &fanplay_control::Invocation) -> ControlResponse {
        let (entries, warnings) = provision::catalog::refresh(&self.root, invocation.string("id"));
        ControlResponse::ok(json!({ "catalogs": entries, "warnings": warnings }))
    }

    fn catalog_games(&self, invocation: &fanplay_control::Invocation) -> ControlResponse {
        let Some(id) = invocation.string("id") else {
            return ControlResponse::bad_request("missing catalog id");
        };
        match provision::catalog::cached(&self.root, id) {
            Some(repository) => ControlResponse::ok(json_value(repository)),
            None => ControlResponse::not_found(format!("catalog `{id}` is not cached yet")),
        }
    }

    fn saves_info(&self, invocation: &fanplay_control::Invocation) -> ControlResponse {
        let location = match self.resolve_game(&ControlRequest {
            invocation: invocation.clone(),
            remote: String::new(),
        }) {
            Ok(location) => location,
            Err(response) => return response,
        };
        let backups = provision::backup::list(&self.root, &location.key).unwrap_or_default();
        let running = self.snapshot().running;
        ControlResponse::ok(json!({
            "game": location.folders_json(),
            "backups": backups,
            "can_restore": !running,
            "running": running,
        }))
    }

    fn saves_backup(&self, invocation: &fanplay_control::Invocation) -> ControlResponse {
        let location = match self.resolve_game(&ControlRequest {
            invocation: invocation.clone(),
            remote: String::new(),
        }) {
            Ok(location) => location,
            Err(response) => return response,
        };
        let source = invocation
            .string("source")
            .map(PathBuf::from)
            .unwrap_or(location.save_directory.clone());
        match provision::backup::create(
            &self.root,
            &location.key,
            &source,
            invocation.string("name"),
        ) {
            Ok(info) => ControlResponse::ok(json_value(info)),
            Err(error) => ControlResponse::error(500, format!("backup failed: {error}")),
        }
    }

    fn saves_restore(&self, invocation: &fanplay_control::Invocation) -> ControlResponse {
        if self.snapshot().running {
            return ControlResponse::unavailable("stop the game before restoring saves");
        }
        let location = match self.resolve_game(&ControlRequest {
            invocation: invocation.clone(),
            remote: String::new(),
        }) {
            Ok(location) => location,
            Err(response) => return response,
        };
        let Some(name) = invocation.string("backup") else {
            return ControlResponse::bad_request("missing `backup` name");
        };
        let destination = invocation
            .string("destination")
            .map(PathBuf::from)
            .unwrap_or(location.save_directory.clone());
        match provision::backup::restore(&self.root, &location.key, name, &destination) {
            Ok(info) => ControlResponse::ok(json!({
                "restored": info,
                "destination": destination,
            })),
            Err(error) => ControlResponse::error(500, format!("restore failed: {error}")),
        }
    }

    fn saves_delete(&self, invocation: &fanplay_control::Invocation) -> ControlResponse {
        let location = match self.resolve_game(&ControlRequest {
            invocation: invocation.clone(),
            remote: String::new(),
        }) {
            Ok(location) => location,
            Err(response) => return response,
        };
        let Some(name) = invocation.string("name") else {
            return ControlResponse::bad_request("missing backup name");
        };
        match provision::backup::delete(&self.root, &location.key, name) {
            Ok(()) => ControlResponse::ok(json!({ "deleted": name })),
            Err(error) => ControlResponse::error(500, error.to_string()),
        }
    }
}

fn json_value<T: Serialize>(value: T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

fn value_to_argument(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        other => other.to_string(),
    }
}

fn url_encode(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push(HEX[(byte >> 4) as usize] as char);
            encoded.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;

    fn temp(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "fanplay-control-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn get(port: u16, target: &str) -> String {
        request(port, "GET", target, "")
    }

    fn request(port: u16, method: &str, target: &str, body: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(
            stream,
            "{method} {target} HTTP/1.1\r\nHost: test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response.split_once("\r\n\r\n").unwrap().1.to_owned()
    }

    #[test]
    fn encoding_is_stable_for_query_values() {
        assert_eq!(url_encode("a b/c"), "a%20b%2Fc");
        assert_eq!(value_to_argument(&json!(true)), "true");
        assert_eq!(value_to_argument(&json!(7)), "7");
    }

    #[test]
    fn one_server_serves_launcher_status_and_discovery() {
        let root = temp("server");
        let (actions, _receiver) = action_channel();
        let server = ControlServer::start(root.clone(), 0, actions).unwrap();
        let status = get(server.port(), "/v1/status");
        assert!(status.contains("\"mode\":\"launcher\""), "{status}");
        assert!(status.contains("\"launcher_available\":true"), "{status}");
        let discovery = get(server.port(), "/v1/discovery");
        assert!(discovery.contains("launcher.launch"), "{discovery}");
        assert!(discovery.contains("\"type\":\"boolean\""), "{discovery}");
        assert!(discovery.contains("/v1/saves/backup"), "{discovery}");
        let updated = request(
            server.port(),
            "PUT",
            "/v1/settings/common",
            r#"{"renderer":"software","fast_forward":true}"#,
        );
        assert!(updated.contains("\"renderer\":\"software\""), "{updated}");
        let common = get(server.port(), "/v1/settings/common");
        assert!(common.contains("\"fast_forward\":true"), "{common}");
        assert!(std::fs::read_to_string(root.join("config/config.json"))
            .unwrap()
            .contains("\"renderer\": \"software\""));
        server.stop();
        std::fs::remove_dir_all(root).unwrap();
    }
}
