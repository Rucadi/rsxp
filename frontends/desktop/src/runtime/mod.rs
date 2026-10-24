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
#[cfg(feature = "rgss")]
use crate::host::DesktopHost;
#[cfg(feature = "rgss")]
use rsxp::engine::{self, RuntimeOptions};
use rsxp_core::Content;
use std::ffi::OsStr;
use std::{error::Error, path::Path};
#[cfg(feature = "rgss")]
use std::{
    fs,
    io::{BufWriter, Write},
};

#[cfg(feature = "rgss")]
mod game;
mod options;

// CLI configuration is applied on the main thread before Winit, CPAL, Ruby,
// or any other subsystem that might create threads is initialized.
fn set_startup_env(name: &str, value: impl AsRef<OsStr>) {
    // SAFETY: all environment changes happen during single-threaded startup.
    unsafe { std::env::set_var(name, value) }
}

#[cfg(feature = "rgss")]
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
    #[cfg(not(feature = "rgss"))]
    let _ = (&renderer, version);
    if let Some(path) = game.as_ref().or(script.as_ref())
        && let Some((content, registration)) = cartridge_content(path)
    {
        return crate::cartridge::run(content, registration, headless, frames, output)
            .map_err(Into::into);
    }
    // Remember where the command was launched so a reset can restart from there
    // (the game directory becomes the working directory below).
    #[cfg(feature = "rgss")]
    {
        let launch_dir = std::env::current_dir()?;
        let game::GameConfig { version, archives } =
            game::GameConfig::prepare(script, game, version)?;
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
    #[cfg(not(feature = "rgss"))]
    {
        Err("RGSS support is not enabled in this build".into())
    }
}

/// Routes a file or directory through the build-time core catalog.
///
/// A directory is treated as cartridge content only when it contains exactly
/// one file claimed by a non-RGSS core; otherwise it remains an RGSS project.
fn cartridge_content(path: &Path) -> Option<(Content, rsxp_core::CoreRegistration)> {
    use std::fs;
    let catalog = rsxp_cores::catalog();
    let content = if path.is_dir() {
        // Never steal a directory that the catalog already identifies as an
        // RGSS project, even if it also ships a single ROM file.
        let directory = Content::directory(path);
        if catalog
            .route(&directory)
            .is_ok_and(|registration| registration.info.id == "rgss")
        {
            return None;
        }
        let mut matches = Vec::new();
        for entry in fs::read_dir(path).ok()?.filter_map(Result::ok) {
            let child = entry.path();
            if !child.is_file() {
                continue;
            }
            let candidate = Content::file(&child);
            if let Ok(registration) = catalog.route(&candidate)
                && registration.info.id != "rgss"
            {
                matches.push((candidate, registration));
            }
        }
        match matches.as_slice() {
            [(content, _)] => content.clone(),
            _ => return None,
        }
    } else {
        Content::file(path)
    };
    let registration = catalog.route(&content).ok()?;
    (registration.info.id != "rgss").then_some((content, registration))
}

/// Restarts the process with the original command line, so a reset rebuilds the
/// engine, Ruby and the game exactly as a fresh launch would.
#[cfg(feature = "rgss")]
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp(label: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "rsxp-runtime-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[cfg(feature = "gba")]
    #[test]
    fn cartridge_files_and_single_rom_directories_route_by_catalog() {
        let dir = temp("gba-route");
        let rom = dir.join("game.gba");
        fs::write(&rom, vec![0u8; 0x200]).unwrap();
        let (content, registration) = cartridge_content(&rom).unwrap();
        assert_eq!(content.path, rom);
        assert_eq!(registration.info.id, "gba");
        let (content, registration) = cartridge_content(&dir).unwrap();
        assert_eq!(content.path, dir.join("game.gba"));
        assert_eq!(registration.info.id, "gba");
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(feature = "rgss")]
    #[test]
    fn rgss_projects_are_not_stolen_by_a_single_rom() {
        let dir = temp("rgss-route");
        fs::write(dir.join("Game.ini"), "[Game]\nTitle=Test\n").unwrap();
        fs::write(dir.join("game.gba"), vec![0u8; 0x200]).unwrap();
        assert!(cartridge_content(&dir).is_none());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unknown_files_do_not_route() {
        let dir = temp("unknown-route");
        let path = dir.join("game.unknown");
        fs::write(&path, b"not a game").unwrap();
        assert!(cartridge_content(&path).is_none());
        fs::remove_dir_all(dir).unwrap();
    }
}
