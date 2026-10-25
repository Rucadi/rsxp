//! Typed per-game launch settings; no arbitrary commands or environment injection.
use crate::{Game, Result, hash, root};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::{Path, PathBuf}};
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub renderer: Option<String>,
    pub rgss: Option<u8>,
    pub jit: Option<String>,
    pub fps: Option<u32>,
    pub fast_forward: bool,
    pub patches: Vec<String>,
    pub patch_values: BTreeMap<String, f64>,
    /// Original action scancode -> replacement physical scancode.
    pub keyboard: BTreeMap<i32, i32>,
    pub gamepad: Option<String>,
    pub nds_renderer: Option<String>,
    pub gba_bios: Option<String>,
    pub nds_bios7: Option<String>,
    pub nds_bios9: Option<String>,
    pub nds_firmware: Option<String>,
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        for (value, allowed) in [(&self.renderer, &["auto", "software", "wgpu"][..]), (&self.jit, &["off", "yjit", "zjit"][..]), (&self.nds_renderer, &["software", "wgpu"][..])] {
            if value.as_ref().is_some_and(|v| !allowed.contains(&v.as_str())) { return Err("Invalid renderer or JIT setting".into()); }
        }
        if self.rgss.is_some_and(|n| !(1..=3).contains(&n)) || self.fps.is_some_and(|n| !(1..=240).contains(&n)) { return Err("RGSS must be 1–3; FPS must be 1–240".into()); }
        let valid_key = |n: i32| (4..=57).contains(&n) || (79..=82).contains(&n);
        if self.keyboard.iter().any(|(&k, &v)| !valid_key(k) || !valid_key(v)) { return Err("Invalid keyboard scancode".into()); }
        let mut physical = std::collections::HashSet::new();
        if self.keyboard.values().any(|v| !physical.insert(v)) { return Err("Each keyboard binding must use a different key".into()); }
        if let Some(value) = &self.gamepad {
            let (zone, buttons) = value.split_once(';').ok_or("Invalid gamepad mapping")?;
            let zone: f32 = zone.parse()?;
            let buttons: Vec<usize> = buttons.split(',').map(str::parse).collect::<std::result::Result<_, _>>()?;
            if !zone.is_finite() || !(0.1..=0.9).contains(&zone) || buttons.len() != 20 || buttons.iter().any(|n| *n >= 13) { return Err("Invalid gamepad mapping".into()); }
        }
        if self.patch_values.values().any(|v| !v.is_finite()) { return Err("Patch values must be finite".into()); }
        Ok(())
    }
    pub fn environment(&self) -> Vec<(&'static str, String)> {
        let mut vars = Vec::new();
        for (name, value) in [("RSXP_RENDERER", &self.renderer), ("RSXP_JIT", &self.jit), ("RSXP_NDS_RENDERER", &self.nds_renderer), ("RSXP_GBA_BIOS", &self.gba_bios), ("RSXP_NDS_BIOS7", &self.nds_bios7), ("RSXP_NDS_BIOS9", &self.nds_bios9), ("RSXP_NDS_FIRMWARE", &self.nds_firmware), ("RSXP_CONTROLS", &self.gamepad)] {
            if let Some(value) = value { vars.push((name, value.clone())); }
        }
        if !self.patches.is_empty() { vars.push(("RSXP_PATCH", self.patches.join(","))); }
        vars
    }
    pub fn translate_key(&self, physical: i32) -> Option<i32> {
        self.keyboard.iter().find_map(|(&action, &key)| (key == physical).then_some(action))
            .or_else(|| (!self.keyboard.contains_key(&physical)).then_some(physical))
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Preferences {
    /// None inherits repository defaults; Some is the complete user override.
    pub settings: Option<Settings>,
    pub cover_sha256: Option<String>,
}
pub fn key_for_path(path: &Path) -> (String, Settings) {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_owned());
    let dir = if path.is_dir() { path.as_path() } else { path.parent().unwrap_or(Path::new(".")) };
    for parent in dir.ancestors() {
        if let Ok(bytes) = fs::read(parent.join(".rsxp-install.json")) {
            if let Ok(game) = serde_json::from_slice::<Game>(&bytes) {
                let repo = parent.parent().and_then(Path::file_name).unwrap_or_default().to_string_lossy();
                return (format!("{repo}/{}", game.id), game.config);
            }
        }
    }
    let path = if path.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("Game.ini")) { dir } else { &path };
    (format!("local:{}", path.display()), Settings::default())
}
fn preferences_path(root: &Path, key: &str) -> PathBuf { root.join("config/games").join(format!("{}.json", hash(key.as_bytes()))) }
pub fn load(root: &Path, key: &str) -> Result<Preferences> {
    match fs::read(preferences_path(root, key)) {
        Ok(bytes) => { let p: Preferences = serde_json::from_slice(&bytes)?; if let Some(s) = &p.settings { s.validate()?; } Ok(p) },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Preferences::default()),
        Err(e) => Err(e.into()),
    }
}
pub fn save(root: &Path, key: &str, prefs: &Preferences) -> Result<()> {
    if let Some(s) = &prefs.settings { s.validate()?; }
    let path = preferences_path(root, key); fs::create_dir_all(path.parent().unwrap())?;
    let tmp = path.with_extension("tmp"); fs::write(&tmp, serde_json::to_vec_pretty(prefs)?)?; fs::rename(tmp, path)?; Ok(())
}
pub fn for_game(path: &Path) -> Result<Settings> {
    let (key, defaults) = key_for_path(path);
    let prefs = load(&root(Some(path)), &key)?;
    let settings = prefs.settings.unwrap_or(defaults); settings.validate()?; Ok(settings)
}
