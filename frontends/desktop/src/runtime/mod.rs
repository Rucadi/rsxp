// SPDX-License-Identifier: GPL-2.0-or-later
//! Desktop application lifecycle for the `rsxp` binary.
//!
//! [`run`] parses [`options::RunOptions`], remembers the launch directory, prepares
//! the game working directory and archives ([`game::GameConfig`]), builds the
//! [`DesktopHost`] and engine `Runtime`, then creates a configured
//! [`ruby_rgss_core::RubyCore`] that starts the RGSS game. It then drops Ruby (running
//! finalizers while the Rust resource registry is still available), prints the native
//! optionally captures a PPM frame, and returns the game status as a `u8`.
//!
//! Environment and lifecycle details:
//! - [`set_startup_env`]/[`remove_startup_env`] must run on the single-threaded main
//!   thread before `winit`, CPAL or Ruby create threads.
//! - Scripted input comes from `RSXP_INPUT`; runtime patches come from
//!   `runtime_patch::load_user_patches` plus `RSXP_PATCH`; `RSXP_JIT` selects
//!   `yjit` (default), `zjit`, or `off`. Legacy `RSXP_YJIT=0` disables the default.
//! - [`validate`] only parses arguments. A reset requested during the run restarts the
//!   process from the launch directory via [`restart`], rebuilding the engine, Ruby and
//!   game exactly as a fresh launch would.
//!
//! Argument validation, game discovery and bridge callbacks live in the `options`,
//! `game` and `bridge` submodules.
use crate::host::DesktopHost;
use rsxp::engine::{self, RuntimeOptions};
use std::{
    error::Error,
    ffi::OsStr,
    fs,
    io::{BufWriter, Write},
    path::Path,
};

mod game;
mod options;

// CLI configuration is applied on the main thread before Winit, CPAL, Ruby,
// or any other subsystem that might create threads is initialized.
fn set_startup_env(name: &str, value: impl AsRef<OsStr>) {
    // SAFETY: all environment changes happen during single-threaded startup.
    unsafe { std::env::set_var(name, value) }
}

fn remove_startup_env(name: &str) {
    // SAFETY: all environment changes happen during single-threaded startup.
    unsafe { std::env::remove_var(name) }
}

pub fn validate(args: &[String]) -> Result<(), Box<dyn Error>> {
    options::RunOptions::parse(args).map(|_| ())
}

pub fn run(args: &[String]) -> Result<u8, Box<dyn Error>> {
    let options::RunOptions {
        renderer,
        script,
        game,
        headless,
        frames,
        output,
        version,
    } = options::RunOptions::parse(args)?;
    // Remember where the command was launched so a reset can restart from there
    // (the game directory becomes the working directory below).
    let launch_dir = std::env::current_dir()?;
    let game::GameConfig { version, archives } = game::GameConfig::prepare(script, game, version)?;
    let (width, height) = if version == 1 { (640, 480) } else { (544, 416) };
    let scripted = match std::env::var("RSXP_INPUT") {
        Ok(value) => engine::parse_scripted(&value)?,
        Err(_) => Vec::new(),
    };
    let host = DesktopHost::new(width, height, headless, &renderer)?;
    let runtime = engine::Runtime::new(RuntimeOptions {
        width,
        height,
        version,
        headless,
        frame_limit: frames,
        rate: if version == 1 { 40 } else { 60 },
        fast_forward: false,
        archives,
        scripted,
        host: Box::new(host),
    })?;
    engine::set_runtime(runtime);
    rsxp::api_bridge::start_from_env().map_err(std::io::Error::other)?;
    // Live frame-rate/fast-forward/reset controls start from their defaults;
    // the workspace UI drives them from here.
    rsxp::runtime_settings::reset();
    // Pre-enable runtime patches so headless runs and CI can exercise them
    // without the workspace UI.
    rsxp::runtime_patch::reset();
    rsxp::runtime_patch::load_user_patches(&std::env::current_dir()?)?;
    if let Ok(value) = std::env::var("RSXP_PATCH") {
        for id in rsxp::runtime_patch::enable_from(&value) {
            eprintln!("Runtime patch enabled: {id}");
        }
    }
    let ruby = ruby_rgss_core::RubyCore::new().map_err(std::io::Error::other)?;
    let status = ruby.startup_status();
    // Run Ruby finalizers while the Rust resource registry is still available.
    drop(ruby);
    // The workspace "Reset game" button stops the game here so everything is
    // recreated from the beginning.
    if rsxp::runtime_settings::reset_requested() {
        return restart(launch_dir.as_path());
    }
    let mut runtime = engine::take_runtime().ok_or("runtime closed unexpectedly")?;
    if let Some(path) = output {
        let mut writer = BufWriter::new(fs::File::create(path)?);
        runtime.capture()?.write_ppm(&mut writer)?;
        writer.flush()?;
    }
    Ok(u8::try_from(status?).unwrap_or(1))
}

/// Restarts the process with the original command line, so a reset rebuilds the
/// engine, Ruby and the game exactly as a fresh launch would.
fn restart(launch_dir: &Path) -> Result<u8, Box<dyn Error>> {
    let exe = std::env::current_exe()?;
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    std::env::set_current_dir(launch_dir)?;
    eprintln!("rsxp: resetting the game…");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = std::process::Command::new(exe).args(args).exec();
        Err(error.into())
    }
    #[cfg(not(unix))]
    {
        std::process::Command::new(exe).args(args).spawn()?;
        Ok(0)
    }
}
