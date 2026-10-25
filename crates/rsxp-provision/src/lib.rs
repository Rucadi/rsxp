//! Versioned software repositories and transactional, hash-verified installation.
pub mod backup;
pub mod catalog;
pub mod config;
pub mod folders;
pub mod ips;
pub mod settings;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;

pub fn root(game: Option<&Path>) -> PathBuf {
    if let Some(path) = std::env::var_os("RSXP_HOME") {
        let path = PathBuf::from(path);
        return if path.is_absolute() {
            path
        } else {
            std::env::current_dir().unwrap_or_default().join(path)
        };
    }
    if let Some(game) = game {
        let absolute = if game.is_absolute() {
            game.to_owned()
        } else {
            std::env::current_dir().unwrap_or_default().join(game)
        };
        let game = absolute.as_path();
        let dir = if game.is_dir() {
            game
        } else {
            game.parent().unwrap_or(Path::new("."))
        };
        // Installed games inherit the enclosing portable library.
        for parent in dir.ancestors() {
            if parent.file_name().is_some_and(|n| n == "RSXP") {
                return parent.to_owned();
            }
        }
        if dir.join("RSXP").is_dir() {
            return dir.join("RSXP");
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        let portable = exe.parent().unwrap().join("RSXP");
        if portable.is_dir() {
            return portable;
        }
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("RSXP")
}
pub fn initialize(root: &Path) -> Result<()> {
    for dir in [
        "games",
        "roms",
        "saves",
        "repositories",
        "repositories/cache",
        "cache",
        "config",
        "config/games",
        "backups",
    ] {
        fs::create_dir_all(root.join(dir))?;
    }
    Ok(())
}
/// Copy an old storage tree without following symlinks or replacing current saves.
pub fn migrate_data(from: &Path, to: &Path) -> Result<()> {
    fn copy(from: &Path, to: &Path, depth: usize) -> Result<()> {
        if depth > 64 {
            return Err("Save directory nesting exceeds limit".into());
        }
        if !from.is_dir() {
            return Ok(());
        }
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            let target = to.join(entry.file_name());
            if kind.is_dir() {
                copy(&entry.path(), &target, depth + 1)?;
            } else if kind.is_file() && !target.exists() {
                let mut output = fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(target)?;
                std::io::copy(&mut fs::File::open(entry.path())?, &mut output)?;
            }
        }
        Ok(())
    }
    copy(from, to, 0)
}
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn valid_hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit())
}
fn verify(bytes: &[u8], expected: &str) -> Result<()> {
    if !valid_hash(expected) || !hash(bytes).eq_ignore_ascii_case(expected) {
        return Err("SHA-256 mismatch".into());
    }
    Ok(())
}
fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}
fn relative(s: &str) -> bool {
    !s.is_empty()
        && !s.contains(['\\', ':'])
        && Path::new(s)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Repository {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub games: Vec<Game>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Game {
    pub id: String,
    pub title: String,
    pub version: String,
    pub platform: Platform,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub image: Option<Artwork>,
    #[serde(default)]
    pub config: settings::Settings,
    pub download: Download,
    /// Required relative Game.ini location for RPG Maker ZIPs.
    pub entrypoint: String,
    #[serde(default)]
    pub base: Option<Base>,
    /// Required for IPS: digest of the patched ROM.
    #[serde(default)]
    pub output_sha256: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artwork {
    pub url: String,
    pub sha256: String,
}
/// Content-addressed artwork cache, independently verified from game downloads.
pub fn cache_artwork(root: &Path, art: &Artwork) -> Result<PathBuf> {
    if !valid_hash(&art.sha256) {
        return Err("Invalid artwork SHA-256".into());
    }
    let path = root
        .join("cache/artwork")
        .join(art.sha256.to_ascii_lowercase());
    if fs::read(&path).is_ok_and(|bytes| verify(&bytes, &art.sha256).is_ok()) {
        return Ok(path);
    }
    let bytes = fetch(&art.url, 16 * 1024 * 1024)?;
    verify(&bytes, &art.sha256)?;
    fs::create_dir_all(path.parent().unwrap())?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(tmp, &path)?;
    Ok(path)
}
pub fn import_artwork(root: &Path, file: &Path) -> Result<String> {
    let bytes = fetch(&file.to_string_lossy(), 16 * 1024 * 1024)?;
    let digest = hash(&bytes);
    cache_artwork(
        root,
        &Artwork {
            url: file.to_string_lossy().into_owned(),
            sha256: digest.clone(),
        },
    )?;
    Ok(digest)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Platform {
    RpgMaker,
    Gba,
    Nds,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Download {
    pub url: String,
    pub sha256: String,
    pub format: Format,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Rom,
    Zip,
    Ips,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Base {
    pub title: String,
    pub sha256: String,
}
impl Repository {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 || !identifier(&self.id) {
            return Err("Unsupported schema or invalid repository id".into());
        }
        let mut ids = std::collections::HashSet::new();
        for g in &self.games {
            g.config.validate()?;
            if g.image.as_ref().is_some_and(|a| !valid_hash(&a.sha256)) {
                return Err("Invalid artwork SHA-256".into());
            }
            if !identifier(&g.id)
                || !ids.insert(&g.id)
                || g.title.trim().is_empty()
                || g.version.is_empty()
                || !relative(&g.entrypoint)
                || !valid_hash(&g.download.sha256)
            {
                return Err("Invalid or duplicate game metadata".into());
            }
            let ext = Path::new(&g.entrypoint)
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            let expected = match g.platform {
                Platform::RpgMaker => "ini",
                Platform::Gba => "gba",
                Platform::Nds => "nds",
            };
            if !ext.eq_ignore_ascii_case(expected) {
                return Err("Entrypoint does not match platform".into());
            }
            if g.platform == Platform::RpgMaker
                && (g.download.format != Format::Zip || !g.entrypoint.ends_with("Game.ini"))
            {
                return Err("RPG Maker requires a ZIP and Game.ini entrypoint".into());
            }
            if g.download.format == Format::Ips {
                if g.platform == Platform::RpgMaker
                    || !g.base.as_ref().is_some_and(|b| valid_hash(&b.sha256))
                    || !g.output_sha256.as_ref().is_some_and(|h| valid_hash(h))
                {
                    return Err("IPS requires a base SHA-256 and output SHA-256".into());
                }
            } else if g.base.is_some() {
                return Err("Base ROM only applies to IPS".into());
            }
        }
        Ok(())
    }
}
/// HTTPS transport uses curl with normal certificate validation, bounded size and timeout.
fn fetch(source: &str, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    if source.starts_with("https://") {
        let mut child = std::process::Command::new("curl")
            .args([
                "--fail",
                "--silent",
                "--show-error",
                "--location",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
                "--max-time",
                "300",
                "--max-filesize",
                &limit.to_string(),
                "--",
                source,
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("HTTPS downloads require curl: {e}"))?;
        let read = child
            .stdout
            .take()
            .unwrap()
            .take(limit + 1)
            .read_to_end(&mut bytes);
        if read.is_err() || bytes.len() as u64 > limit {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Download exceeds size limit or failed".into());
        }
        if !child.wait()?.success() {
            return Err(format!("Download failed: {source}").into());
        }
    } else {
        if source.contains("://") {
            return Err("Only HTTPS URLs or local file paths are supported".into());
        }
        fs::File::open(source)?
            .take(limit + 1)
            .read_to_end(&mut bytes)?;
    }
    if bytes.len() as u64 > limit {
        return Err("File exceeds size limit".into());
    }
    Ok(bytes)
}
pub fn load_repository(source: &str) -> Result<Repository> {
    let mut repo: Repository = serde_json::from_slice(&fetch(source, 8 * 1024 * 1024)?)?;
    repo.validate()?;
    for game in &mut repo.games {
        let mut urls = vec![&mut game.download.url];
        if let Some(art) = &mut game.image {
            urls.push(&mut art.url);
        }
        for url in urls {
            if !url.contains("://") {
                if source.starts_with("https://") {
                    if !relative(url) {
                        return Err(
                            "Remote repositories require relative or HTTPS downloads".into()
                        );
                    }
                    *url = format!("{}/{}", source.rsplit_once('/').unwrap().0, url);
                } else {
                    *url = Path::new(source)
                        .parent()
                        .unwrap_or(Path::new("."))
                        .join(&*url)
                        .to_string_lossy()
                        .into_owned();
                }
            }
        }
    }
    Ok(repo)
}
pub fn installed(root: &Path, repo: &Repository, game: &Game) -> Option<PathBuf> {
    let dir = root.join("games").join(&repo.id).join(&game.id);
    let receipt: Game =
        serde_json::from_slice(&fs::read(dir.join(".rsxp-install.json")).ok()?).ok()?;
    (receipt.version == game.version
        && receipt.download.sha256 == game.download.sha256
        && receipt.output_sha256 == game.output_sha256
        && receipt.entrypoint == game.entrypoint
        && dir.join(&game.entrypoint).is_file())
    .then(|| dir.join(&game.entrypoint))
}
pub fn import_base(root: &Path, path: &Path) -> Result<String> {
    initialize(root)?;
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !["gba", "nds"].contains(&ext.as_str()) {
        return Err("Choose a GBA or NDS ROM".into());
    }
    let bytes = fetch(&path.to_string_lossy(), MAX_DOWNLOAD)?;
    let digest = hash(&bytes);
    fs::write(root.join("roms").join(format!("{digest}.{ext}")), bytes)?;
    Ok(digest)
}
pub fn install(root: &Path, repo: &Repository, game: &Game) -> Result<PathBuf> {
    repo.validate()?;
    if !repo
        .games
        .iter()
        .any(|g| g.id == game.id && serde_json::to_vec(g).ok() == serde_json::to_vec(game).ok())
    {
        return Err("Game is not in repository".into());
    }
    initialize(root)?;
    let base =
        if let Some(base) = &game.base {
            let mut found = None;
            for entry in fs::read_dir(root.join("roms"))? {
                let path = entry?.path();
                if path.is_file()
                    && path.extension().is_some_and(|e| {
                        e.eq_ignore_ascii_case("gba") || e.eq_ignore_ascii_case("nds")
                    })
                {
                    let bytes = fetch(&path.to_string_lossy(), MAX_DOWNLOAD)?;
                    if hash(&bytes).eq_ignore_ascii_case(&base.sha256) {
                        found = Some(bytes);
                        break;
                    }
                }
            }
            Some(found.ok_or_else(|| {
                format!("Import base ROM: {} (SHA-256 {})", base.title, base.sha256)
            })?)
        } else {
            None
        };
    if let Some(path) = installed(root, repo, game) {
        if game.download.format != Format::Zip {
            let expected = game.output_sha256.as_ref().unwrap_or(&game.download.sha256);
            verify(&fetch(&path.to_string_lossy(), MAX_DOWNLOAD)?, expected)?;
        }
        return Ok(path);
    }
    let bytes = fetch(&game.download.url, MAX_DOWNLOAD)?;
    verify(&bytes, &game.download.sha256)?;
    let parent = root.join("games").join(&repo.id);
    fs::create_dir_all(&parent)?;
    let dest = parent.join(&game.id);
    // Never overwrite an existing installation: RPG Maker may save in its own directory.
    if dest.exists() {
        return Err("An older installation exists. Back up saves and move its folder before installing this version.".into());
    }
    let stage = parent.join(format!(".{}-staging", game.id));
    fs::create_dir(&stage)?;
    let result = (|| -> Result<PathBuf> {
        match game.download.format {
            Format::Rom | Format::Ips => {
                let output = if let Some(base) = base {
                    ips::apply(&base, &bytes)?
                } else {
                    bytes
                };
                if let Some(expected) = &game.output_sha256 {
                    verify(&output, expected)?;
                }
                let file = stage.join(&game.entrypoint);
                fs::create_dir_all(file.parent().unwrap())?;
                fs::write(file, output)?;
            }
            Format::Zip => {
                let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
                let mut total = 0u64;
                if zip.len() > 100_000 {
                    return Err("Too many archive entries".into());
                }
                for i in 0..zip.len() {
                    let mut file = zip.by_index(i)?;
                    if !relative(file.name().trim_end_matches('/'))
                        || file.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000)
                    {
                        return Err("Unsafe ZIP path or symlink".into());
                    }
                    total = total
                        .checked_add(file.size())
                        .ok_or("Archive size overflow")?;
                    if total > 2 * 1024 * 1024 * 1024 {
                        return Err("Expanded archive exceeds 2 GiB".into());
                    }
                    let path = stage.join(file.name());
                    if file.is_dir() {
                        fs::create_dir_all(path)?;
                    } else {
                        fs::create_dir_all(path.parent().unwrap())?;
                        let mut out = fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(path)?;
                        std::io::copy(&mut file, &mut out)?;
                        out.flush()?;
                    }
                }
            }
        }
        if !stage.join(&game.entrypoint).is_file() {
            return Err("Entrypoint missing from download".into());
        }
        fs::write(
            stage.join(".rsxp-install.json"),
            serde_json::to_vec_pretty(game)?,
        )?;
        fs::rename(&stage, &dest)?;
        Ok(dest.join(&game.entrypoint))
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn temp() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "rsxp-provision-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }
    fn repo(url: &Path, bytes: &[u8]) -> Repository {
        Repository {
            schema_version: 1,
            id: "homebrew".into(),
            name: "Homebrew".into(),
            games: vec![Game {
                id: "demo".into(),
                title: "Demo".into(),
                version: "1".into(),
                platform: Platform::Gba,
                description: String::new(),
                image: None,
                config: settings::Settings::default(),
                download: Download {
                    url: url.to_string_lossy().into_owned(),
                    sha256: hash(bytes),
                    format: Format::Rom,
                },
                entrypoint: "demo.gba".into(),
                base: None,
                output_sha256: None,
            }],
        }
    }
    #[test]
    fn migration_keeps_layout_and_current_saves() {
        let dir = temp();
        let old = dir.join("old");
        let new = dir.join("new");
        fs::create_dir_all(old.join("save")).unwrap();
        fs::create_dir_all(old.join("states")).unwrap();
        fs::write(old.join("save/game.sav"), b"old").unwrap();
        fs::write(old.join("states/1.bin"), b"state").unwrap();
        fs::create_dir_all(new.join("save")).unwrap();
        fs::write(new.join("save/game.sav"), b"current").unwrap();
        migrate_data(&old, &new).unwrap();
        assert_eq!(fs::read(new.join("save/game.sav")).unwrap(), b"current");
        assert_eq!(fs::read(new.join("states/1.bin")).unwrap(), b"state");
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn documented_catalog_is_valid() {
        let r: Repository =
            serde_json::from_str(include_str!("../../../docs/repositories/example.json")).unwrap();
        r.validate().unwrap();
        assert_eq!(r.games.len(), 5);
    }
    #[test]
    fn install_verified_rom_and_reject_corruption() {
        let dir = temp();
        let file = dir.join("source.gba");
        fs::write(&file, b"homebrew").unwrap();
        let r = repo(&file, b"homebrew");
        let root = dir.join("RSXP");
        let p = install(&root, &r, &r.games[0]).unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"homebrew");
        assert_eq!(install(&root, &r, &r.games[0]).unwrap(), p);
        let bad = repo(&file, b"wrong hash");
        assert!(install(&dir.join("other"), &bad, &bad.games[0]).is_err());
        assert!(!dir.join("other/games/homebrew/demo").exists());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn ips_requires_matching_base_and_output() {
        let dir = temp();
        let file = dir.join("patch.ips");
        let patch = b"PATCH\0\0\x01\0\x01XEOF";
        fs::write(&file, patch).unwrap();
        let mut r = repo(&file, patch);
        r.games[0].download.format = Format::Ips;
        r.games[0].base = Some(Base {
            title: "Original".into(),
            sha256: hash(b"abc"),
        });
        r.games[0].output_sha256 = Some(hash(b"aXc"));
        let root = dir.join("RSXP");
        assert!(install(&root, &r, &r.games[0]).is_err());
        fs::write(root.join("roms/wrong.gba"), b"wrong").unwrap();
        assert!(install(&root, &r, &r.games[0]).is_err());
        fs::write(root.join("roms/base.gba"), b"abc").unwrap();
        r.games[0].output_sha256 = Some(hash(b"bad"));
        assert!(install(&root, &r, &r.games[0]).is_err());
        assert!(!root.join("games/homebrew/demo").exists());
        r.games[0].output_sha256 = Some(hash(b"aXc"));
        let path = install(&root, &r, &r.games[0]).unwrap();
        assert_eq!(fs::read(path).unwrap(), b"aXc");
        assert_eq!(fs::read(root.join("roms/base.gba")).unwrap(), b"abc");
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn manifests_reject_escape_and_duplicate_ids() {
        let mut r = repo(Path::new("demo.gba"), b"demo");
        for entry in [
            "../demo.gba",
            "/demo.gba",
            "C:\\demo.gba",
            "a/../../demo.gba",
        ] {
            r.games[0].entrypoint = entry.into();
            assert!(r.validate().is_err());
        }
        r.games[0].entrypoint = "demo.gba".into();
        r.games.push(r.games[0].clone());
        assert!(r.validate().is_err());
    }
    #[test]
    fn zip_install_and_traversal_rejection() {
        let dir = temp();
        for name in ["Game.ini", "../escaped"] {
            let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"[Game]\nTitle=Demo").unwrap();
            let bytes = writer.finish().unwrap().into_inner();
            let file = dir.join("game.zip");
            fs::write(&file, &bytes).unwrap();
            let mut r = repo(&file, &bytes);
            r.games[0].platform = Platform::RpgMaker;
            r.games[0].download.format = Format::Zip;
            r.games[0].entrypoint = "Game.ini".into();
            let root = dir.join(if name == "Game.ini" { "good" } else { "bad" });
            assert_eq!(install(&root, &r, &r.games[0]).is_ok(), name == "Game.ini");
            assert!(!root.join("games/homebrew/escaped").exists());
        }
        fs::remove_dir_all(dir).unwrap();
    }
}
