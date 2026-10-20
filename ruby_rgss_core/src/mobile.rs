// SPDX-License-Identifier: GPL-2.0-or-later
//! Fiber-pumped MRI lifecycle for the cargo-mobile2 shell.
//!
//! `mobile_bridge.c` owns every Ruby C API call and exception longjmp boundary.
//! The Rust side only passes paths and requests, then resumes the game fiber
//! once per UI step. `Graphics.update` yields after every frame so the WebView
//! stays responsive without running Ruby on another thread.
use std::{
    ffi::{c_char, CStr, CString},
    path::Path,
};

unsafe extern "C" {
    fn rsxp_mri_initialize(path: *const c_char) -> i32;
    fn rsxp_mri_start(
        source: *const c_char,
        root: *const c_char,
        scripts: *const c_char,
        script: *const c_char,
        title: *const c_char,
        saves: *const c_char,
    ) -> i32;
    fn rsxp_mri_step() -> i32;
    fn rsxp_mri_error() -> *const c_char;
    fn rsxp_mri_stop();
}

fn error() -> String {
    unsafe {
        CStr::from_ptr(rsxp_mri_error())
            .to_string_lossy()
            .into_owned()
    }
}

fn cstring(value: &str) -> Result<CString, String> {
    CString::new(value).map_err(|error| error.to_string())
}

/// Initialize MRI and prepare the RGSS game fiber.
pub fn start(
    runtime: &Path,
    root: &Path,
    scripts: &str,
    script: Option<&str>,
    title: &str,
    saves: &Path,
) -> Result<(), String> {
    let runtime = cstring(&runtime.to_string_lossy())?;
    let root = cstring(&root.to_string_lossy())?;
    let source = cstring(rsxp::ruby::RGSS_SOURCE)?;
    let scripts = cstring(scripts)?;
    let script = script.map(cstring).transpose()?;
    let title = cstring(title)?;
    let saves = cstring(&saves.to_string_lossy())?;
    unsafe {
        if rsxp_mri_initialize(runtime.as_ptr()) != 0 {
            return Err(error());
        }
        if rsxp_mri_start(
            source.as_ptr(),
            root.as_ptr(),
            scripts.as_ptr(),
            script
                .as_ref()
                .map_or(std::ptr::null(), |script| script.as_ptr()),
            title.as_ptr(),
            saves.as_ptr(),
        ) != 0
        {
            return Err(error());
        }
    }
    Ok(())
}

/// Resume the game fiber. Returns `false` when the script has finished.
pub fn step() -> Result<bool, String> {
    match unsafe { rsxp_mri_step() } {
        0 => Ok(true),
        1 => Ok(false),
        _ => Err(error()),
    }
}

/// Stop the game fiber and run finalizers even when the shell closes early.
pub fn stop() {
    unsafe { rsxp_mri_stop() }
}
