// SPDX-License-Identifier: GPL-2.0-or-later
//! Content identity and frontend-owned save storage.
//!
//! Saves are keyed by core id and content identity, not by a human title, so
//! two games with the same title cannot share a save directory. Existing
//! title-based saves can be copied forward explicitly with
//! [`SaveStore::migrate_from`], which never deletes the old files.
//!
//! [`StorageWorker`] moves writes off the core thread: the producer only
//! enqueues a bounded request, the worker performs atomic replacement, and
//! [`StorageWorker::flush`] waits for every earlier write to finish.

use fanplay_core::{
    content_hash, Content, ContentKind, StateEnvelope, StateError, MAX_STATE_PAYLOAD,
};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};

/// Stable identity of the bytes a state belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentId {
    pub hash: u64,
}

impl ContentId {
    /// Hashes already-loaded content without performing filesystem I/O.
    pub fn of_bytes(bytes: &[u8]) -> Self {
        Self {
            hash: content_hash(bytes),
        }
    }

    pub fn of(content: &Content) -> io::Result<Self> {
        match content.kind {
            ContentKind::File => {
                let bytes = fs::read(&content.path)?;
                Ok(Self {
                    hash: content_hash(&bytes),
                })
            }
            ContentKind::Directory => Self::of_directory(&content.path),
        }
    }

    pub fn of_file(path: &Path) -> io::Result<Self> {
        Ok(Self {
            hash: content_hash(&fs::read(path)?),
        })
    }

    /// Hashes a directory tree deterministically (sorted relative paths plus
    /// file contents). Symlinks are rejected rather than followed.
    pub fn of_directory(path: &Path) -> io::Result<Self> {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        hash_tree(path, path, &mut hash, 0)?;
        Ok(Self { hash })
    }

    pub fn hex(self) -> String {
        format!("{:016x}", self.hash)
    }
}

fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash = (*hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
    }
}

fn hash_tree(root: &Path, path: &Path, hash: &mut u64, depth: usize) -> io::Result<()> {
    if depth > 64 {
        return Err(io::Error::other("content directory nesting exceeds limit"));
    }
    let mut entries: Vec<_> = fs::read_dir(path)?.filter_map(Result::ok).collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            return Err(io::Error::other("content directory contains a symlink"));
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(io::Error::other)?
            .to_string_lossy()
            .replace('\\', "/");
        hash_bytes(hash, relative.as_bytes());
        hash_bytes(hash, &[0]);
        if kind.is_dir() {
            hash_tree(root, &entry.path(), hash, depth + 1)?;
        } else if kind.is_file() {
            let bytes = fs::read(entry.path())?;
            hash_bytes(hash, &(bytes.len() as u64).to_le_bytes());
            hash_bytes(hash, &bytes);
        }
    }
    Ok(())
}

/// Failure from the storage layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StorageError {
    Io(String),
    InvalidKey(String),
    QueueFull,
    Closed,
    InvalidState(StateError),
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(message) => f.write_str(message),
            Self::InvalidKey(key) => write!(f, "invalid storage key {key:?}"),
            Self::QueueFull => f.write_str("save storage queue is full"),
            Self::Closed => f.write_str("save storage worker is closed"),
            Self::InvalidState(error) => write!(f, "invalid save state: {error}"),
        }
    }
}

impl Error for StorageError {}

impl From<io::Error> for StorageError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

impl From<StateError> for StorageError {
    fn from(error: StateError) -> Self {
        Self::InvalidState(error)
    }
}

/// Byte-oriented store. Keys are relative, slash-separated paths.
pub trait Storage: Send + Sync {
    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError>;
    fn write(&self, key: &str, bytes: &[u8]) -> Result<(), StorageError>;
    fn list(&self) -> Result<Vec<String>, StorageError>;
}

/// Native filesystem implementation with atomic replacement.
pub struct FsStorage {
    root: PathBuf,
}

impl FsStorage {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

fn checked_path(root: &Path, key: &str) -> Result<PathBuf, StorageError> {
    let path = Path::new(key);
    if key.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(StorageError::InvalidKey(key.to_owned()));
    }
    Ok(root.join(path))
}

impl Storage for FsStorage {
    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
        let path = checked_path(&self.root, key)?;
        match fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn write(&self, key: &str, bytes: &[u8]) -> Result<(), StorageError> {
        let path = checked_path(&self.root, key)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let nonce = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("save");
        let temp = path.with_file_name(format!(".{name}.tmp-{nonce}"));
        let result = (|| -> io::Result<()> {
            fs::write(&temp, bytes)?;
            // Make the payload durable before it becomes visible under its
            // final name.
            fs::OpenOptions::new().write(true).open(&temp)?.sync_all()?;
            match fs::rename(&temp, &path) {
                Ok(()) => Ok(()),
                // Windows does not replace an existing destination with
                // `rename`; remove it and retry. A crash between the two steps
                // leaves the previous file absent, so the caller still has the
                // acknowledged worker result to report.
                Err(error)
                    if cfg!(windows)
                        && matches!(
                            error.kind(),
                            io::ErrorKind::AlreadyExists | io::ErrorKind::PermissionDenied
                        ) =>
                {
                    fs::remove_file(&path)?;
                    fs::rename(&temp, &path)
                }
                Err(error) => Err(error),
            }
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result.map_err(StorageError::from)
    }

    fn list(&self) -> Result<Vec<String>, StorageError> {
        let mut out = Vec::new();
        collect_keys(&self.root, &self.root, &mut out, 0)?;
        out.sort();
        Ok(out)
    }
}

fn collect_keys(
    root: &Path,
    path: &Path,
    out: &mut Vec<String>,
    depth: usize,
) -> Result<(), StorageError> {
    if depth > 64 {
        return Err(StorageError::Io(
            "save directory nesting exceeds limit".into(),
        ));
    }
    if !path.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect_keys(root, &entry.path(), out, depth + 1)?;
        } else if kind.is_file() {
            if let Ok(relative) = entry.path().strip_prefix(root) {
                out.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    Ok(())
}

/// In-memory implementation for contract tests and non-native hosts.
#[derive(Default)]
pub struct MemoryStorage {
    files: Mutex<BTreeMap<String, Vec<u8>>>,
}

impl MemoryStorage {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Storage for MemoryStorage {
    fn read(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
        checked_path(Path::new("."), key)?;
        Ok(self.files.lock().unwrap().get(key).cloned())
    }

    fn write(&self, key: &str, bytes: &[u8]) -> Result<(), StorageError> {
        checked_path(Path::new("."), key)?;
        self.files
            .lock()
            .unwrap()
            .insert(key.to_owned(), bytes.to_vec());
        Ok(())
    }

    fn list(&self) -> Result<Vec<String>, StorageError> {
        Ok(self.files.lock().unwrap().keys().cloned().collect())
    }
}

/// One content-scoped save area.
pub struct SaveStore {
    core_id: String,
    content: ContentId,
    storage: Arc<dyn Storage>,
    native_root: Option<PathBuf>,
}

/// Metadata shown for a saved emulator snapshot. The payload itself remains
/// in the versioned `.rsxpstate` file; this small sidecar keeps listing slots
/// cheap and lets frontends show when a snapshot was created.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateSnapshot {
    pub slot: String,
    pub saved_at: Option<u64>,
    pub bytes: usize,
}

impl SaveStore {
    /// Opens the native save area `<base>/<core>/<content-hash>` and creates it.
    pub fn new(
        base: impl AsRef<Path>,
        core_id: impl Into<String>,
        content: ContentId,
    ) -> io::Result<Self> {
        let core_id = core_id.into();
        let root = base
            .as_ref()
            .join(safe_component(&core_id))
            .join(content.hex());
        fs::create_dir_all(root.join("save"))?;
        fs::create_dir_all(root.join("states"))?;
        Ok(Self {
            core_id,
            content,
            storage: Arc::new(FsStorage::new(root.clone())),
            native_root: Some(root),
        })
    }

    /// Opens a store over an arbitrary backend (used by in-memory tests).
    pub fn with_storage(
        core_id: impl Into<String>,
        content: ContentId,
        storage: Arc<dyn Storage>,
    ) -> Self {
        Self {
            core_id: core_id.into(),
            content,
            storage,
            native_root: None,
        }
    }

    /// Native directory for backends that still write a file directly. Always
    /// inside the content-scoped store, never the shared title directory.
    pub fn native_save_dir(&self) -> Option<PathBuf> {
        let path = self.native_root.as_ref()?.join("save");
        let _ = fs::create_dir_all(&path);
        Some(path)
    }

    pub fn core_id(&self) -> &str {
        &self.core_id
    }

    pub fn content_id(&self) -> ContentId {
        self.content
    }

    pub fn storage(&self) -> &Arc<dyn Storage> {
        &self.storage
    }

    // Keep the historical extension so existing state slots remain discoverable.
    pub fn state_key(slot: &str) -> String {
        format!("states/{}.rsxpstate", safe_component(slot))
    }

    pub fn state_metadata_key(slot: &str) -> String {
        format!("states/{}.json", safe_component(slot))
    }

    pub fn state_metadata(saved_at: u64, bytes: usize) -> Vec<u8> {
        format!("{saved_at}\n{bytes}").into_bytes()
    }

    pub fn read_state_metadata(&self, slot: &str) -> Result<Option<(u64, usize)>, StorageError> {
        let Some(bytes) = self.storage.read(&Self::state_metadata_key(slot))? else {
            return Ok(None);
        };
        let text = String::from_utf8_lossy(&bytes);
        let mut fields = text.lines();
        let Some(saved_at) = fields.next().and_then(|value| value.parse().ok()) else {
            return Ok(None);
        };
        let Some(bytes) = fields.next().and_then(|value| value.parse().ok()) else {
            return Ok(None);
        };
        Ok(Some((saved_at, bytes)))
    }

    pub fn save_key(name: &str) -> String {
        format!("save/{}", safe_component(name))
    }

    /// Writes an already-encoded state blob atomically.
    pub fn write_state(&self, slot: &str, data: &[u8]) -> Result<(), StorageError> {
        if data.len() > MAX_STATE_PAYLOAD + 64 {
            return Err(StorageError::InvalidState(StateError::TooLarge));
        }
        self.storage.write(&Self::state_key(slot), data)
    }

    /// Reads a state and verifies the core id, revision and content identity.
    pub fn read_state(
        &self,
        slot: &str,
        revision: u32,
    ) -> Result<Option<StateEnvelope>, StorageError> {
        let Some(bytes) = self.storage.read(&Self::state_key(slot))? else {
            return Ok(None);
        };
        StateEnvelope::decode_for(&bytes, &self.core_id, revision, self.content.hash)
            .map(Some)
            .map_err(StorageError::from)
    }

    pub fn list_states(&self) -> Result<Vec<String>, StorageError> {
        let prefix = "states/";
        let suffix = ".rsxpstate";
        Ok(self
            .storage
            .list()?
            .into_iter()
            .filter_map(|key| {
                key.strip_prefix(prefix)
                    .and_then(|key| key.strip_suffix(suffix))
                    .map(str::to_owned)
            })
            .collect())
    }

    pub fn list_state_snapshots(&self) -> Result<Vec<StateSnapshot>, StorageError> {
        let mut snapshots = self
            .list_states()?
            .into_iter()
            .map(|slot| {
                let metadata = self.read_state_metadata(&slot).ok().flatten();
                let (saved_at, mut bytes) =
                    metadata.map_or((None, 0), |(saved_at, bytes)| (Some(saved_at), bytes));
                if bytes == 0 {
                    bytes = self
                        .storage
                        .read(&Self::state_key(&slot))
                        .ok()
                        .flatten()
                        .map_or(0, |data| data.len());
                }
                StateSnapshot {
                    slot,
                    saved_at,
                    bytes,
                }
            })
            .collect::<Vec<_>>();
        snapshots.sort_by(|a, b| a.slot.cmp(&b.slot));
        Ok(snapshots)
    }

    pub fn write_save(&self, name: &str, bytes: &[u8]) -> Result<(), StorageError> {
        self.storage.write(&Self::save_key(name), bytes)
    }

    pub fn read_save(&self, name: &str) -> Result<Option<Vec<u8>>, StorageError> {
        self.storage.read(&Self::save_key(name))
    }

    /// Copies an existing title-based directory into this content-scoped area.
    ///
    /// The source is never modified or removed. Existing files in the
    /// destination are left alone so a migration can be retried.
    pub fn migrate_from(&self, legacy: &Path) -> Result<usize, StorageError> {
        if !legacy.is_dir() {
            return Ok(0);
        }
        let mut copied = 0;
        copy_missing(legacy, legacy, self.storage.as_ref(), &mut copied, 0)?;
        Ok(copied)
    }

    /// Copies the files that belonged to one title from a shared legacy
    /// directory: `<stem>.sav` becomes a live save and `<stem>.ss<slot>` files
    /// are preserved as legacy state files. Other titles are left alone.
    pub fn migrate_named_from(&self, legacy: &Path, stem: &str) -> Result<usize, StorageError> {
        if !legacy.is_dir() {
            return Ok(0);
        }
        let save_name = format!("{stem}.sav");
        let state_prefix = format!("{stem}.ss");
        let mut copied = 0;
        let mut entries: Vec<_> = fs::read_dir(legacy)?.filter_map(Result::ok).collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let kind = entry.file_type()?;
            if !kind.is_file() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let key = if name == save_name {
                format!("save/{name}")
            } else if name.starts_with(&state_prefix) {
                format!("legacy/{name}")
            } else {
                continue;
            };
            if self.storage.read(&key)?.is_none() {
                self.storage.write(&key, &fs::read(entry.path())?)?;
                copied += 1;
            }
        }
        Ok(copied)
    }
}

fn copy_missing(
    root: &Path,
    path: &Path,
    storage: &dyn Storage,
    copied: &mut usize,
    depth: usize,
) -> Result<(), StorageError> {
    if depth > 64 {
        return Err(StorageError::Io(
            "legacy save directory nesting exceeds limit".into(),
        ));
    }
    let mut entries: Vec<_> = fs::read_dir(path)?.filter_map(Result::ok).collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            copy_missing(root, &entry.path(), storage, copied, depth + 1)?;
        } else if kind.is_file() {
            let relative = entry
                .path()
                .strip_prefix(root)
                .map_err(io::Error::other)?
                .to_owned();
            let relative_key = relative.to_string_lossy().replace('\\', "/");
            // Battery saves become live saves in the new store. Save states
            // and unknown files are preserved under `legacy/` so nothing is
            // silently discarded.
            let is_save = relative
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("sav"));
            let key = if is_save {
                format!("save/{relative_key}")
            } else {
                format!("legacy/{relative_key}")
            };
            if storage.read(&key)?.is_none() {
                storage.write(&key, &fs::read(entry.path())?)?;
                *copied += 1;
            }
        }
    }
    Ok(())
}

fn safe_component(value: &str) -> String {
    let mut out: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() || out == "." || out == ".." {
        out.insert_str(0, "item_");
    }
    out
}

/// Acknowledged counters shared with the producer.
#[derive(Default)]
pub struct WorkerStatus {
    writes: AtomicU64,
    failures: AtomicU64,
    last_error: Mutex<Option<String>>,
    pending: AtomicU64,
}

impl WorkerStatus {
    pub fn writes(&self) -> u64 {
        self.writes.load(Ordering::Relaxed)
    }
    pub fn failures(&self) -> u64 {
        self.failures.load(Ordering::Relaxed)
    }
    pub fn pending(&self) -> u64 {
        self.pending.load(Ordering::Relaxed)
    }
    pub fn last_error(&self) -> Option<String> {
        self.last_error.lock().unwrap().clone()
    }
    fn record_failure(&self, error: &StorageError) {
        self.failures.fetch_add(1, Ordering::Relaxed);
        *self.last_error.lock().unwrap() = Some(error.to_string());
    }
}

enum Request {
    Write {
        key: String,
        bytes: Vec<u8>,
    },
    Flush {
        ack: SyncSender<Result<(), StorageError>>,
    },
    Shutdown,
}

/// Cloneable producer handle used by event sinks and other core-thread callers.
#[derive(Clone)]
pub struct StorageSubmitter {
    sender: SyncSender<Request>,
    status: Arc<WorkerStatus>,
}

impl StorageSubmitter {
    pub fn submit(&self, key: String, bytes: Vec<u8>) -> Result<(), StorageError> {
        match self.sender.try_send(Request::Write { key, bytes }) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(StorageError::QueueFull),
            Err(TrySendError::Disconnected(_)) => Err(StorageError::Closed),
        }
    }

    pub fn status(&self) -> &Arc<WorkerStatus> {
        &self.status
    }
}

/// Bounded writer thread. Cloning the storage handle is cheap; the worker owns
/// the only mutable access needed by atomic replace.
pub struct StorageWorker {
    sender: SyncSender<Request>,
    join: Option<std::thread::JoinHandle<()>>,
    status: Arc<WorkerStatus>,
}

impl StorageWorker {
    pub fn spawn(storage: Arc<dyn Storage>, capacity: usize) -> Self {
        let (sender, receiver) = mpsc::sync_channel::<Request>(capacity.max(1));
        let status = Arc::new(WorkerStatus::default());
        let worker_status = Arc::clone(&status);
        let join = std::thread::Builder::new()
            .name("fanplay-save-storage".into())
            .spawn(move || {
                while let Ok(request) = receiver.recv() {
                    match request {
                        Request::Write { key, bytes } => {
                            worker_status.pending.fetch_add(1, Ordering::Relaxed);
                            let result = storage.write(&key, &bytes);
                            worker_status.pending.fetch_sub(1, Ordering::Relaxed);
                            match result {
                                Ok(()) => {
                                    worker_status.writes.fetch_add(1, Ordering::Relaxed);
                                }
                                Err(error) => worker_status.record_failure(&error),
                            }
                        }
                        Request::Flush { ack } => {
                            let _ = ack.send(Ok(()));
                        }
                        Request::Shutdown => break,
                    }
                }
            })
            .expect("could not start save storage thread");
        Self {
            sender,
            join: Some(join),
            status,
        }
    }

    pub fn status(&self) -> &Arc<WorkerStatus> {
        &self.status
    }

    /// Returns a cloneable producer for core threads and event sinks.
    pub fn submitter(&self) -> StorageSubmitter {
        StorageSubmitter {
            sender: self.sender.clone(),
            status: Arc::clone(&self.status),
        }
    }

    /// Enqueues a write without blocking. The worker reports failures through
    /// [`WorkerStatus`]; use [`flush`](Self::flush) to observe ordering.
    pub fn submit(&self, key: String, bytes: Vec<u8>) -> Result<(), StorageError> {
        match self.sender.try_send(Request::Write { key, bytes }) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(StorageError::QueueFull),
            Err(TrySendError::Disconnected(_)) => Err(StorageError::Closed),
        }
    }

    /// Waits until every previously submitted write has been processed.
    pub fn flush(&self) -> Result<(), StorageError> {
        let (ack, receiver) = mpsc::sync_channel(1);
        self.sender
            .send(Request::Flush { ack })
            .map_err(|_| StorageError::Closed)?;
        receiver.recv().map_err(|_| StorageError::Closed)?
    }

    /// Stops the worker after draining queued requests.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        let _ = self.sender.send(Request::Shutdown);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Drop for StorageWorker {
    fn drop(&mut self) {
        // Preserve pending saves while releasing the producer.
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanplay_core::StateEnvelope;

    fn temp(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "fanplay-storage-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn state(store: &SaveStore) -> StateEnvelope {
        StateEnvelope::new(store.core_id(), 1, store.content_id().hash, vec![1, 2, 3])
    }

    #[test]
    fn same_title_content_does_not_collide() {
        let root = temp("identity");
        let a = SaveStore::new(&root, "gba", ContentId { hash: 1 }).unwrap();
        let b = SaveStore::new(&root, "gba", ContentId { hash: 2 }).unwrap();
        a.write_save("game.sav", b"a").unwrap();
        b.write_save("game.sav", b"b").unwrap();
        assert_eq!(a.read_save("game.sav").unwrap().unwrap(), b"a");
        assert_eq!(b.read_save("game.sav").unwrap().unwrap(), b"b");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn state_identity_and_corruption_are_rejected_before_restore() {
        let storage = Arc::new(MemoryStorage::new());
        let store = SaveStore::with_storage("gba", ContentId { hash: 7 }, storage);
        let encoded = state(&store).encode().unwrap();
        store.write_state("1", &encoded).unwrap();
        assert_eq!(
            store.read_state("1", 1).unwrap().unwrap().payload,
            vec![1, 2, 3]
        );
        assert!(matches!(
            store.read_state("1", 2),
            Err(StorageError::InvalidState(StateError::WrongRevision { .. }))
        ));

        let other =
            SaveStore::with_storage("gba", ContentId { hash: 8 }, Arc::new(MemoryStorage::new()));
        other.write_state("1", &encoded).unwrap();
        assert!(matches!(
            other.read_state("1", 1),
            Err(StorageError::InvalidState(StateError::WrongContent { .. }))
        ));

        let mut corrupt = encoded.clone();
        corrupt[36] ^= 1;
        store.write_state("1", &corrupt).unwrap();
        assert!(matches!(
            store.read_state("1", 1),
            Err(StorageError::InvalidState(StateError::Checksum))
        ));
    }

    #[test]
    fn migration_copies_without_touching_the_legacy_directory() {
        let root = temp("migrate");
        let legacy = root.join("Old Title");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("game.sav"), b"old").unwrap();
        let store = SaveStore::new(&root, "gba", ContentId { hash: 9 }).unwrap();
        assert_eq!(store.migrate_from(&legacy).unwrap(), 1);
        assert_eq!(
            store.storage().read("save/game.sav").unwrap().unwrap(),
            b"old"
        );
        assert_eq!(fs::read(legacy.join("game.sav")).unwrap(), b"old");
        // Retrying is idempotent.
        assert_eq!(store.migrate_from(&legacy).unwrap(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn named_migration_does_not_copy_other_titles() {
        let root = temp("named-migrate");
        let legacy = root.join("shared");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("mine.sav"), b"mine").unwrap();
        fs::write(legacy.join("mine.ss1"), b"state").unwrap();
        fs::write(legacy.join("other.sav"), b"other").unwrap();
        let store = SaveStore::new(&root, "gba", ContentId { hash: 10 }).unwrap();
        assert_eq!(store.migrate_named_from(&legacy, "mine").unwrap(), 2);
        assert_eq!(store.read_save("mine.sav").unwrap().unwrap(), b"mine");
        assert_eq!(
            store.storage().read("legacy/mine.ss1").unwrap().unwrap(),
            b"state"
        );
        assert!(store.read_save("other.sav").unwrap().is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn worker_acknowledges_ordered_writes_and_reports_failures() {
        let storage = Arc::new(MemoryStorage::new());
        let worker = StorageWorker::spawn(storage.clone(), 4);
        worker.submit("a".into(), b"one".to_vec()).unwrap();
        worker.submit("b".into(), b"two".to_vec()).unwrap();
        worker.flush().unwrap();
        assert_eq!(storage.read("a").unwrap().unwrap(), b"one");
        assert_eq!(storage.read("b").unwrap().unwrap(), b"two");
        assert_eq!(worker.status().writes(), 2);
        assert_eq!(worker.status().failures(), 0);
        worker.shutdown();
    }

    #[test]
    fn keys_cannot_escape_the_store() {
        let storage = MemoryStorage::new();
        assert!(matches!(
            storage.write("../escape", b"x"),
            Err(StorageError::InvalidKey(_))
        ));
        assert!(matches!(
            storage.write("/absolute", b"x"),
            Err(StorageError::InvalidKey(_))
        ));
    }
}
