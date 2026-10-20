// SPDX-License-Identifier: GPL-2.0-or-later
//! Engine bridge shared by the cargo-mobile2 shell, the desktop preview binary,
//! and the integration tests.
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use rsxp::{
    engine::{self, Runtime, RuntimeOptions},
    graphics::{Bitmap, Color},
    host::{Host, InputEvent},
};
use ruby_rgss_core::mobile as mri;
use serde_json::{json, Value};
use std::{
    cell::RefCell,
    fs,
    io::{Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    sync::OnceLock,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

static LIBRARY: OnceLock<PathBuf> = OnceLock::new();
static RUBY_RUNTIME: OnceLock<PathBuf> = OnceLock::new();
const DEMO: &str = include_str!("../demo.rb");

struct ImportSession {
    name: String,
    temporary: PathBuf,
}

thread_local! {
    static GPU: RefCell<Option<rsxp::gpu::GpuRenderer>> = const { RefCell::new(None) };
    static IMPORT: RefCell<Option<ImportSession>> = const { RefCell::new(None) };
}

fn safe_file(root: &Path, name: &str) -> Option<PathBuf> {
    let normalized = name.replace('\\', "/");
    let relative = Path::new(&normalized);
    if relative
        .components()
        .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
    {
        return None;
    }
    let mut path = root.to_owned();
    for part in relative.components() {
        let Component::Normal(name) = part else {
            continue;
        };
        let direct = path.join(name);
        path = if direct.exists() {
            direct
        } else {
            fs::read_dir(&path)
                .ok()?
                .filter_map(Result::ok)
                .find(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .eq_ignore_ascii_case(&name.to_string_lossy())
                })?
                .path()
        };
    }
    let path = path.canonicalize().ok()?;
    (path.starts_with(root) && path.is_file()).then_some(path)
}

fn safe_relative(name: &str) -> Option<PathBuf> {
    let normalized = name.replace('\\', "/");
    let path = Path::new(&normalized);
    if normalized.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
    {
        return None;
    }
    Some(path.to_owned())
}

struct MobileHost {
    root: PathBuf,
    gpu: Option<rsxp::gpu::GpuRenderer>,
    audio: crate::audio::DesktopAudio,
    started: Instant,
}

impl Drop for MobileHost {
    fn drop(&mut self) {
        let gpu = self.gpu.take();
        let _ = GPU.try_with(|slot| *slot.borrow_mut() = gpu);
    }
}

/// Install a surface-compatible compositor before launching a game.
pub fn set_gpu(gpu: rsxp::gpu::GpuRenderer) {
    GPU.with(|slot| *slot.borrow_mut() = Some(gpu));
}

/// Native presentation, including the library screen between games.
pub fn with_native_frame<T>(
    present: impl FnOnce(&mut rsxp::gpu::GpuRenderer, Option<&Bitmap>) -> T,
) -> Option<T> {
    let mut present = Some(present);
    if let Some(result) =
        engine::with_runtime_mut(|runtime| runtime.with_native_frame(present.take().unwrap()))
    {
        return result;
    }
    GPU.with(|slot| {
        slot.borrow_mut()
            .as_mut()
            .map(|gpu| present.take().unwrap()(gpu, None))
    })
}

pub fn input(event: InputEvent) {
    engine::with_runtime_mut(|runtime| runtime.queue_input(event));
}

pub fn step() -> Result<bool, String> {
    mri::step()
}

pub fn stats() -> (u64, u32) {
    engine::with_runtime(|runtime| (runtime.frame_count(), runtime.rate())).unwrap_or((0, 60))
}

pub fn stop() {
    mri::stop();
    engine::take_runtime();
}

impl Host for MobileHost {
    fn gpu(&mut self) -> Option<&mut rsxp::gpu::GpuRenderer> {
        self.gpu.as_mut()
    }
    fn audio(&mut self) -> Option<&mut dyn rsxp::host::AudioBackend> {
        Some(&mut self.audio)
    }
    fn has_performance_ui(&self) -> bool {
        true
    }
    fn now(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    fn read_file(&self, path: &str) -> Option<Vec<u8>> {
        fs::read(safe_file(&self.root, path)?).ok()
    }
    fn image(&self, bytes: &[u8]) -> Result<Bitmap, String> {
        rsxp::portable_media::image(bytes)
    }
    fn text(
        &self,
        font: Option<&[u8]>,
        size: i32,
        bold: bool,
        italic: bool,
        text: &str,
        color: Color,
        render: bool,
    ) -> Result<(u32, u32, Option<Bitmap>), String> {
        rsxp::portable_media::text(font, size, bold, italic, text, color, render)
    }
}

fn prepare(id: &str) -> Result<Value, String> {
    if Path::new(id).components().count() != 1 || id == ".." || id.contains(['/', '\\']) {
        return Err("Invalid game identifier".into());
    }
    let library = LIBRARY.get().ok_or("Library is not configured")?;
    let root = library.join(id).canonicalize().map_err(|e| e.to_string())?;
    if !root.starts_with(library) {
        return Err("Game is outside the library".into());
    }
    prepare_root(id, root)
}

fn prepare_root(id: &str, root: PathBuf) -> Result<Value, String> {
    let ini = safe_file(&root, "Game.ini")
        .and_then(|p| fs::read_to_string(p).ok())
        .unwrap_or_default();
    let field = |name: &str| {
        ini.lines().find_map(|line| {
            let (key, value) = line.split_once('=')?;
            key.trim()
                .eq_ignore_ascii_case(name)
                .then(|| value.trim().replace('\\', "/"))
        })
    };
    let scripts = field("Scripts").unwrap_or_else(|| "Data/Scripts.rxdata".into());
    let mut archives = Vec::new();
    for name in ["Game.rgssad", "Game.rgss2a", "Game.rgss3a"] {
        if let Some(path) = safe_file(&root, name) {
            archives.push(fs::read(path).map_err(|e| e.to_string())?);
        }
    }
    let version = if scripts.ends_with("rvdata2") || safe_file(&root, "Game.rgss3a").is_some() {
        3
    } else if scripts.ends_with("rvdata") || safe_file(&root, "Game.rgss2a").is_some() {
        2
    } else {
        1
    };
    if id != ".demo" && safe_file(&root, &scripts).is_none() && archives.is_empty() {
        return Err("No RPG Maker scripts or archive found in this folder".into());
    }
    let (width, height, rate) = if version == 1 {
        (640, 480, 40)
    } else {
        (544, 416, 60)
    };
    // Return the previous host GPU before constructing its successor.
    mri::stop();
    engine::take_runtime();
    engine::set_runtime(Runtime::new(RuntimeOptions {
        width,
        height,
        version,
        rate,
        headless: false,
        frame_limit: None,
        fast_forward: true,
        archives,
        scripted: std::env::var("RSXP_INPUT").map(|value| engine::parse_scripted(&value)).transpose()?.unwrap_or_default(),
        host: Box::new(MobileHost {
            root: root.clone(),
            gpu: GPU.with(|slot| slot.borrow_mut().take()),
            audio: if cfg!(test) || std::env::var_os("RSXP_SILENT").is_some() {
                crate::audio::DesktopAudio::silent()
            } else {
                crate::audio::DesktopAudio::new()
            },
            started: Instant::now(),
        }),
    })?);
    rsxp::runtime_patch::reset();
    rsxp::runtime_patch::load_user_patches(&root)?;
    if let Ok(patches) = std::env::var("RSXP_PATCH") { rsxp::runtime_patch::enable_from(&patches); }
    rsxp::runtime_settings::reset();
    rsxp::runtime_settings::set_fast_forward(false); // The event loop paces Ruby fibers without sleeping.
    Ok(
        json!({"title":field("Title").unwrap_or_else(|| id.into()), "version":version,
        "width":width,"height":height,"rate":rate,"scripts":scripts,
        "script":if id == ".demo" { Some("demo.rb") } else { None }}),
    )
}

pub fn initialize(id: &str) -> Result<Value, String> {
    let config = prepare(id)?;
    let library = LIBRARY.get().ok_or("Library is not configured")?;
    let root = library.join(id).canonicalize().map_err(|e| e.to_string())?;
    start(config, root)
}

/// Launch an explicitly selected native directory in place (no import/copy).
pub fn initialize_path(path: &Path) -> Result<Value, String> {
    let root = path.canonicalize().map_err(|e| e.to_string())?;
    let title = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Game")
        .to_owned();
    let config = prepare_root(&title, root.clone())?;
    start(config, root)
}

fn start(config: Value, root: PathBuf) -> Result<Value, String> {
    let library = LIBRARY.get().ok_or("Library is not configured")?;
    mri::start(
        RUBY_RUNTIME.get().ok_or("Native Ruby is not configured")?,
        &root,
        config["scripts"].as_str().unwrap(),
        config["script"].as_str(),
        config["title"].as_str().unwrap(),
        &library.join(".saves"),
    )?;
    Ok(config)
}

fn import_begin(name: &str) -> Result<Value, String> {
    if name.is_empty() || name.starts_with('.') || name.contains(['/', '\\']) {
        return Err("Invalid game folder name".into());
    }
    let library = LIBRARY.get().ok_or("Library is not configured")?;
    if library.join(name).exists() {
        return Err("A game with this name is already imported".into());
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let temporary = library.join(format!(".import-{nonce}"));
    let _ = fs::remove_dir_all(&temporary);
    fs::create_dir_all(&temporary).map_err(|error| error.to_string())?;
    IMPORT.with(|slot| {
        *slot.borrow_mut() = Some(ImportSession {
            name: name.to_owned(),
            temporary: temporary.clone(),
        })
    });
    Ok(json!({"temporary": temporary.to_string_lossy()}))
}

fn import_chunk(path: &str, offset: u64, data: &str) -> Result<Value, String> {
    let relative = safe_relative(path).ok_or("Invalid file path")?;
    let bytes = B64.decode(data).map_err(|error| error.to_string())?;
    IMPORT.with(|slot| {
        let session = slot.borrow();
        let session = session.as_ref().ok_or("No import is in progress")?;
        let destination = session.temporary.join(&relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut file = fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&destination)
            .map_err(|error| error.to_string())?;
        if offset == 0 {
            file.set_len(0).map_err(|error| error.to_string())?;
        }
        file.seek(SeekFrom::Start(offset))
            .map_err(|error| error.to_string())?;
        file.write_all(&bytes).map_err(|error| error.to_string())?;
        Ok::<Value, String>(json!({"size": offset + bytes.len() as u64}))
    })
}

fn import_end() -> Result<Value, String> {
    let session = IMPORT
        .with(|slot| slot.borrow_mut().take())
        .ok_or("No import is in progress")?;
    let library = LIBRARY.get().ok_or("Library is not configured")?;
    let destination = library.join(&session.name);
    if destination.exists() {
        let _ = fs::remove_dir_all(&session.temporary);
        return Err("A game with this name is already imported".into());
    }
    fs::rename(&session.temporary, &destination).map_err(|error| {
        let _ = fs::remove_dir_all(&session.temporary);
        error.to_string()
    })?;
    Ok(json!({"id": session.name}))
}

fn import_cancel() -> Result<Value, String> {
    if let Some(session) = IMPORT.with(|slot| slot.borrow_mut().take()) {
        let _ = fs::remove_dir_all(session.temporary);
    }
    Ok(Value::Null)
}

/// Import a directory selected by the Linux desktop's native folder chooser.
pub fn import_directory(source: &std::path::Path) -> Result<Value, String> {
    let source = source.canonicalize().map_err(|error| error.to_string())?;
    if !source.is_dir() {
        return Err("Choose a game folder, not individual files.".into());
    }
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Invalid game folder name")?;
    import_begin(name)?;

    fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
        fs::create_dir_all(destination).map_err(|error| error.to_string())?;
        for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let kind = entry.file_type().map_err(|error| error.to_string())?;
            let target = destination.join(entry.file_name());
            if kind.is_dir() {
                copy_tree(&entry.path(), &target)?;
            } else if kind.is_file() {
                fs::copy(entry.path(), target).map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    }
    let temporary = IMPORT.with(|slot| slot.borrow().as_ref().unwrap().temporary.clone());

    match copy_tree(&source, &temporary) {
        Ok(()) => import_end(),
        Err(error) => {
            let _ = import_cancel();
            Err(error)
        }
    }
}

fn dispatch(request: Value) -> Result<Value, String> {
    match request["op"].as_str().unwrap_or("") {
        "info" => Ok(
            json!({"engine":"Native Rust + MRI Ruby", "platform":std::env::consts::OS,"arch":std::env::consts::ARCH,"nativePresentation":true,"gpu":true,"nativeBindings":true}),
        ),
        "games" => {
            let mut games = vec![json!({"id":".demo","title":"Touch controls demo","demo":true})];
            for entry in fs::read_dir(LIBRARY.get().ok_or("Library is not configured")?)
                .map_err(|e| e.to_string())?
            {
                let entry = entry.map_err(|e| e.to_string())?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if !name.starts_with('.') && entry.file_type().map_err(|e| e.to_string())?.is_dir()
                {
                    games.push(json!({"id":name,"title":name}));
                }
            }
            Ok(json!(games))
        }
        "init" => initialize(request["id"].as_str().ok_or("Missing game id")?),
        "key" => {
            let code = request["code"].as_i64().ok_or("Missing key code")? as i32;
            let down = request["down"].as_bool().ok_or("Missing key state")?;
            engine::with_runtime_mut(|r| {
                r.queue_input(if down {
                    InputEvent::KeyDown(code)
                } else {
                    InputEvent::KeyUp(code)
                })
            })
            .ok_or("No game is running")?;
            Ok(Value::Null)
        }
        "frame" => engine::with_runtime_mut(|r| {
            let mut rgba = Vec::new();
            let target = r.capture()?;
            target.write_rgba(&mut rgba);
            Ok::<Value, String>(
                json!({"width":target.width(),"height":target.height(),"rgba":B64.encode(rgba)}),
            )
        })
        .ok_or_else(|| "No game is running".to_string())?,
        "stop" => {
            mri::stop();
            engine::take_runtime();
            Ok(Value::Null)
        }
        "step" => {
            // Ruby may reenter the engine, so finish the fiber step before
            // borrowing the runtime for the performance counters.
            let running = mri::step()?;
            let stats = engine::with_runtime(|r| {
                json!({"frames":r.frame_count(),"rate":r.rate(),"fastForward":r.is_fast_forward()})
            })
            .unwrap_or_else(|| json!({"frames":0,"rate":0,"fastForward":false}));
            Ok(
                json!({"running":running,"frames":stats["frames"],"rate":stats["rate"],"fastForward":stats["fastForward"]}),
            )
        }
        "importBegin" => import_begin(request["name"].as_str().ok_or("Missing game name")?),
        "importChunk" => import_chunk(
            request["path"].as_str().ok_or("Missing file path")?,
            request["offset"].as_u64().unwrap_or(0),
            request["data"].as_str().ok_or("Missing file data")?,
        ),
        "importEnd" => import_end(),
        "importCancel" => import_cancel(),
        _ => Err("Unknown mobile operation".into()),
    }
}

/// Configure the library and native Ruby runtime once before creating the shell.
pub fn configure(library: PathBuf, ruby_runtime: Option<PathBuf>) -> Result<(), String> {
    fs::create_dir_all(&library).map_err(|error| error.to_string())?;
    let library = fs::canonicalize(&library).map_err(|error| error.to_string())?;
    let demo = library.join(".demo");
    fs::create_dir_all(&demo).map_err(|error| error.to_string())?;
    fs::write(demo.join("demo.rb"), DEMO).map_err(|error| error.to_string())?;
    fs::write(demo.join("Game.ini"), "[Game]\nTitle=Touch controls demo\n")
        .map_err(|error| error.to_string())?;
    let runtime = ruby_runtime.unwrap_or_else(|| PathBuf::from(env!("RSXP_NATIVE_RUBY_DEFAULT")));
    RUBY_RUNTIME
        .set(runtime)
        .map_err(|_| "Ruby runtime already configured".to_string())?;
    LIBRARY
        .set(library)
        .map_err(|_| "Library already configured".to_string())
}

/// Execute one UI request and wrap the result in the `{ok}`/`{error}` envelope
/// used by integration tests and library management (never frame presentation).
pub fn request(value: Value) -> Value {
    let result = std::panic::catch_unwind(|| dispatch(value));
    match result {
        Ok(Ok(value)) => json!({"ok":value}),
        Ok(Err(error)) => json!({"error":error}),
        Err(_) => json!({"error":"Native engine panic"}),
    }
}

#[repr(C)]
pub struct NativeResult {
    data: *mut u8,
    length: usize,
    status: i32,
}

/// Called by `ruby/bridge.c` for engine-native Ruby calls.
///
/// # Safety
/// The request must point to `length` readable bytes, valid for this call.
#[no_mangle]
pub unsafe extern "C" fn rsxp_mobile_native_bytes(
    request: *const u8,
    length: usize,
) -> NativeResult {
    let result = std::panic::catch_unwind(|| {
        engine::native_call_bytes(std::slice::from_raw_parts(request, length))
    });
    let (bytes, status) = match result {
        Ok(Ok(Some(bytes))) => (bytes, 0),
        Ok(Ok(None)) => (vec![], 1),
        Ok(Err(error)) => (error.into_bytes(), 2),
        Err(_) => (b"Native engine panic".to_vec(), 2),
    };
    let bytes = bytes.into_boxed_slice();
    let length = bytes.len();
    let data = Box::into_raw(bytes) as *mut u8;
    NativeResult {
        data,
        length,
        status,
    }
}

/// # Safety
/// Pass the exact pointer and length returned by `rsxp_mobile_native_bytes`, once.
#[no_mangle]
pub unsafe extern "C" fn rsxp_mobile_native_free(data: *mut u8, length: usize) {
    if !data.is_null() {
        drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
            data, length,
        )));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(value: Value) -> Value {
        let response = request(value);
        assert!(response.get("error").is_none(), "{response}");
        response["ok"].clone()
    }

    #[test]
    fn native_lifecycle_renders_and_handles_input() {
        let root = std::env::temp_dir().join(format!("rsxp-mobile-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        configure(root.clone(), None).unwrap();
        assert_eq!(
            call(json!({"op":"info"}))["engine"],
            "Native Rust + MRI Ruby"
        );
        prepare(".demo").unwrap();
        call(json!({"op":"key","code":79,"down":true}));
        assert!(call(json!({"op":"frame"}))["rgba"].as_str().unwrap().len() > 100);
        assert!(prepare("../escape").is_err());
        assert!(safe_file(&root, "../escape").is_none());
        fs::write(root.join("MixedCase.txt"), b"asset").unwrap();
        assert_eq!(
            fs::read(safe_file(&root, "mixedcase.TXT").unwrap()).unwrap(),
            b"asset"
        );
        let error = request(json!({"op":"unknown"}));
        assert!(error["error"].as_str().unwrap().contains("Unknown"));
        let imported = root.join("Imported Game");
        fs::create_dir_all(imported.join("Data")).unwrap();
        fs::write(
            imported.join("Game.ini"),
            "[Game]\nTitle=Imported title\nScripts=Data/Scripts.rvdata2\n",
        )
        .unwrap();
        fs::write(imported.join("Data/Scripts.rvdata2"), b"fixture").unwrap();
        let config = prepare("Imported Game").unwrap();
        assert_eq!(config["version"], 3);
        assert_eq!(config["title"], "Imported title");
        assert_eq!(config["width"], 544);
        call(json!({"op":"stop"}));
        assert!(request(json!({"op":"frame"}))["error"].is_string());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn import_rejects_escape_paths() {
        assert!(safe_relative("Data/Map001.rxdata").is_some());
        assert!(safe_relative("../escape").is_none());
        assert!(safe_relative("/absolute").is_none());
        assert!(safe_relative("").is_none());
    }
}
