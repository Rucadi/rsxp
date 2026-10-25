// SPDX-License-Identifier: GPL-2.0-or-later
//! Native game discovery and the persistent RSXP library.
//! Library entries live under RSXP/config; game data lives under RSXP/saves.
use std::{
    fs,
    path::{Path, PathBuf},
};
pub fn preferences_path() -> Option<PathBuf> {
    Some(rsxp_provision::root(None).join("config/library.json"))
}
pub fn load() -> Vec<String> {
    preferences_path()
        .and_then(|p| fs::read(p).ok())
        .or_else(|| {
            let base = std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
            fs::read(base.join("rsxp/library.json")).ok()
        })
        .and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok())
        .unwrap_or_default()
        .into_iter()
        .take(10000)
        .collect()
}
pub fn remember(recent: &mut Vec<String>, path: &Path) -> std::io::Result<()> {
    let path = path.to_string_lossy().into_owned();
    recent.retain(|p| p != &path);
    recent.insert(0, path);

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
        if path.extension().is_some_and(|extension| {
            extension.eq_ignore_ascii_case("gba") || extension.eq_ignore_ascii_case("nds")
        }) {
            return Ok(GameSource::Cartridge(path));
        }
        return Err("Choose a game directory, Game.ini, or a .gba/.nds ROM".into());
    }
    if !path.is_dir() {
        return Err("Choose a game directory, Game.ini, or a .gba/.nds ROM".into());
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
    let roms: Vec<PathBuf> = fs::read_dir(&path)
        .map_err(|error| format!("Could not read folder: {error}"))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|entry| {
            entry.is_file()
                && entry.extension().is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("gba") || extension.eq_ignore_ascii_case("nds")
                })
        })
        .collect();
    match roms.as_slice() {
        [rom] => Ok(GameSource::Cartridge(rom.clone())),
        [] => Err("No RPG Maker game or .gba/.nds ROM found in that directory.".into()),
        _ => Err("That folder contains multiple cartridge ROMs; choose one file instead.".into()),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GameSource {
    Rgss(PathBuf),
    Cartridge(PathBuf),
}

impl GameSource {
    pub fn path(&self) -> &Path {
        match self {
            Self::Rgss(path) | Self::Cartridge(path) => path,
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
pub fn data_directory(root: &Path, title: &str) -> PathBuf {
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
    let title = if title.is_empty() || title == "." || title == ".." {
        "game"
    } else {
        &title
    };
    rsxp_provision::root(Some(root))
        .join("saves/rgss")
        .join(title)
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
            GameSource::Cartridge(rom.canonicalize().unwrap())
        );
        assert_eq!(
            discover(dir.to_str().unwrap()).unwrap(),
            GameSource::Cartridge(rom.canonicalize().unwrap())
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn discovers_nds_and_rejects_ambiguous_cartridge_folders() {
        let dir = temp("nds");
        let rom = dir.join("game.NDS");
        fs::write(&rom, [0u8; 512]).unwrap();
        assert_eq!(
            discover(rom.to_str().unwrap()).unwrap(),
            GameSource::Cartridge(rom.canonicalize().unwrap())
        );
        assert_eq!(
            discover(dir.to_str().unwrap()).unwrap(),
            GameSource::Cartridge(rom.canonicalize().unwrap())
        );
        fs::write(dir.join("other.gba"), [0u8; 512]).unwrap();
        assert!(discover(dir.to_str().unwrap()).is_err());
        fs::write(dir.join("Game.ini"), "[Game]\nTitle=RGSS").unwrap();
        assert_eq!(
            discover(dir.to_str().unwrap()).unwrap(),
            GameSource::Rgss(dir.clone())
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
