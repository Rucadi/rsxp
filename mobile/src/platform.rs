// SPDX-License-Identifier: GPL-2.0-or-later
//! Host paths for the cargo-mobile2 shell.
//!
//! The only platform-specific work left here is finding the app data directory
//! and unpacking the Ruby runtime on Android, where cargo-mobile2 packages
//! assets inside the APK.
use std::path::PathBuf;

pub fn data_dir() -> Result<PathBuf, String> {
    #[cfg(target_os = "android")]
    {
        android::data_dir()
    }
    #[cfg(target_os = "ios")]
    {
        let home = std::env::var_os("HOME").ok_or("iOS has no HOME directory")?;
        let documents = PathBuf::from(home).join("Documents");
        std::fs::create_dir_all(&documents).map_err(|error| error.to_string())?;
        Ok(documents)
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let base = if cfg!(target_os = "macos") {
            std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join("Library/Application Support/RSXP"))
                .ok_or("no HOME directory")?
        } else if cfg!(windows) {
            std::env::var_os("APPDATA")
                .map(|home| PathBuf::from(home).join("RSXP"))
                .ok_or("no APPDATA directory")?
        } else {
            std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
                })
                .ok_or("no XDG_DATA_HOME or HOME directory")?
                .join("rsxp")
        };
        std::fs::create_dir_all(&base).map_err(|error| error.to_string())?;
        Ok(base)
    }
}

pub fn library_dir() -> Result<PathBuf, String> {
    let library = data_dir()?.join("Games");
    std::fs::create_dir_all(&library).map_err(|error| error.to_string())?;
    Ok(library)
}

pub fn ruby_runtime() -> Result<PathBuf, String> {
    #[cfg(target_os = "android")]
    {
        android::ruby_runtime()
    }
    #[cfg(not(target_os = "android"))]
    {
        // cargo-mobile2 copies the `assets` resource directory into the iOS app
        // bundle; the desktop binary finds it next to Cargo.toml.
        let mut candidates = Vec::new();
        if let Ok(executable) = std::env::current_exe() {
            if let Some(bundle) = executable.parent() {
                candidates.push(bundle.join("assets/runtime"));
            }
        }
        candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/runtime"));
        candidates
            .into_iter()
            .find(|path| path.join("lib").is_dir())
            .ok_or_else(|| {
                "the native Ruby runtime is missing; run cargo build once to stage assets".into()
            })
    }
}

#[cfg(target_os = "android")]
mod android {
    use super::*;
    use jni::{
        objects::{JObject, JString},
        JValue, JavaVM,
    };
    use ndk::asset::AssetManager;
    use std::{ffi::CString, fs, io, ptr::NonNull};

    fn with_environment<T>(
        operation: impl FnOnce(&mut jni::JNIEnv<'_>, &JObject<'_>) -> Result<T, String>,
    ) -> Result<T, String> {
        let context = ndk_context::android_context();
        let vm =
            unsafe { JavaVM::from_raw(context.vm().cast()) }.map_err(|error| error.to_string())?;
        let mut env = vm
            .attach_current_thread()
            .map_err(|error| error.to_string())?;
        let application = unsafe { JObject::from_raw(context.context().cast()) };
        operation(&mut env, &application)
    }

    fn string(env: &mut jni::JNIEnv<'_>, value: JString<'_>) -> Result<String, String> {
        env.get_string(&value)
            .map(|value| value.to_string_lossy().into_owned())
            .map_err(|error| error.to_string())
    }

    pub fn data_dir() -> Result<PathBuf, String> {
        with_environment(|env, application| {
            let files = env
                .call_method(application, "getFilesDir", "()Ljava/io/File;", &[])
                .and_then(|value| value.l())
                .map_err(|error| error.to_string())?;
            let path = env
                .call_method(&files, "getAbsolutePath", "()Ljava/lang/String;", &[])
                .and_then(|value| value.l())
                .map_err(|error| error.to_string())?;
            Ok(PathBuf::from(string(env, path.into())?))
        })
    }

    pub fn ruby_runtime() -> Result<PathBuf, String> {
        let target = data_dir()?.join("runtime");
        let marker = target.join(".staged");
        let version = option_env!("RSXP_RUBY_VERSION").unwrap_or("native");
        if fs::read_to_string(&marker).is_ok_and(|value| value == version) {
            return Ok(target);
        }
        let _ = fs::remove_dir_all(&target);
        fs::create_dir_all(&target).map_err(|error| error.to_string())?;

        with_environment(|env, application| {
            let assets = env
                .call_method(
                    application,
                    "getAssets",
                    "()Landroid/content/res/AssetManager;",
                    &[],
                )
                .and_then(|value| value.l())
                .map_err(|error| error.to_string())?;
            let pointer = unsafe {
                ndk_sys::AAssetManager_fromJava(env.get_native_interface(), assets.as_raw())
            };
            let pointer =
                NonNull::new(pointer).ok_or("Android did not provide an asset manager")?;
            let manager = unsafe { AssetManager::from_ptr(pointer) };
            copy_dir(env, &assets, &manager, "runtime", &target)?;
            Ok(())
        })?;
        fs::write(marker, version).map_err(|error| error.to_string())?;
        Ok(target)
    }

    fn copy_dir(
        env: &mut jni::JNIEnv<'_>,
        assets: &JObject<'_>,
        manager: &AssetManager,
        source: &str,
        destination: &Path,
    ) -> Result<(), String> {
        let path = env.new_string(source).map_err(|error| error.to_string())?;
        let list = env
            .call_method(
                assets,
                "list",
                "(Ljava/lang/String;)[Ljava/lang/String;",
                &[JValue::Object(&path)],
            )
            .and_then(|value| value.l())
            .map_err(|error| error.to_string())?;
        let length = env
            .get_array_length(&list)
            .map_err(|error| error.to_string())?;
        if length == 0 {
            let name = CString::new(source).map_err(|error| error.to_string())?;
            let mut asset = manager
                .open(&name)
                .ok_or_else(|| format!("Android asset {source} could not be opened"))?;
            let mut file = fs::File::create(destination).map_err(|error| error.to_string())?;
            io::copy(&mut asset, &mut file).map_err(|error| error.to_string())?;
            return Ok(());
        }

        fs::create_dir_all(destination).map_err(|error| error.to_string())?;
        for index in 0..length {
            let entry = env
                .get_object_array_element(&list, index)
                .map_err(|error| error.to_string())?;
            let name = string(env, JString::from(entry))?;
            if name.is_empty() || name == "." || name == ".." || name.contains('/') {
                continue;
            }
            let child = if source.is_empty() {
                name.clone()
            } else {
                format!("{source}/{name}")
            };
            copy_dir(env, assets, manager, &child, &destination.join(name))?;
        }
        Ok(())
    }
}
