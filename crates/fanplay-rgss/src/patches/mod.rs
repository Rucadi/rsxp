// SPDX-License-Identifier: GPL-2.0-or-later
//! Runtime patches: small self-contained Ruby programs the workspace UI can
//! turn on while a game is running.
//!
//! The engine owns the generic user-patch catalog and enabled set. Essentials
//! behavior is deliberately not compiled here: its Rhai plugins inject Ruby
//! and own their controls and lifecycle. The Ruby shim still polls this state
//! for generic user patches, evaluates their source, and calls their
//! `apply`/`revert`/`value` procs.
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

/// One available generic patch. `source` is a Ruby program that evaluates to a
/// hash of `apply`/`revert`/`value` procs.
#[derive(Clone)]
pub struct RuntimePatch {
    pub id: Cow<'static, str>,
    pub name: Cow<'static, str>,
    pub description: Cow<'static, str>,
    pub source: Cow<'static, str>,
    pub parameter: Option<PatchParameter>,
}

/// Essentials-specific patches are supplied by the `essentials-runtime-patches`
/// Rhai plugin. Keeping the native catalog empty is intentional: generic user
/// patches still use this registry, while game-specific Ruby never lives in
/// the Rust binary.
pub const PATCHES: &[RuntimePatch] = &[];

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

/// Remove a host-provided patch after its Ruby `revert` callback has run.
/// Built-in patches and patches loaded from the game's `patches/` directory
/// are never removable through this API.
pub fn remove_user_patch(name: &str) -> bool {
    let id = name.strip_prefix("user/").unwrap_or(name);
    if id.is_empty() || !id.starts_with("rhai-") {
        return false;
    }
    let removed = USER_PATCHES.with(|user| {
        let mut user = user.borrow_mut();
        let before = user.len();
        user.retain(|patch| patch.id != format!("user/{id}"));
        user.len() != before
    });
    if removed {
        ENABLED.with(|enabled| enabled.borrow_mut().remove(&format!("user/{id}")));
        VALUES.with(|values| values.borrow_mut().remove(&format!("user/{id}")));
    }
    removed
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
/// unknown ids. Used for the `FANPLAY_PATCH` startup override.
pub fn enable_from(value: &str) -> Vec<String> {
    let mut enabled = Vec::new();
    for id in value
        .split([',', ' ', '\t', '\n'])
        .filter(|id| !id.is_empty())
    {
        if set_enabled(id, true) {
            enabled.push(id.to_owned());
        } else {
            eprintln!("fanplay: ignoring unknown runtime patch: {id}");
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
        let root = std::env::temp_dir().join(format!("fanplay-user-patches-{}", std::process::id()));
        std::fs::create_dir_all(root.join("patches")).unwrap();
        let source = "{ apply: proc {}, revert: proc {} }";
        std::fs::write(root.join("patches/test_patch.rb"), source).unwrap();
        std::fs::write(root.join("patches/readme.txt"), "not Ruby").unwrap();
        load_user_patches(&root).unwrap();
        assert_eq!(find("user/test_patch").unwrap().source, source);
        assert!(!is_enabled("user/test_patch"));
        assert!(set_enabled("user/test_patch", true));
        assert!(state_report().contains("user/test_patch=1\t\n"));
        assert!(list_report().contains("patches/test_patch.rb"));
        reset();
        assert!(find("user/test_patch").is_none());
        assert!(!is_enabled("user/test_patch"));
        assert!(patches().is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
}
