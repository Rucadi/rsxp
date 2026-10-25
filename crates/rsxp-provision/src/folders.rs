// SPDX-License-Identifier: GPL-2.0-or-later
//! Locating and opening the folders a user needs to manage a game.
//!
//! The launcher exposes the same three locations through the UI and the control
//! API: the game installation, the game's save directory, and its `patches/`
//! directory. Centralising path policy here prevents the desktop UI, the
//! browser adapter, and the HTTP API from disagreeing about where a file lives.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The game installation directory. A `Game.ini` path resolves to its parent;
/// a cartridge path resolves to the directory containing the ROM.
pub fn game_directory(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let absolute = fs::canonicalize(&absolute).unwrap_or(absolute);
    if absolute.is_file() {
        absolute.parent().unwrap_or(Path::new(".")).to_owned()
    } else {
        absolute
    }
}

/// `patches/` beside the game content. The folder is not created automatically
/// by a read-only query; call [`ensure`] before writing a patch.
pub fn patches_directory(path: &Path) -> PathBuf {
    game_directory(path).join("patches")
}

/// Conventional RGSS save directory under the RSXP home.
pub fn save_directory(root: &Path, title: &str) -> PathBuf {
    root.join("saves/rgss").join(safe_title(title))
}

pub fn backups_directory(root: &Path, game_key: &str) -> PathBuf {
    crate::backup::backup_root(root, game_key)
}

/// Creates a directory tree if needed and returns its canonical path when
/// possible.
pub fn ensure(path: &Path) -> std::io::Result<PathBuf> {
    fs::create_dir_all(path)?;
    Ok(fs::canonicalize(path).unwrap_or_else(|_| path.to_owned()))
}

/// Reads `Title=` from Game.ini, falling back to the directory name.
pub fn title(path: &Path) -> String {
    let directory = game_directory(path);
    let ini = ["Game.ini", "game.ini"]
        .iter()
        .map(|name| directory.join(name))
        .find(|candidate| candidate.is_file());
    let title = ini
        .and_then(|ini| fs::read_to_string(ini).ok())
        .and_then(|text| {
            text.lines().find_map(|line| {
                let (key, value) = line.split_once('=')?;
                key.trim()
                    .eq_ignore_ascii_case("Title")
                    .then(|| value.trim().to_owned())
            })
        })
        .filter(|title| !title.is_empty());
    title.unwrap_or_else(|| {
        directory
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    })
}

/// Opens a path in the host file manager without routing through a shell.
///
/// The child is detached and its output is discarded; a missing opener is
/// reported so the API can return `opened: false` with the resolved path.
pub fn open(path: &Path) -> std::io::Result<()> {
    let mut command = if cfg!(target_os = "windows") {
        let mut command = Command::new("explorer");
        command.arg(path);
        command
    } else if cfg!(target_os = "macos") {
        let mut command = Command::new("open");
        command.arg(path);
        command
    } else {
        let mut command = Command::new("xdg-open");
        command.arg(path);
        command
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}

fn safe_title(title: &str) -> String {
    let title = title
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || "_. -".contains(character) {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let title = title.trim();
    if title.is_empty() || title == "." || title == ".." {
        "game".into()
    } else {
        title.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "rsxp-folders-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn resolves_game_save_and_patch_folders() {
        let root = temp("game");
        let game = root.join("My Game");
        fs::create_dir_all(&game).unwrap();
        fs::write(game.join("Game.ini"), "[Game]\nTitle=Thing: The Game\n").unwrap();
        assert_eq!(
            game_directory(&game.join("Game.ini")),
            game.canonicalize().unwrap()
        );
        assert_eq!(title(&game), "Thing: The Game");
        assert_eq!(
            save_directory(&root.join("RSXP"), &title(&game)),
            root.join("RSXP/saves/rgss/Thing_ The Game")
        );
        assert_eq!(patches_directory(&game), game.join("patches"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unsafe_titles_stay_inside_the_save_tree() {
        assert_eq!(safe_title("../../escape"), ".._.._escape");
        assert_eq!(safe_title("   "), "game");
    }
}
