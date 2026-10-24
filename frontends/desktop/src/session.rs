// SPDX-License-Identifier: GPL-2.0-or-later
//! Desktop-only transport to an isolated "runner" process; the dashboard never
//! owns Ruby state itself.
//!
//! The dashboard selects TCP on non-Windows platforms and standard process
//! pipes on Windows. The runner enables the API relay and spawns a background
//! `rsxp-session-controls` thread that decodes control messages into a bounded
//! channel. If no dashboard transport is configured, connect returns `Ok(None)`.
//!
//! Frames are sent one-way: a JSON header from [`Runner::frame_header`] (fps/frame
//! counters, input/rate state and relay
//! replies) followed by the pixel payload. [`Runner::frame`] writes little-endian
//! pixel bytes (zero-copy `bytemuck` cast when the `gpu` feature is on and the target
//! is little-endian); with the `gpu` feature, [`Runner::gpu_frame`] instead writes an
//! encoded GPU frame and records `gpu_bytes`. [`write_message`]/[`read_message`] use a
//! 4-byte little-endian length prefix with a 16 MiB cap.
//!
//! [`Runner::poll`] sends a poll request and applies queued controls. Only the first
//! poll waits (up to 30s) for the handshake; afterward controls are drained without
//! blocking, so GPU packet decoding never forces a round trip on every game update.
//! [`Runner::apply_control`] maps JSON fields onto `Input` (keys, mouse, scroll, text),
//! runtime settings (fast-forward, rate override) and runtime patches, returning
//! `false` on a `stop` request or when the stream/peer is lost.
use serde_json::{json, Value};
use std::io::{Read, Write};
#[cfg(any(not(windows), test))]
use std::net::TcpStream;

pub trait WriteStream: Write + Send {}
impl<T: Write + Send> WriteStream for T {}

/// Input surface understood by the transport. The RGSS host implements this
/// for its engine input buffer; cartridge hosts use the same mapping without
/// linking the RGSS crate.
pub trait TransportInput {
    fn key_down(&mut self, code: i32);
    fn key_up(&mut self, code: i32);
    fn set_pointer(&mut self, x: i32, y: i32, inside: bool);
    fn add_scroll(&mut self, delta: i32);
    fn push_text(&mut self, text: &str);

    fn fast_forward(&mut self, _enabled: bool) {}
    fn rate_override(&mut self, _rate: Option<u32>) {}
    fn patch(&mut self, _id: &str, _enabled: bool, _value: Option<f64>) {}
    fn relay_api(&mut self, _request: &str) {}
}

#[cfg(feature = "rgss")]
impl TransportInput for rsxp::input::Input {
    fn key_down(&mut self, code: i32) {
        self.key_down(code);
    }
    fn key_up(&mut self, code: i32) {
        self.key_up(code);
    }
    fn set_pointer(&mut self, x: i32, y: i32, inside: bool) {
        self.mouse_x = x;
        self.mouse_y = y;
        self.mouse_inside = inside;
    }
    fn add_scroll(&mut self, delta: i32) {
        self.scroll_v += delta;
    }
    fn push_text(&mut self, text: &str) {
        self.push_text(text);
    }
    fn fast_forward(&mut self, enabled: bool) {
        rsxp::runtime_settings::set_fast_forward(enabled);
    }
    fn rate_override(&mut self, rate: Option<u32>) {
        if let Some(rate) = rate {
            rsxp::runtime_settings::set_rate_override(rate);
        }
    }
    fn patch(&mut self, id: &str, enabled: bool, value: Option<f64>) {
        rsxp::runtime_patch::set_enabled(id, enabled);
        if let Some(value) = value {
            rsxp::runtime_patch::set_value(id, value);
        }
    }
    fn relay_api(&mut self, request: &str) {
        rsxp::api_bridge::relay_requests(request.as_bytes());
    }
}

pub fn write_message<W: Write + ?Sized>(stream: &mut W, value: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    stream
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .and_then(|_| stream.write_all(&bytes))
        .and_then(|_| stream.flush())
        .map_err(|e| e.to_string())
}
pub fn read_message<R: Read + ?Sized>(stream: &mut R) -> Result<Value, String> {
    let mut length = [0; 4];
    stream.read_exact(&mut length).map_err(|e| e.to_string())?;
    let length = u32::from_le_bytes(length) as usize;
    if length > 16 * 1024 * 1024 {
        return Err("runner message exceeds limit".into());
    }
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

pub struct Runner {
    stream: Box<dyn WriteStream>,
    controls: std::sync::mpsc::Receiver<Result<Value, String>>,
    first_control: bool,
    pub paused: bool,
    pub performance: bool,
    pub text_input: bool,
    pub cursor: bool,
    reported_rate: u32,
    frames: u64,
    fps: f64,
    fps_frames: u32,
    fps_start: std::time::Instant,
}
impl Runner {
    pub fn connect() -> Result<Option<Self>, String> {
        #[cfg(windows)]
        let (stream, control_stream): (Box<dyn WriteStream>, Box<dyn Read + Send>) = {
            if std::env::var_os("RSXP_SESSION_STDIO").is_none() {
                return Ok(None);
            }
            (Box::new(std::io::stdout()), Box::new(std::io::stdin()))
        };
        #[cfg(not(windows))]
        let (stream, control_stream): (Box<dyn WriteStream>, Box<dyn Read + Send>) = {
            let Ok(address) = std::env::var("RSXP_SESSION_ADDR") else {
                return Ok(None);
            };
            let mut stream = TcpStream::connect(&address)
                .map_err(|e| format!("dashboard IPC connect to {address} failed: {e}"))?;
            stream
                .set_nodelay(true)
                .map_err(|e| format!("dashboard IPC set_nodelay failed: {e}"))?;
            let token = json!({"token": std::env::var("RSXP_SESSION_TOKEN").unwrap_or_default()});
            write_message(&mut stream, &token)
                .map_err(|error| format!("dashboard IPC handshake write failed: {error}"))?;
            let mut control_stream = TcpStream::connect(&address)
                .map_err(|e| format!("dashboard IPC control connect to {address} failed: {e}"))?;
            control_stream
                .set_nodelay(true)
                .map_err(|e| format!("dashboard IPC control set_nodelay failed: {e}"))?;
            write_message(&mut control_stream, &token).map_err(|error| {
                format!("dashboard IPC control handshake write failed: {error}")
            })?;
            (Box::new(stream), Box::new(control_stream))
        };
        let (send, controls) = std::sync::mpsc::sync_channel(8);
        std::thread::Builder::new()
            .name("rsxp-session-controls".into())
            .spawn(move || {
                let mut reader = control_stream;
                loop {
                    let value = read_message(&mut *reader)
                        .map_err(|error| format!("dashboard IPC control read failed: {error}"));
                    let failed = value.is_err();
                    if send.send(value).is_err() || failed {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        #[cfg(feature = "rgss")]
        rsxp::api_bridge::relay_enable();
        Ok(Some(Self {
            stream,
            controls,
            first_control: true,
            paused: false,
            performance: false,
            text_input: false,
            cursor: true,
            reported_rate: 0,
            frames: 0,
            fps: 0.0,
            fps_frames: 0,
            fps_start: std::time::Instant::now(),
        }))
    }
    fn frame_header(&mut self, width: u32, height: u32) -> Value {
        self.frames += 1;
        self.fps_frames += 1;
        if self.fps_start.elapsed().as_millis() >= 500 {
            self.fps = f64::from(self.fps_frames) / self.fps_start.elapsed().as_secs_f64();
            self.fps_frames = 0;
            self.fps_start = std::time::Instant::now();
        }
        #[cfg(feature = "rgss")]
        let replies = rsxp::api_bridge::relay_replies();
        #[cfg(not(feature = "rgss"))]
        let replies = Value::Null;
        #[cfg(feature = "rgss")]
        let reported_rate = if self.reported_rate == 0 {
            rsxp::runtime_settings::current_rate()
        } else {
            self.reported_rate
        };
        #[cfg(not(feature = "rgss"))]
        let reported_rate = self.reported_rate;
        json!({"fps":self.fps,"frames":self.frames,"text_input":self.text_input,"cursor":self.cursor,"width":width,"height":height,"rate":reported_rate,"replies":replies})
    }

    /// Writes a frame already converted to packed ARGB (native-endian `u32`).
    pub fn frame_argb(&mut self, width: u32, height: u32, pixels: &[u32]) -> Result<(), String> {
        let header = self.frame_header(width, height);
        write_message(&mut self.stream, &header)?;
        #[cfg(target_endian = "little")]
        self.stream
            .write_all(bytemuck::cast_slice(pixels))
            .map_err(|e| e.to_string())?;
        #[cfg(not(target_endian = "little"))]
        {
            let mut bytes = Vec::with_capacity(pixels.len() * 4);
            for pixel in pixels {
                bytes.extend_from_slice(&pixel.to_le_bytes());
            }
            self.stream.write_all(&bytes).map_err(|e| e.to_string())?;
        }
        self.stream.flush().map_err(|e| e.to_string())
    }

    #[cfg(feature = "rgss")]
    pub fn frame(&mut self, frame: &rsxp::graphics::Bitmap) -> Result<(), String> {
        self.frame_argb(frame.width(), frame.height(), frame.pixels())
    }

    #[cfg(all(feature = "gpu", feature = "rgss"))]
    pub fn gpu_frame(&mut self, frame: rsxp::gpu::GpuFrame) -> Result<(), String> {
        let (width, height) = frame.size();
        let bytes = frame.encode();
        let mut header = self.frame_header(width, height);
        header["gpu_bytes"] = json!(bytes.len());
        write_message(&mut self.stream, &header)?;
        self.stream
            .write_all(&bytes)
            .and_then(|_| self.stream.flush())
            .map_err(|e| e.to_string())
    }
    pub fn poll(&mut self, input: &mut impl TransportInput) -> bool {
        // The first poll also announces readiness before a frame exists.
        if write_message(&mut self.stream, &json!({"poll":true})).is_err() {
            return false;
        }
        // Wait only for the initial handshake. Afterwards, controls arrive on
        // their own reader thread; GPU packet decoding in the dashboard must
        // never impose a round trip on every game update. Drain every reply so
        // one-shot input and inspector requests are applied exactly once.
        if self.first_control {
            self.first_control = false;
            let Ok(Ok(value)) = self
                .controls
                .recv_timeout(std::time::Duration::from_secs(30))
            else {
                return false;
            };
            if !self.apply_control(input, value) {
                return false;
            }
        }
        loop {
            match self.controls.try_recv() {
                Ok(Ok(value)) => {
                    if !self.apply_control(input, value) {
                        return false;
                    }
                }
                Ok(Err(_)) | Err(std::sync::mpsc::TryRecvError::Disconnected) => return false,
                Err(std::sync::mpsc::TryRecvError::Empty) => return true,
            }
        }
    }
    fn apply_control(&mut self, input: &mut impl TransportInput, value: Value) -> bool {
        self.paused = value["paused"].as_bool().unwrap_or(false);
        self.performance = value["performance"].as_bool().unwrap_or(false);
        if value["stop"].as_bool() == Some(true) {
            return false;
        }
        let keys: Vec<i32> = value["keys"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_i64().map(|v| v as i32))
            .collect();
        for field in ["downs", "ups"] {
            if let Some(events) = value[field].as_array() {
                for key in events.iter().filter_map(|v| v.as_i64()) {
                    if field == "downs" {
                        input.key_down(key as i32);
                    } else {
                        input.key_up(key as i32);
                    }
                }
            }
        }
        for key in -8..512 {
            if keys.contains(&key) {
                input.key_down(key);
            } else {
                input.key_up(key);
            }
        }
        input.set_pointer(
            value["x"].as_i64().unwrap_or(0) as i32,
            value["y"].as_i64().unwrap_or(0) as i32,
            value["inside"].as_bool().unwrap_or(false),
        );
        input.add_scroll(value["scroll"].as_i64().unwrap_or(0) as i32);
        input.push_text(value["text"].as_str().unwrap_or(""));
        input.fast_forward(value["fast"].as_bool().unwrap_or(false));
        let rate = value["rate"]
            .as_u64()
            .filter(|rate| *rate > 0)
            .map(|r| r as u32);
        self.reported_rate = rate.unwrap_or(0);
        input.rate_override(rate);
        if let Some(patches) = value["patches"].as_array() {
            for patch in patches {
                if let Some(id) = patch[0].as_str() {
                    input.patch(id, patch[1].as_bool().unwrap_or(false), patch[2].as_f64());
                }
            }
        }
        if let Some(api) = value["api"].as_str() {
            input.relay_api(api);
        }
        true
    }
}

#[cfg(all(test, feature = "rgss"))]
mod tests {
    use super::*;
    use rsxp::input::Input;
    #[test]
    fn game_poll_does_not_wait_for_the_next_control_reply() {
        use std::net::TcpStream;
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let _silent_peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_millis(20)))
            .unwrap();
        let (send, controls) = std::sync::mpsc::channel();
        send.send(Ok(json!({"keys":[40],"fast":true}))).unwrap();
        let mut runner = Runner {
            stream: Box::new(stream),
            controls,
            first_control: true,
            paused: false,
            performance: false,
            text_input: false,
            cursor: true,
            reported_rate: 0,
            frames: 0,
            fps: 0.0,
            fps_frames: 0,
            fps_start: std::time::Instant::now(),
        };
        let mut input = Input::default();
        assert!(runner.poll(&mut input));
        assert!(input.pressed(40));
        input.begin_frame();
        // The peer deliberately never sends another reply. Advancing the game
        // must still succeed, preserving held input without repeating its edge.
        assert!(runner.poll(&mut input));
        assert!(input.pressed(40));
        assert!(!input.triggered(40));
        drop(send);
        assert!(!runner.poll(&mut input));
    }
}
