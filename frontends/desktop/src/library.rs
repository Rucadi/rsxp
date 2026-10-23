// SPDX-License-Identifier: GPL-2.0-or-later
//! Native library persistence and game discovery, separate from presentation.
//!
//! The "library" is the list of recently opened games plus the helpers the
//! dashboard uses to turn a user-chosen path into a usable game root.
//! - [`load`] and [`remember`] maintain up to 12 recent entries in
//!   `library.json` under `$XDG_CONFIG_HOME` (or `~/.config`) `rsxp/`.
//!   `remember` moves a path to the front, deduplicates, truncates, and writes
//!   atomically through a pid-suffixed temp file and `rename`.
//! - [`discover`] canonicalizes the chosen file or directory. A `Game.ini`
//!   path selects its parent directory; an `.gba` path is a GBA game. A
//!   directory must contain an RPG Maker marker (`Game.ini`, an RGSS archive,
//!   or a `Data/Scripts.*` file) or exactly one `.gba` ROM. RPG Maker markers
//!   are matched case-insensitively via [`crate::host::case_path`].
//! - [`title`] reads the `Title=` value from `Game.ini`, falling back to the
//!   directory name. [`GameSource::title`] also names GBA ROMs from their file
//!   stem. [`save_directory`] returns the on-disk save folder under
//!   `$XDG_DATA_HOME` (or `~/.local/share`) `rsxp/<sanitized title>` if it
//!   already exists; [`data_directory`] returns the path without requiring it.
use std::{
    fs,
    path::{Path, PathBuf},
};
pub fn preferences_path() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
        .map(|p| p.join("rsxp/library.json"))
}
pub fn load() -> Vec<String> {
    preferences_path()
        .and_then(|p| fs::read(p).ok())
        .and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok())
        .unwrap_or_default()
        .into_iter()
        .take(12)
        .collect()
}
pub fn remember(recent: &mut Vec<String>, path: &Path) -> std::io::Result<()> {
    let path = path.to_string_lossy().into_owned();
    recent.retain(|p| p != &path);
    recent.insert(0, path);
    recent.truncate(12);
    if let Some(file) = preferences_path() {
        fs::create_dir_all(file.parent().unwrap())?;
        let temp = file.with_extension(format!("{}.tmp", std::process::id()));
        fs::write(&temp, serde_json::to_vec_pretty(recent)?)?;
        fs::rename(temp, file)?;
    }
    Ok(())
}
pub fn discover(path: &str) -> Result<GameSource, String> {
    let path = PathBuf::from(path);
    // Avoid fs::canonicalize on game directories: Windows canonicalization
    // turns network paths into `\\?\UNC\...`, which Wine may not be able to
    // use for SMB shares. Preserve the user's path and let the OS resolve it
    // when opening files / changing the working directory.
    let mut path = if path.is_dir() {
        path
    } else {
        fs::canonicalize(&path).map_err(|e| format!("Could not open project path: {e}"))?
    };
    if path.is_file()
        && path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("Game.ini"))
    {
        path.pop();
    } else if path.is_file() {
        if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("gba"))
        {
            return Ok(GameSource::Gba(path));
        }
        return Err("Choose a game directory, Game.ini, or a .gba ROM".into());
    }
    if !path.is_dir() {
        return Err("Choose a game directory, Game.ini, or a .gba ROM".into());
    }
    if [
        "Game.ini",
        "Game.rgssad",
        "Game.rgss2a",
        "Game.rgss3a",
        "Data/Scripts.rxdata",
        "Data/Scripts.rvdata",
        "Data/Scripts.rvdata2",
    ]
    .iter()
    .any(|name| crate::host::case_path(&path.join(name)).is_some())
    {
        return Ok(GameSource::Rgss(path));
    }
    let gba: Vec<PathBuf> = fs::read_dir(&path)
        .map_err(|error| format!("Could not read folder: {error}"))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|entry| {
            entry.is_file()
                && entry
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("gba"))
        })
        .collect();
    match gba.as_slice() {
        [rom] => Ok(GameSource::Gba(rom.clone())),
        [] => Err("No RPG Maker game or .gba ROM found in that directory.".into()),
        _ => Err("That folder contains multiple .gba ROMs; choose one file instead.".into()),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GameSource {
    Rgss(PathBuf),
    Gba(PathBuf),
}

impl GameSource {
    pub fn path(&self) -> &Path {
        match self {
            Self::Rgss(path) | Self::Gba(path) => path,
        }
    }
}

pub fn title(root: &Path) -> String {
    crate::host::case_path(&root.join("Game.ini"))
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|ini| {
            ini.lines().find_map(|line| {
                let (k, v) = line.split_once('=')?;
                k.trim()
                    .eq_ignore_ascii_case("Title")
                    .then(|| v.trim().to_owned())
            })
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            root.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        })
}
pub fn save_directory(title: &str) -> Option<PathBuf> {
    data_directory(title).filter(|path| path.is_dir())
}

/// Data directory for a game, whether or not it already exists. GBA sessions
/// create it before the core starts so battery saves have a home.
pub fn data_directory(title: &str) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share")))?;
    let title: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || "_. -".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    Some(base.join("rsxp").join(title))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "rsxp-library-{label}-{}-{}",
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
    fn discovers_gba_files_and_single_rom_folders() {
        let dir = temp("gba");
        let rom = dir.join("game.gba");
        fs::write(&rom, vec![0u8; 0x200]).unwrap();
        assert_eq!(
            discover(rom.to_str().unwrap()).unwrap(),
            GameSource::Gba(rom.canonicalize().unwrap())
        );
        assert_eq!(
            discover(dir.to_str().unwrap()).unwrap(),
            GameSource::Gba(rom.canonicalize().unwrap())
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_folders_without_a_game() {
        let dir = temp("empty");
        assert!(discover(dir.to_str().unwrap()).is_err());
        let _ = fs::remove_dir_all(dir);
    }
}
