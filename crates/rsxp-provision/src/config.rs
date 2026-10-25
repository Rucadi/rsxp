// SPDX-License-Identifier: GPL-2.0-or-later
//! One persistent JSON configuration in the RSXP home.
//!
//! The file lives at `<RSXP_HOME>/config/config.json`. It owns settings that
//! apply to the whole installation: the common game settings, permanently
//! stored catalogs, the loopback control API, and the last selected game.
//!
//! Per-game overrides remain in `config/games/<hash>.json` for backward
//! compatibility with existing installations, but they are validated and
//! merged through the same [`crate::settings`] module. Keeping the
//! installation-wide file separate makes a backup, inspection or edit of the
//! "global" configuration a single file operation.

use crate::{settings, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_API_PORT: u16 = 47_821;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ApiConfig {
    /// Start the control API with the launcher by default. It binds loopback
    /// only; `RSXP_API=0` overrides this for one process.
    pub enabled: bool,
    pub port: u16,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            port: DEFAULT_API_PORT,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,
    pub api: ApiConfig,
    pub settings: settings::Settings,
    pub catalogs: Vec<crate::catalog::CatalogEntry>,
    /// Per-game overrides. New writes go here; old `config/games/*.json` files
    /// are still read as a fallback for existing installations.
    pub games: std::collections::BTreeMap<String, settings::Preferences>,
    pub last_game: Option<String>,
}

impl Config {
    pub fn new() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            ..Default::default()
        }
    }

    pub fn normalize(&mut self) -> Result<()> {
        if self.schema_version == 0 {
            self.schema_version = SCHEMA_VERSION;
        }
        if self.schema_version > SCHEMA_VERSION {
            return Err(format!(
                "configuration schema {} is newer than this build supports ({})",
                self.schema_version, SCHEMA_VERSION
            )
            .into());
        }
        if self.api.port == 0 {
            self.api.port = DEFAULT_API_PORT;
        }
        self.settings.validate()?;
        for preferences in self.games.values() {
            if let Some(settings) = &preferences.settings {
                settings.validate()?;
            }
        }
        for entry in &self.catalogs {
            entry.validate()?;
        }
        Ok(())
    }
}

pub fn path(root: &Path) -> PathBuf {
    root.join("config/config.json")
}

/// Loads the installation configuration, creating the directory tree and
/// migrating the old `repositories/sources.json` list on first use.
///
/// A corrupt configuration is moved aside as `config.json.corrupt-<pid>` and a
/// fresh default is returned, so a bad edit cannot prevent the launcher from
/// starting. The error is printed for the user to inspect.
pub fn load(root: &Path) -> Result<Config> {
    crate::initialize(root)?;
    let file = path(root);
    let existed = file.exists();
    let mut config = match fs::read(&file) {
        Ok(bytes) => match serde_json::from_slice::<Config>(&bytes) {
            Ok(config) => config,
            Err(error) => {
                let backup = file.with_extension(format!("json.corrupt-{}", std::process::id()));
                let _ = fs::rename(&file, &backup);
                eprintln!(
                    "rsxp: {} is invalid ({error}); moved to {}",
                    file.display(),
                    backup.display()
                );
                Config::new()
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Config::new(),
        Err(error) => return Err(error.into()),
    };
    let migrated = if config.catalogs.is_empty() {
        let migrated = migrate_sources(root)?;
        let changed = !migrated.is_empty();
        config.catalogs = migrated;
        changed
    } else {
        false
    };
    config.normalize()?;
    if !existed || migrated {
        save(root, &config)?;
    }
    Ok(config)
}

/// Loads and immediately persists a normalized configuration.
pub fn load_and_save(root: &Path) -> Result<Config> {
    let config = load(root)?;
    save(root, &config)?;
    Ok(config)
}

pub fn save(root: &Path, config: &Config) -> Result<()> {
    let mut config = config.clone();
    config.normalize()?;
    let file = path(root);
    fs::create_dir_all(file.parent().unwrap())?;
    let temp = file.with_extension(format!("json.tmp-{}", std::process::id()));
    let bytes = serde_json::to_vec_pretty(&config)?;
    fs::write(&temp, bytes)?;
    fs::rename(temp, file)?;
    Ok(())
}

/// Converts the legacy bare source list into permanent catalog entries.
fn migrate_sources(root: &Path) -> Result<Vec<crate::catalog::CatalogEntry>> {
    let file = root.join("repositories/sources.json");
    let sources = match fs::read(&file) {
        Ok(bytes) => serde_json::from_slice::<Vec<String>>(&bytes).unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    let mut entries = Vec::new();
    for source in sources {
        if source.trim().is_empty() {
            continue;
        }
        let entry = crate::catalog::CatalogEntry::from_source(&source, None, true);
        if !entries
            .iter()
            .any(|candidate: &crate::catalog::CatalogEntry| {
                candidate.id == entry.id || candidate.source == entry.source
            })
        {
            entries.push(entry);
        }
    }
    Ok(entries)
}

/// Reads the last library list, falling back to the pre-RSXP location.
pub fn library(root: &Path) -> Vec<String> {
    let file = root.join("config/library.json");
    let legacy = || {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
        fs::read(base.join("rsxp/library.json")).ok()
    };
    fs::read(&file)
        .ok()
        .or_else(legacy)
        .and_then(|bytes| serde_json::from_slice::<Vec<String>>(&bytes).ok())
        .unwrap_or_default()
        .into_iter()
        .take(10_000)
        .collect()
}

pub fn save_library(root: &Path, recent: &[String]) -> Result<()> {
    let file = root.join("config/library.json");
    fs::create_dir_all(file.parent().unwrap())?;
    let temp = file.with_extension(format!("json.tmp-{}", std::process::id()));
    fs::write(&temp, serde_json::to_vec_pretty(recent)?)?;
    fs::rename(temp, file)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "rsxp-config-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn defaults_are_written_atomically_in_the_home() {
        let root = temp("defaults");
        let config = load_and_save(&root).unwrap();
        assert_eq!(config.schema_version, SCHEMA_VERSION);
        assert!(config.api.enabled);
        assert_eq!(config.api.port, DEFAULT_API_PORT);
        let bytes = fs::read(path(&root)).unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("\"api\""));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn corrupt_config_is_preserved_and_replaced() {
        let root = temp("corrupt");
        fs::create_dir_all(root.join("config")).unwrap();
        fs::write(path(&root), b"{ not json").unwrap();
        let config = load(&root).unwrap();
        assert_eq!(config.schema_version, SCHEMA_VERSION);
        let backups = fs::read_dir(root.join("config"))
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().contains("corrupt"))
            .count();
        assert_eq!(backups, 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_sources_become_permanent_catalog_entries() {
        let root = temp("sources");
        fs::create_dir_all(root.join("repositories")).unwrap();
        fs::write(
            root.join("repositories/sources.json"),
            br#"["https://example.org/games.json", "/tmp/local.json"]"#,
        )
        .unwrap();
        let config = load(&root).unwrap();
        assert_eq!(config.catalogs.len(), 2);
        assert!(config.catalogs.iter().all(|entry| entry.selected));
        assert!(config.catalogs.iter().all(|entry| !entry.id.is_empty()));
        fs::remove_dir_all(root).unwrap();
    }
}
