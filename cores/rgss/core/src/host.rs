// SPDX-License-Identifier: GPL-2.0-or-later
//! RGSS `Host` implementation backed by the generic core sinks.

use rsxp::graphics::{Bitmap, Color};
use rsxp::host::{AudioBackend, AudioChannel as RgssChannel, Host};
use rsxp::input::Input;
use rsxp_core::{
    AudioChannel, AudioStream, CoreContext, CoreEvent, CoreLoadInfo, InputEvent, PixelFormat,
    VideoFrame, WindowCommand,
};

/// VideoSink packet kind for the RGSS encoded GPU frame (`GpuFrame::encode`).
pub(crate) const GPU_PACKET_KIND: &str = "rgss-gpu-v1";

pub(crate) struct CoreHost {
    context: CoreContext,
    gpu: Option<rsxp::gpu::GpuRenderer>,
    events: Vec<InputEvent>,
}

impl CoreHost {
    pub(crate) fn new(context: CoreContext, width: u32, height: u32) -> Result<Self, String> {
        let renderer = std::env::var("RSXP_RENDERER").unwrap_or_else(|_| "auto".into());
        let gpu = if renderer == "software" {
            None
        } else {
            match rsxp::gpu::GpuRenderer::new(width, height) {
                Ok(mut gpu) => {
                    // Frontends receive portable packets, never process-local
                    // GPU handles.
                    gpu.enable_remote();
                    Some(gpu)
                }
                Err(error) if renderer == "wgpu" => return Err(error),
                Err(error) => {
                    eprintln!(
                        "RGSS GPU renderer unavailable ({error}); using software composition"
                    );
                    None
                }
            }
        };
        Ok(Self {
            context,
            gpu,
            events: Vec::with_capacity(32),
        })
    }

    pub(crate) fn emit_loaded(&mut self, width: u32, height: u32) {
        self.context.events.emit(CoreEvent::Loaded(CoreLoadInfo {
            width,
            height,
            sample_rate: 48_000,
            audio_channels: 2,
        }));
    }

    fn window(&mut self, command: WindowCommand) {
        self.context.events.emit(CoreEvent::Window(command));
    }
}

fn audio_channel(channel: RgssChannel) -> AudioChannel {
    match channel {
        RgssChannel::Bgm => AudioChannel::MUSIC,
        RgssChannel::Bgs => AudioChannel::AMBIENT,
        RgssChannel::Me => AudioChannel::VOICE,
        RgssChannel::Se => AudioChannel::EFFECT,
    }
}

impl Host for CoreHost {
    fn audio(&mut self) -> Option<&mut dyn AudioBackend> {
        Some(self)
    }

    fn gpu(&mut self) -> Option<&mut rsxp::gpu::GpuRenderer> {
        self.gpu.as_mut()
    }

    fn present_gpu(&mut self) -> Result<(), String> {
        let gpu = self
            .gpu
            .as_mut()
            .ok_or_else(|| "GPU frame has no owning renderer".to_string())?;
        let encoded = gpu.take_remote_frame()?.encode();
        if self.context.video.present_packet(GPU_PACKET_KIND, &encoded) {
            return Ok(());
        }
        // The frontend does not understand this core's acceleration packet.
        // Materialize once so the game remains visible instead of dropping the
        // frame; cores that advertise a CPU fallback must keep this path valid.
        let bitmap = self
            .gpu
            .as_mut()
            .ok_or_else(|| "GPU renderer disappeared".to_string())?
            .readback()?;
        self.present(&bitmap)
    }

    fn read_file(&self, path: &str) -> Option<Vec<u8>> {
        self.context.platform.read_content(path).ok()
    }

    fn image(&self, bytes: &[u8]) -> Result<Bitmap, String> {
        rsxp::portable_media::image(bytes)
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
        rsxp::portable_media::text(font, size, bold, italic, text, color, render)
    }

    fn poll(&mut self, input: &mut Input) -> bool {
        let control = &self.context.control;
        rsxp::runtime_settings::set_fast_forward(control.fast_forward());
        control.input().take_events(&mut self.events);
        for event in self.events.drain(..) {
            match event {
                InputEvent::Key { code, down, .. } => {
                    let code = i32::try_from(code).unwrap_or(i32::MAX);
                    if down {
                        input.key_down(code);
                    } else {
                        input.key_up(code);
                    }
                }
                InputEvent::Text(text) => input.push_text(&text),
                InputEvent::PointerMove { x, y, inside } => {
                    input.mouse_x = x;
                    input.mouse_y = y;
                    input.mouse_inside = inside;
                }
                InputEvent::Wheel { y, .. } => input.scroll_v += y,
                InputEvent::PointerButton { .. } | InputEvent::Focus(_) => {}
            }
        }
        !control.should_stop()
    }

    fn present(&mut self, frame: &Bitmap) -> Result<(), String> {
        // Bitmap pixels are packed ARGB in a native-endian u32. On little-endian
        // machines their byte order is BGRA; on big-endian it is RGBA. The cast
        // is zero-copy and the core reuses the bitmap only after this call.
        let pixels: &[u8] = bytemuck::cast_slice(frame.pixels());
        let format = if cfg!(target_endian = "little") {
            PixelFormat::Bgra8
        } else {
            PixelFormat::Rgba8
        };
        let width = frame.width();
        let height = frame.height();
        self.context.video.present(VideoFrame::new(
            width,
            height,
            width as usize * 4,
            format,
            pixels,
        ));
        Ok(())
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<(), String> {
        self.window(WindowCommand::Resize { width, height });
        Ok(())
    }

    fn set_title(&mut self, title: &str) {
        self.window(WindowCommand::SetTitle(title.to_owned()));
    }

    fn set_fullscreen(&mut self, enabled: bool) -> Result<(), String> {
        self.window(WindowCommand::SetFullscreen(enabled));
        Ok(())
    }

    fn set_scale(&mut self, scale: f64) {
        self.window(WindowCommand::SetScale(scale));
    }

    fn center(&mut self) {
        self.window(WindowCommand::Center);
    }

    fn show_cursor(&mut self, shown: bool) {
        self.window(WindowCommand::ShowCursor(shown));
    }

    fn text_input(&mut self, enabled: bool) {
        self.window(WindowCommand::TextInput(enabled));
    }

    fn now(&self) -> f64 {
        self.context.platform.now()
    }

    fn sleep(&self, duration: std::time::Duration) {
        self.context.platform.sleep(duration);
    }
}

impl AudioBackend for CoreHost {
    fn play(
        &mut self,
        channel: RgssChannel,
        path: &str,
        bytes: &[u8],
        volume: i32,
        pitch: f64,
        position: f64,
    ) {
        self.context.audio.play_stream(AudioStream {
            channel: audio_channel(channel),
            path,
            data: bytes,
            looping: matches!(channel, RgssChannel::Bgm | RgssChannel::Bgs),
            volume,
            pitch,
            position,
        });
    }

    fn stop(&mut self, channel: RgssChannel) {
        self.context.audio.stop(audio_channel(channel));
    }

    fn fade(&mut self, channel: RgssChannel, ms: i32) {
        self.context.audio.fade(audio_channel(channel), ms);
    }

    fn position(&self, channel: RgssChannel) -> f64 {
        self.context.audio.position(audio_channel(channel))
    }

    fn set_volume(&mut self, channel: RgssChannel, volume: i32) {
        self.context
            .audio
            .set_volume(audio_channel(channel), volume);
    }

    fn volume(&self, channel: RgssChannel) -> i32 {
        self.context.audio.volume(audio_channel(channel))
    }
}
