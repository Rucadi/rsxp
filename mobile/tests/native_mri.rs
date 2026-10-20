// SPDX-License-Identifier: GPL-2.0-or-later
// MRI must be initialized and resumed on the native process's main thread.
use rsxp_mobile::{configure, request};
use serde_json::{json, Value};

fn call(value: Value) -> Value {
    let response = request(value);
    assert!(response.get("error").is_none(), "{response}");
    response["ok"].clone()
}

fn main() {
    let path = std::env::temp_dir().join(format!("rsxp-native-mri-{}", std::process::id()));
    configure(path.clone(), None).unwrap();
    call(json!({"op":"init","id":".demo"}));
    assert_eq!(call(json!({"op":"step"}))["running"], true);
    let first = call(json!({"op":"frame"}));
    assert_eq!(first["width"], 320);
    call(json!({"op":"key","code":79,"down":true}));
    call(json!({"op":"step"}));
    call(json!({"op":"step"}));
    let moved = call(json!({"op":"frame"}));
    assert_ne!(
        first["rgba"], moved["rgba"],
        "Native Ruby did not respond to input"
    );
    call(json!({"op":"stop"}));
    std::env::set_current_dir(std::env::temp_dir()).unwrap();
    std::fs::remove_dir_all(path).unwrap();
    println!(
        "NATIVE_MRI_GAMEPLAY_OK: native Ruby executed RGSS and native Rust rendered/input updated"
    );
}
