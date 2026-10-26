// SPDX-License-Identifier: GPL-2.0-or-later
//! Permanent, selectable JSON catalogs.
//!
//! A catalog source is stored in `config/config.json` once and stays there
//! until the user removes it. Downloads are cached under
//! `repositories/cache/<id>.json`, so refreshing an unreachable source never
//! discards the last good catalog. This mirrors the useful part of emulator
//! game-list designs: the list is a user-owned document, not an ephemeral
//! network response.
//!
//! Catalog ids are stable and derived from the source URL/path. Two different
//! sources cannot silently replace each other's cached games.

use crate::{hash, load_repository, Repository, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CatalogEntry {
    pub id: String,
    pub name: String,
    pub source: String,
    pub selected: bool,
    #[serde(default)]
    pub last_refreshed: Option<u64>,
    #[serde(default)]
    pub game_count: Option<usize>,
    #[serde(default)]
    pub last_error: Option<String>,
}

impl CatalogEntry {
    pub fn from_source(source: &str, name: Option<&str>, selected: bool) -> Self {
        let source = source.trim().to_owned();
        let id = stable_id(&source);
        Self {
            id,
            name: name
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| display_name(&source)),
            source,
            selected,
            last_refreshed: None,
            game_count: None,
            last_error: None,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.id.is_empty()
            || !self
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err("Invalid catalog id".into());
        }
        if self.source.trim().is_empty() {
            return Err("Catalog source cannot be empty".into());
        }
        if self.name.trim().is_empty() {
            return Err("Catalog name cannot be empty".into());
        }
        Ok(())
    }
}

fn stable_id(source: &str) -> String {
    let stem = display_name(source);
    let mut id = stem
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    while id.contains("--") {
        id = id.replace("--", "-");
    }
    let id = id.trim_matches('-');
    let id = if id.is_empty() { "catalog" } else { id };
    format!(
        "{}-{}",
        &id[..id.len().min(32)],
        &hash(source.as_bytes())[..8]
    )
}

fn display_name(source: &str) -> String {
    let trimmed = source.trim_end_matches('/');
    if let Some(rest) = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
    {
        let host = rest.split('/').next().unwrap_or(rest);
        let file = rest.rsplit('/').next().unwrap_or("catalog");
        let stem = file.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(file);
        if stem.is_empty() || stem == host {
            host.to_owned()
        } else {
            format!("{host}/{stem}")
        }
    } else {
        Path::new(trimmed)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|stem| !stem.is_empty())
            .unwrap_or(trimmed)
            .to_owned()
    }
}

pub fn cache_path(root: &Path, id: &str) -> PathBuf {
    root.join("repositories/cache").join(format!("{id}.json"))
}

pub fn load_entries(root: &Path) -> Vec<CatalogEntry> {
    crate::config::load(root)
        .map(|config| config.catalogs)
        .unwrap_or_default()
}

pub fn save_entries(root: &Path, entries: &[CatalogEntry]) -> Result<()> {
    let mut config = crate::config::load(root)?;
    config.catalogs = entries.to_vec();
    crate::config::save(root, &config)
}

/// Adds a source permanently. Re-adding the same URL/path updates the display
/// name and selection but keeps the existing cache and id.
pub fn add(root: &Path, source: &str, name: Option<&str>, selected: bool) -> Result<CatalogEntry> {
    let mut config = crate::config::load(root)?;
    let source = source.trim();
    if source.is_empty() {
        return Err("Catalog source cannot be empty".into());
    }
    if let Some(entry) = config
        .catalogs
        .iter_mut()
        .find(|entry| entry.source == source)
    {
        if let Some(name) = name.map(str::trim).filter(|name| !name.is_empty()) {
            entry.name = name.to_owned();
        }
        entry.selected = selected;
        let entry = entry.clone();
        crate::config::save(root, &config)?;
        return Ok(entry);
    }
    let entry = CatalogEntry::from_source(source, name, selected);
    entry.validate()?;
    if config
        .catalogs
        .iter()
        .any(|candidate| candidate.id == entry.id)
    {
        return Err(format!("Catalog id {} is already in use", entry.id).into());
    }
    config.catalogs.push(entry.clone());
    crate::config::save(root, &config)?;
    Ok(entry)
}

pub fn remove(root: &Path, id: &str) -> Result<bool> {
    let mut config = crate::config::load(root)?;
    let before = config.catalogs.len();
    config.catalogs.retain(|entry| entry.id != id);
    let removed = config.catalogs.len() != before;
    if removed {
        crate::config::save(root, &config)?;
    }
    Ok(removed)
}

pub fn select(root: &Path, id: &str, selected: bool) -> Result<()> {
    let mut config = crate::config::load(root)?;
    let entry = config
        .catalogs
        .iter_mut()
        .find(|entry| entry.id == id)
        .ok_or_else(|| format!("Unknown catalog id: {id}"))?;
    entry.selected = selected;
    crate::config::save(root, &config)
}

/// Refreshes one or every catalog and updates the permanent entries.
///
/// Network failures are recorded but never remove the previous cache. The
/// returned strings are user-facing warnings.
pub fn refresh(root: &Path, only: Option<&str>) -> (Vec<CatalogEntry>, Vec<String>) {
    let mut config = match crate::config::load(root) {
        Ok(config) => config,
        Err(error) => return (Vec::new(), vec![error.to_string()]),
    };
    let mut warnings = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for entry in config
        .catalogs
        .iter_mut()
        .filter(|entry| only.is_none_or(|id| id == entry.id))
    {
        if !seen.insert(entry.id.clone()) {
            warnings.push(format!("Duplicate catalog id: {}", entry.id));
            continue;
        }
        match load_repository(&entry.source) {
            Ok(repo) => {
                if let Err(error) = write_cache(root, entry, &repo) {
                    warnings.push(format!("{}: {error}", entry.name));
                    entry.last_error = Some(error.to_string());
                    continue;
                }
                entry.last_refreshed = Some(now());
                entry.game_count = Some(repo.games.len());
                entry.last_error = None;
            }
            Err(error) => {
                entry.last_error = Some(error.to_string());
                if cached(root, &entry.id).is_some() {
                    warnings.push(format!(
                        "{}: {error} (using the cached catalog)",
                        entry.name
                    ));
                } else {
                    warnings.push(format!("{}: {error}", entry.name));
                }
            }
        }
    }
    if let Err(error) = crate::config::save(root, &config) {
        warnings.push(error.to_string());
    }
    (config.catalogs, warnings)
}

fn write_cache(root: &Path, entry: &CatalogEntry, repo: &Repository) -> Result<()> {
    let path = cache_path(root, &entry.id);
    fs::create_dir_all(path.parent().unwrap())?;
    let temp = path.with_extension(format!("json.tmp-{}", std::process::id()));
    fs::write(&temp, serde_json::to_vec_pretty(repo)?)?;
    fs::rename(temp, path)?;
    Ok(())
}

/// Reads a selected or unselected cached catalog.
pub fn cached(root: &Path, id: &str) -> Option<Repository> {
    let from_cache = fs::read(cache_path(root, id))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Repository>(&bytes).ok());
    if from_cache.is_some() {
        return from_cache;
    }
    // Pre-0.2 installations kept one combined `repositories/catalog.json`.
    fs::read(root.join("repositories/catalog.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<Repository>>(&bytes).ok())
        .and_then(|repositories| {
            repositories
                .into_iter()
                .find(|repository| repository.id == id)
        })
}

pub fn cached_games(root: &Path, id: &str) -> Option<Repository> {
    cached(root, id)
}

/// Loads every selected catalog from cache. The second value lists warnings for
/// selected sources that have never been refreshed.
pub fn selected(root: &Path) -> (Vec<Repository>, Vec<String>) {
    let config = match crate::config::load(root) {
        Ok(config) => config,
        Err(error) => return (Vec::new(), vec![error.to_string()]),
    };
    if config.catalogs.is_empty() {
        // Pre-0.2 installations kept one combined repository cache. Keep those
        // games visible until the user adds a permanent source.
        let repositories = fs::read(root.join("repositories/catalog.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Vec<Repository>>(&bytes).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|repository| repository.validate().is_ok())
            .collect::<Vec<_>>();
        if !repositories.is_empty() {
            return (
                repositories,
                vec![
                    "Legacy cached catalogs are shown; add a JSON source to keep them updated"
                        .into(),
                ],
            );
        }
    }
    let mut repositories = Vec::new();
    let mut warnings = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for entry in config.catalogs.iter().filter(|entry| entry.selected) {
        match cached(root, &entry.id) {
            Some(repository) if seen.insert(repository.id.clone()) => repositories.push(repository),
            Some(repository) => {
                warnings.push(format!("Duplicate repository id: {}", repository.id))
            }
            None => warnings.push(format!("{} has not been refreshed yet", entry.name)),
        }
    }
    (repositories, warnings)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{settings, Download, Format, Game, Platform};

    fn temp(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "fanplay-catalog-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn write_repo(root: &Path, name: &str) -> PathBuf {
        let bytes = b"rom";
        let repo = Repository {
            schema_version: 1,
            id: name.into(),
            name: name.into(),
            games: vec![Game {
                id: "demo".into(),
                title: "Demo".into(),
                version: "1".into(),
                platform: Platform::Gba,
                description: String::new(),
                image: None,
                config: settings::Settings::default(),
                download: Download {
                    url: "demo.gba".into(),
                    sha256: hash(bytes),
                    format: Format::Rom,
                },
                entrypoint: "demo.gba".into(),
                base: None,
                output_sha256: None,
            }],
        };
        let path = root.join(format!("{name}.json"));
        fs::write(&path, serde_json::to_vec(&repo).unwrap()).unwrap();
        path
    }

    #[test]
    fn catalogs_are_permanent_and_selectable() {
        let root = temp("select");
        let source = write_repo(&root, "one");
        let entry = add(&root, &source.to_string_lossy(), Some("One"), true).unwrap();
        let (entries, warnings) = refresh(&root, None);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(entries[0].game_count, Some(1));
        assert!(cached(&root, &entry.id).is_some());
        select(&root, &entry.id, false).unwrap();
        assert!(selected(&root).0.is_empty());
        select(&root, &entry.id, true).unwrap();
        assert_eq!(selected(&root).0.len(), 1);
        assert!(remove(&root, &entry.id).unwrap());
        assert!(cached(&root, &entry.id).is_some(), "cache survives removal");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refresh_failure_keeps_last_good_cache() {
        let root = temp("offline");
        let source = write_repo(&root, "offline");
        let entry = add(&root, &source.to_string_lossy(), None, true).unwrap();
        refresh(&root, None);
        fs::remove_file(&source).unwrap();
        let (entries, warnings) = refresh(&root, None);
        assert!(entries[0].last_error.as_deref().is_some());
        assert!(warnings[0].contains("cached"));
        assert!(cached(&root, &entry.id).is_some());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ids_are_stable_and_distinct() {
        let a = CatalogEntry::from_source("https://example.org/games.json", None, true);
        let b = CatalogEntry::from_source("https://example.org/games.json", None, true);
        let c = CatalogEntry::from_source("https://elsewhere.org/games.json", None, true);
        assert_eq!(a.id, b.id);
        assert_ne!(a.id, c.id);
        assert!(a.validate().is_ok());
    }
}
