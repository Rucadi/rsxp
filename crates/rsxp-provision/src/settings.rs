//! Typed per-game launch settings; no arbitrary commands or environment injection.
use crate::{hash, root, Game, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
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
        for (value, allowed) in [
            (&self.renderer, &["auto", "software", "wgpu"][..]),
            (&self.jit, &["off", "yjit", "zjit"][..]),
            (&self.nds_renderer, &["software", "wgpu"][..]),
        ] {
            if value
                .as_ref()
                .is_some_and(|v| !allowed.contains(&v.as_str()))
            {
                return Err("Invalid renderer or JIT setting".into());
            }
        }
        if self.rgss.is_some_and(|n| !(1..=3).contains(&n))
            || self.fps.is_some_and(|n| !(1..=240).contains(&n))
        {
            return Err("RGSS must be 1–3; FPS must be 1–240".into());
        }
        let valid_key = |n: i32| (4..=57).contains(&n) || (79..=82).contains(&n);
        if self
            .keyboard
            .iter()
            .any(|(&k, &v)| !valid_key(k) || !valid_key(v))
        {
            return Err("Invalid keyboard scancode".into());
        }
        let mut physical = std::collections::HashSet::new();
        if self.keyboard.values().any(|v| !physical.insert(v)) {
            return Err("Each keyboard binding must use a different key".into());
        }
        if let Some(value) = &self.gamepad {
            let (zone, buttons) = value.split_once(';').ok_or("Invalid gamepad mapping")?;
            let zone: f32 = zone.parse()?;
            let buttons: Vec<usize> = buttons
                .split(',')
                .map(str::parse)
                .collect::<std::result::Result<_, _>>()?;
            if !zone.is_finite()
                || !(0.1..=0.9).contains(&zone)
                || buttons.len() != 20
                || buttons.iter().any(|n| *n >= 13)
            {
                return Err("Invalid gamepad mapping".into());
            }
        }
        if self.patch_values.values().any(|v| !v.is_finite()) {
            return Err("Patch values must be finite".into());
        }
        Ok(())
    }
    pub fn environment(&self) -> Vec<(&'static str, String)> {
        let mut vars = Vec::new();
        for (name, value) in [
            ("RSXP_RENDERER", &self.renderer),
            ("RSXP_JIT", &self.jit),
            ("RSXP_NDS_RENDERER", &self.nds_renderer),
            ("RSXP_GBA_BIOS", &self.gba_bios),
            ("RSXP_NDS_BIOS7", &self.nds_bios7),
            ("RSXP_NDS_BIOS9", &self.nds_bios9),
            ("RSXP_NDS_FIRMWARE", &self.nds_firmware),
            ("RSXP_CONTROLS", &self.gamepad),
        ] {
            if let Some(value) = value {
                vars.push((name, value.clone()));
            }
        }
        if !self.patches.is_empty() {
            vars.push(("RSXP_PATCH", self.patches.join(",")));
        }
        vars
    }
    pub fn translate_key(&self, physical: i32) -> Option<i32> {
        self.keyboard
            .iter()
            .find_map(|(&action, &key)| (key == physical).then_some(action))
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
    let dir = if path.is_dir() {
        path.as_path()
    } else {
        path.parent().unwrap_or(Path::new("."))
    };
    for parent in dir.ancestors() {
        if let Ok(bytes) = fs::read(parent.join(".rsxp-install.json"))
            && let Ok(game) = serde_json::from_slice::<Game>(&bytes)
        {
            let repo = parent
                .parent()
                .and_then(Path::file_name)
                .unwrap_or_default()
                .to_string_lossy();
            return (format!("{repo}/{}", game.id), game.config);
        }
    }
    let path = if path
        .file_name()
        .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("Game.ini"))
    {
        dir
    } else {
        &path
    };
    (format!("local:{}", path.display()), Settings::default())
}

/// A stable, human-readable key for a local path (used by the control API).
pub fn key_for_path_only(path: &Path) -> String {
    key_for_path(path).0
}

/// The installation-wide settings shared by every game.
pub fn common(root: &Path) -> Result<Settings> {
    let settings = crate::config::load(root)?.settings;
    settings.validate()?;
    Ok(settings)
}

pub fn save_common(root: &Path, settings: &Settings) -> Result<()> {
    settings.validate()?;
    let mut config = crate::config::load(root)?;
    config.settings = settings.clone();
    crate::config::save(root, &config)
}

/// Merges a later settings layer over an earlier one.
///
/// Option fields in the overlay win when present. Patches and keyboard/gamepad
/// bindings are additive so common patches can stay enabled across games.
/// `fast_forward` is a positive flag: a common override of `true` cannot be
/// disabled per game, which matches how the workspace's uncapped toggle works.
pub fn merge(mut base: Settings, overlay: Settings) -> Settings {
    macro_rules! prefer {
        ($($field:ident),+ $(,)?) => {
            $(
                if overlay.$field.is_some() {
                    base.$field = overlay.$field.clone();
                }
            )+
        };
    }
    prefer!(
        renderer,
        rgss,
        jit,
        fps,
        nds_renderer,
        gba_bios,
        nds_bios7,
        nds_bios9,
        nds_firmware,
        gamepad,
    );
    if overlay.fast_forward {
        base.fast_forward = true;
    }
    for patch in overlay.patches {
        if !base.patches.contains(&patch) {
            base.patches.push(patch);
        }
    }
    base.patch_values.extend(overlay.patch_values);
    base.keyboard.extend(overlay.keyboard);
    base
}

/// Common settings followed by an optional per-game override.
pub fn effective(root: &Path, key: &str, defaults: Settings) -> Result<Settings> {
    let config = crate::config::load(root)?;
    let prefs = load(root, key)?;
    let mut settings = merge(defaults, config.settings);
    if let Some(overrides) = prefs.settings {
        settings = merge(settings, overrides);
    }
    settings.validate()?;
    Ok(settings)
}

/// Reads the per-game preferences without applying defaults.
pub fn preferences(root: &Path, key: &str) -> Result<Preferences> {
    load(root, key)
}

fn preferences_path(root: &Path, key: &str) -> PathBuf {
    root.join("config/games")
        .join(format!("{}.json", hash(key.as_bytes())))
}
pub fn load(root: &Path, key: &str) -> Result<Preferences> {
    if let Ok(config) = crate::config::load(root)
        && let Some(preferences) = config.games.get(key)
    {
        return Ok(preferences.clone());
    }
    match fs::read(preferences_path(root, key)) {
        Ok(bytes) => {
            let p: Preferences = serde_json::from_slice(&bytes)?;
            if let Some(s) = &p.settings {
                s.validate()?;
            }
            Ok(p)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Preferences::default()),
        Err(e) => Err(e.into()),
    }
}
pub fn save(root: &Path, key: &str, prefs: &Preferences) -> Result<()> {
    if let Some(s) = &prefs.settings {
        s.validate()?;
    }
    let mut config = crate::config::load(root)?;
    config.games.insert(key.to_owned(), prefs.clone());
    crate::config::save(root, &config)
}
pub fn for_game(path: &Path) -> Result<Settings> {
    let (key, defaults) = key_for_path(path);
    effective(&root(Some(path)), &key, defaults)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "rsxp-settings-{label}-{}-{}",
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
    fn common_settings_override_repository_defaults() {
        let root = temp("merge");
        let defaults = Settings {
            renderer: Some("software".into()),
            fps: Some(40),
            ..Default::default()
        };
        let mut common = Settings {
            renderer: Some("wgpu".into()),
            fast_forward: true,
            patches: vec!["user/common".into()],
            ..Default::default()
        };
        save_common(&root, &common).unwrap();
        let effective = effective(&root, "local:/game", defaults).unwrap();
        assert_eq!(effective.renderer.as_deref(), Some("wgpu"));
        assert_eq!(effective.fps, Some(40));
        assert!(effective.fast_forward);
        assert!(effective.patches.contains(&"user/common".to_string()));

        common.fps = Some(60);
        save_common(&root, &common).unwrap();
        let mut preferences = Preferences {
            settings: Some(Settings {
                fps: Some(30),
                patches: vec!["user/game".into()],
                ..Default::default()
            }),
            ..Default::default()
        };
        save(&root, "local:/game", &preferences).unwrap();
        let effective = for_game_key(&root, "local:/game", Settings::default());
        assert_eq!(effective.fps, Some(30));
        assert!(effective.patches.contains(&"user/common".to_string()));
        assert!(effective.patches.contains(&"user/game".to_string()));
        let stored = fs::read_to_string(crate::config::path(&root)).unwrap();
        assert!(stored.contains("\"local:/game\""), "{stored}");
        assert!(stored.contains("\"fps\": 30"), "{stored}");
        preferences.settings = None;
        save(&root, "local:/game", &preferences).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    fn for_game_key(root: &Path, key: &str, defaults: Settings) -> Settings {
        effective(root, key, defaults).unwrap()
    }
}
