// SPDX-License-Identifier: GPL-2.0-or-later
//! Desktop-only transport to an isolated "runner" process; the dashboard never
//! owns Ruby state itself.
//!
//! When `RSXP_SESSION_ADDR` is set, [`Runner::connect`] opens a TCP connection,
//! sends a length-prefixed JSON handshake carrying `RSXP_SESSION_TOKEN`, enables the
//! API relay, and spawns a background `rsxp-session-controls` thread that decodes
//! control messages into a bounded channel. If the variable is unset, connect returns
//! `Ok(None)` and no runner is used.
//!
//! Frames are sent one-way: a JSON header from [`Runner::frame_header`] (fps/frame
//! counters, optional [`rsxp::host::ProfileStats`], input/rate state and relay
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
use rsxp::{graphics::Bitmap, input::Input};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::TcpStream,
};

pub fn write_message(stream: &mut TcpStream, value: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    stream
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .and_then(|_| stream.write_all(&bytes))
        .map_err(|e| e.to_string())
}
pub fn read_message(stream: &mut TcpStream) -> Result<Value, String> {
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
    stream: TcpStream,
    controls: std::sync::mpsc::Receiver<Result<Value, String>>,
    first_control: bool,
    pub paused: bool,
    pub profiler: bool,
    pub text_input: bool,
    pub cursor: bool,
    frames: u64,
    fps: f64,
    fps_frames: u32,
    fps_start: std::time::Instant,
}
impl Runner {
    pub fn connect() -> Result<Option<Self>, String> {
        let Ok(address) = std::env::var("RSXP_SESSION_ADDR") else {
            return Ok(None);
        };
        let mut stream = TcpStream::connect(address).map_err(|e| e.to_string())?;
        stream.set_nodelay(true).map_err(|e| e.to_string())?;
        write_message(
            &mut stream,
            &json!({"token": std::env::var("RSXP_SESSION_TOKEN").unwrap_or_default()}),
        )?;
        let mut reader = stream.try_clone().map_err(|e| e.to_string())?;
        let (send, controls) = std::sync::mpsc::sync_channel(8);
        std::thread::Builder::new()
            .name("rsxp-session-controls".into())
            .spawn(move || loop {
                let value = read_message(&mut reader);
                let failed = value.is_err();
                if send.send(value).is_err() || failed {
                    break;
                }
            })
            .map_err(|e| e.to_string())?;
        rsxp::api_bridge::relay_enable();
        Ok(Some(Self {
            stream,
            controls,
            first_control: true,
            paused: false,
            profiler: false,
            text_input: false,
            cursor: true,
            frames: 0,
            fps: 0.0,
            fps_frames: 0,
            fps_start: std::time::Instant::now(),
        }))
    }
    fn frame_header(
        &mut self,
        width: u32,
        height: u32,
        profile: Option<rsxp::host::ProfileStats>,
    ) -> Value {
        self.frames += 1;
        self.fps_frames += 1;
        if self.fps_start.elapsed().as_millis() >= 500 {
            self.fps = f64::from(self.fps_frames) / self.fps_start.elapsed().as_secs_f64();
            self.fps_frames = 0;
            self.fps_start = std::time::Instant::now();
        }
        let profile=profile.map(|p|json!({"ruby_ms":p.ruby_ms,"bridge_ms":p.bridge_ms,"native_ms":p.native_ms,"render_ms":p.render_ms,"present_ms":p.present_ms,"calls":p.calls,"hotspot_age_secs":p.hotspot_age_secs,"hotspots":p.hotspots.iter().map(|h|json!({"operation":h.operation,"calls":h.calls,"total_ms":h.total_ms})).collect::<Vec<_>>()}));
        json!({"fps":self.fps,"frames":self.frames,"profile":profile,"text_input":self.text_input,"cursor":self.cursor,"width":width,"height":height,"rate":rsxp::runtime_settings::current_rate(),"replies":rsxp::api_bridge::relay_replies()})
    }
    pub fn frame(
        &mut self,
        frame: &Bitmap,
        profile: Option<rsxp::host::ProfileStats>,
    ) -> Result<(), String> {
        let header = self.frame_header(frame.width(), frame.height(), profile);
        write_message(&mut self.stream, &header)?;
        #[cfg(all(feature = "gpu", target_endian = "little"))]
        self.stream
            .write_all(bytemuck::cast_slice(frame.pixels()))
            .map_err(|e| e.to_string())?;
        #[cfg(not(all(feature = "gpu", target_endian = "little")))]
        {
            let bytes: Vec<_> = frame
                .pixels()
                .iter()
                .flat_map(|p| p.to_le_bytes())
                .collect();
            self.stream.write_all(&bytes).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    #[cfg(feature = "gpu")]
    pub fn gpu_frame(
        &mut self,
        frame: rsxp::gpu::GpuFrame,
        profile: Option<rsxp::host::ProfileStats>,
    ) -> Result<(), String> {
        let (width, height) = frame.size();
        let bytes = frame.encode();
        let mut header = self.frame_header(width, height, profile);
        header["gpu_bytes"] = json!(bytes.len());
        write_message(&mut self.stream, &header)?;
        self.stream.write_all(&bytes).map_err(|e| e.to_string())
    }
    pub fn poll(&mut self, input: &mut Input) -> bool {
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
    fn apply_control(&mut self, input: &mut Input, value: Value) -> bool {
        self.paused = value["paused"].as_bool().unwrap_or(false);
        self.profiler = value["profiler"].as_bool().unwrap_or(false);
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
        input.mouse_x = value["x"].as_i64().unwrap_or(0) as i32;
        input.mouse_y = value["y"].as_i64().unwrap_or(0) as i32;
        input.mouse_inside = value["inside"].as_bool().unwrap_or(false);
        input.scroll_v += value["scroll"].as_i64().unwrap_or(0) as i32;
        input.push_text(value["text"].as_str().unwrap_or(""));
        rsxp::runtime_settings::set_fast_forward(value["fast"].as_bool().unwrap_or(false));
        if let Some(rate) = value["rate"].as_u64().filter(|r| *r > 0) {
            rsxp::runtime_settings::set_rate_override(rate as u32);
        }
        if let Some(patches) = value["patches"].as_array() {
            for patch in patches {
                if let Some(id) = patch[0].as_str() {
                    rsxp::runtime_patch::set_enabled(id, patch[1].as_bool().unwrap_or(false));
                    if let Some(v) = patch[2].as_f64() {
                        rsxp::runtime_patch::set_value(id, v);
                    }
                }
            }
        }
        if let Some(api) = value["api"].as_str() {
            rsxp::api_bridge::relay_requests(api.as_bytes());
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn game_poll_does_not_wait_for_the_next_control_reply() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let _silent_peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_millis(20)))
            .unwrap();
        let (send, controls) = std::sync::mpsc::channel();
        send.send(Ok(json!({"keys":[40],"fast":true}))).unwrap();
        let mut runner = Runner {
            stream,
            controls,
            first_control: true,
            paused: false,
            profiler: false,
            text_input: false,
            cursor: true,
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
