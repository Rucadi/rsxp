// SPDX-License-Identifier: GPL-3.0-or-later
//! Unpaced core timings, excluding startup and 120 warmup frames.
//! Usage: cargo run --release -p fanplay-core-nds --example frame_bench -- ROM [frames]
//! Saves are discarded; this does not measure desktop transport or presentation.
use fanplay_core::{
    AudioSink, Content, CoreContext, CoreControl, EventSink, Platform, PlatformResult, VideoFrame,
    VideoSink,
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};

struct Host;
impl Platform for Host {
    fn save_dir(&self) -> PathBuf {
        PathBuf::new()
    }
    fn write_save(&self, _: &str, _: &[u8]) -> PlatformResult<()> {
        Ok(())
    }
}
impl AudioSink for Host {}
impl EventSink for Host {
    fn emit(&mut self, event: fanplay_core::CoreEvent) {
        if let fanplay_core::CoreEvent::Error(message) = event {
            eprintln!("{message}");
        }
    }
}
struct Timings {
    previous: Instant,
    seen: usize,
    milliseconds: Vec<f64>,
}
struct Video(Arc<Mutex<Timings>>);
impl VideoSink for Video {
    fn present(&mut self, _: VideoFrame<'_>) {
        let now = Instant::now();
        let mut timings = self.0.lock().unwrap();
        let elapsed = now.duration_since(timings.previous).as_secs_f64() * 1000.0;
        timings.previous = now;
        if timings.seen >= 120 {
            timings.milliseconds.push(elapsed);
        }
        timings.seen += 1;
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let rom = PathBuf::from(args.next().ok_or("usage: frame_bench ROM [frames]")?);
    let frames: usize = args
        .next()
        .map(|v| v.to_string_lossy().parse())
        .transpose()?
        .unwrap_or(1200);
    if frames == 0 {
        return Err("frames must be positive".into());
    }
    let control = Arc::new(CoreControl::new());
    control.set_fast_forward(true);
    control.set_frame_limit(Some(frames as u64 + 120));
    let timings = Arc::new(Mutex::new(Timings {
        previous: Instant::now(),
        seen: 0,
        milliseconds: Vec::with_capacity(frames),
    }));
    let mut core = fanplay_core_nds::create();
    core.load(
        Content::file(rom),
        CoreContext::new(
            Arc::new(Host),
            Box::new(Video(timings.clone())),
            Box::new(Host),
            Box::new(Host),
            control,
        ),
    )?;
    core.run()?;
    let timings = timings.lock().unwrap();
    let mut values = timings.milliseconds.clone();
    if values.is_empty() {
        return Err("core stopped before warmup finished".into());
    }
    values.sort_by(f64::total_cmp);
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let percentile = |p: f64| values[((values.len() - 1) as f64 * p).round() as usize];
    println!("frames={} mean_ms={mean:.3} p50_ms={:.3} p95_ms={:.3} p99_ms={:.3} max_ms={:.3} over_budget={} fps={:.1}", values.len(), percentile(0.5), percentile(0.95), percentile(0.99), values.last().unwrap(), values.iter().filter(|&&ms| ms > 560190.0 / 33513982.0 * 1000.0).count(), 1000.0 / mean);
    Ok(())
}
