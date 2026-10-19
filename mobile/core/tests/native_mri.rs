// SPDX-License-Identifier: GPL-2.0-or-later
// MRI must be initialized and resumed on the native process's main thread.
use rsxp_mobile::{rsxp_mobile_configure, rsxp_mobile_free, rsxp_mobile_request};
use serde_json::{json, Value};
use std::ffi::{CStr, CString};
fn request(value: Value) -> Value {
    let input = CString::new(value.to_string()).unwrap();
    let pointer = unsafe { rsxp_mobile_request(input.as_ptr()) };
    let response: Value =
        serde_json::from_str(unsafe { CStr::from_ptr(pointer) }.to_str().unwrap()).unwrap();
    unsafe { rsxp_mobile_free(pointer) };
    assert!(response.get("error").is_none(), "{response}");
    response["ok"].clone()
}
fn main() {
    let path = std::env::temp_dir().join(format!("rsxp-native-mri-{}", std::process::id()));
    let root = CString::new(path.to_str().unwrap()).unwrap();
    assert_eq!(
        unsafe { rsxp_mobile_configure(root.as_ptr(), std::ptr::null()) },
        1
    );
    request(json!({"op":"init","id":".demo"}));
    // The typed Magnus bindings must be installed, or the game silently falls
    // back to the slower serialized bridge.
    let ruby = magnus::Ruby::get().unwrap();
    assert!(
        ruby.eval::<bool>(
            "Sprite.private_method_defined?(:__rsxp_sprite_sync) && defined?(Table)"
        )
        .unwrap(),
        "Magnus bindings are not installed"
    );
    assert_eq!(request(json!({"op":"step"}))["running"], true);
    let first = request(json!({"op":"frame"}));
    assert_eq!(first["width"], 320);
    request(json!({"op":"key","code":79,"down":true}));
    request(json!({"op":"step"}));
    request(json!({"op":"step"}));
    let moved = request(json!({"op":"frame"}));
    assert_ne!(
        first["rgba"], moved["rgba"],
        "Native Ruby did not respond to input"
    );
    request(json!({"op":"stop"}));
    std::env::set_current_dir(std::env::temp_dir()).unwrap();
    std::fs::remove_dir_all(path).unwrap();
    println!(
        "NATIVE_MRI_GAMEPLAY_OK: native Ruby executed RGSS and native Rust rendered/input updated"
    );
}
