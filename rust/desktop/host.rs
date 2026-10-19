// SPDX-License-Identifier: GPL-2.0-or-later
//! Pure-Rust desktop host: winit windows, wgpu presentation and Rust media APIs.
//!
//! [`DesktopHost`] implements [`rsxp::host::Host`], supplying the engine with
//! windowing, input, audio, image/text rendering, timing, and presentation.
//! It runs in one of three modes chosen by `new`:
//! - a local windowed session with a winit [`Window`], a wgpu surface and a
//!   [`crate::gpu_present::GpuDisplay`];
//! - a headless session (no window, silent audio) for scripted runs;
//! - a remote *runner* connected back to the dashboard over TCP when
//!   `RSXP_SESSION_ADDR` is set (see [`crate::session::Runner`]), in which case
//!   it owns no window and streams frames to the dashboard.
//!
//! The `renderer` argument (`auto`, `wgpu`, or `software`) selects composition:
//! `software` keeps RGSS composition on the CPU but still presents through
//! wgpu, while `auto`/`wgpu` use the GPU compositor. Headless runs default to
//! software unless `wgpu` is requested explicitly. The `gpu` feature gates all
//! wgpu/window code paths; without it only headless and remote modes compile.
//!
//! Notable behavior: [`case_path`] resolves paths case-insensitively so
//! Windows-authored game data loads on case-sensitive filesystems; when
//! `RSXP_RECORD` is set, every file read is appended (slash-normalized) to a
//! process-global log via a thread-local writer; a paused remote runner mutes
//! all four audio channels and restores their volumes on resume; and
//! [`DesktopHost`] merges its render/present timings into
//! [`rsxp::host::ProfileStats`]. The GPU surface is declared before the window
//! and device so it is dropped first.
use crate::media;
use crate::window::Window;
use rsxp::graphics::{Bitmap, Color};
use rsxp::host::Host;
use rsxp::input::Input;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Resolves a path case-insensitively on case-sensitive filesystems, as the
/// original engine does on Windows-authored game data.
pub fn case_path(path: &Path) -> Option<PathBuf> {
    if path.is_file() {
        return Some(path.to_owned());
    }
    let mut current = PathBuf::new();
    for part in path.components() {
        let direct = current.join(part);
        if direct.exists() {
            current = direct;
            continue;
        }
        let name = part.as_os_str().to_string_lossy();
        let base = if current.as_os_str().is_empty() {
            Path::new(".")
        } else {
            &current
        };
        let matched = fs::read_dir(base)
            .ok()?
            .filter_map(Result::ok)
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&name)
            })?;
        current = matched.path();
    }
    current.is_file().then_some(current)
}

pub struct DesktopHost {
    runner: Option<crate::session::Runner>,
    paused_volumes: Option<[i32; 4]>,
    // The surface must be dropped before its window and GPU device.
    #[cfg(feature = "gpu")]
    display: Option<crate::gpu_present::GpuDisplay>,
    #[cfg(feature = "gpu")]
    gpu: Option<rsxp::gpu::GpuRenderer>,
    window: Option<Window>,
    #[cfg(feature = "gpu")]
    gpu_composition: bool,
    profile_render_ms: f64,
    profile_present_ms: f64,
    global_fullscreen: bool,
    start: std::time::Instant,
    audio: crate::audio::DesktopAudio,
}

thread_local! {
    static RECORD: std::cell::RefCell<Option<std::io::BufWriter<std::fs::File>>> =
        const { std::cell::RefCell::new(None) };
}

impl DesktopHost {
    pub fn new(width: u32, height: u32, headless: bool, renderer: &str) -> Result<Self, String> {
        if let Ok(path) = std::env::var("RSXP_RECORD") {
            let file = std::fs::File::create(path).map_err(|error| error.to_string())?;
            RECORD.with(|slot| *slot.borrow_mut() = Some(std::io::BufWriter::new(file)));
        }
        if !["auto", "software", "wgpu"].contains(&renderer) {
            return Err("renderer must be auto, software, or wgpu".into());
        }
        let runner = crate::session::Runner::connect()?;
        let remote = runner.is_some();
        let audio = if headless {
            crate::audio::DesktopAudio::silent()
        } else {
            crate::audio::DesktopAudio::new()
        };

        #[cfg(feature = "gpu")]
        {
            let window = if headless || remote {
                None
            } else {
                Some(Window::new(width, height)?)
            };
            let (gpu, display) = if let Some(window) = &window {
                let instance = wgpu::Instance::new(
                    wgpu::InstanceDescriptor::new_with_display_handle(Box::new(window.native())),
                );
                let surface = instance
                    .create_surface(window.native())
                    .map_err(|error| error.to_string())?;
                let gpu =
                    rsxp::gpu::GpuRenderer::new_for_surface(width, height, instance, &surface)?;
                let display =
                    crate::gpu_present::GpuDisplay::new(&gpu, surface, window.physical_size())?;
                eprintln!("Renderer: wgpu ({})", gpu.adapter_name);
                (Some(gpu), Some(display))
            } else if renderer == "wgpu" || (remote && renderer != "software") {
                let mut gpu = rsxp::gpu::GpuRenderer::new(width, height)?;
                if remote {
                    gpu.enable_remote();
                }
                eprintln!("Renderer: wgpu ({})", gpu.adapter_name);
                (Some(gpu), None)
            } else {
                (None, None)
            };
            Ok(Self {
                runner,
                paused_volumes: None,
                display,
                gpu,
                window,
                gpu_composition: renderer != "software",
                profile_render_ms: 0.0,
                profile_present_ms: 0.0,
                global_fullscreen: false,
                start: std::time::Instant::now(),
                audio,
            })
        }

        #[cfg(not(feature = "gpu"))]
        {
            if !headless && !remote {
                return Err("desktop presentation requires the gpu feature".into());
            }
            let _ = (width, height);
            Ok(Self {
                runner,
                paused_volumes: None,
                window: None,
                profile_render_ms: 0.0,
                profile_present_ms: 0.0,
                global_fullscreen: false,
                start: std::time::Instant::now(),
                audio,
            })
        }
    }

    #[cfg(feature = "gpu")]
    fn present_egui(&mut self, bitmap: Option<&Bitmap>) -> Result<(), String> {
        let profile = crate::runtime::profile_stats();
        let global_fullscreen = self.global_fullscreen;
        let Some(window) = self.window.as_mut() else {
            return Ok(());
        };
        let input = window.take_egui_input();
        let context = window.egui_context().clone();
        let (output, fullscreen_request, game_pointer_rect) = {
            let (Some(display), Some(gpu)) = (&mut self.display, &self.gpu) else {
                return Ok(());
            };
            let presentation = crate::gpu_present::Presentation {
                size: window.physical_size(),
                context: &context,
                input,
                global_fullscreen,
                profile,
            };
            let output = if let Some(bitmap) = bitmap {
                display.present_bitmap(gpu, bitmap, presentation)?
            } else {
                display.present(gpu, presentation)?
            };
            (
                output,
                display.take_global_fullscreen_request(),
                display.game_pointer_rect(),
            )
        };
        window.handle_egui_platform_output(output);
        window.set_game_pointer_rect(game_pointer_rect);
        if let Some(enabled) = fullscreen_request {
            window.fullscreen(enabled)?;
            self.global_fullscreen = enabled;
        }
        Ok(())
    }
}

impl Host for DesktopHost {
    fn audio(&mut self) -> Option<&mut dyn rsxp::host::AudioBackend> {
        Some(&mut self.audio)
    }

    fn profile_stats(&self) -> Option<rsxp::host::ProfileStats> {
        let mut profile = crate::runtime::profile_stats()?;
        profile.render_ms = self.profile_render_ms;
        profile.present_ms = self.profile_present_ms;
        Some(profile)
    }

    fn has_profiler_ui(&self) -> bool {
        self.window.is_some() || self.runner.is_some()
    }

    fn set_frame_profile(&mut self, render_ms: f64, present_ms: f64) {
        self.profile_render_ms = render_ms;
        self.profile_present_ms = present_ms;
    }

    fn profiler_enabled(&self) -> Option<bool> {
        if let Some(runner) = &self.runner {
            return Some(runner.profiler);
        }
        #[cfg(feature = "gpu")]
        {
            self.display
                .as_ref()
                .map(crate::gpu_present::GpuDisplay::profiler_visible)
        }
        #[cfg(not(feature = "gpu"))]
        {
            None
        }
    }

    #[cfg(feature = "gpu")]
    fn gpu(&mut self) -> Option<&mut rsxp::gpu::GpuRenderer> {
        if self.gpu_composition {
            self.gpu.as_mut()
        } else {
            None
        }
    }

    #[cfg(feature = "gpu")]
    fn present_gpu(&mut self) -> Result<(), String> {
        let profile = self.profile_stats();
        if let Some(runner) = &mut self.runner {
            let frame = self
                .gpu
                .as_mut()
                .ok_or("missing runner GPU")?
                .take_remote_frame()?;
            return runner.gpu_frame(frame, profile);
        }
        self.present_egui(None)
    }

    fn read_file(&self, path: &str) -> Option<Vec<u8>> {
        let found = case_path(Path::new(path)).and_then(|path| fs::read(path).ok());
        if found.is_some() {
            RECORD.with(|slot| {
                if let Some(writer) = slot.borrow_mut().as_mut() {
                    use std::io::Write;
                    let _ = writeln!(writer, "{}", path.replace('\\', "/"));
                }
            });
        }
        found
    }

    fn image(&self, bytes: &[u8]) -> Result<Bitmap, String> {
        media::image(bytes)
    }

    fn text(
        &self,
        font: Option<&[u8]>,
        size: i32,
        bold: bool,
        italic: bool,
        text: &str,
        color: Color,
        render: bool,
    ) -> Result<(u32, u32, Option<Bitmap>), String> {
        media::text(font, size, bold, italic, text, color, render)
    }

    fn poll(&mut self, input: &mut Input) -> bool {
        if let Some(runner) = &mut self.runner {
            use rsxp::host::{AudioBackend, AudioChannel};
            const CHANNELS: [AudioChannel; 4] = [
                AudioChannel::Bgm,
                AudioChannel::Bgs,
                AudioChannel::Me,
                AudioChannel::Se,
            ];
            loop {
                if !runner.poll(input) {
                    return false;
                }
                if runner.paused {
                    if self.paused_volumes.is_none() {
                        self.paused_volumes = Some(CHANNELS.map(|c| self.audio.volume(c)));
                        for c in CHANNELS {
                            self.audio.set_volume(c, 0);
                        }
                    }
                    std::thread::sleep(Duration::from_millis(16));
                } else {
                    if let Some(volumes) = self.paused_volumes.take() {
                        for (c, v) in CHANNELS.into_iter().zip(volumes) {
                            self.audio.set_volume(c, v);
                        }
                    }
                    return true;
                }
            }
        }
        self.window
            .as_mut()
            .is_none_or(|window| window.poll(input, false))
    }

    fn present(&mut self, frame: &Bitmap) -> Result<(), String> {
        let profile = self.profile_stats();
        if let Some(runner) = &mut self.runner {
            return runner.frame(frame, profile);
        }
        #[cfg(feature = "gpu")]
        if !self.gpu_composition {
            self.present_egui(Some(frame))?;
        }
        #[cfg(not(feature = "gpu"))]
        let _ = frame;
        Ok(())
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<(), String> {
        if let Some(window) = &mut self.window {
            window.resize_logical(width, height);
        }
        Ok(())
    }

    fn set_title(&mut self, title: &str) {
        if let Some(window) = &mut self.window {
            let _ = window.set_title(title);
        }
    }

    fn set_fullscreen(&mut self, enabled: bool) -> Result<(), String> {
        self.global_fullscreen = enabled;
        match &mut self.window {
            Some(window) => window.fullscreen(enabled),
            None => Ok(()),
        }
    }

    fn set_scale(&mut self, scale: f64) {
        if let Some(window) = &mut self.window {
            window.scale(scale);
        }
    }

    fn center(&mut self) {
        if let Some(window) = &mut self.window {
            window.center();
        }
    }

    fn show_cursor(&mut self, shown: bool) {
        if let Some(runner) = &mut self.runner {
            runner.cursor = shown;
        }
        if let Some(window) = &mut self.window {
            window.cursor(shown);
        }
    }

    fn text_input(&mut self, enabled: bool) {
        if let Some(runner) = &mut self.runner {
            runner.text_input = enabled;
        }
        if let Some(window) = &mut self.window {
            window.text_input(enabled);
        }
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }

    fn now(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }
}
