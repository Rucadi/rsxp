// SPDX-License-Identifier: GPL-2.0-or-later
//! Native mobile ABI. The same engine library is linked by iOS and Android.
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use rsxp_core::{
    engine::{self, Runtime, RuntimeOptions},
    graphics::{Bitmap, Color},
    host::{Host, InputEvent},
};
use serde_json::{json, Value};
use std::{
    ffi::{c_char, CStr, CString},
    fs,
    path::{Component, Path, PathBuf},
    sync::OnceLock,
};

static LIBRARY: OnceLock<PathBuf> = OnceLock::new();
static RUBY_RUNTIME: OnceLock<PathBuf> = OnceLock::new();
mod mri;
mod present;
/// Appends diagnostic lines to `<library>/.gpu-debug.log` for device builds
/// where stderr is not attached to a console.
pub(crate) fn debug_log(message: &str) {
    let Some(library) = LIBRARY.get() else {
        return;
    };
    use std::io::Write;
    if let Ok(mut file) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(library.join(".gpu-debug.log"))
    {
        let _ = writeln!(file, "{message}");
    }
}
thread_local! {static FRAME: std::cell::RefCell<Vec<u8>>=const {std::cell::RefCell::new(Vec::new())};}
// Platform drawable attached by the shell before the first game starts.
thread_local! {
    static SURFACE: std::cell::Cell<*mut std::ffi::c_void> = const { std::cell::Cell::new(std::ptr::null_mut()) };
    static SURFACE_SIZE: std::cell::Cell<(u32, u32)> = const { std::cell::Cell::new((1, 1)) };
    static GPU_ACTIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
const DEMO: &str = include_str!("../../demo.rb");

fn safe_path(root: &Path, name: &str) -> Option<PathBuf> {
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
    path.starts_with(root).then_some(path)
}

fn safe_file(root: &Path, name: &str) -> Option<PathBuf> {
    safe_path(root, name).filter(|path| path.is_file())
}

fn safe_dir(root: &Path, name: &str) -> Option<PathBuf> {
    safe_path(root, name).filter(|path| path.is_dir())
}

struct MobileHost {
    root: PathBuf,
    /// Compositor owned by this host; `None` keeps the software renderer.
    gpu: Option<rsxp_core::gpu::GpuRenderer>,
    presenter: Option<present::Presenter>,
}
impl Host for MobileHost {
    fn read_file(&self, path: &str) -> Option<Vec<u8>> {
        fs::read(safe_file(&self.root, path)?).ok()
    }
    fn gpu(&mut self) -> Option<&mut rsxp_core::gpu::GpuRenderer> {
        if std::env::var_os("RSXP_FORCE_SOFTWARE").is_some() {
            return None;
        }
        self.gpu.as_mut()
    }
    fn present_gpu(&mut self) -> Result<(), String> {
        let (Some(gpu), Some(presenter)) = (self.gpu.as_mut(), self.presenter.as_mut()) else {
            return Ok(());
        };
        let (width, height) = SURFACE_SIZE.with(|size| size.get());
        presenter.resize(gpu, width, height);
        presenter.present(gpu)
    }
    fn present(&mut self, frame: &Bitmap) -> Result<(), String> {
        // Scenes the GPU compositor cannot reproduce exactly fall back to the
        // software reference renderer; upload that frame and present it.
        let (Some(gpu), Some(presenter)) = (self.gpu.as_mut(), self.presenter.as_mut()) else {
            return Ok(());
        };
        let (width, height) = SURFACE_SIZE.with(|size| size.get());
        presenter.resize(gpu, width, height);
        presenter.present_bitmap(gpu, frame)
    }
    fn image(&self, bytes: &[u8]) -> Result<Bitmap, String> {
        rsxp_core::portable_media::image(bytes)
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
        rsxp_core::portable_media::text(font, size, bold, italic, text, color, render)
    }
}

fn prepare(id: &str) -> Result<Value, String> {
    if Path::new(id).components().count() != 1 || id == ".." || id.contains(['/', '\\']) {
        return Err("Invalid game identifier".into());
    }
    let library = LIBRARY.get().ok_or("Library is not configured")?;
    let requested = library.join(id);
    let linked = requested
        .symlink_metadata()
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false);
    let root = requested.canonicalize().map_err(|e| e.to_string())?;
    if !root.starts_with(library) && !linked {
        return Err("Game is outside the library".into());
    }
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
    let mut scripts = field("Scripts").unwrap_or_else(|| "Data/Scripts.rxdata".into());
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
        // MKXP-style releases often extract the script bundle into a directory
        // of `.rb` files while leaving the original `Scripts=Data\Scripts.rxdata`
        // entry in Game.ini.
        if safe_dir(&root, &scripts).is_some() {
            // The configured path itself is a directory.
        } else if safe_dir(&root, "Data/Scripts").is_some() {
            scripts = "Data/Scripts".into();
        } else {
            return Err("No RPG Maker scripts or archive found in this folder".into());
        }
    }
    let (width, height, rate) = if version == 1 {
        (640, 480, 40)
    } else {
        (544, 416, 60)
    };
    // Tear down any previous game before reusing the platform drawable: two
    // wgpu surfaces must not be attached to the same CAMetalLayer at once.
    GPU_ACTIVE.with(|active| active.set(false));
    mri::stop();
    engine::take_runtime();
    // Attach the platform drawable (a CAMetalLayer on iOS) so scenes composite
    // on the GPU and present without a CPU readback. If Metal is unavailable
    // the runtime falls back to the software renderer.
    let layer = SURFACE.with(|surface| surface.get());
    let (gpu, presenter) = if layer.is_null() {
        (None, None)
    } else {
        let (view_width, view_height) = SURFACE_SIZE.with(|size| size.get());
        match unsafe { present::Presenter::attach(layer, view_width, view_height) } {
            Ok((gpu, presenter)) => {
                debug_log(&format!("gpu presentation ready on {}", gpu.adapter_name));
                (Some(gpu), Some(presenter))
            }
            Err(error) => {
                debug_log(&format!("gpu presentation unavailable: {error}"));
                eprintln!("RSXP GPU presentation unavailable: {error}");
                (None, None)
            }
        }
    };
    GPU_ACTIVE.with(|active| active.set(gpu.is_some()));
    engine::set_runtime(Runtime::new(RuntimeOptions {
        width,
        height,
        version,
        rate,
        headless: false,
        frame_limit: None,
        fast_forward: true,
        archives,
        scripted: vec![],
        host: Box::new(MobileHost {
            root: root.clone(),
            gpu,
            presenter,
        }),
    })?);
    rsxp_core::runtime_settings::reset();
    rsxp_core::runtime_settings::set_fast_forward(true); // The shared UI paces Ruby fibers.
    Ok(
        json!({"title":field("Title").unwrap_or_else(|| id.into()), "version":version,
        "width":width,"height":height,"rate":rate,"scripts":scripts,
        "script":if id == ".demo" { Some("demo.rb") } else { None }}),
    )
}

fn initialize(id: &str) -> Result<Value, String> {
    let config = prepare(id)?;
    let library = LIBRARY.get().ok_or("Library is not configured")?;
    let root = library.join(id).canonicalize().map_err(|e| e.to_string())?;
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

fn dispatch(request: Value) -> Result<Value, String> {
    match request["op"].as_str().unwrap_or("") {
        "info" => Ok(
            json!({"engine":"Native Rust + MRI Ruby", "platform":std::env::consts::OS,"arch":std::env::consts::ARCH,"nativePresentation":cfg!(any(target_os="ios",target_os="android")),"gpu":GPU_ACTIVE.with(|active| active.get())}),
        ),
        "games" => {
            let mut games = vec![json!({"id":".demo","title":"Touch controls demo","demo":true})];
            let library = LIBRARY.get().ok_or("Library is not configured")?;
            for entry in fs::read_dir(library).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let name = entry.file_name().to_string_lossy().into_owned();
                // External Android games are linked into the library with a
                // symlink, so follow the entry instead of checking file_type().
                if !name.starts_with('.') && entry.path().is_dir() {
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
            // Read back a GPU-composited frame when necessary.
            let target = r.capture()?;
            target.write_rgba(&mut rgba);
            Ok::<Value, String>(json!({"width":target.width(),"height":target.height(),"rgba":B64.encode(rgba)}))
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
            Ok(json!({"running":running,"frames":stats["frames"],"rate":stats["rate"],"fastForward":stats["fastForward"]}))
        }
        _ => Err("Unknown mobile operation".into()),
    }
}

/// # Safety
/// `directory` must point to a valid, NUL-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rsxp_mobile_configure(
    directory: *const c_char,
    ruby_runtime: *const c_char,
) -> i32 {
    if directory.is_null() {
        return 0;
    }
    let result = std::panic::catch_unwind(|| -> Result<(), String> {
        let path = unsafe { CStr::from_ptr(directory) }
            .to_str()
            .map_err(|e| e.to_string())?;
        fs::create_dir_all(path).map_err(|e| e.to_string())?;
        let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
        let demo = path.join(".demo");
        fs::create_dir_all(&demo).map_err(|e| e.to_string())?;
        fs::write(demo.join("demo.rb"), DEMO).map_err(|e| e.to_string())?;
        fs::write(demo.join("Game.ini"), "[Game]\nTitle=Touch controls demo\n")
            .map_err(|e| e.to_string())?;
        let runtime = if ruby_runtime.is_null() {
            PathBuf::from(env!("RSXP_NATIVE_RUBY_DEFAULT"))
        } else {
            PathBuf::from(
                unsafe { CStr::from_ptr(ruby_runtime) }
                    .to_str()
                    .map_err(|e| e.to_string())?,
            )
        };
        RUBY_RUNTIME
            .set(runtime)
            .map_err(|_| "Ruby runtime already configured".to_string())?;
        LIBRARY
            .set(path)
            .map_err(|_| "Library already configured".to_string())
    });
    i32::from(matches!(result, Ok(Ok(()))))
}

/// # Safety
/// `request` must point to a valid, NUL-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rsxp_mobile_request(request: *const c_char) -> *mut c_char {
    let result = std::panic::catch_unwind(|| -> Result<Value, String> {
        if request.is_null() {
            return Err("Null request".into());
        }
        let text = unsafe { CStr::from_ptr(request) }
            .to_str()
            .map_err(|e| e.to_string())?;
        dispatch(serde_json::from_str(text).map_err(|e| e.to_string())?)
    });
    let response = match result {
        Ok(Ok(value)) => json!({"ok":value}),
        Ok(Err(error)) => json!({"error":error}),
        Err(_) => json!({"error":"Native engine panic"}),
    };
    CString::new(response.to_string()).unwrap().into_raw()
}

/// # Safety
/// `response` must be null or an unfreed pointer returned by this library.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rsxp_mobile_free(response: *mut c_char) {
    if !response.is_null() {
        drop(unsafe { CString::from_raw(response) });
    }
}

#[repr(C)]
pub struct NativeResult {
    data: *mut u8,
    length: usize,
    status: i32,
}
/// # Safety
/// The request must point to `length` readable bytes, valid for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rsxp_mobile_native_bytes(
    request: *const u8,
    length: usize,
) -> NativeResult {
    let result = std::panic::catch_unwind(|| {
        engine::native_call_bytes(unsafe { std::slice::from_raw_parts(request, length) })
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rsxp_mobile_native_free(data: *mut u8, length: usize) {
    if !data.is_null() {
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(data, length)) });
    }
}
/// # Safety
/// Output pointers must be non-null. Returned RGBA bytes are borrowed until the
/// next frame call, and must be consumed on the same native engine thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rsxp_mobile_frame(
    width: *mut u32,
    height: *mut u32,
    length: *mut usize,
) -> *const u8 {
    if width.is_null() || height.is_null() || length.is_null() {
        return std::ptr::null();
    }
    unsafe {
        *width = 0;
        *height = 0;
        *length = 0;
    }
    engine::with_runtime_mut(|runtime| {
        FRAME.with(|slot| {
            let mut bytes = slot.borrow_mut();
            let target = match runtime.capture() {
                Ok(target) => target,
                Err(_) => return std::ptr::null(),
            };
            target.write_rgba(&mut bytes);
            unsafe {
                *width = target.width();
                *height = target.height();
                *length = bytes.len();
            }
            bytes.as_ptr()
        })
    })
    .unwrap_or(std::ptr::null())
}

/// Attaches the platform drawable used for GPU presentation. `layer` is a
/// `CAMetalLayer` on iOS; `width` and `height` are its size in physical pixels.
/// Call before starting a game. Returns 1 when the drawable was accepted.
///
/// # Safety
/// `layer` must stay alive for as long as games present to it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rsxp_mobile_attach_surface(
    layer: *mut std::ffi::c_void,
    width: u32,
    height: u32,
) -> i32 {
    if layer.is_null() {
        return 0;
    }
    SURFACE.with(|surface| surface.set(layer));
    SURFACE_SIZE.with(|size| size.set((width.max(1), height.max(1))));
    1
}

/// Resizes the game drawable in physical pixels after a rotation or layout
/// change. Returns 1 when a drawable is attached.
#[unsafe(no_mangle)]
pub extern "C" fn rsxp_mobile_resize_surface(width: u32, height: u32) -> i32 {
    if SURFACE.with(|surface| surface.get()).is_null() {
        return 0;
    }
    SURFACE_SIZE.with(|size| size.set((width.max(1), height.max(1))));
    1
}

/// Whether the active game composites and presents through the GPU.
#[unsafe(no_mangle)]
pub extern "C" fn rsxp_mobile_gpu_active() -> i32 {
    i32::from(GPU_ACTIVE.with(|active| active.get()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_lifecycle_renders_and_handles_input() {
        let root = std::env::temp_dir().join(format!("rsxp-mobile-{}", std::process::id()));
        let path = CString::new(root.to_str().unwrap()).unwrap();
        assert_eq!(
            unsafe { rsxp_mobile_configure(path.as_ptr(), std::ptr::null()) },
            1
        );
        assert_eq!(
            dispatch(json!({"op":"info"})).unwrap()["engine"],
            "Native Rust + MRI Ruby"
        );
        prepare(".demo").unwrap();
        dispatch(json!({"op":"key","code":79,"down":true})).unwrap();
        assert!(
            dispatch(json!({"op":"frame"})).unwrap()["rgba"]
                .as_str()
                .unwrap()
                .len()
                > 100
        );
        assert!(prepare("../escape").is_err());
        assert!(safe_file(&root, "../escape").is_none());
        fs::write(root.join("MixedCase.txt"), b"asset").unwrap();
        assert_eq!(
            fs::read(safe_file(&root, "mixedcase.TXT").unwrap()).unwrap(),
            b"asset"
        );
        let request = CString::new("{\"op\":\"unknown\"}").unwrap();
        let response = unsafe { rsxp_mobile_request(request.as_ptr()) };
        let error: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(response) }.to_str().unwrap()).unwrap();
        assert!(error["error"].as_str().unwrap().contains("Unknown"));
        unsafe { rsxp_mobile_free(response) };
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
        // MKXP-style extracted scripts: Game.ini still names Scripts.rxdata,
        // but the game ships a directory of loose Ruby files.
        let loose = root.join("Loose Game");
        fs::create_dir_all(loose.join("Data/Scripts")).unwrap();
        fs::write(
            loose.join("Game.ini"),
            "[Game]\nTitle=Loose\nScripts=Data\\Scripts.rxdata\n",
        )
        .unwrap();
        fs::write(loose.join("Data/Scripts/001_Test.rb"), b"# test\n").unwrap();
        let config = prepare("Loose Game").unwrap();
        assert_eq!(config["scripts"], "Data/Scripts");
        assert_eq!(config["version"], 1);
        dispatch(json!({"op":"stop"})).unwrap();
        assert!(dispatch(json!({"op":"frame"})).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
