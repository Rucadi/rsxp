// SPDX-License-Identifier: GPL-2.0-or-later
//! Native winit/egui/wgpu frontend with the shared optimized MRI bindings.
mod app;
#[path = "../../src/desktop/audio.rs"]
mod audio;
pub mod engine;
#[allow(dead_code)]
#[path = "../../src/desktop/gpu_present.rs"]
mod gpu_present;
mod input;
#[path = "../../src/native_keys.rs"]
mod native_keys;
mod platform;

pub use app::run;
pub use engine::{configure, request};

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub fn main() {
    app::run();
}

#[cfg(target_os = "ios")]
#[no_mangle]
pub extern "C" fn start_app() {
    app::run();
}

#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    use winit::platform::android::EventLoopBuilderExtAndroid;
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("RSXP"),
    );
    let event_loop = winit::event_loop::EventLoop::builder()
        .with_android_app(app)
        .build()
        .expect("Android event loop");
    app::run_event_loop(event_loop);
}
