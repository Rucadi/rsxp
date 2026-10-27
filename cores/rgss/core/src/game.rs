// SPDX-License-Identifier: GPL-2.0-or-later
//! RGSS content discovery and startup environment.
//!
//! The adapter owns the process working directory because the RGSS script
//! loader and SabiRuby use `Dir.pwd`. The isolated runner already gives each game
//! its own process. In-process use must therefore keep the RGSS core in a
//! dedicated worker process until the shim is made cwd-independent.

#[cfg(not(target_arch = "wasm32"))]
use fanplay_core::ContentKind;
use fanplay_core::{Content, CoreError, CoreResult, Platform};
use fanplay_rgss::archive::Archive;
#[cfg(not(target_arch = "wasm32"))]
use std::ffi::OsStr;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

pub(crate) struct GameConfig {
    pub version: u8,
    pub width: u32,
    pub height: u32,
    pub rate: u32,
    pub archives: Vec<Vec<u8>>,
    #[cfg(target_arch = "wasm32")]
    pub scripts: String,
    #[cfg(target_arch = "wasm32")]
    pub title: String,
}

// Environment mutation is restricted to the core's startup path, before SabiRuby,
// audio or any other subsystem creates threads. A RGSS core is process-exclusive
// by contract (see the module comment).
#[cfg(not(target_arch = "wasm32"))]
fn set_startup_env(name: &str, value: impl AsRef<OsStr>) {
    // SAFETY: called during single-threaded core startup.
    unsafe { std::env::set_var(name, value) }
}

#[cfg(not(target_arch = "wasm32"))]
fn remove_startup_env(name: &str) {
    // SAFETY: called during single-threaded core startup.
    unsafe { std::env::remove_var(name) }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn prepare(content: &Content, platform: &dyn Platform) -> CoreResult<GameConfig> {
    let (root, script) = match content.kind {
        ContentKind::Directory => {
            let root = content.path.canonicalize().map_err(|error| {
                CoreError::content(format!("{}: {error}", content.path.display()))
            })?;
            (root, None)
        }
        ContentKind::File => {
            let script = content.path.canonicalize().map_err(|error| {
                CoreError::content(format!("{}: {error}", content.path.display()))
            })?;
            let root = script
                .parent()
                .ok_or_else(|| CoreError::content("script has no parent directory"))?
                .to_owned();
            (root, Some(script))
        }
    };
    std::env::set_current_dir(&root).map_err(|error| {
        CoreError::content(format!("could not enter {}: {error}", root.display()))
    })?;

    let ini = read_file(&root, platform, "Game.ini")
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_default();
    let field = |name: &str| {
        ini.lines().find_map(|line| {
            let (key, value) = line.split_once('=')?;
            key.trim()
                .eq_ignore_ascii_case(name)
                .then(|| value.trim().replace('\\', "/"))
        })
    };

    if let Some(title) = field("Title") {
        set_startup_env("FANPLAY_GAME_TITLE", title);
    } else if let Some(script) = &script
        && let Some(stem) = script.file_stem()
    {
        set_startup_env("FANPLAY_GAME_TITLE", stem);
    }

    let version = match &script {
        Some(script) => version_from_extension(script),
        None => field("Scripts")
            .as_deref()
            .and_then(version_from_name)
            .unwrap_or_else(|| infer_directory_version(&root)),
    };

    if let Some(script) = script {
        set_startup_env("FANPLAY_SCRIPT", &script);
        remove_startup_env("FANPLAY_SCRIPTS");
    } else {
        remove_startup_env("FANPLAY_SCRIPT");
        let scripts = field("Scripts").unwrap_or_else(|| {
            format!(
                "Data/Scripts.{}",
                match version {
                    1 => "rxdata",
                    2 => "rvdata",
                    _ => "rvdata2",
                }
            )
        });
        set_startup_env("FANPLAY_SCRIPTS", scripts);
    }

    let mut archives = Vec::new();
    for name in ["Game.rgssad", "Game.rgss2a", "Game.rgss3a"] {
        let Some(bytes) = read_file(&root, platform, name) else {
            continue;
        };
        Archive::parse(&bytes).map_err(|error| CoreError::content(format!("{name}: {error}")))?;
        archives.push(bytes);
    }

    let (width, height) = if version == 1 { (640, 480) } else { (544, 416) };
    Ok(GameConfig {
        version,
        width,
        height,
        rate: if version == 1 { 40 } else { 60 },
        archives,
    })
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn prepare(content: &Content, platform: &dyn Platform) -> CoreResult<GameConfig> {
    let name = content.path.to_string_lossy().replace('\\', "/");
    let extension = content.extension().unwrap_or_default().to_ascii_lowercase();
    let bytes = platform
        .read_content(&name)
        .or_else(|_| {
            let default_name = match extension.as_str() {
                "rxdata" => "Data/Scripts.rxdata",
                "rvdata" => "Data/Scripts.rvdata",
                "rvdata2" => "Data/Scripts.rvdata2",
                _ => return Err(fanplay_core::PlatformError::not_found(&name)),
            };
            platform.read_content(default_name)
        })
        .map_err(|error| CoreError::content(format!("could not read RGSS content: {error}")))?;
    let version = match extension.as_str() {
        "rvdata2" | "rgss3a" => 3,
        "rvdata" | "rgss2a" => 2,
        _ => 1,
    };
    let archives = if matches!(extension.as_str(), "rgssad" | "rgss2a" | "rgss3a") {
        Archive::parse(&bytes).map_err(|error| CoreError::content(format!("{name}: {error}")))?;
        vec![bytes]
    } else {
        Vec::new()
    };
    if archives.is_empty() && !matches!(extension.as_str(), "rxdata" | "rvdata" | "rvdata2") {
        return Err(CoreError::content(
            "web RGSS content must be a Game.rgssad/Game.rgss2a/Game.rgss3a archive or a Data/Scripts.* bundle",
        ));
    }
    let scripts = match version {
        1 => "Data/Scripts.rxdata",
        2 => "Data/Scripts.rvdata",
        _ => "Data/Scripts.rvdata2",
    }
    .to_owned();
    Ok(GameConfig {
        version,
        width: if version == 1 { 640 } else { 544 },
        height: if version == 1 { 480 } else { 416 },
        rate: if version == 1 { 40 } else { 60 },
        archives,
        scripts,
        title: content.label.clone(),
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn version_from_extension(path: &Path) -> u8 {
    match path.extension().and_then(|value| value.to_str()) {
        Some(value) if value.eq_ignore_ascii_case("rvdata2") => 3,
        Some(value) if value.eq_ignore_ascii_case("rvdata") => 2,
        _ => 1,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn version_from_name(value: &str) -> Option<u8> {
    if value.ends_with(".rvdata2") {
        Some(3)
    } else if value.ends_with(".rvdata") {
        Some(2)
    } else if value.ends_with(".rxdata") {
        Some(1)
    } else {
        None
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn infer_directory_version(root: &Path) -> u8 {
    if root.join("Game.rgss3a").exists() || root.join("Data/Scripts.rvdata2").exists() {
        3
    } else if root.join("Game.rgss2a").exists() || root.join("Data/Scripts.rvdata").exists() {
        2
    } else {
        1
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn read_file(root: &Path, platform: &dyn Platform, name: &str) -> Option<Vec<u8>> {
    platform
        .read_content(name)
        .ok()
        .or_else(|| std::fs::read(root.join(name)).ok())
}
