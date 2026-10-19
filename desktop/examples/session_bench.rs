// SPDX-License-Identifier: GPL-2.0-or-later
//! `session-bench`: compares the isolated runner transport with an identical
//! scene, GPU workload and control stream.
//!
//! Usage: `cargo run --release --example session-bench -- /path/to/rsxp
//! [SCRIPT] [FRAME_LIMIT]`. The example spawns the given `rsxp` binary with
//! `run --script ... --renderer wgpu --frames N` and `RSXP_SESSION_ADDR`/
//! `RSXP_SESSION_TOKEN` pointing at a local listener (`RSXP_API=0`), then drives
//! the length-prefixed JSON session protocol: it answers `poll` messages and
//! consumes one frame packet per frame, applying GPU command frames or uploading
//! pixels as a baseline. The first 60 frames are warmup and excluded, and the
//! GPU is fenced at both measurement edges. It prints a JSON summary (frames,
//! seconds, fps, ms/frame, payload bytes) plus any `BENCH`/`GPU:` diagnostics.
//! When no script is given it generates a temporary sprite-spinning script.
use rsxp_core::gpu::{GpuFrame, GpuRenderer};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
fn read(stream: &mut TcpStream) -> Result<Value, Box<dyn std::error::Error>> {
    let mut length = [0; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    if length > 16 * 1024 * 1024 {
        return Err("Message exceeds transport limit".into());
    }
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}
fn write(stream: &mut TcpStream, value: Value) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(&value)?;
    stream.write_all(&(bytes.len() as u32).to_le_bytes())?;
    stream.write_all(&bytes)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let binary = args
        .next()
        .ok_or("usage: session-bench /path/to/rsxp [script frame-limit]")?;
    let custom_script = args.next().map(std::path::PathBuf::from);
    let limit = args
        .next()
        .map(|v| v.to_string_lossy().parse::<u32>())
        .transpose()?
        .unwrap_or(360);
    let generated =
        std::env::temp_dir().join(format!("rsxp-session-bench-{}.rb", std::process::id()));
    std::fs::write(
        &generated,
        r#"
bitmap=Bitmap.new(32,32)
bitmap.fill_rect(0,0,32,32,Color.new(40,180,120))
bitmap.fill_rect(4,4,24,24,Color.new(120,210,180))
sprites=Array.new(160) do |i|
  s=Sprite.new; s.bitmap=bitmap; s.x=(i%20)*32; s.y=(i/20)*48; s
end
360.times do |frame|
  sprites.each_with_index { |s,i| s.x=(i%20)*32+(frame%8) }
  Graphics.update
  Input.update
end
"#,
    )?;
    let script = custom_script.clone().unwrap_or_else(|| generated.clone());
    if limit <= 60 {
        return Err("frame limit must exceed the 60-frame warmup".into());
    }
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    listener.set_nonblocking(true)?;
    let mut child = Command::new(binary)
        .args([
            "run",
            "--script",
            script.to_str().unwrap(),
            "--renderer",
            "wgpu",
        ])
        .args(["--frames", &limit.to_string()])
        .env("RSXP_SESSION_ADDR", listener.local_addr()?.to_string())
        .env("RSXP_SESSION_TOKEN", "local-benchmark")
        .env("RSXP_API", "0")
        .env(
            "RSXP_PROFILE",
            std::env::var("RSXP_PROFILE").unwrap_or_else(|_| "0".into()),
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let stdout = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = std::io::Read::read_to_end(&mut std::io::BufReader::new(stdout), &mut bytes);
        bytes
    });
    let stderr = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = std::io::Read::read_to_end(&mut std::io::BufReader::new(stderr), &mut bytes);
        bytes
    });
    let mut gpu = GpuRenderer::new(640, 480)?;
    let start = Instant::now();
    let mut stream = loop {
        match listener.accept() {
            Ok((s, _)) => break s,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if start.elapsed() > Duration::from_secs(30) || child.try_wait()?.is_some() {
                    let _ = child.kill();
                    return Err("Runner did not connect".into());
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(e) => return Err(e.into()),
        }
    };
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    if read(&mut stream)?["token"] != "local-benchmark" {
        return Err("Bad handshake".into());
    }
    let mut pixels = Vec::new();
    let mut upload = None;
    let mut frames = 0;
    let mut bytes_received = 0;
    let mut timed = None;
    while frames < limit {
        let header = read(&mut stream)?;
        if header["poll"] == true {
            write(&mut stream, json!({"fast":true,"keys":[],"api":""}))?;
            continue;
        }
        let width = header["width"].as_u64().ok_or("missing width")? as u32;
        let height = header["height"].as_u64().ok_or("missing height")? as u32;
        let len = header["gpu_bytes"]
            .as_u64()
            .map_or(width as usize * height as usize * 4, |v| v as usize);
        if len > 512 * 1024 * 1024 {
            return Err("Packet too large".into());
        }
        pixels.resize(len, 0);
        stream.read_exact(&mut pixels)?;
        if header["gpu_bytes"].is_number() {
            gpu.apply_remote_frame(GpuFrame::decode(&pixels)?)?;
        } else {
            if upload
                .as_ref()
                .is_none_or(|b: &wgpu::Buffer| b.size() != len as u64)
            {
                upload = Some(gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("baseline framebuffer upload"),
                    size: len as u64,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
            }
            gpu.queue.write_buffer(upload.as_ref().unwrap(), 0, &pixels);
            gpu.queue.submit([]);
        }
        frames += 1;
        if frames == 60 {
            gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
            timed = Some(Instant::now());
        }
        if frames > 60 {
            bytes_received += len;
        }
    }
    gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
    let seconds = timed.unwrap().elapsed().as_secs_f64();
    drop(stream);
    let status = child.wait()?;
    let stdout = stdout.join().map_err(|_| "stdout collector failed")?;
    let stderr = stderr.join().map_err(|_| "stderr collector failed")?;
    let _ = std::fs::remove_file(generated);
    let measured = limit - 60;
    println!(
        "{}",
        json!({"frames":measured,"seconds":seconds,"fps":f64::from(measured)/seconds,"ms_per_frame":seconds*1000.0/f64::from(measured),"payload_bytes_per_frame":bytes_received/measured as usize,"adapter":gpu.adapter_name,"runner_success":status.success()})
    );
    for line in String::from_utf8_lossy(&stdout)
        .lines()
        .filter(|l| l.contains("BENCH"))
    {
        println!("{line}");
    }
    for line in String::from_utf8_lossy(&stderr)
        .lines()
        .filter(|l| l.starts_with("GPU:"))
    {
        println!("{line}");
    }
    Ok(())
}
