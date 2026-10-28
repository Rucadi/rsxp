// SPDX-License-Identifier: GPL-2.0-or-later
//! Save-directory backup and restore, shared by the launcher and the player.
//!
//! Emulators commonly offer two layers of save safety: battery saves live in a
//! content-scoped directory, and an explicit snapshot/export path lets a user
//! move progress between machines. FANPLAY keeps the explicit layer here:
//!
//! * [`create`] copies a game's save directory into
//!   `<FANPLAY_HOME>/backups/<game>/<timestamp>/`.
//! * [`list`] reads the small metadata files beside each snapshot.
//! * [`restore`] stages every file first, preserves overwritten originals in a
//!   `before-restore-<nonce>` folder, then atomically renames matching files.
//! * [`delete`] removes only the named snapshot, never live saves.
//!
//! The lower-level [`copy`] is also used by the desktop path-based Saves screen.
//! It refuses overlapping trees, symlinks, incompatible destination types, and
//! runaway recursion or file counts. A validation failure leaves the live
//! saves untouched.

use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupInfo {
    pub name: String,
    pub created_at: u64,
    pub created_at_utc: String,
    pub files: usize,
    pub bytes: u64,
    pub path: String,
}

/// The directory that owns metadata and snapshots for one game key.
pub fn backup_root(root: &Path, game_key: &str) -> PathBuf {
    root.join("backups").join(safe_component(game_key))
}

pub fn default_name() -> String {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!(
        "backup-{}-{:03}",
        utc_timestamp(duration.as_secs()),
        duration.subsec_millis()
    )
}

/// Creates a named snapshot of `source`.
pub fn create(
    root: &Path,
    game_key: &str,
    source: &Path,
    name: Option<&str>,
) -> io::Result<BackupInfo> {
    let name = name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(default_name);
    validate_name(&name)?;
    let parent = backup_root(root, game_key);
    fs::create_dir_all(&parent)?;
    let destination = parent.join(&name);
    if destination.exists() || metadata_path(&parent, &name).exists() {
        return Err(io::Error::other(format!("backup `{name}` already exists")));
    }
    copy(
        &source.to_string_lossy(),
        &destination.to_string_lossy(),
        false,
    )?;
    let (files, bytes) = stats(&destination, 0)?;
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let info = BackupInfo {
        name: name.clone(),
        created_at: seconds,
        created_at_utc: utc_timestamp(seconds),
        files,
        bytes,
        path: destination.to_string_lossy().into_owned(),
    };
    let metadata = metadata_path(&parent, &name);
    let temp = metadata.with_extension(format!("json.tmp-{}", std::process::id()));
    fs::write(&temp, serde_json::to_vec_pretty(&info)?)?;
    fs::rename(temp, metadata)?;
    Ok(info)
}

pub fn list(root: &Path, game_key: &str) -> io::Result<Vec<BackupInfo>> {
    let parent = backup_root(root, game_key);
    let entries = match fs::read_dir(&parent) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut backups = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let Some(info) = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<BackupInfo>(&bytes).ok())
        else {
            continue;
        };
        if parent.join(&info.name).is_dir() {
            backups.push(info);
        }
    }
    backups.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.name.cmp(&left.name))
    });
    Ok(backups)
}

/// Restores a named snapshot over `destination`, preserving originals.
pub fn restore(
    root: &Path,
    game_key: &str,
    name: &str,
    destination: &Path,
) -> io::Result<BackupInfo> {
    validate_name(name)?;
    let parent = backup_root(root, game_key);
    let source = parent.join(name);
    if !source.is_dir() {
        return Err(io::Error::other(format!("unknown backup: {name}")));
    }
    let info = fs::read(metadata_path(&parent, name))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<BackupInfo>(&bytes).ok())
        .unwrap_or_else(|| BackupInfo {
            name: name.to_owned(),
            created_at: 0,
            created_at_utc: String::new(),
            files: 0,
            bytes: 0,
            path: source.to_string_lossy().into_owned(),
        });
    copy(
        &source.to_string_lossy(),
        &destination.to_string_lossy(),
        true,
    )?;
    Ok(info)
}

pub fn delete(root: &Path, game_key: &str, name: &str) -> io::Result<()> {
    validate_name(name)?;
    let parent = backup_root(root, game_key);
    let directory = parent.join(name);
    if !directory.is_dir() {
        return Err(io::Error::other(format!("unknown backup: {name}")));
    }
    fs::remove_dir_all(directory)?;
    let _ = fs::remove_file(metadata_path(&parent, name));
    Ok(())
}

fn metadata_path(parent: &Path, name: &str) -> PathBuf {
    parent.join(format!("{name}.json"))
}

fn validate_name(name: &str) -> io::Result<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\', '\0'])
        || name.chars().any(char::is_control)
    {
        return Err(io::Error::other("invalid backup name"));
    }
    Ok(())
}

fn stats(root: &Path, depth: usize) -> io::Result<(usize, u64)> {
    if depth > 64 {
        return Err(io::Error::other("backup nesting exceeds limit"));
    }
    let mut files = 0;
    let mut bytes = 0;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            return Err(io::Error::other("backup cannot contain symlinks"));
        }
        if kind.is_dir() {
            let (nested_files, nested_bytes) = stats(&entry.path(), depth + 1)?;
            files += nested_files;
            bytes += nested_bytes;
        } else if kind.is_file() {
            files += 1;
            bytes += entry.metadata()?.len();
        }
    }
    Ok((files, bytes))
}

fn safe_component(value: &str) -> String {
    let mut out = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    if out.is_empty() || out == "." || out == ".." {
        out.insert_str(0, "game_");
    }
    let digest = crate::hash(value.as_bytes());
    out.truncate(80);
    format!("{out}-{}", &digest[..8])
}

/// Copies every file from `from` into `to`.
///
/// Backup mode (`restore == false`) requires an empty destination. Restore mode
/// preserves existing files under `before-restore-<nonce>` before replacing
/// them. Validation happens before any destination file is touched.
pub fn copy(from: &str, to: &str, restore: bool) -> io::Result<()> {
    let source = fs::canonicalize(from)?;
    let files = collect(&source, 0)?;
    if files.is_empty() {
        return Err(io::Error::other("Source directory contains no files"));
    }
    fs::create_dir_all(to)?;
    let target = fs::canonicalize(to)?;
    if source.starts_with(&target) || target.starts_with(&source) {
        return Err(io::Error::other(
            "Save and backup directories must be separate",
        ));
    }
    if !restore && fs::read_dir(&target)?.next().is_some() {
        return Err(io::Error::other("Choose an empty backup directory"));
    }
    for path in &files {
        validate_destination(
            &target,
            path.strip_prefix(&source).map_err(io::Error::other)?,
        )?;
    }
    let nonce = format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let safety = target.join(format!("before-restore-{nonce}"));
    let staging = target.join(format!(".fanplay-copy-{nonce}"));
    fs::create_dir(&staging)?;
    let result = (|| {
        for path in &files {
            let relative = path.strip_prefix(&source).map_err(io::Error::other)?;
            let staged = staging.join(relative);
            fs::create_dir_all(staged.parent().unwrap())?;
            fs::copy(path, &staged)?;
            if restore && target.join(relative).is_file() {
                let backup = safety.join(relative);
                fs::create_dir_all(backup.parent().unwrap())?;
                fs::copy(target.join(relative), backup)?;
            }
        }
        for path in &files {
            let relative = path.strip_prefix(&source).map_err(io::Error::other)?;
            let dest = target.join(relative);
            validate_destination(&target, relative)?;
            fs::create_dir_all(dest.parent().unwrap())?;
            fs::rename(staging.join(relative), dest)?;
        }
        Ok(())
    })();
    let _ = fs::remove_dir_all(staging);
    result
}

fn validate_destination(root: &Path, relative: &Path) -> io::Result<()> {
    let mut path = root.to_owned();
    let count = relative.components().count();
    for (index, part) in relative.components().enumerate() {
        if !matches!(part, Component::Normal(_)) {
            return Err(io::Error::other("Invalid save path"));
        }
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(io::Error::other("Destination cannot contain symlinks"));
            }
            Ok(metadata)
                if (index + 1 < count && !metadata.is_dir())
                    || (index + 1 == count && !metadata.is_file()) =>
            {
                return Err(io::Error::other(
                    "Destination has an incompatible file type",
                ));
            }
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
    }
    Ok(())
}

fn collect(root: &Path, depth: usize) -> io::Result<Vec<PathBuf>> {
    if depth > 64 {
        return Err(io::Error::other("Save directory nesting exceeds limit"));
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            return Err(io::Error::other("Save directories cannot contain symlinks"));
        }
        if kind.is_dir() {
            files.extend(collect(&entry.path(), depth + 1)?);
        } else if kind.is_file() {
            files.push(entry.path());
        }
        if files.len() > 100_000 {
            return Err(io::Error::other("Save directory contains too many files"));
        }
    }
    Ok(files)
}

/// Minimal UTC formatter: `YYYYMMDD-HHMMSS`. Avoids a date dependency for one
/// human-readable backup label.
fn utc_timestamp(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let day_seconds = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        day_seconds / 3_600,
        (day_seconds % 3_600) / 60,
        day_seconds % 60
    )
}

/// Howard Hinnant's `civil_from_days`, valid for the full i64 day range used
/// by Unix timestamps.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    (year + i64::from(month <= 2), month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "fanplay-backup-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn restore_preserves_originals_and_unrelated_files() {
        let root = temp("restore");
        let source = root.join("backup");
        let target = root.join("saves");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(source.join("nested/slot.sav"), b"new").unwrap();
        fs::create_dir(target.join("nested")).unwrap();
        fs::write(target.join("nested/slot.sav"), b"old").unwrap();
        fs::write(target.join("other.sav"), b"keep").unwrap();
        copy(source.to_str().unwrap(), target.to_str().unwrap(), true).unwrap();
        assert_eq!(fs::read(target.join("nested/slot.sav")).unwrap(), b"new");
        assert_eq!(fs::read(target.join("other.sav")).unwrap(), b"keep");
        let safety = fs::read_dir(&target)
            .unwrap()
            .flatten()
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("before-restore-")
            })
            .unwrap()
            .path();
        assert_eq!(fs::read(safety.join("nested/slot.sav")).unwrap(), b"old");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn named_backups_round_trip_and_delete() {
        let root = temp("named");
        let library = root.join("FANPLAY");
        let saves = root.join("saves");
        fs::create_dir_all(&saves).unwrap();
        fs::write(saves.join("Game.rxdata"), b"one").unwrap();
        let info = create(&library, "local:/game", &saves, Some("first")).unwrap();
        assert_eq!(info.files, 1);
        assert!(info.created_at_utc.len() == 15);
        let listed = list(&library, "local:/game").unwrap();
        assert_eq!(listed.len(), 1);
        fs::write(saves.join("Game.rxdata"), b"two").unwrap();
        restore(&library, "local:/game", "first", &saves).unwrap();
        assert_eq!(fs::read(saves.join("Game.rxdata")).unwrap(), b"one");
        delete(&library, "local:/game", "first").unwrap();
        assert!(list(&library, "local:/game").unwrap().is_empty());
        assert!(create(&library, "local:/game", &saves, Some("../escape")).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refuses_overlapping_directories_and_existing_backups() {
        let root = temp("overlap");
        fs::write(root.join("slot.sav"), b"data").unwrap();
        assert!(copy(
            root.to_str().unwrap(),
            root.join("backup").to_str().unwrap(),
            false
        )
        .is_err());
        let target = temp("existing");
        fs::write(target.join("existing"), b"keep").unwrap();
        assert!(copy(root.to_str().unwrap(), target.to_str().unwrap(), false).is_err());
        assert_eq!(fs::read(target.join("existing")).unwrap(), b"keep");
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(target).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_destination_symlinks_before_replacing_any_files() {
        let root = temp("symlink");
        let source = root.join("backup");
        let target = root.join("saves");
        let outside = root.join("outside");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&target).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(source.join("slot.sav"), b"new").unwrap();
        fs::write(outside.join("slot.sav"), b"old").unwrap();
        std::os::unix::fs::symlink(outside.join("slot.sav"), target.join("slot.sav")).unwrap();
        assert!(copy(source.to_str().unwrap(), target.to_str().unwrap(), true).is_err());
        assert_eq!(fs::read(outside.join("slot.sav")).unwrap(), b"old");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn utc_labels_use_the_real_calendar() {
        assert_eq!(utc_timestamp(0), "19700101-000000");
        assert_eq!(utc_timestamp(1_700_000_000), "20231114-221320");
    }
}
