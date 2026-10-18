// SPDX-License-Identifier: GPL-2.0-or-later
//! Native library persistence and game discovery, separate from presentation.
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
pub fn discover(path: &str) -> Result<PathBuf, String> {
    let mut path = fs::canonicalize(path).map_err(|e| e.to_string())?;
    if path.is_file()
        && path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("Game.ini"))
    {
        path.pop();
    }
    if !path.is_dir() {
        return Err("Choose a game directory or Game.ini".into());
    }
    if ![
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
        return Err("No RPG Maker game found in that directory (expected Game.ini, scripts, or an RGSS archive)".into());
    }
    Ok(path)
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
    let path = base.join("rsxp").join(title);
    path.is_dir().then_some(path)
}
