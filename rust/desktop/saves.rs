// SPDX-License-Identifier: GPL-2.0-or-later
//! Directory save/backup copying with preflight validation and atomic replacement.
//!
//! [`copy`] recursively copies every file from `from` into `to`. In backup mode
//! (`restore == false`) the destination must be empty; in restore mode existing
//! destination files are first preserved under a `before-restore-<nonce>` directory.
//! Source and target must not overlap, neither tree may contain symlinks, and a
//! recursion depth cap (64) and file-count cap (100,000) guard against runaway trees.
//!
//! The operation is transactional: all incoming files are staged into a
//! `.rsxp-copy-<nonce>` directory inside the target and every destination path is
//! validated before any existing file is replaced, so a validation failure leaves
//! saves untouched. The staging directory is always cleaned up afterward.
//! [`validate_destination`] rejects symlinks and file/directory type mismatches, and
//! [`collect`] walks the source rejecting symlinks.
use std::{
    fs, io,
    path::{Component, Path, PathBuf},
};
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
    // Validate every destination before replacing any file.
    for path in &files {
        validate_destination(
            &target,
            path.strip_prefix(&source).map_err(io::Error::other)?,
        )?;
    }
    let nonce = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let safety = target.join(format!("before-restore-{nonce}"));
    let staging = target.join(format!(".rsxp-copy-{nonce}"));
    fs::create_dir(&staging)?;
    let result = (|| {
        // Stage all incoming files and preserve all originals before committing.
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
                return Err(io::Error::other("Destination cannot contain symlinks"))
            }
            Ok(metadata)
                if (index + 1 < count && !metadata.is_dir())
                    || (index + 1 == count && !metadata.is_file()) =>
            {
                return Err(io::Error::other(
                    "Destination has an incompatible file type",
                ))
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
#[cfg(test)]
mod tests {
    use super::*;
    fn temp() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "rsxp-saves-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&p).unwrap();
        p
    }
    #[test]
    fn restore_preserves_originals_and_unrelated_files() {
        let root = temp();
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
            .find(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("before-restore-")
            })
            .unwrap()
            .path();
        assert_eq!(fs::read(safety.join("nested/slot.sav")).unwrap(), b"old");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn refuses_overlapping_directories_and_existing_backups() {
        let root = temp();
        fs::write(root.join("slot.sav"), b"data").unwrap();
        assert!(copy(
            root.to_str().unwrap(),
            root.join("backup").to_str().unwrap(),
            false
        )
        .is_err());
        let target = temp();
        fs::write(target.join("existing"), b"keep").unwrap();
        assert!(copy(root.to_str().unwrap(), target.to_str().unwrap(), false).is_err());
        assert_eq!(fs::read(target.join("existing")).unwrap(), b"keep");
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(target).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn rejects_destination_symlinks_before_replacing_any_files() {
        let root = temp();
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
}
