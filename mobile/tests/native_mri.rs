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
    std::fs::write(
        path.join(".demo/demo.rb"),
        format!(
            "{}\n{}",
            r#"
raise 'Table binding missing' unless Table.instance_method(:__get).source_location.nil?
raise 'Sprite setter binding missing' unless Sprite.instance_method(:x=).source_location.nil?
raise 'Retained bindings missing' unless respond_to?(:__rsxp_retained_source, true)
"#,
            include_str!("../demo.rb")
        ),
    )
    .unwrap();
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
    // Restart the same VM and verify the native compositor agrees with software.
    // GPU execution is opt-in so ordinary tests work without a graphics device.
    if std::env::var_os("RSXP_TEST_GPU").is_some() {
        rsxp_mobile::engine::set_gpu(rsxp::gpu::GpuRenderer::new(640, 480).unwrap());
        call(json!({"op":"init","id":".demo"}));
        call(json!({"op":"step"}));
        assert!(
            rsxp_mobile::engine::with_native_frame(|_, bitmap| bitmap.is_none()).unwrap(),
            "GPU composition silently fell back to software"
        );
        let gpu_first = call(json!({"op":"frame"}));
        assert_eq!(
            first["rgba"], gpu_first["rgba"],
            "GPU and software demo frames differ"
        );
        call(json!({"op":"key","code":79,"down":true}));
        call(json!({"op":"step"}));
        call(json!({"op":"step"}));
        assert!(rsxp_mobile::engine::with_native_frame(|_, bitmap| bitmap.is_none()).unwrap());
        assert_eq!(moved["rgba"], call(json!({"op":"frame"}))["rgba"]);
        call(json!({"op":"stop"}));
        assert!(
            rsxp_mobile::engine::with_native_frame(|_, _| true).unwrap(),
            "GPU was not returned to the library on stop"
        );
    }
    std::env::set_current_dir(std::env::temp_dir()).unwrap();
    std::fs::remove_dir_all(path).unwrap();
    println!(
        "NATIVE_MRI_GAMEPLAY_OK: native Ruby executed RGSS and native Rust rendered/input updated"
    );
}
