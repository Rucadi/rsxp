// SPDX-License-Identifier: GPL-2.0-or-later
//! Repeatable compositing workload; no Ruby, disk, window, or FPS cap.
use rsxp::graphics::{Bitmap, Color, Rect, SpriteFx, Tone};
use std::{hint::black_box, time::Instant};
fn main() {
    let frames: usize = std::env::args()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(20)
        .max(1);
    let mut source = Bitmap::new(160, 128).unwrap();
    source.clear(Color::rgba(100, 180, 230, 150));
    for i in 0..8 {
        source.fill_rect(
            Rect::new(i * 20, 0, 10, 128),
            Color::rgba(170, 20, 110, 180),
        );
    }
    let mut pattern = Bitmap::new(16, 16).unwrap();
    pattern.clear(Color::rgba(160, 20, 110, 110));
    let fx = SpriteFx {
        opacity: 195,
        color: Color::rgba(20, 80, 160, 40),
        tone: Tone {
            red: 12,
            green: -7,
            blue: 3,
            gray: 75,
        },
        pattern: Some(&pattern),
        ..Default::default()
    };
    let mut cpu = Bitmap::new(640, 480).unwrap();
    let start = Instant::now();
    for frame in 0..frames {
        cpu.clear(Color::rgb(0, 0, 0));
        for i in 0..80 {
            let dst = Rect::new(
                ((i * 37 + frame * 2) % 620) as i32 - 60,
                ((i * 29) % 460) as i32 - 60,
                160,
                128,
            );
            cpu.draw_sprite(dst, &source, source.rect(), cpu.rect(), &fx);
        }
        black_box(cpu.pixels());
    }
    let cpu_ms = start.elapsed().as_secs_f64() * 1000.0 / frames as f64;
    let copies = 2000;
    let large = Bitmap::new(512, 512).unwrap();
    let start = Instant::now();
    for _ in 0..copies {
        black_box(large.clone());
    }
    let clone_ns = start.elapsed().as_nanos() as f64 / copies as f64;
    println!("software_ms_per_frame={cpu_ms:.3}\nbitmap_clone_ns={clone_ns:.1}\nframes={frames}");
    #[cfg(feature = "gpu")]
    {
        use rsxp::{gpu::GpuRenderer, graphics::SpriteTransform};
        let mut gpu = GpuRenderer::new(640, 480).expect("wgpu adapter for benchmark");
        let draw = |gpu: &mut GpuRenderer, frame: usize| {
            gpu.begin(640, 480).unwrap();
            for i in 0..80 {
                let tr = SpriteTransform {
                    x: ((i * 37 + frame * 2) % 620) as f64 - 60.0,
                    y: ((i * 29) % 460) as f64 - 60.0,
                    ..Default::default()
                };
                gpu.sprite(
                    1,
                    &source,
                    source.rect(),
                    cpu.rect(),
                    tr,
                    (0, 0),
                    &fx,
                    2,
                    false,
                )
                .unwrap();
            }
            gpu.submit().unwrap();
        };
        draw(&mut gpu, 0);
        let _ = gpu.readback().unwrap();
        let uploaded = gpu.uploaded_bytes;
        let start = Instant::now();
        for frame in 0..frames {
            draw(&mut gpu, frame);
        }
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let gpu_ms = start.elapsed().as_secs_f64() * 1000.0 / frames as f64;
        let result = gpu.readback().unwrap();
        let max_difference = cpu
            .pixels()
            .iter()
            .zip(result.pixels())
            .flat_map(|(&a, &b)| {
                [0, 8, 16, 24]
                    .map(move |s| (((a >> s) & 255) as i32 - ((b >> s) & 255) as i32).abs())
            })
            .max()
            .unwrap();
        println!("adapter={}\nwgpu_ms_per_frame={gpu_ms:.3}\ncompositor_speedup={:.2}x\nmax_channel_difference={max_difference}\nuploaded_bytes_after_warmup={}",gpu.adapter_name,cpu_ms/gpu_ms,gpu.uploaded_bytes-uploaded);
        assert!(
            max_difference <= 3,
            "GPU output differs from software reference"
        );
    }
}
