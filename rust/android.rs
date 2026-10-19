// SPDX-License-Identifier: GPL-2.0-or-later
//! Android NativeActivity frontend with a target-compiled embedded CRuby.
use crate::{
    engine::{self, Runtime, RuntimeOptions},
    graphics::{Bitmap, Color},
    host::Host,
    input::Input,
};
use android_activity::{
    input::{InputEvent as AndroidInputEvent, MotionAction},
    AndroidApp, InputStatus, MainEvent, PollEvent,
};
use magnus::{function, RString, Ruby};
use ndk::native_window::NativeWindow;
use std::{
    fs,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const WIDTH: u32 = 544;
const HEIGHT: u32 = 416;

// Magnus initializes the embedded Ruby VM, but Ruby's executable normally
// registers statically linked extensions before running the application.
unsafe extern "C" {
    fn Init_enc();
    fn Init_ext();
}

struct AndroidHost {
    app: AndroidApp,
    window: Arc<Mutex<Option<NativeWindow>>>,
    start: Instant,
    destroyed: bool,
}

impl Host for AndroidHost {
    fn read_file(&self, path: &str) -> Option<Vec<u8>> {
        let relative = Path::new(path.trim_start_matches('/'));
        if relative.components().any(|part| {
            matches!(
                part,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        }) {
            return None;
        }
        let relative = relative.to_str()?;
        match crate::android_saf::read_file(&self.app, relative) {
            Ok(bytes) => bytes,
            Err(error) => {
                log::warn!("Could not read game file {relative}: {error}");
                None
            }
        }
    }
    fn file_exists(&self, path: &str) -> Option<bool> {
        match crate::android_saf::file_exists(&self.app, path) {
            Ok(exists) => Some(exists),
            Err(error) => {
                log::warn!("Could not check game file {path}: {error}");
                None
            }
        }
    }
    fn is_dir(&self, path: &str) -> Option<bool> {
        match crate::android_saf::is_directory(&self.app, path) {
            Ok(is_dir) => Some(is_dir),
            Err(error) => {
                log::warn!("Could not inspect game directory {path}: {error}");
                None
            }
        }
    }
    fn read_dir(&self, path: &str) -> Option<Vec<String>> {
        match crate::android_saf::read_dir(&self.app, path) {
            Ok(entries) => entries,
            Err(error) => {
                log::warn!("Could not list game directory {path}: {error}");
                None
            }
        }
    }
    fn image(&self, bytes: &[u8]) -> Result<Bitmap, String> {
        crate::desktop_media::image(bytes)
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
        crate::desktop_media::text(font, size, bold, italic, text, color, render)
    }
    fn poll(&mut self, input: &mut Input) -> bool {
        let app = self.app.clone();
        let window = self.window.clone();
        let destroyed = &mut self.destroyed;
        app.poll_events(Some(Duration::ZERO), |event| match event {
            PollEvent::Main(MainEvent::InitWindow { .. }) => {
                if let Some(w) = app.native_window() {
                    let _ = w.set_buffers_geometry(
                        WIDTH as i32,
                        HEIGHT as i32,
                        Some(ndk::hardware_buffer_format::HardwareBufferFormat::R8G8B8A8_UNORM),
                    );
                    if let Ok(mut slot) = window.lock() {
                        *slot = Some(w);
                    }
                }
            }
            PollEvent::Main(MainEvent::TerminateWindow { .. }) => {
                if let Ok(mut slot) = window.lock() {
                    *slot = None;
                }
            }
            PollEvent::Main(MainEvent::Destroy) => *destroyed = true,
            PollEvent::Main(MainEvent::InputAvailable) => {
                if let Ok(mut events) = app.input_events_iter() {
                    while events.next(|event| {
                        if let AndroidInputEvent::MotionEvent(motion) = event {
                            let action = motion.action();
                            if motion.pointer_count() > 0
                                && matches!(
                                    action,
                                    MotionAction::Down
                                        | MotionAction::PointerDown
                                        | MotionAction::Move
                                )
                            {
                                let p = motion.pointer_at_index(0);
                                let (sw, sh) = window
                                    .lock()
                                    .ok()
                                    .and_then(|w| w.clone())
                                    .map_or((WIDTH as i32, HEIGHT as i32), |w| {
                                        (w.width(), w.height())
                                    });
                                input.mouse_x = p.x() as i32 * WIDTH as i32 / sw.max(1);
                                input.mouse_y = p.y() as i32 * HEIGHT as i32 / sh.max(1);
                                input.mouse_inside = true;
                            }
                            if matches!(action, MotionAction::Down | MotionAction::PointerDown) {
                                input.key_down(1);
                            }
                            if matches!(
                                action,
                                MotionAction::Up | MotionAction::PointerUp | MotionAction::Cancel
                            ) {
                                input.key_up(1);
                            }
                            InputStatus::Handled
                        } else {
                            InputStatus::Unhandled
                        }
                    }) {}
                }
            }
            _ => {}
        });
        !self.destroyed
    }
    fn present(&mut self, frame: &Bitmap) -> Result<(), String> {
        let window = self
            .window
            .lock()
            .map_err(|_| "window lock poisoned")?
            .clone();
        let Some(window) = window else {
            return Ok(());
        };
        let mut buffer = window.lock(None).map_err(|e| e.to_string())?;
        let (width, height, stride) = (
            buffer.width().min(frame.width() as usize),
            buffer.height().min(frame.height() as usize),
            buffer.stride(),
        );
        let bytes = buffer.bytes().ok_or("unsupported Android surface format")?;
        for y in 0..height {
            for x in 0..width {
                let [a, r, g, b] = frame.pixels()[y * frame.width() as usize + x].to_be_bytes();
                let i = (y * stride + x) * 4;
                bytes[i].write(r);
                bytes[i + 1].write(g);
                bytes[i + 2].write(b);
                bytes[i + 3].write(a);
            }
        }
        Ok(())
    }
    fn now(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }
    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

fn native_call(ruby: &Ruby, request: RString) -> Result<Option<RString>, magnus::Error> {
    let bytes = unsafe { request.as_slice() }.to_vec();
    match engine::native_call_bytes(&bytes) {
        Ok(Some(result)) => Ok(Some(ruby.str_from_slice(&result))),
        Ok(None) => Ok(None),
        Err(message) => Err(magnus::Error::new(ruby.exception_runtime_error(), message)),
    }
}

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    static LOGGER: std::sync::Once = std::sync::Once::new();
    LOGGER.call_once(|| {
        android_logger::init_once(android_logger::Config::default().with_tag("rsxp"))
    });
    let private = match app.internal_data_path() {
        Some(path) => path,
        None => {
            log::error!("Android did not provide an app-private data path");
            return;
        }
    };
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_game(&app, &private)));
    let failure = match result {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(error),
        Err(payload) => Some(format!("Native engine panic: {}", panic_message(payload))),
    };
    if let Some(error) = failure {
        log::error!("{error}");
        if let Err(write_error) = fs::write(private.join("last_error.txt"), error.as_bytes()) {
            log::error!("Could not save error report for launcher: {write_error}");
        }
    }
}

fn run_game(app: &AndroidApp, private: &Path) -> Result<(), String> {
    let game_ini = crate::android_saf::read_file(app, "Game.ini")?
        .ok_or_else(|| "Selected game folder does not contain Game.ini".to_string())?;
    let ini = String::from_utf8_lossy(&game_ini);
    let scripts = ini
        .lines()
        .find_map(|line| {
            let (k, v) = line.split_once('=')?;
            k.trim()
                .eq_ignore_ascii_case("Scripts")
                .then(|| v.trim().replace('\\', "/"))
        })
        .unwrap_or_else(|| "Data/Scripts.rvdata2".into());
    let version = if scripts.ends_with(".rxdata") {
        1
    } else if scripts.ends_with(".rvdata") {
        2
    } else {
        3
    };
    let title = ini
        .lines()
        .find_map(|line| {
            let (k, v) = line.split_once('=')?;
            k.trim()
                .eq_ignore_ascii_case("Title")
                .then(|| v.trim().to_owned())
        })
        .unwrap_or_else(|| "RPG Maker game".into());
    unsafe {
        std::env::set_var("RSXP_SCRIPTS", &scripts);
        std::env::set_var("RSXP_GAME_TITLE", title);
        std::env::set_var("RSXP_SAF_FILES", "1");
        let abi = if std::env::consts::ARCH == "aarch64" {
            "arm64-v8a"
        } else {
            "x86_64"
        };
        let stdlib = private.join("ruby").join(abi).join("3.4.0");
        let ruby_arch = if abi == "arm64-v8a" {
            "aarch64-linux-android-android"
        } else {
            "x86_64-linux-android-android"
        };
        std::env::set_var(
            "RUBYLIB",
            format!("{}:{}", stdlib.display(), stdlib.join(ruby_arch).display()),
        );
    }
    let game_id = crate::android_saf::game_id(app)?;
    let save_root = private.join("game-saves").join(game_id);
    fs::create_dir_all(&save_root)
        .map_err(|error| format!("Could not create game save folder: {error}"))?;
    unsafe {
        std::env::set_var("RSXP_GAME_SAVE_ROOT", &save_root);
        std::env::set_current_dir(&save_root)
            .map_err(|error| format!("Could not set game save folder: {error}"))?;
    }
    let archives = ["Game.rgssad", "Game.rgss2a", "Game.rgss3a"]
        .iter()
        .filter_map(|name| match crate::android_saf::read_file(app, name) {
            Ok(bytes) => bytes,
            Err(error) => {
                log::warn!("Could not read game archive {name}: {error}");
                None
            }
        })
        .collect();
    let window = Arc::new(Mutex::new(None));
    let host = AndroidHost {
        app: app.clone(),
        window,
        start: Instant::now(),
        destroyed: false,
    };
    let runtime = Runtime::new(RuntimeOptions {
        width: WIDTH,
        height: HEIGHT,
        version,
        headless: false,
        frame_limit: None,
        rate: if version == 1 { 40 } else { 60 },
        fast_forward: false,
        archives,
        scripted: vec![],
        host: Box::new(host),
    })
    .map_err(|error| format!("Could not create engine runtime: {error}"))?;
    engine::set_runtime(runtime);
    let ruby = unsafe { magnus::embed::init() };
    unsafe { Init_enc() };
    unsafe { Init_ext() };
    let result = run_ruby(&ruby).map_err(|error| format!("Ruby game failed:\n{error}"));
    drop(ruby);
    let _ = engine::take_runtime();
    result
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else {
        "unknown panic payload".into()
    }
}

fn run_ruby(ruby: &Ruby) -> Result<(), String> {
    crate::bindings::bind(ruby).map_err(|e| e.to_string())?;
    ruby.define_global_function("__rsxp_native", function!(native_call, 1))
        .map_err(|e| e.to_string())?;
    ruby.define_global_function("__mkxp_native", function!(native_call, 1))
        .map_err(|e| e.to_string())?;
    ruby.eval::<magnus::Value>(include_str!("rgss.rb"))
        .map_err(|e| e.to_string())?;
    Ok(())
}
