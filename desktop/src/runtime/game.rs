// SPDX-License-Identifier: GPL-2.0-or-later
//! Game discovery: working directory, `Game.ini`, RGSS version inference and
//! archive loading.
//!
//! [`GameConfig`] carries the resolved RGSS `version` and the raw bytes of any
//! encrypted archives that must be searched. [`GameConfig::prepare`] runs before any
//! subsystem that creates threads: it canonicalizes `--script` or `--game`, sets the
//! process working directory, and publishes the Ruby shim's startup environment via
//! `set_startup_env`/`remove_startup_env` (`RSXP_SCRIPT`, `RSXP_GAME_TITLE`,
//! `RSXP_SCRIPTS`).
//!
//! The RGSS version is the explicit `--rgss` override when given, else inferred from
//! the `Scripts=` entry in `Game.ini` (`.rvdata2`→3, `.rvdata`→2, `.rxdata`→1), else
//! from the presence of `Game.rgss3a`/`Game.rgss2a` plus `Data/Scripts.*`, defaulting
//! to RGSS1. Archives are discovered case-insensitively through `crate::host::case_path`
//! across `Game.rgssad`, `Game.rgss2a` and `Game.rgss3a`, and each is parsed with
//! `Archive::parse` to validate it before being retained.
use super::{remove_startup_env, set_startup_env};
use crate::host::case_path;
use rsxp_core::archive::Archive;
use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
};

pub(super) struct GameConfig {
    pub version: u8,
    pub archives: Vec<Vec<u8>>,
}

impl GameConfig {
    // Must run before initializing subsystems that create threads: this sets
    // the game working directory and the Ruby shim's startup environment.
    pub fn prepare(
        script: Option<PathBuf>,
        game: Option<PathBuf>,
        version: Option<u8>,
    ) -> Result<Self, Box<dyn Error>> {
        if let Some(path) = script {
            let path = fs::canonicalize(path)?;
            set_startup_env("RSXP_SCRIPT", &path);
            std::env::set_current_dir(path.parent().ok_or("script has no parent")?)?;
        } else if let Some(path) = game {
            std::env::set_current_dir(fs::canonicalize(path)?)?;
            remove_startup_env("RSXP_SCRIPT");
        }
        let ini = case_path(Path::new("Game.ini"))
            .and_then(|p| fs::read_to_string(p).ok())
            .unwrap_or_default();
        let script_path = ini.lines().find_map(|line| {
            let (k, v) = line.split_once('=')?;
            k.trim()
                .eq_ignore_ascii_case("Scripts")
                .then(|| v.trim().replace('\\', "/"))
        });
        if let Some(title) = ini.lines().find_map(|line| {
            let (key, value) = line.split_once('=')?;
            key.trim()
                .eq_ignore_ascii_case("Title")
                .then_some(value.trim())
        }) {
            set_startup_env("RSXP_GAME_TITLE", title);
        }
        let inferred = script_path.as_ref().and_then(|p| {
            if p.ends_with(".rvdata2") {
                Some(3)
            } else if p.ends_with(".rvdata") {
                Some(2)
            } else if p.ends_with(".rxdata") {
                Some(1)
            } else {
                None
            }
        });
        let version = version.or(inferred).unwrap_or_else(|| {
            if Path::new("Game.rgss3a").exists() || Path::new("Data/Scripts.rvdata2").exists() {
                3
            } else if Path::new("Game.rgss2a").exists() || Path::new("Data/Scripts.rvdata").exists()
            {
                2
            } else {
                1
            }
        });
        set_startup_env(
            "RSXP_SCRIPTS",
            script_path.unwrap_or_else(|| {
                format!(
                    "Data/Scripts.{}",
                    match version {
                        1 => "rxdata",
                        2 => "rvdata",
                        _ => "rvdata2",
                    }
                )
            }),
        );
        let mut archives = Vec::new();
        for name in ["Game.rgssad", "Game.rgss2a", "Game.rgss3a"] {
            if let Some(path) = case_path(Path::new(name)) {
                let bytes = fs::read(path)?;
                Archive::parse(&bytes)?;
                archives.push(bytes);
            }
        }
        Ok(Self { version, archives })
    }
}
