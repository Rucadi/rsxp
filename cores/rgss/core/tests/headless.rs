// SPDX-License-Identifier: GPL-2.0-or-later
//! End-to-end check that the RGSS core adapter satisfies the generic contract.
//!
//! Requires the same native MRI toolchain as the desktop build. Run with:
//! `nix-shell --run 'cargo test -p rsxp-core-rgss --test headless'`.

use rsxp_core::audio::NullAudioSink;
use rsxp_core::{Content, CoreContext, CoreEvent, CoreLoadInfo, CoreResult, EventSink, Platform};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "rsxp-rgss-core-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct TestPlatform {
    root: PathBuf,
    start: Instant,
}

impl Platform for TestPlatform {
    fn read_file(&self, path: &str) -> Option<Vec<u8>> {
        std::fs::read(self.root.join(path)).ok()
    }
    fn save_dir(&self) -> PathBuf {
        self.root.clone()
    }
    fn now(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }
    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

struct CountingVideo(Arc<AtomicU32>);

impl rsxp_core::VideoSink for CountingVideo {
    fn present(&mut self, _frame: rsxp_core::VideoFrame<'_>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

struct RecordingEvents(Arc<Mutex<Option<CoreLoadInfo>>>);

impl EventSink for RecordingEvents {
    fn emit(&mut self, event: CoreEvent) {
        if let CoreEvent::Loaded(info) = event {
            *self.0.lock().unwrap() = Some(info);
        }
    }
}

fn platform(root: &Path) -> Arc<dyn Platform> {
    Arc::new(TestPlatform {
        root: root.to_owned(),
        start: Instant::now(),
    })
}

#[test]
fn rgss_core_loads_runs_and_presents_a_frame() {
    // Keep CI and headless tests away from an actual wgpu adapter. The GPU path
    // is covered by the existing desktop integration suite.
    // SAFETY: this test is the only environment mutator in its process.
    unsafe { std::env::set_var("RSXP_RENDERER", "software") };

    let dir = TempDir::new("smoke");
    let script = dir.0.join("smoke.rb");
    std::fs::write(
        &script,
        r#"
bitmap = Bitmap.new(8, 8)
bitmap.fill_rect(0, 0, 8, 8, Color.new(255, 0, 0, 255))
Graphics.update
"#,
    )
    .unwrap();

    let frames = Arc::new(AtomicU32::new(0));
    let loaded = Arc::new(Mutex::new(None));
    let context = CoreContext::new(
        platform(&dir.0),
        Box::new(CountingVideo(Arc::clone(&frames))),
        Box::new(NullAudioSink),
        Box::new(RecordingEvents(Arc::clone(&loaded))),
        Arc::new(rsxp_core::CoreControl::new()),
    );

    let mut core = rsxp_core_rgss::create();
    core.load(Content::file(&script), context).unwrap();
    let info = *loaded.lock().unwrap();
    assert_eq!(info.unwrap().width, 640);
    let status: CoreResult<_> = core.run();
    assert!(status.unwrap().is_success());
    assert_eq!(frames.load(Ordering::Relaxed), 1);
}
