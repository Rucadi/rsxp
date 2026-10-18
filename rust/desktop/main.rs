// SPDX-License-Identifier: GPL-2.0-or-later
mod audio;
#[path = "../bindings/mod.rs"]
mod bindings;
mod bundled_runtime;
mod controller;
#[cfg(feature = "gpu")]
mod dashboard;
#[cfg(feature = "gpu")]
mod gpu_present;
mod host;
#[cfg(feature = "gpu")]
mod library;
mod media;
mod profiling;
mod runtime;
#[cfg(feature = "gpu")]
mod saves;
mod session;
mod window;
use rsxp::{
    archive::Archive,
    graphics::{Bitmap, Color, Rect, Scene, Sprite},
    input::Input,
};
use std::{
    error::Error,
    fs,
    io::{self, Write},
    path::PathBuf,
    time::{Duration, Instant},
};

const HELP: &str = "rsxp — standalone Rust engine prototype

Usage:
  rsxp                         Open the game dashboard
  rsxp [demo] [--headless] [--frames N] [--output frame.ppm]
  rsxp archive-list ARCHIVE
  rsxp archive-cat ARCHIVE ENTRY
  rsxp run --script FILE [--rgss 1|2|3] [--headless] [--frames N] [--output frame.ppm]
  rsxp run --game DIRECTORY [--rgss 1|2|3] [--headless] [--frames N] [--output frame.ppm]
  rsxp --help
  rsxp --version

Arrow keys move the demo sprite; Escape quits the demo. Headless demo mode
defaults to one frame; Ruby scripts run to completion or --frames N updates.
--output saves the last frame as a PPM image.
archive-cat writes decrypted bytes to stdout. Use shell redirection to save.

run options: --trace-scripts logs each script section. Broken `class X < X`
self-inheritance in Data/PluginScripts.rxdata is repaired in memory
automatically. Frame pacing (uncapped mode and the FPS cap, default 40 for RGSS1
and 60 otherwise) and a full game reset are configured at runtime from the
workspace toolbar. Gameplay starts with the UI hidden. F1 toggles the workspace, F2 tools,
F3 output, F4 fullscreen, F10 performance and F11 uncapped mode. Fullscreen
entry hides the UI; F1 can reveal it again.
--profile NAME is accepted for compatibility.
--ruby-profile FILE records sampled Ruby stacks and writes FILE, FILE.txt and
FILE.json. --ruby-profile-start-frame N skips N Graphics updates;
--ruby-profile-frames N records N updates, then saves while the game continues.
--ruby-profile-mode cpu|wall defaults to cpu; --ruby-profile-interval US
defaults to 1000 microseconds. The development shell includes StackProf.
--renderer auto|wgpu|software selects the desktop compositor (default auto).
Auto uses the wgpu compositor; software keeps RGSS composition on the CPU while
still presenting through wgpu. Headless runs use software unless wgpu is
explicitly requested. RSXP_RENDERER also sets the default. Build with
--release for game performance.
RSXP_INPUT=FRAME:SCANCODE[:HOLD][,...] injects scripted input for
headless runs and tests.
Runtime patches are toggled from the left panel of the workspace and applied
live to the running game (Map panoramic mode shows more of the current map).
RSXP_PATCH=ID[,ID...] pre-enables patches at startup.

The run command embeds MRI Ruby through Magnus. Core RGSS graphics, input,
and Table bindings are available; full game compatibility is still in progress.
See docs/rust-port.md for supported APIs and remaining limitations.";

fn archive_command(args: &[String]) -> Result<(), Box<dyn Error>> {
    let cat = args[0] == "archive-cat";
    if args.len() != if cat { 3 } else { 2 } {
        return Err("expected ARCHIVE and, for archive-cat, ENTRY".into());
    }
    let bytes = fs::read(&args[1])?;
    let archive = Archive::parse(&bytes)?;
    let stdout = io::stdout();
    let mut output = stdout.lock();
    if cat {
        let index = archive
            .entries()
            .iter()
            .position(|e| e.name == args[2].as_bytes())
            .ok_or("entry not found")?;
        // Bound transient allocation while streaming large archive entries.
        let len = archive.entries()[index].len();
        for offset in (0..len).step_by(64 * 1024) {
            output.write_all(&archive.read(index, offset, 64 * 1024)?)?;
        }
    } else {
        for entry in archive.entries() {
            writeln!(
                output,
                "{}\t{}",
                entry.len(),
                String::from_utf8_lossy(&entry.name)
            )?;
        }
    }
    Ok(())
}

fn demo(args: &[String]) -> Result<(), Box<dyn Error>> {
    let mut headless = false;
    let mut frames = None;
    let mut output = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--headless" => headless = true,
            "--frames" => {
                let count: u64 = args
                    .next()
                    .ok_or("--frames requires a positive integer")?
                    .parse()?;
                if count == 0 {
                    return Err("--frames must be greater than zero".into());
                }
                frames = Some(count);
            }
            "--output" => {
                output = Some(PathBuf::from(
                    args.next().ok_or("--output requires a filename")?,
                ))
            }
            other => return Err(format!("unknown argument: {other}; run --help for usage").into()),
        }
    }
    if headless && frames.is_none() {
        frames = Some(1);
    }
    let (width, height) = (640, 480);
    let mut target = Bitmap::new(width, height)?;
    let mut scene = Scene::default();
    let mut background = Bitmap::new(width, height)?;
    background.clear(Color::rgb(22, 28, 44));
    for y in (0..height).step_by(32) {
        for x in (0..width).step_by(32) {
            if (x / 32 + y / 32) % 2 == 0 {
                background.fill_rect(
                    Rect::new(x as i32, y as i32, 32, 32),
                    Color::rgb(30, 38, 58),
                );
            }
        }
    }
    scene.sprites.push(Sprite::new(background));
    let mut tile = Bitmap::new(48, 48)?;
    tile.fill_rect(Rect::new(0, 0, 48, 48), Color::rgb(64, 208, 160));
    tile.fill_rect(Rect::new(8, 8, 32, 32), Color::rgb(210, 255, 230));
    tile.fill_rect(Rect::new(14, 14, 8, 8), Color::rgb(22, 28, 44));
    tile.fill_rect(Rect::new(28, 14, 8, 8), Color::rgb(22, 28, 44));
    let mut player = Sprite::new(tile.clone());
    player.x = 296;
    player.y = 216;
    player.z = 2;
    let mut orbit = Sprite::new(tile);
    orbit.z = 1;
    orbit.opacity = 140;
    orbit.width = 80;
    orbit.height = 80;
    scene.sprites.extend([orbit, player]);
    let mut input = Input::default();
    let mut window = if headless {
        None
    } else {
        Some(window::Window::new(width, height)?)
    };
    #[cfg(feature = "gpu")]
    let (gpu, mut display) = if let Some(window) = &window {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let surface = instance.create_surface(window.native())?;
        let gpu = rsxp::gpu::GpuRenderer::new_for_surface(width, height, instance, &surface)?;
        let display = crate::gpu_present::GpuDisplay::new(&gpu, surface, window.physical_size())?;
        (Some(gpu), Some(display))
    } else {
        (None, None)
    };
    #[cfg(not(feature = "gpu"))]
    if !headless {
        return Err("desktop presentation requires the gpu feature".into());
    }
    let frame_duration = Duration::from_nanos(1_000_000_000 / 60);
    let mut frame = 0u64;
    #[cfg(feature = "gpu")]
    let mut global_fullscreen = false;
    loop {
        let start = Instant::now();
        input.begin_frame();
        if let Some(window) = &mut window
            && !window.poll(&mut input, true)
        {
            break;
        }
        let player = &mut scene.sprites[2];
        player.x = (player.x + 3 * (i32::from(input.pressed(79)) - i32::from(input.pressed(80))))
            .clamp(0, width as i32 - 48);
        player.y = (player.y + 3 * (i32::from(input.pressed(81)) - i32::from(input.pressed(82))))
            .clamp(0, height as i32 - 48);
        let angle = frame as f64 / 60.0;
        scene.sprites[1].x = 280 + (angle.cos() * 150.0) as i32;
        scene.sprites[1].y = 200 + (angle.sin() * 100.0) as i32;
        scene.render(&mut target, Color::rgb(22, 28, 44));
        #[cfg(feature = "gpu")]
        if let (Some(display), Some(gpu), Some(window)) = (&mut display, &gpu, &mut window) {
            let input = window.take_egui_input();
            let context = window.egui_context().clone();
            let output = display.present_bitmap(
                gpu,
                &target,
                crate::gpu_present::Presentation {
                    size: window.physical_size(),
                    context: &context,
                    input,
                    global_fullscreen,
                    profile: None,
                },
            )?;
            window.set_game_pointer_rect(display.game_pointer_rect());
            window.handle_egui_platform_output(output);
            if let Some(enabled) = display.take_global_fullscreen_request() {
                window.fullscreen(enabled)?;
                global_fullscreen = enabled;
            }
        }
        frame += 1;
        if frames.is_some_and(|limit| frame >= limit) {
            break;
        }
        if !headless {
            std::thread::sleep(frame_duration.saturating_sub(start.elapsed()));
        }
    }
    if let Some(path) = output {
        let mut writer = io::BufWriter::new(fs::File::create(path)?);
        target.write_ppm(&mut writer)?;
        writer.flush()?;
    }
    Ok(())
}

fn run() -> Result<u8, Box<dyn Error>> {
    bundled_runtime::configure()?;
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--help" | "-h") => println!("{HELP}"),
        Some("--version" | "-V") => println!("rsxp {}", env!("CARGO_PKG_VERSION")),
        Some("archive-list" | "archive-cat") => archive_command(&args)?,
        Some("demo") => demo(&args[1..])?,
        Some("run") => {
            runtime::validate(&args[1..])?;
            #[cfg(feature = "gpu")]
            if !args.iter().any(|a| a == "--headless")
                && std::env::var_os("RSXP_SESSION_ADDR").is_none()
            {
                return dashboard::run(Some(args[1..].to_vec()));
            }
            return runtime::run(&args[1..]);
        }
        #[cfg(feature = "gpu")]
        None => return dashboard::run(None),
        _ => demo(&args)?,
    }
    Ok(0)
}
fn main() -> std::process::ExitCode {
    match run() {
        Ok(status) => std::process::ExitCode::from(status),
        Err(error) => {
            eprintln!("rsxp: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
