// SPDX-License-Identifier: GPL-2.0-or-later
//! Persistent native shell: the dashboard / game-workspace window.
//!
//! This is what `rsxp` (no subcommand) and an interactive `rsxp run` open. It
//! is a control-plane process only: it never embeds Ruby. It launches itself
//! again as a child with the `run` subcommand, which acts as the actual game
//! runner, and talks to it using the length-prefixed JSON framing in
//! [`crate::session`]: process pipes on Windows and loopback TCP elsewhere.
//! Wire details live there; this module relies on `read_message`/`write_message`.
//!
//! Main types and flow:
//! - [`Exchange`] is the shared state between the UI thread and the blocking
//!   I/O thread: the latest CPU [`Bitmap`] or [`rsxp::gpu::GpuFrame`], drained
//!   runtime logs, the pending `control` payload for the runner, the last
//!   error, frame rate/frame counters, performance state, and text-input/cursor
//!   requests. It is guarded by a `Mutex` inside an `Arc`.
//! - [`Session`] owns the child process, the shared [`Exchange`], a stop flag,
//!   the selected transport, and output drain threads. Dropping it kills and
//!   reaps the child. `Session::start` selects process pipes on Windows and
//!   authenticated loopback TCP elsewhere, then spawns the runner and I/O thread.
//! - [`forward_output`] copies a child pipe verbatim to the launching terminal
//!   while also pushing a bounded (300-entry), lossy UTF-8 view into
//!   `Exchange::logs` for the embedded output panel. A closed terminal must
//!   not stop log collection.
//! - [`run`] is the main loop: it creates the winit [`Window`], a wgpu surface
//!   and [`GpuDisplay`], and the egui workspace panel; then per frame it pumps
//!   input, (re)launches or stops sessions via the workspace actions, folds
//!   input into the runner `control` message, applies returned frames to the
//!   GPU, and presents. Sessions restart on "restart", stop/launch requests,
//!   or a reset requested through `rsxp::runtime_settings`; save backup/restore
//!   runs on a detached thread and reports through a channel.
//!
//! Protocol notes enforced here: TCP sessions use a handshake, 30 s connect
//! timeout, token check, and 5 s write timeout. Poll replies carry `control`;
//! frame messages carry `width`/`height` plus either a
//! `gpu_bytes` packet (bounded to 512 MiB, decoded via `GpuFrame::decode`) or
//! raw ARGB pixels (bounded to 16,777,216 pixels per frame). GPU packets are
//! merged into a pending frame and, once queued upload bytes exceed 64 MiB,
//! the I/O thread throttles to apply backpressure; ordered atlas deltas mean
//! non-draw-list data must not be dropped.
use crate::{
    gpu_present::{GpuDisplay, Presentation},
    session::{read_message, write_message},
    window::Window,
};
use rsxp::{graphics::Bitmap, input::Input};
use serde_json::{json, Value};
#[cfg(not(windows))]
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

#[derive(Default)]
struct Exchange {
    frame: Option<Bitmap>,
    gpu_frame: Option<rsxp::gpu::GpuFrame>,
    gpu_view: bool,
    logs: std::collections::VecDeque<String>,
    control: Value,
    error: Option<String>,
    rate: u32,
    fps: f64,
    frames: u64,
    text_input: bool,
    cursor: bool,
}
struct Session {
    child: Child,
    exchange: Arc<Mutex<Exchange>>,
    stopping: Arc<AtomicBool>,
    transport: Arc<PlatformTransport>,
    output_threads: Vec<std::thread::JoinHandle<()>>,
}
impl Drop for Session {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Relaxed);
        self.transport.wake();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
// Forward the original bytes to the launch terminal while retaining a bounded,
// lossy UTF-8 view for egui. read_until also delivers an unterminated final line.
fn forward_output(
    mut reader: impl std::io::BufRead,
    mut terminal: impl std::io::Write,
    exchange: &Mutex<Exchange>,
) {
    let mut bytes = Vec::new();
    loop {
        bytes.clear();
        match reader.read_until(b'\n', &mut bytes) {
            Ok(0) => break,
            Ok(_) => {
                // A closed terminal must not prevent collection of GUI diagnostics.
                let _ = terminal.write_all(&bytes);
                let _ = terminal.flush();
                let text = String::from_utf8_lossy(&bytes);
                let mut state = exchange.lock().unwrap();
                state.logs.push_back(
                    text.trim_end_matches(['\r', '\n'])
                        .chars()
                        .take(4096)
                        .collect(),
                );
                if state.logs.len() > 300 {
                    state.logs.pop_front();
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => {
                let message = format!("rsxp: could not read game output: {error}");
                let _ = writeln!(terminal, "{message}");
                let mut state = exchange.lock().unwrap();
                if state.logs.len() >= 300 {
                    state.logs.pop_front();
                }
                state.logs.push_back(message);
                break;
            }
        }
    }
}

#[cfg(not(windows))]
fn accept_runner(
    listener: TcpListener,
    address: SocketAddr,
    stopping: &AtomicBool,
    timeout: Duration,
) -> std::io::Result<Option<(TcpStream, TcpStream)>> {
    // Keep the listener in its default blocking mode. In particular, avoid
    // Winsock's nonblocking ioctlsocket path, which can fail under Wine.
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = listener
            .accept()
            .map_err(|error| {
                std::io::Error::new(
                    error.kind(),
                    format!("dashboard IPC listener.accept (data socket) failed: {error}"),
                )
            })
            .and_then(|(data, _)| {
                listener
                    .accept()
                    .map(|(control, _)| (data, control))
                    .map_err(|error| {
                        std::io::Error::new(
                            error.kind(),
                            format!(
                                "dashboard IPC listener.accept (control socket) failed: {error}"
                            ),
                        )
                    })
            });
        let _ = send.send(result);
    });
    let start = std::time::Instant::now();
    loop {
        if stopping.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let elapsed = start.elapsed();
        if elapsed >= timeout {
            // The accept thread owns the listener. Connect twice to release
            // both accepts, just as Session::drop does when stopping a game.
            for _ in 0..2 {
                let _ = TcpStream::connect_timeout(&address, Duration::from_millis(100));
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("Game did not connect within {} seconds", timeout.as_secs()),
            ));
        }
        match receive.recv_timeout(Duration::from_millis(10).min(timeout - elapsed)) {
            Ok(result) => return result.map(Some),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err(std::io::Error::other("runner accept thread stopped"));
            }
        }
    }
}

type ProtocolReader = Box<dyn std::io::Read + Send>;
type ProtocolWriter = Box<dyn std::io::Write + Send>;

trait SessionTransport: Send + Sync {
    fn new() -> Result<Self, String>
    where
        Self: Sized;
    fn configure(&self, command: &mut Command);
    fn attach_output(
        &self,
        child: &mut Child,
        exchange: &Arc<Mutex<Exchange>>,
    ) -> Result<Vec<std::thread::JoinHandle<()>>, String>;
    fn streams(
        &self,
        stopping: &AtomicBool,
    ) -> Result<Option<(ProtocolReader, ProtocolWriter)>, String>;
    fn wake(&self);
}

#[cfg(not(windows))]
struct TcpTransport {
    listener: Mutex<Option<TcpListener>>,
    address: SocketAddr,
    token: String,
}

#[cfg(not(windows))]
impl SessionTransport for TcpTransport {
    fn new() -> Result<Self, String> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|e| format!("dashboard IPC listener bind failed: {e}"))?;
        let address = listener
            .local_addr()
            .map_err(|e| format!("dashboard IPC listener local_addr failed: {e}"))?;
        let mut nonce = [0u8; 32];
        getrandom::fill(&mut nonce).map_err(|e| e.to_string())?;
        let token = nonce.iter().map(|b| format!("{b:02x}")).collect();
        Ok(Self {
            listener: Mutex::new(Some(listener)),
            address,
            token,
        })
    }

    fn configure(&self, command: &mut Command) {
        command
            .env("RSXP_SESSION_ADDR", self.address.to_string())
            .env("RSXP_SESSION_TOKEN", &self.token)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
    }

    fn attach_output(
        &self,
        child: &mut Child,
        exchange: &Arc<Mutex<Exchange>>,
    ) -> Result<Vec<std::thread::JoinHandle<()>>, String> {
        let mut threads = Vec::new();
        for (stderr, pipe) in [
            child
                .stdout
                .take()
                .map(|p| Box::new(p) as Box<dyn std::io::Read + Send>),
            child
                .stderr
                .take()
                .map(|p| Box::new(p) as Box<dyn std::io::Read + Send>),
        ]
        .into_iter()
        .enumerate()
        .filter_map(|(index, pipe)| pipe.map(|pipe| (index == 1, pipe)))
        {
            let logs = exchange.clone();
            threads.push(std::thread::spawn(move || {
                let reader = std::io::BufReader::new(pipe);
                if stderr {
                    forward_output(reader, std::io::stderr(), &logs);
                } else {
                    forward_output(reader, std::io::stdout(), &logs);
                }
            }));
        }
        Ok(threads)
    }

    fn streams(
        &self,
        stopping: &AtomicBool,
    ) -> Result<Option<(ProtocolReader, ProtocolWriter)>, String> {
        let listener = self
            .listener
            .lock()
            .map_err(|_| "dashboard IPC listener lock is unavailable")?
            .take()
            .ok_or("dashboard IPC listener was already consumed")?;
        let Some((mut data, mut control)) =
            accept_runner(listener, self.address, stopping, Duration::from_secs(30))
                .map_err(|e| e.to_string())?
        else {
            return Ok(None);
        };
        data.set_nodelay(true)
            .map_err(|e| format!("dashboard IPC data socket set_nodelay failed: {e}"))?;
        data.set_read_timeout(Some(Duration::from_secs(30)))
            .map_err(|e| format!("dashboard IPC data socket set_read_timeout failed: {e}"))?;
        let handshake = read_message(&mut data)
            .map_err(|error| format!("dashboard IPC handshake read failed: {error}"))?;
        if handshake["token"].as_str() != Some(&self.token) {
            return Err("Invalid runner handshake".into());
        }
        control
            .set_nodelay(true)
            .map_err(|e| format!("dashboard IPC control socket set_nodelay failed: {e}"))?;
        control
            .set_read_timeout(Some(Duration::from_secs(30)))
            .map_err(|e| format!("dashboard IPC control socket set_read_timeout failed: {e}"))?;
        let control_handshake = read_message(&mut control)
            .map_err(|error| format!("dashboard IPC control handshake read failed: {error}"))?;
        if control_handshake["token"].as_str() != Some(&self.token) {
            return Err("Invalid runner control handshake".into());
        }
        data.set_read_timeout(None)
            .map_err(|e| format!("dashboard IPC data socket clear timeout failed: {e}"))?;
        control
            .set_read_timeout(None)
            .map_err(|e| format!("dashboard IPC control socket clear timeout failed: {e}"))?;
        control
            .set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| format!("dashboard IPC control socket set_write_timeout failed: {e}"))?;
        Ok(Some((Box::new(data), Box::new(control))))
    }

    fn wake(&self) {
        for _ in 0..2 {
            let _ = TcpStream::connect_timeout(&self.address, Duration::from_millis(100));
        }
    }
}

#[cfg(windows)]
struct StdioTransport {
    output: Mutex<Option<std::process::ChildStdout>>,
    input: Mutex<Option<std::process::ChildStdin>>,
}

#[cfg(windows)]
impl SessionTransport for StdioTransport {
    fn new() -> Result<Self, String> {
        Ok(Self {
            output: Mutex::new(None),
            input: Mutex::new(None),
        })
    }

    fn configure(&self, command: &mut Command) {
        command
            .env("RSXP_SESSION_STDIO", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
    }

    fn attach_output(
        &self,
        child: &mut Child,
        exchange: &Arc<Mutex<Exchange>>,
    ) -> Result<Vec<std::thread::JoinHandle<()>>, String> {
        *self
            .output
            .lock()
            .map_err(|_| "runner stdout lock is unavailable")? = Some(
            child
                .stdout
                .take()
                .ok_or("could not capture game runner protocol output")?,
        );
        *self
            .input
            .lock()
            .map_err(|_| "runner stdin lock is unavailable")? = Some(
            child
                .stdin
                .take()
                .ok_or("could not open game runner control input")?,
        );
        let mut threads = Vec::new();
        if let Some(pipe) = child.stderr.take() {
            let logs = exchange.clone();
            threads.push(std::thread::spawn(move || {
                forward_output(std::io::BufReader::new(pipe), std::io::stderr(), &logs);
            }));
        }
        Ok(threads)
    }

    fn streams(
        &self,
        _stopping: &AtomicBool,
    ) -> Result<Option<(ProtocolReader, ProtocolWriter)>, String> {
        let output = self
            .output
            .lock()
            .map_err(|_| "runner stdout lock is unavailable")?
            .take()
            .ok_or("runner stdout was already consumed")?;
        let input = self
            .input
            .lock()
            .map_err(|_| "runner stdin lock is unavailable")?
            .take()
            .ok_or("runner stdin was already consumed")?;
        Ok(Some((Box::new(output), Box::new(input))))
    }

    fn wake(&self) {}
}

#[cfg(windows)]
type PlatformTransport = StdioTransport;
#[cfg(not(windows))]
type PlatformTransport = TcpTransport;

impl Session {
    fn start(args: &[String], jit: rsxp::ui::session::RubyJit) -> Result<Self, String> {
        let transport = Arc::new(PlatformTransport::new()?);
        let executable = std::env::current_exe()
            .map_err(|e| format!("could not locate runner executable: {e}"))?;
        let mut command = Command::new(executable);
        command
            .arg("run")
            .args(args)
            .env("RSXP_API", "0")
            .env("RSXP_JIT", jit.as_str());
        transport.configure(&mut command);
        let mut child = command
            .spawn()
            .map_err(|e| format!("could not start game runner process: {e}"))?;
        let exchange = Arc::new(Mutex::new(Exchange::default()));
        let output_threads = transport.attach_output(&mut child, &exchange)?;
        let stopping = Arc::new(AtomicBool::new(false));
        let shared = exchange.clone();
        let stop = stopping.clone();
        let io_transport = Arc::clone(&transport);
        std::thread::spawn(move || {
            let result = (|| -> Result<(), String> {
                let Some((mut stream, mut control_stream)) = io_transport.streams(&stop)? else {
                    return Ok(());
                };
                loop {
                    let header = read_message(&mut *stream)
                        .map_err(|error| format!("dashboard IPC message read failed: {error}"))?;
                    if header["poll"].as_bool() == Some(true) {
                        let control = {
                            let mut state = shared.lock().unwrap();
                            let control = state.control.clone();
                            state.control["downs"] = json!([]);
                            state.control["ups"] = json!([]);
                            state.control["scroll"] = json!(0);
                            state.control["text"] = json!("");
                            state.control["api"] = json!("");
                            control
                        };
                        write_message(&mut *control_stream, &control).map_err(|error| {
                            format!("dashboard IPC control write failed: {error}")
                        })?;
                    } else {
                        let w = header["width"].as_u64().ok_or("Missing frame width")? as u32;
                        let h = header["height"].as_u64().ok_or("Missing frame height")? as u32;
                        if w == 0 || h == 0 || u64::from(w) * u64::from(h) > 16_777_216 {
                            return Err("Invalid runner frame".into());
                        }
                        let (frame, gpu_frame) = if let Some(length) = header["gpu_bytes"].as_u64()
                        {
                            if length > 512 * 1024 * 1024 {
                                return Err("GPU packet exceeds transport limit".into());
                            }
                            let mut bytes = vec![0; length as usize];
                            std::io::Read::read_exact(&mut *stream, &mut bytes).map_err(
                                |error| format!("dashboard IPC GPU frame read failed: {error}"),
                            )?;
                            (None, Some(rsxp::gpu::GpuFrame::decode(&bytes)?))
                        } else {
                            let mut bytes = vec![0; w as usize * h as usize * 4];
                            std::io::Read::read_exact(&mut *stream, &mut bytes).map_err(
                                |error| format!("dashboard IPC frame read failed: {error}"),
                            )?;
                            let pixels = bytes
                                .chunks_exact(4)
                                .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
                                .collect();
                            (
                                Some(
                                    Bitmap::from_argb_owned(w, h, pixels)
                                        .map_err(|e| e.to_string())?,
                                ),
                                None,
                            )
                        };
                        if let Some(replies) = header["replies"].as_array() {
                            for reply in replies {
                                if let (Some(id), Some(body)) =
                                    (reply[0].as_u64(), reply[1].as_str())
                                {
                                    let _ = rsxp::api_bridge::respond(id, body);
                                }
                            }
                        }
                        let mut state = shared.lock().unwrap();
                        state.fps = header["fps"].as_f64().unwrap_or(0.0);
                        state.frames = header["frames"].as_u64().unwrap_or(0);
                        state.text_input = header["text_input"].as_bool().unwrap_or(false);
                        state.cursor = header["cursor"].as_bool().unwrap_or(true);
                        if let Some(frame) = frame {
                            state.frame = Some(frame);
                            state.gpu_view = false;
                        }
                        if let Some(frame) = gpu_frame {
                            if let Some(pending) = &mut state.gpu_frame {
                                pending.merge(frame);
                            } else {
                                state.gpu_frame = Some(frame);
                            }
                            state.gpu_view = true;
                        }
                        // Backpressure only when the UI has stopped draining updates.
                        // Draw lists coalesce; ordered atlas deltas must not be lost.
                        let over_budget = state
                            .gpu_frame
                            .as_ref()
                            .is_some_and(|f| f.upload_bytes() > 64 * 1024 * 1024);
                        drop(state);
                        if over_budget {
                            while !stop.load(Ordering::Relaxed)
                                && shared
                                    .lock()
                                    .unwrap()
                                    .gpu_frame
                                    .as_ref()
                                    .is_some_and(|f| f.upload_bytes() > 64 * 1024 * 1024)
                            {
                                std::thread::sleep(Duration::from_millis(1));
                            }
                        }
                        let mut state = shared.lock().unwrap();
                        state.rate = header["rate"].as_u64().unwrap_or(0) as u32;
                    }
                }
            })();
            if let Err(error) = result {
                shared.lock().unwrap().error = Some(error);
            }
        });
        Ok(Self {
            child,
            exchange,
            stopping,
            transport,
            output_threads,
        })
    }
}

pub fn run(initial: Option<Vec<String>>) -> Result<u8, Box<dyn std::error::Error>> {
    let mut window = Window::new(1100, 740)?;
    window.set_title("rsxp · Game workspace")?;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
        Box::new(window.native()),
    ));
    let surface = instance.create_surface(window.native())?;
    let mut gpu = rsxp::gpu::GpuRenderer::new_for_surface(640, 480, instance, &surface)?;
    let mut display = GpuDisplay::new(&gpu, surface, window.physical_size())?;
    let mut panel = rsxp::ui::session::SessionPanel::default();
    panel.ruby_jit = match std::env::var("RSXP_JIT").ok().as_deref() {
        Some("zjit") => rsxp::ui::session::RubyJit::Zjit,
        Some("off") => rsxp::ui::session::RubyJit::Disabled,
        Some("yjit") => rsxp::ui::session::RubyJit::Yjit,
        _ if std::env::var("RSXP_YJIT").as_deref() == Ok("0") => {
            rsxp::ui::session::RubyJit::Disabled
        }
        _ => rsxp::ui::session::RubyJit::Yjit,
    };
    panel.recent = super::library::load();
    display.workspace_mut().session = Some(panel);
    rsxp::api_bridge::start_from_env()?;
    let mut session = None;
    let mut last_args = initial.clone().unwrap_or_default();
    let mut launch = initial;
    let mut input = Input::default();
    let mut frame = Bitmap::new(640, 480)?;
    let mut gpu_view = false;
    let mut logical_size = (640, 480);
    let mut fullscreen = false;
    let mut save_task: Option<std::sync::mpsc::Receiver<Result<(), String>>> = None;
    loop {
        let start = std::time::Instant::now();
        if let Some(task) = &save_task
            && let Ok(result) = task.try_recv()
        {
            let panel = display.workspace_mut().session.as_mut().unwrap();
            panel.busy = false;
            match result {
                Ok(()) => panel.status = "Save files copied".into(),
                Err(error) => panel.error = Some(error),
            }
            save_task = None;
        }
        input.begin_frame();
        if !window.poll(&mut input, false) {
            break;
        }
        if window.egui_context().egui_wants_keyboard_input() {
            input.release_all();
        }
        if window.egui_context().egui_wants_pointer_input() {
            for key in -8..0 {
                input.key_up(key);
            }
        }
        if let Some(args) = launch.take() {
            session.take();
            input.release_all();
            window.cursor(true);
            window.text_input(false);
            rsxp::runtime_settings::reset();
            rsxp::runtime_patch::reset();
            rsxp::api_bridge::end_session();
            let panel = display.workspace_mut().session.take();
            *display.workspace_mut() = rsxp::ui::Workspace::new();
            display.workspace_mut().session = panel;
            frame = Bitmap::new(640, 480)?;
            gpu_view = false;
            logical_size = (640, 480);
            let game_path = args
                .windows(2)
                .find(|a| a[0] == "--game")
                .map(|a| PathBuf::from(&a[1]));
            let cartridge = game_path.as_ref().filter(|path| {
                path.extension().is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("gba") || extension.eq_ignore_ascii_case("nds")
                })
            });
            let root = game_path
                .as_ref()
                .filter(|_| cartridge.is_none())
                .and_then(|path| std::fs::canonicalize(path).ok());
            if let Some(root) = &root {
                if let Err(error) = rsxp::runtime_patch::load_user_patches(root) {
                    eprintln!("Could not load runtime patches: {error}");
                }
            }
            if let Ok(patches) = std::env::var("RSXP_PATCH") {
                rsxp::runtime_patch::enable_from(&patches);
            }
            display.workspace_mut().backend =
                Box::new(rsxp::workspace_services::LocalServices::new(root.clone()));
            let jit = display.workspace_mut().session.as_ref().unwrap().ruby_jit;
            match Session::start(&args, jit) {
                Ok(s) => {
                    session = Some(s);
                    last_args = args;
                    let panel = display.workspace_mut().session.as_mut().unwrap();
                    panel.active = true;
                    panel.paused = false;
                    panel.status = "Starting…".into();
                    panel.error = None;
                    if let Some(rom) = cartridge {
                        let title = rom
                            .file_stem()
                            .map(|stem| stem.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "Cartridge game".into());
                        panel.title = title.clone();
                        panel.source = rom.to_string_lossy().into_owned();
                        panel.save_directory = super::library::data_directory(&title)
                            .map(|path| path.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        if let Err(e) = super::library::remember(&mut panel.recent, rom) {
                            panel.error = Some(format!("Could not remember game: {e}"));
                        }
                    } else if let Some(root) = &root {
                        panel.title = super::library::title(root);
                        panel.source = root.to_string_lossy().into_owned();
                        panel.save_directory = super::library::save_directory(&panel.title)
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        if let Err(e) = super::library::remember(&mut panel.recent, root) {
                            panel.error = Some(format!("Could not remember game: {e}"));
                        }
                    } else {
                        panel.title = "Script session".into();
                    }
                }
                Err(e) => display.workspace_mut().session.as_mut().unwrap().error = Some(e),
            }
        }
        if let Some(s) = &mut session {
            let mut state = s.exchange.lock().unwrap();
            display.workspace_mut().logs = state.logs.iter().cloned().collect();
            let gpu_frame = state.gpu_frame.take();
            gpu_view = state.gpu_view;
            if let Some(next) = state.frame.take() {
                logical_size = (next.width(), next.height());
                frame = next;
                display.workspace_mut().session.as_mut().unwrap().status = "Running".into();
            }
            display.set_game_metrics(state.fps, state.frames);
            window.text_input(state.text_input);
            window.cursor(state.cursor);
            if let Some(frame) = &gpu_frame
                && gpu_view
            {
                logical_size = frame.size();
                display.workspace_mut().session.as_mut().unwrap().status = "Running".into();
            }
            if state.rate > 0 {
                rsxp::runtime_settings::set_current_rate(state.rate);
            }
            let old_api = state.control["api"].as_str().unwrap_or("").to_owned();
            let api = String::from_utf8_lossy(&rsxp::api_bridge::poll()).into_owned();
            let scroll = state.control["scroll"].as_i64().unwrap_or(0) + i64::from(input.scroll_v);
            let text = format!(
                "{}{}",
                state.control["text"].as_str().unwrap_or(""),
                std::mem::take(&mut input.text)
            );
            if input.triggered(68) {
                rsxp::runtime_settings::toggle_fast_forward();
            }
            let mut downs = state.control["downs"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let mut ups = state.control["ups"].as_array().cloned().unwrap_or_default();
            downs.extend(
                (-8..512)
                    .filter(|k| ![58, 59, 60, 61, 67, 68].contains(k) && input.triggered(*k))
                    .map(|k| json!(k)),
            );
            ups.extend(
                (-8..512)
                    .filter(|k| ![58, 59, 60, 61, 67, 68].contains(k) && input.released(*k))
                    .map(|k| json!(k)),
            );
            let rect = display.game_pointer_rect();
            let scale = (rect.width() / logical_size.0 as f32)
                .min(rect.height() / logical_size.1 as f32)
                .max(0.001);
            let origin = rect.center()
                - egui::vec2(logical_size.0 as f32, logical_size.1 as f32) * scale * 0.5;
            let mouse = (egui::pos2(input.mouse_x as f32, input.mouse_y as f32) - origin) / scale;
            let paused = display.workspace_mut().session.as_ref().unwrap().paused;
            if paused {
                display.workspace_mut().session.as_mut().unwrap().status = "Paused".into();
            }
            state.control = json!({"paused":paused,"performance":display.performance_visible(),"downs":downs,"ups":ups,"keys":(-8..512).filter(|k| ![58,59,60,61,67,68].contains(k) && input.pressed(*k)).collect::<Vec<_>>(),"x":mouse.x as i32,"y":mouse.y as i32,"inside":!window.egui_context().egui_wants_pointer_input() && input.mouse_inside && mouse.x>=0.0 && mouse.y>=0.0 && mouse.x<logical_size.0 as f32 && mouse.y<logical_size.1 as f32,"scroll":scroll,"text":text,"fast":rsxp::runtime_settings::fast_forward(),"rate":rsxp::runtime_settings::rate_override(),"api":old_api+&api,"patches":rsxp::runtime_patch::patches().iter().map(|p|json!([p.id,rsxp::runtime_patch::is_enabled(&p.id),rsxp::runtime_patch::value(&p.id)])).collect::<Vec<_>>()});
            if let Some(error) = state.error.take() {
                display.workspace_mut().session.as_mut().unwrap().error = Some(error);
            }
            drop(state);
            if let Some(frame) = gpu_frame {
                gpu.apply_remote_frame(frame)?;
            }
            // Process exit can precede the pipe readers consuming the backtrace.
            // Keep polling the UI until both readers reach EOF, then copy the
            // final logs before releasing the session. Never join a live reader
            // on the UI thread.
            if let Some(status) = s.child.try_wait()?
                && s.output_threads
                    .iter()
                    .all(std::thread::JoinHandle::is_finished)
            {
                for thread in s.output_threads.drain(..) {
                    let _ = thread.join();
                }
                display.workspace_mut().logs =
                    s.exchange.lock().unwrap().logs.iter().cloned().collect();
                if !status.success() {
                    let message = format!("Game exited: {status}");
                    eprintln!("rsxp: {message}");
                    display.workspace_mut().logs.push(message);
                }
                let panel = display.workspace_mut().session.as_mut().unwrap();
                panel.active = false;
                panel.status = format!("Stopped · {status}");
                if status.success() {
                    panel.error = None;
                } else {
                    panel.error = Some(format!("Game exited: {status}. See Runtime output below."));
                }
                session.take();
                rsxp::api_bridge::end_session();
            }
        }
        if session.is_none() {
            rsxp::api_bridge::end_session();
        }
        let context = window.egui_context().clone();
        if context.egui_wants_keyboard_input() {
            input.release_all();
        }
        let dropped =
            context.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf()));
        if let Some(path) = dropped {
            let panel = display.workspace_mut().session.as_mut().unwrap();
            panel.source = path.to_string_lossy().into_owned();
            panel.open = true;
            panel.show_saves = false;
        }
        let presentation = Presentation {
            size: window.physical_size(),
            context: &context,
            input: window.take_egui_input(),
            global_fullscreen: fullscreen,
        };
        let output = if gpu_view {
            display.present(&gpu, presentation)?
        } else {
            display.present_bitmap(&gpu, &frame, presentation)?
        };
        window.set_game_pointer_rect(display.game_pointer_rect());
        window.handle_egui_platform_output(output);
        if let Some(value) = display.take_global_fullscreen_request() {
            window.fullscreen(value)?;
            fullscreen = value;
        }
        let actions = std::mem::take(&mut display.workspace_mut().session_actions);
        if actions.browse {
            if let Some(path) = rfd::FileDialog::new()
                .set_title("Choose an RPG Maker game folder or GBA/DS ROM")
                .pick_folder()
            {
                let panel = display.workspace_mut().session.as_mut().unwrap();
                panel.source = path.to_string_lossy().into_owned();
                match super::library::discover(&panel.source) {
                    Ok(source) => {
                        panel.source = source.path().to_string_lossy().into_owned();
                        launch = Some(vec!["--game".into(), panel.source.clone()]);
                        panel.open = false;
                    }
                    Err(_) => {
                        panel.error = Some("That folder does not contain a supported game.".into());
                    }
                }
            }
        }
        if actions.browse_game_file {
            if let Some(path) = rfd::FileDialog::new()
                .set_title("Choose Game.ini or a GBA/DS ROM")
                .add_filter("Games", &["ini", "gba", "nds"])
                .pick_file()
            {
                let panel = display.workspace_mut().session.as_mut().unwrap();
                panel.source = path.to_string_lossy().into_owned();
                match super::library::discover(&panel.source) {
                    Ok(source) => {
                        panel.source = source.path().to_string_lossy().into_owned();
                        launch = Some(vec!["--game".into(), panel.source.clone()]);
                        panel.open = false;
                    }
                    Err(error) => panel.error = Some(error),
                }
            }
        }
        if actions.stop {
            session.take();
            rsxp::api_bridge::end_session();
            input.release_all();
            window.cursor(true);
            window.text_input(false);
            let panel = display.workspace_mut().session.as_mut().unwrap();
            panel.active = false;
            panel.status = "Stopped".into();
        }
        if actions.restart || rsxp::runtime_settings::reset_requested() {
            launch = Some(last_args.clone());
        }
        if let Some(path) = actions.open {
            match super::library::discover(&path) {
                Ok(source) if save_task.is_none() => {
                    launch = Some(vec![
                        "--game".into(),
                        source.path().to_string_lossy().into_owned(),
                    ])
                }
                Ok(_) => {}
                Err(error) => display.workspace_mut().session.as_mut().unwrap().error = Some(error),
            }
        }
        for (restore, operation) in [(false, actions.backup), (true, actions.restore)] {
            if let Some((from, to)) = operation {
                let panel = display.workspace_mut().session.as_mut().unwrap();
                if save_task.is_none() && session.is_none() {
                    let (sender, receiver) = std::sync::mpsc::channel();
                    panel.busy = true;
                    panel.error = None;
                    std::thread::spawn(move || {
                        let _ = sender.send(
                            super::saves::copy(&from, &to, restore).map_err(|e| e.to_string()),
                        );
                    });
                    save_task = Some(receiver);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(16).saturating_sub(start.elapsed()));
    }
    drop(session);
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(windows))]
    #[test]
    fn runner_connection_waits_for_fragmented_messages() {
        use std::{io::Write, net::TcpListener};
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let mut client = TcpStream::connect(address).unwrap();
        let _control_client = TcpStream::connect(address).unwrap();
        let (mut server, _control_server) = accept_runner(
            listener,
            address,
            &AtomicBool::new(false),
            Duration::from_secs(2),
        )
        .unwrap()
        .unwrap();
        server
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let payload = br#"{"poll":true}"#;
        let length = (payload.len() as u32).to_le_bytes();
        client.write_all(&length[..2]).unwrap();
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            client.write_all(&length[2..]).unwrap();
            client.write_all(payload).unwrap();
        });
        let result = read_message(&mut server);
        writer.join().unwrap();
        assert_eq!(result.unwrap(), json!({"poll":true}));
    }

    #[test]
    fn crash_output_reaches_terminal_and_survives_pipe_eof() {
        let exchange = Mutex::new(Exchange::default());
        let mut terminal = Vec::new();
        let output = b"script.rb:7:in 'update': crash marker (RuntimeError)\n\tfrom script.rb:12\nfinal diagnostic";
        forward_output(std::io::Cursor::new(output), &mut terminal, &exchange);
        assert_eq!(terminal, output);
        let logs: Vec<_> = exchange.lock().unwrap().logs.iter().cloned().collect();
        assert_eq!(
            logs,
            [
                "script.rb:7:in 'update': crash marker (RuntimeError)",
                "\tfrom script.rb:12",
                "final diagnostic",
            ]
        );
    }

    #[test]
    fn unavailable_terminal_does_not_discard_gui_diagnostics() {
        let exchange = Mutex::new(Exchange::default());
        let output = b"crash marker\ninvalid byte: \xff\n";
        forward_output(
            std::io::Cursor::new(output),
            std::io::Cursor::new(&mut [][..]),
            &exchange,
        );
        let state = exchange.lock().unwrap();
        assert_eq!(state.logs[0], "crash marker");
        assert_eq!(state.logs[1], "invalid byte: \u{fffd}");
    }
}
