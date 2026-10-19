// SPDX-License-Identifier: GPL-2.0-or-later
//! Native MRI. Ruby game code never enters a WebView or a WASM interpreter.
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
pub fn start(
    runtime: &Path,
    root: &Path,
    scripts: &str,
    script: Option<&str>,
    title: &str,
    saves: &Path,
) -> Result<(), String> {
    let c = |value: &str| CString::new(value).map_err(|e| e.to_string());
    let runtime = c(&runtime.to_string_lossy())?;
    let root = c(&root.to_string_lossy())?;
    let source = c(include_str!("../../../rust/rgss.rb"))?;
    let scripts = c(scripts)?;
    let script = script.map(c).transpose()?;
    let title = c(title)?;
    let saves = c(&saves.to_string_lossy())?;
    unsafe {
        if rsxp_mri_initialize(runtime.as_ptr()) != 0 {
            return Err(error());
        }
        if rsxp_mri_start(
            source.as_ptr(),
            root.as_ptr(),
            scripts.as_ptr(),
            script.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
            title.as_ptr(),
            saves.as_ptr(),
        ) != 0
        {
            return Err(error());
        }
    }
    Ok(())
}
pub fn step() -> Result<bool, String> {
    match unsafe { rsxp_mri_step() } {
        0 => Ok(true),
        1 => Ok(false),
        _ => Err(error()),
    }
}
pub fn stop() {
    unsafe { rsxp_mri_stop() }
}
