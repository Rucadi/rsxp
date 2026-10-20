// SPDX-License-Identifier: GPL-2.0-or-later
//! Relocate packaged Ruby before any subsystem creates threads or embeds Ruby.
//!
//! [`configure`] makes a relocatable installation work: it locates the
//! `ruby-runtime.json` manifest that ships beside the executable (or, on
//! macOS, in `../Resources/` so the bundle's `MacOS` directory holds only
//! signed Mach-O code), then rewrites the process environment so the embedded
//! MRI finds its packaged standard library and gems:
//! - `RUBYLIB` from the manifest's `load_paths`, and `GEM_PATH`/`GEM_HOME`
//!   from `gem_paths`/`gem_home`, all resolved relative to the executable
//!   directory via the [`paths`] helper (which joins relative entries and
//!   rejects non-array or non-string entries).
//! - `PATH` is prefixed with the executable directory because the packaged
//!   layout also carries the Windows DLLs Ruby needs.
//!
//! This must run first in the process entry point, before Ruby, audio, or UI
//! threads exist: `env::set_var` is only safe while the process is still
//! single-threaded. When no manifest is present, a colocated `ruby/lib` is
//! used if available; otherwise the ambient Ruby installation is used.
use std::{env, error::Error, fs, path::Path};

fn paths(base: &Path, entries: &serde_json::Value) -> Result<std::ffi::OsString, Box<dyn Error>> {
    let entries = entries.as_array().ok_or("invalid bundled Ruby path list")?;
    let paths = entries
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .map(|path| base.join(path))
                .ok_or("invalid bundled Ruby path")
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(env::join_paths(paths)?)
}

pub fn configure() -> Result<(), Box<dyn Error>> {
    let exe = env::current_exe()?;
    let base = exe.parent().ok_or("missing executable directory")?;
    // macOS stores the manifest in Resources so the bundle's MacOS directory
    // holds only signed Mach-O code; other layouts keep it beside the binary.
    let manifest = [
        base.join("ruby-runtime.json"),
        base.join("../Resources/ruby-runtime.json"),
    ]
    .into_iter()
    .find(|path| path.is_file());
    let Some(manifest) = manifest else {
        // Windows distributions can keep the Ruby standard library beside the
        // executable without carrying a manifest.
        let ruby_root = base.join("ruby/lib/ruby");
        if !ruby_root.is_dir() {
            return Ok(());
        }
        let version_dir = fs::read_dir(&ruby_root)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| path.is_dir() && path.join("x64-mingw32").is_dir())
            .ok_or("could not find the bundled Ruby version directory")?;
        let version = version_dir
            .file_name()
            .ok_or("bundled Ruby version directory has no name")?;
        let ruby_arch = base.join("ruby/arch");
        let ruby_arch = if ruby_arch.is_dir() {
            ruby_arch
        } else {
            version_dir.join("x64-mingw32")
        };
        let load_paths = [
            version_dir.clone(),
            ruby_arch,
            ruby_root.join("site_ruby").join(version),
            ruby_root.join("vendor_ruby").join(version),
        ]
        .into_iter()
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
        let gem_home = ruby_root.join("gems").join(version);
        // SAFETY: called once at process startup, before Ruby, audio, or UI threads.
        unsafe {
            env::set_var("RUBYLIB", env::join_paths(load_paths)?);
            if gem_home.is_dir() {
                env::set_var("GEM_HOME", &gem_home);
                env::set_var("GEM_PATH", &gem_home);
            }
            let mut search = vec![base.to_owned()];
            search.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
            env::set_var("PATH", env::join_paths(search)?);
        }
        return Ok(());
    };
    let config: serde_json::Value = serde_json::from_slice(&fs::read(manifest)?)?;
    let ruby_lib = paths(base, &config["load_paths"])?;
    let gem_paths = paths(base, &config["gem_paths"])?;
    // SAFETY: called once at process startup, before Ruby, audio, or UI threads.
    unsafe {
        env::set_var("RUBYLIB", ruby_lib);
        env::set_var("GEM_PATH", &gem_paths);
        env::set_var(
            "GEM_HOME",
            base.join(config["gem_home"].as_str().ok_or("missing gem_home")?),
        );
        // The packaged executable directory also contains Windows DLLs.
        let mut search = vec![base.to_owned()];
        search.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
        env::set_var("PATH", env::join_paths(search)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ruby_paths_follow_the_installation_directory() {
        let base = env::temp_dir().join("RSXP moved directory");
        let joined = paths(&base, &serde_json::json!(["ruby/lib", "ruby/arch"])).unwrap();
        assert_eq!(
            env::split_paths(&joined).collect::<Vec<_>>(),
            [base.join("ruby/lib"), base.join("ruby/arch")]
        );
        assert!(paths(&base, &serde_json::json!([null])).is_err());
    }
}
