// SPDX-License-Identifier: GPL-2.0-or-later
//! Runtime patches: small self-contained Ruby programs the workspace UI can
//! turn on while a game is running.
//!
//! The engine owns the catalog, the enabled set and (for patches that expose
//! one) a numeric parameter such as a zoom level. A host UI toggles patches and
//! drives the parameter (the desktop egui panel and the browser page both do);
//! the Ruby shim polls the state each frame (`RuntimePatches` in `rgss.rb`),
//! fetches a patch's source the first time it is enabled, evaluates it and
//! calls its `apply`/`revert`/`value` procs. Because the source only runs when
//! enabled, a patch can touch game internals without affecting games that never
//! enable it.
//!
//! ## State
//! The catalog, enabled set and parameter values all live in thread-locals, so
//! they follow the game thread. The built-in catalog is compiled in; a game may
//! also drop Ruby files into its `patches/` folder ([`load_user_patches`]), and
//! [`patches`] reports built-ins followed by those user patches. [`reset`]
//! clears all state between runs.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

/// A tunable numeric parameter shown as a slider next to a patch's toggle.
#[derive(Clone)]
pub struct PatchParameter {
    pub label: &'static str,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub default: f64,
    pub suffix: &'static str,
}

/// One available patch. `source` is a Ruby program that evaluates to a hash of
/// `apply`/`revert`/`value` procs; see `patches/map_panoramic.rb`.
#[derive(Clone)]
pub struct RuntimePatch {
    pub id: Cow<'static, str>,
    pub name: Cow<'static, str>,
    pub description: Cow<'static, str>,
    pub source: Cow<'static, str>,
    pub parameter: Option<PatchParameter>,
}

/// The catalog of built-in patches. Everything here is compiled into the engine
/// and available to every game; game-specific behaviour is guarded inside the
/// Ruby source with `defined?` checks.
pub const PATCHES: &[RuntimePatch] = &[RuntimePatch {
    id: Cow::Borrowed("map_panoramic"),
    name: Cow::Borrowed("Map panoramic mode"),
    description: Cow::Borrowed("Renders a wider slice of the current map and scales it to fit, \
                  so more of the map is visible. Only active while a map is on \
                  screen."),
    source: Cow::Borrowed(include_str!("map_panoramic.rb")),
    parameter: Some(PatchParameter {
        label: "Zoom",
        min: 1.0,
        max: 3.0,
        step: 0.1,
        default: 1.5,
        suffix: "×",
    }),
}, RuntimePatch {
    id: Cow::Borrowed("cable_club_rng"),
    name: Cow::Borrowed("Cable Club RNG compatibility"),
    description: Cow::Borrowed("Repairs Cable Club's per-turn RNG sync in memory for Ruby versions where its Random#srand calls fail. Enable before starting an online battle."),
    source: Cow::Borrowed(include_str!("cable_club_rng.rb")),
    parameter: None,
}];

thread_local! {
    static USER_PATCHES: RefCell<Vec<std::sync::Arc<RuntimePatch>>> = const { RefCell::new(Vec::new()) };
    static ENABLED: RefCell<BTreeSet<String>> = const { RefCell::new(BTreeSet::new()) };
    static VALUES: RefCell<BTreeMap<String, f64>> = const { RefCell::new(BTreeMap::new()) };
}

/// Every patch in the catalog.
pub fn patches() -> Vec<std::sync::Arc<RuntimePatch>> {
    static BUILTINS: std::sync::LazyLock<Vec<std::sync::Arc<RuntimePatch>>> =
        std::sync::LazyLock::new(|| PATCHES.iter().cloned().map(std::sync::Arc::new).collect());
    let mut patches = BUILTINS.clone();
    USER_PATCHES.with(|user| patches.extend(user.borrow().iter().cloned()));
    patches
}

/// Register one host-provided file from patches/<name>.rb. Desktop files and
/// browser virtual files share naming, metadata, replacement and sort rules.
pub fn register_user_patch(name: &str, source: String) -> bool {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return false;
    }
    let patch = std::sync::Arc::new(RuntimePatch {
        id: Cow::Owned(format!("user/{name}")),
        name: Cow::Owned(name.replace('_', " ")),
        description: Cow::Owned(format!("User patch: patches/{name}.rb")),
        source: Cow::Owned(source),
        parameter: None,
    });
    USER_PATCHES.with(|user| {
        let mut user = user.borrow_mut();
        user.retain(|existing| existing.id != patch.id);
        user.push(patch);
        user.sort_by(|a, b| a.id.cmp(&b.id));
    });
    true
}

/// Load Ruby files returning an apply/revert hash from a game's patches folder.
pub fn load_user_patches(root: &std::path::Path) -> std::io::Result<()> {
    USER_PATCHES.with(|user| user.borrow_mut().clear());
    let entries = match std::fs::read_dir(root.join("patches")) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let mut paths = entries
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    for path in paths {
        if !path.is_file() || path.extension().and_then(|v| v.to_str()) != Some("rb") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|v| v.to_str()) else {
            continue;
        };
        if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            continue;
        }
        register_user_patch(name, std::fs::read_to_string(&path)?);
    }
    Ok(())
}

/// Looks up a patch by id.
pub fn find(id: &str) -> Option<std::sync::Arc<RuntimePatch>> {
    patches().into_iter().find(|patch| patch.id == id)
}

/// Whether a patch is currently enabled. Unknown ids are treated as disabled.
pub fn is_enabled(id: &str) -> bool {
    ENABLED.with(|enabled| enabled.borrow().contains(id))
}

/// Enables or disables a patch. Returns `false` (and changes nothing) if the id
/// is not in the catalog.
pub fn set_enabled(id: &str, enabled: bool) -> bool {
    if find(id).is_none() {
        return false;
    }
    ENABLED.with(|slot| {
        let mut slot = slot.borrow_mut();
        if enabled {
            slot.insert(id.to_owned());
        } else {
            slot.remove(id);
        }
    });
    true
}

/// The current value of a patch's parameter, or `None` if it has no parameter.
pub fn value(id: &str) -> Option<f64> {
    let default = find(id)?.parameter.as_ref().map(|p| p.default)?;
    Some(
        VALUES
            .with(|values| values.borrow().get(id).copied())
            .unwrap_or(default),
    )
}

/// Sets a patch's parameter, clamped to its range. Returns `false` if the id is
/// unknown or has no parameter.
pub fn set_value(id: &str, value: f64) -> bool {
    let Some(parameter) = find(id).and_then(|patch| patch.parameter.clone()) else {
        return false;
    };
    let value = value.clamp(parameter.min, parameter.max);
    VALUES.with(|values| {
        values.borrow_mut().insert(id.to_owned(), value);
    });
    true
}

/// Enables every comma- or whitespace-separated id in `value`, ignoring
/// unknown ids. Used for the `RSXP_PATCH` startup override.
pub fn enable_from(value: &str) -> Vec<String> {
    let mut enabled = Vec::new();
    for id in value
        .split([',', ' ', '\t', '\n'])
        .filter(|id| !id.is_empty())
    {
        if set_enabled(id, true) {
            enabled.push(id.to_owned());
        } else {
            eprintln!("rsxp: ignoring unknown runtime patch: {id}");
        }
    }
    enabled
}

/// Clears the enabled set and parameter values. Tests use this so a patch does
/// not leak between runs in the same process.
pub fn reset() {
    USER_PATCHES.with(|slot| slot.borrow_mut().clear());
    ENABLED.with(|slot| slot.borrow_mut().clear());
    VALUES.with(|slot| slot.borrow_mut().clear());
}

/// `id=<0|1><TAB><value>` lines reported to the Ruby shim. The value is empty
/// for patches without a parameter.
pub fn state_report() -> String {
    let mut report = String::new();
    for patch in patches() {
        report.push_str(&patch.id);
        report.push('=');
        report.push(if is_enabled(&patch.id) { '1' } else { '0' });
        report.push('\t');
        if let Some(value) = value(&patch.id) {
            report.push_str(&value.to_string());
        }
        report.push('\n');
    }
    report
}

/// `id<TAB>name<TAB>enabled<TAB>value<TAB>min<TAB>max<TAB>step<TAB>suffix<TAB>description`
/// lines, used by the browser panel (which has no shared process to read the
/// catalog from).
pub fn list_report() -> String {
    let mut report = String::new();
    for patch in patches() {
        report.push_str(&patch.id);
        report.push('\t');
        report.push_str(&patch.name);
        report.push('\t');
        report.push(if is_enabled(&patch.id) { '1' } else { '0' });
        report.push('\t');
        match &patch.parameter {
            Some(parameter) => {
                report.push_str(&value(&patch.id).unwrap_or(parameter.default).to_string());
                report.push('\t');
                report.push_str(&parameter.min.to_string());
                report.push('\t');
                report.push_str(&parameter.max.to_string());
                report.push('\t');
                report.push_str(&parameter.step.to_string());
                report.push('\t');
                report.push_str(parameter.suffix);
            }
            None => {
                // Empty value/min/max/step/suffix.
                report.push_str("\t\t\t\t");
            }
        }
        report.push('\t');
        report.push_str(&patch.description);
        report.push('\n');
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_patches_join_the_runtime_catalog_and_reset_between_games() {
        reset();
        let root = std::env::temp_dir().join(format!("rsxp-user-patches-{}", std::process::id()));
        std::fs::create_dir_all(root.join("patches")).unwrap();
        let source = "{ apply: proc {}, revert: proc {} }";
        std::fs::write(root.join("patches/map_panoramic.rb"), source).unwrap();
        std::fs::write(root.join("patches/readme.txt"), "not Ruby").unwrap();
        load_user_patches(&root).unwrap();
        assert_eq!(find("user/map_panoramic").unwrap().source, source);
        assert_ne!(find("map_panoramic").unwrap().source, source);
        assert!(!is_enabled("user/map_panoramic"));
        assert!(set_enabled("user/map_panoramic", true));
        assert!(state_report().contains("user/map_panoramic=1\t\n"));
        assert!(list_report().contains("patches/map_panoramic.rb"));
        reset();
        assert!(find("user/map_panoramic").is_none());
        assert!(!is_enabled("user/map_panoramic"));
        assert!(find("map_panoramic").is_some());
        std::fs::remove_dir_all(root).unwrap();
    }
}
