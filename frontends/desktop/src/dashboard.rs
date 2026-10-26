// SPDX-License-Identifier: GPL-2.0-or-later
//! Persistent native shell: the dashboard / game-workspace window.
//!
//! This is what `fanplay` (no subcommand) and an interactive `fanplay run` open. It
//! is a control-plane process only: it never embeds Ruby. It launches itself
//! again as a child with the `run` subcommand, which acts as the actual game
//! runner, and talks to it using the length-prefixed JSON framing in
//! [`crate::session`]: process pipes on Windows and loopback TCP elsewhere.
//! Wire details live there; this module relies on `read_message`/`write_message`.
//!
//! Main types and flow:
//! - [`Exchange`] is the shared state between the UI thread and the blocking
//!   I/O thread: the latest CPU [`Bitmap`] or [`fanplay_rgss::gpu::GpuFrame`], drained
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
//! - [`run`] starts eframe's native window and drives the shared `fanplay-ui`
//!   application. Its App implementation polls sessions, translates egui input,
//!   and executes launcher actions; eframe owns the window and presentation.
//!
//! Protocol notes enforced here: TCP sessions use a handshake, 30 s connect
//! timeout, token check, and 5 s write timeout. Poll replies carry `control`;
//! frame messages carry `width`/`height` plus either a
//! `gpu_bytes` packet (bounded to 512 MiB, decoded via `GpuFrame::decode`) or
//! raw ARGB pixels (bounded to 16,777,216 pixels per frame). GPU packets are
//! merged into a pending frame and, once queued upload bytes exceed 64 MiB,
//! the I/O thread throttles to apply backpressure; ordered atlas deltas mean
//! non-draw-list data must not be dropped.
use crate::gpu_callback::{GameCallback, GameGpu};
use crate::session::{read_message, write_message};
use fanplay_rgss::{graphics::Bitmap, input::Input};
use fanplay_extension_api::{OperationError, OperationRequest, OperationResponse};
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

fn start_control_server(
    root: &std::path::Path,
    sender: std::sync::mpsc::Sender<crate::control::HostAction>,
) -> (Option<crate::control::ControlServer>, Option<String>) {
    let config = fanplay_provision::config::load(root).unwrap_or_default();
    let enabled = match std::env::var("FANPLAY_API").ok().as_deref() {
        Some("0") => false,
        Some("1") => true,
        _ => config.api.enabled,
    };
    if !enabled {
        return (None, None);
    }
    let port = std::env::var("FANPLAY_API_PORT")
        .ok()
        .and_then(|port| port.parse::<u16>().ok())
        .filter(|port| *port != 0)
        .unwrap_or(config.api.port);
    match crate::control::ControlServer::start(root.to_owned(), port, sender) {
        Ok(server) => (Some(server), None),
        Err(error) => (None, Some(error)),
    }
}

#[derive(Default)]
struct Exchange {
    frame: Option<Bitmap>,
    gpu_frame: Option<fanplay_rgss::gpu::GpuFrame>,
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
    core_id: &'static str,
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
                let message = format!("fanplay: could not read game output: {error}");
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
            .env("FANPLAY_SESSION_ADDR", self.address.to_string())
            .env("FANPLAY_SESSION_TOKEN", &self.token)
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
            .env("FANPLAY_SESSION_STDIO", "1")
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
    fn start(
        args: &[String],
        jit: fanplay_ui::session::RubyJit,
        gpu_available: bool,
    ) -> Result<Self, String> {
        let transport = Arc::new(PlatformTransport::new()?);
        let executable = std::env::current_exe()
            .map_err(|e| format!("could not locate runner executable: {e}"))?;
        let mut command = Command::new(executable);
        let is_nds = args.windows(2).any(|pair| {
            pair[0] == "--game"
                && PathBuf::from(&pair[1])
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("nds"))
        });
        command
            .arg("run")
            .args(args)
            .env("FANPLAY_API", "0")
            .env(
                "FANPLAY_RENDERER",
                if gpu_available { "wgpu" } else { "software" },
            )
            .env(
                "FANPLAY_NDS_RENDERER",
                if is_nds { "wgpu" } else { "software" },
            )
            .env("FANPLAY_JIT", jit.as_str());
        if !gpu_available {
            command.args(["--renderer", "software"]);
        }
        if let Some(path) = args
            .windows(2)
            .find(|a| a[0] == "--game")
            .map(|a| PathBuf::from(&a[1]))
        {
            let settings = fanplay_provision::settings::for_game(&path).map_err(|e| e.to_string())?;
            for (key, value) in settings.environment() {
                command.env(key, value);
            }
            if let Some(version) = settings.rgss {
                if !args.iter().any(|a| a == "--rgss") {
                    command.args(["--rgss", &version.to_string()]);
                }
            }
        }
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
                            (None, Some(fanplay_rgss::gpu::GpuFrame::decode(&bytes)?))
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
                                    let _ = fanplay_rgss::api_bridge::respond(id, body);
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
            core_id: if is_nds { "nds" } else { "rgss" },
            child,
            exchange,
            stopping,
            transport,
            output_threads,
        })
    }
}

pub fn run(initial: Option<Vec<String>>) -> Result<u8, Box<dyn std::error::Error>> {
    crate::webview::prepare_runtime();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("fanplay · Game workspace")
            .with_inner_size([1100.0, 740.0])
            .with_min_inner_size([460.0, 340.0]),
        ..Default::default()
    };
    eframe::run_native(
        "fanplay · Game workspace",
        options,
        Box::new(move |_| Ok(Box::new(DesktopApp::new(initial)))),
    )?;
    Ok(0)
}

struct DesktopApp {
    launcher: crate::launcher::Launcher,
    root: PathBuf,
    control: Option<crate::control::ControlServer>,
    control_sender: std::sync::mpsc::Sender<crate::control::HostAction>,
    control_actions: std::sync::mpsc::Receiver<crate::control::HostAction>,
    control_error: Option<String>,
    active_settings: fanplay_provision::settings::Settings,
    controllers: crate::controller::Controllers,
    shared: fanplay_ui::SharedApp,
    session: Option<Session>,
    input: Input,
    frame: Bitmap,
    logical_size: (u32, u32),
    game_rect: egui::Rect,
    touch_rect: egui::Rect,
    launch: Option<Vec<String>>,
    last_args: Vec<String>,
    save_task: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    text_input: bool,
    fps: f64,
    frames: u64,
    gpu_view: bool,
    gpu_state: Arc<Mutex<GameGpu>>,
    gpu_callback: Option<Arc<GameCallback>>,
    window_before_fit: Option<egui::Vec2>,
    fit_window: Option<FitWindow>,
    last_fit_size: Option<egui::Vec2>,
    extension_bridge: std::collections::HashMap<u64, (u64, u64)>,
    plugin_webview_api_bridge: std::collections::HashMap<u64, u64>,
    pending_webview_assets: Vec<(u64, String, String, u8)>,
    plugin_webview: Option<(String, String, crate::webview::PluginWebView)>,
    pending_webview_commands: Vec<PluginWebViewCommand>,
    shift_forwarded: bool,
}

/// The size a game-only window is being driven toward. Keeping the computed
/// target fixed avoids a resize feedback loop with asynchronous window
/// managers and tiling policies.
struct FitWindow {
    aspect: f32,
}

enum PluginWebViewCommand {
    Open {
        plugin_id: String,
        view_id: String,
        title: String,
        html: String,
    },
    Close {
        plugin_id: String,
        view_id: String,
    },
}

impl DesktopApp {
    fn new(initial: Option<Vec<String>>) -> Self {
        let mut panel = fanplay_ui::session::SessionPanel::default();
        panel.capabilities = fanplay_ui::session::SessionCapabilities::DESKTOP;
        panel.ruby_jit = match std::env::var("FANPLAY_JIT").ok().as_deref() {
            Some("zjit") => fanplay_ui::session::RubyJit::Zjit,
            Some("off") => fanplay_ui::session::RubyJit::Disabled,
            Some("yjit") => fanplay_ui::session::RubyJit::Yjit,
            _ if std::env::var("FANPLAY_YJIT").as_deref() == Ok("0") => {
                fanplay_ui::session::RubyJit::Disabled
            }
            _ => fanplay_ui::session::RubyJit::Yjit,
        };
        panel.recent = super::library::load();
        let mut shared = fanplay_ui::SharedApp::default();
        shared.workspace.session = Some(panel);
        let root = fanplay_provision::root(None);
        let (control_sender, control_actions) = crate::control::action_channel();
        let (control, control_error) = start_control_server(&root, control_sender.clone());
        if control.is_none()
            && let Ok(config) = fanplay_provision::config::load(&root)
            && let Some(panel) = shared.workspace.session.as_mut()
        {
            panel.api_port = config.api.port.to_string();
        }
        Self {
            launcher: crate::launcher::Launcher::default(),
            root,
            control,
            control_sender,
            control_actions,
            control_error,
            active_settings: Default::default(),
            controllers: crate::controller::Controllers::new(),
            shared,
            session: None,
            input: Input::default(),
            frame: Bitmap::new(640, 480).expect("640×480 bitmap allocation"),
            logical_size: (640, 480),
            game_rect: egui::Rect::ZERO,
            touch_rect: egui::Rect::ZERO,
            last_args: initial.clone().unwrap_or_default(),
            launch: initial,
            save_task: None,
            text_input: false,
            fps: 0.0,
            frames: 0,
            gpu_view: false,
            gpu_state: Arc::new(Mutex::new(GameGpu {
                renderer: None,
                rect: egui::Rect::ZERO,
                logical_size: (640, 480),
            })),
            gpu_callback: None,
            window_before_fit: None,
            fit_window: None,
            last_fit_size: None,
            extension_bridge: std::collections::HashMap::new(),
            plugin_webview_api_bridge: std::collections::HashMap::new(),
            pending_webview_assets: Vec::new(),
            plugin_webview: None,
            pending_webview_commands: Vec::new(),
            shift_forwarded: false,
        }
    }

    fn stop_session(&mut self) {
        self.session.take();
        if self.shift_forwarded {
            self.input.key_up(fanplay_ui::controls::keyboard_binding(225));
            self.shift_forwarded = false;
        }
        fanplay_rgss::api_bridge::end_session();
        self.shared.reset_plugins();
        self.extension_bridge.clear();
        self.plugin_webview_api_bridge.clear();
        self.plugin_webview = None;
        self.pending_webview_commands.clear();
        self.input.release_all();
        if let Some(panel) = self.shared.workspace.session.as_mut() {
            panel.active = false;
            panel.status = "Stopped".into();
        }
    }

    fn apply_window_aspect(&mut self, ctx: &egui::Context, fit_aspect: Option<f32>, ready: bool) {
        let fullscreen = ctx.input(|input| input.viewport().fullscreen.unwrap_or(false));
        match fit_aspect {
            Some(aspect) if ready && aspect.is_finite() && aspect > 0.05 && !fullscreen => {
                let current = ctx.content_rect().size();
                if current.x < 1.0 || current.y < 1.0 {
                    return;
                }
                if self.window_before_fit.is_none() {
                    self.window_before_fit = Some(current);
                }
                let needs_fit = self
                    .fit_window
                    .as_ref()
                    .is_none_or(|fit| (fit.aspect - aspect).abs() > 0.001);
                if needs_fit {
                    let base = self.window_before_fit.unwrap_or(current);
                    let monitor = ctx
                        .input(|input| input.viewport().monitor_size)
                        .unwrap_or(current);
                    let target = fitted_window_size(base, aspect, monitor);
                    self.fit_window = Some(FitWindow { aspect });
                    self.last_fit_size = Some(current);
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(target));
                    ctx.request_repaint();
                } else {
                    let ratio = current.x / current.y;
                    let previous = self.last_fit_size.unwrap_or(current);
                    let delta = current - previous;
                    if (ratio - aspect).abs() > 0.01 {
                        let width_driven = delta.x.abs() >= delta.y.abs();
                        let driven = if width_driven {
                            egui::vec2(current.x, current.x / aspect)
                        } else {
                            egui::vec2(current.y * aspect, current.y)
                        };
                        let monitor = ctx
                            .input(|input| input.viewport().monitor_size)
                            .unwrap_or(current);
                        let target = fitted_window_size(driven, aspect, monitor);
                        if (target - current).length() > 0.5 {
                            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(target));
                            ctx.request_repaint();
                            self.last_fit_size = Some(target);
                        }
                    } else {
                        self.last_fit_size = Some(current);
                    }
                }
            }
            Some(_) if !ready => {}
            _ if !fullscreen => {
                if let Some(size) = self.window_before_fit.take() {
                    self.fit_window = None;
                    self.last_fit_size = None;
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
                    ctx.request_repaint();
                }
            }
            _ => {}
        }
    }

    fn process_control_actions(&mut self, ctx: &egui::Context) {
        if let Some(control) = &self.control {
            control.attach_context(ctx);
        }
        while let Ok(action) = self.control_actions.try_recv() {
            match action {
                crate::control::HostAction::Launch { path, reply } => {
                    match crate::library::discover(&path) {
                        Ok(source) => {
                            let source = source.path().to_string_lossy().into_owned();
                            self.launch = Some(vec!["--game".into(), source.clone()]);
                            let _ = reply.send(Ok(json!({"queued": true, "path": source})));
                        }
                        Err(error) => {
                            let _ = reply.send(Err(error));
                        }
                    }
                }
                crate::control::HostAction::Stop { reply } => {
                    self.stop_session();
                    let _ = reply.send(Ok(json!({"stopped": true})));
                }
                crate::control::HostAction::Restart { reply } => {
                    if self.last_args.is_empty() {
                        let _ = reply.send(Err("no game has been started yet".into()));
                    } else {
                        self.launch = Some(self.last_args.clone());
                        let _ = reply.send(Ok(json!({"restarting": true, "args": self.last_args})));
                    }
                }
                crate::control::HostAction::Pause { paused, reply } => {
                    if let Some(panel) = self.shared.workspace.session.as_mut() {
                        panel.paused = paused;
                        let _ = reply.send(Ok(json!({"paused": paused})));
                    } else {
                        let _ = reply.send(Err("no game session".into()));
                    }
                }
                crate::control::HostAction::Live {
                    fast_forward,
                    fps,
                    paused,
                    reply,
                } => {
                    if let Some(value) = fast_forward {
                        fanplay_rgss::runtime_settings::set_fast_forward(value);
                    }
                    if let Some(fps) = fps {
                        if fps == 0 {
                            fanplay_rgss::runtime_settings::clear_rate_override();
                        } else {
                            fanplay_rgss::runtime_settings::set_rate_override(fps);
                        }
                    }
                    if let Some(paused) = paused
                        && let Some(panel) = self.shared.workspace.session.as_mut()
                    {
                        panel.paused = paused;
                    }
                    let _ = reply.send(Ok(json!({
                        "fast_forward": fanplay_rgss::runtime_settings::fast_forward(),
                        "fps": fanplay_rgss::runtime_settings::rate_override(),
                        "paused": self.shared.workspace.session.as_ref().is_some_and(|panel| panel.paused),
                    })));
                }
            }
        }
        self.update_control_state();
        ctx.request_repaint();
    }

    fn stop_control(&mut self) {
        if let Some(control) = self.control.take() {
            control.stop();
            if let Ok(mut config) = fanplay_provision::config::load(&self.root) {
                config.api.enabled = false;
                let _ = fanplay_provision::config::save(&self.root, &config);
            }
        }
    }

    fn start_control(&mut self, port: u16) {
        self.control.take();
        match crate::control::ControlServer::start(
            self.root.clone(),
            port,
            self.control_sender.clone(),
        ) {
            Ok(server) => {
                self.control = Some(server);
                self.control_error = None;
                self.shared.workspace.session.as_mut().unwrap().api_error = None;
                if let Ok(mut config) = fanplay_provision::config::load(&self.root) {
                    config.api.enabled = true;
                    config.api.port = port;
                    let _ = fanplay_provision::config::save(&self.root, &config);
                }
            }
            Err(error) => {
                self.control_error = Some(error.clone());
                self.shared.workspace.session.as_mut().unwrap().api_error = Some(error);
            }
        }
    }

    fn update_control_state(&mut self) {
        let control = self
            .control
            .as_ref()
            .map(|control| (control.port(), control.discovery_url(), control.snapshot()));
        let control_error = self.control_error.clone();
        if let Some(panel) = self.shared.workspace.session.as_mut() {
            panel.api_running = control.is_some();
            if let Some((port, url, _)) = &control {
                panel.api_port = port.to_string();
                panel.api_status = format!("Listening · {url}");
            } else {
                panel.api_status = String::new();
            }
            panel.api_error = control_error;
        }
        let Some((_, _, mut state)) = control else {
            return;
        };
        let panel = self.shared.workspace.session.as_ref();
        if let Some(panel) = panel {
            state.running = self.session.is_some() && panel.active;
            state.paused = panel.paused;
            state.title = panel.title.clone();
            state.source = panel.source.clone();
            state.save_directory = panel.save_directory.clone();
            state.game_key = if panel.source.is_empty() {
                String::new()
            } else {
                fanplay_provision::settings::key_for_path(std::path::Path::new(&panel.source)).0
            };
        }
        state.fast_forward = fanplay_rgss::runtime_settings::fast_forward();
        state.fps = fanplay_rgss::runtime_settings::rate_override();
        if let Some(control) = &self.control {
            control.update(state);
        }
    }

    fn start_game(&mut self, args: Vec<String>, ctx: &egui::Context) {
        let settings = args
            .windows(2)
            .find(|a| a[0] == "--game")
            .map(|a| fanplay_provision::settings::for_game(std::path::Path::new(&a[1])))
            .transpose();
        match settings {
            Ok(settings) => self.active_settings = settings.unwrap_or_default(),
            Err(error) => {
                self.shared.workspace.session.as_mut().unwrap().error = Some(error.to_string());
                return;
            }
        }
        self.controllers
            .use_game_mapping(self.active_settings.gamepad.as_deref());
        self.session.take();
        self.input.release_all();
        fanplay_rgss::runtime_settings::reset();
        fanplay_rgss::runtime_settings::set_fast_forward(self.active_settings.fast_forward);
        if let Some(fps) = self.active_settings.fps {
            fanplay_rgss::runtime_settings::set_rate_override(fps);
        }
        fanplay_rgss::runtime_patch::reset();
        fanplay_ui::controls::load_keyboard(&self.active_settings.keyboard);
        fanplay_rgss::api_bridge::end_session();
        self.shared.reset_plugins();
        self.extension_bridge.clear();
        self.plugin_webview_api_bridge.clear();
        self.pending_webview_assets.clear();
        self.plugin_webview = None;
        self.pending_webview_commands.clear();
        let panel = self.shared.workspace.session.take();
        self.shared.workspace = fanplay_ui::Workspace::new();
        self.shared.workspace.session = panel;
        self.frame = Bitmap::new(640, 480).expect("640×480 bitmap allocation");
        self.logical_size = (640, 480);
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
            if let Err(error) = fanplay_rgss::runtime_patch::load_user_patches(root) {
                eprintln!("Could not load runtime patches: {error}");
            }
        }
        if let Ok(patches) = std::env::var("FANPLAY_PATCH") {
            fanplay_rgss::runtime_patch::enable_from(&patches);
        }
        for patch in &self.active_settings.patches {
            fanplay_rgss::runtime_patch::set_enabled(patch, true);
        }
        for (patch, value) in &self.active_settings.patch_values {
            fanplay_rgss::runtime_patch::set_value(patch, *value);
        }
        self.shared.workspace.backend =
            Box::new(fanplay_ui::workspace_services::LocalServices::new(root.clone()));
        let jit = self.shared.workspace.session.as_ref().unwrap().ruby_jit;
        // Cartridge cores currently produce CPU bitmaps. Keep those frames on
        // the shared eframe texture path instead of starting an unused RGSS
        // remote compositor in the child process.
        let game_uses_rgss_compositor = cartridge.is_none();
        match Session::start(
            &args,
            jit,
            self.gpu_callback.is_some() && game_uses_rgss_compositor,
        ) {
            Ok(session) => {
                self.session = Some(session);
                self.last_args = args;
                {
                    let panel = self.shared.workspace.session.as_mut().unwrap();
                    panel.active = true;
                    panel.paused = false;
                    panel.status = "Starting…".into();
                    panel.error = None;
                }
                if game_uses_rgss_compositor {
                    if let Err(error) = self.shared.activate_plugins() {
                        self.shared.workspace.session.as_mut().unwrap().error = Some(error);
                    }
                }
                let panel = self.shared.workspace.session.as_mut().unwrap();
                if let Some(rom) = cartridge {
                    let title = rom
                        .file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "Cartridge game".into());
                    panel.title = title.clone();
                    panel.source = rom.to_string_lossy().into_owned();
                    panel.save_directory = fanplay_frontend::ContentId::of_file(rom)
                        .ok()
                        .map(|id| {
                            fanplay_provision::root(Some(rom))
                                .join("saves")
                                .join(
                                    rom.extension()
                                        .unwrap()
                                        .to_string_lossy()
                                        .to_ascii_lowercase(),
                                )
                                .join(id.hex())
                                .to_string_lossy()
                                .into_owned()
                        })
                        .unwrap_or_default();
                    if let Err(error) = super::library::remember(&mut panel.recent, rom) {
                        panel.error = Some(format!("Could not remember game: {error}"));
                    }
                } else if let Some(root) = &root {
                    panel.title = super::library::title(root);
                    panel.source = root.to_string_lossy().into_owned();
                    panel.save_directory = super::library::data_directory(root, &panel.title)
                        .to_string_lossy()
                        .into_owned();
                    if let Err(error) = super::library::remember(&mut panel.recent, root) {
                        panel.error = Some(format!("Could not remember game: {error}"));
                    }
                } else {
                    panel.title = "Script session".into();
                }
            }
            Err(error) => self.shared.workspace.session.as_mut().unwrap().error = Some(error),
        }
        ctx.request_repaint();
    }

    fn collect_input(&mut self, ctx: &egui::Context) {
        self.input.begin_frame();
        let shift = ctx.input(|input| input.modifiers.shift);
        if self.session.is_some() && shift && !self.shift_forwarded {
            self.input.key_down(fanplay_ui::controls::keyboard_binding(225));
            self.shift_forwarded = true;
        } else if (!shift || self.session.is_none()) && self.shift_forwarded {
            self.input.key_up(fanplay_ui::controls::keyboard_binding(225));
            self.shift_forwarded = false;
        }
        if ctx.egui_wants_keyboard_input() {
            self.input.release_all();
            return;
        }
        let events = ctx.input(|input| input.events.clone());
        for event in events {
            match event {
                egui::Event::Key {
                    key,
                    physical_key,
                    pressed,
                    repeat: false,
                    ..
                } => {
                    if let Some(code) = egui_scancode(physical_key.unwrap_or(key))
                        .map(fanplay_ui::controls::keyboard_binding)
                    {
                        if pressed {
                            if code == 68 {
                                fanplay_rgss::runtime_settings::toggle_fast_forward();
                            }
                            self.input.key_down(code);
                        } else {
                            self.input.key_up(code);
                        }
                    }
                }
                egui::Event::Text(text) if self.text_input => self.input.push_text(&text),
                egui::Event::PointerButton {
                    button, pressed, ..
                } if (if self.touch_rect != egui::Rect::ZERO {
                    self.touch_rect
                } else {
                    self.game_rect
                })
                .contains(
                    ctx.input(|input| input.pointer.latest_pos().unwrap_or(egui::Pos2::ZERO)),
                ) && (self.touch_rect != egui::Rect::ZERO
                    || !ctx.egui_wants_pointer_input()) =>
                {
                    let code = match button {
                        egui::PointerButton::Primary
                            if self
                                .session
                                .as_ref()
                                .is_some_and(|session| session.core_id == "nds") => 1,
                        egui::PointerButton::Primary => -1,
                        egui::PointerButton::Middle => -2,
                        egui::PointerButton::Secondary => -3,
                        egui::PointerButton::Extra1 => -4,
                        egui::PointerButton::Extra2 => -5,
                    };
                    if pressed {
                        self.input.key_down(code);
                    } else {
                        self.input.key_up(code);
                    }
                }
                egui::Event::MouseWheel { delta, .. }
                    if (if self.touch_rect != egui::Rect::ZERO {
                        self.touch_rect
                    } else {
                        self.game_rect
                    })
                    .contains(
                        ctx.input(|input| input.pointer.latest_pos().unwrap_or(egui::Pos2::ZERO)),
                    ) && (self.touch_rect != egui::Rect::ZERO
                        || !ctx.egui_wants_pointer_input()) =>
                {
                    self.input.scroll_v += delta.y.signum() as i32;
                }
                _ => {}
            }
        }
    }

    fn poll_plugins(&mut self) {
        for request in self.shared.take_plugin_requests() {
            if let Some(plugin_id) = request.operation.strip_suffix(".__webview.open") {
                if let (Some(view_id), Some(title), Some(html)) = (
                    request.arguments.get("view_id").and_then(Value::as_str),
                    request.arguments.get("title").and_then(Value::as_str),
                    request.arguments.get("html").and_then(Value::as_str),
                ) {
                    self.pending_webview_commands
                        .push(PluginWebViewCommand::Open {
                            plugin_id: plugin_id.to_owned(),
                            view_id: view_id.to_owned(),
                            title: title.to_owned(),
                            html: html.to_owned(),
                        });
                }
                let _ = self.shared.dispatch_plugin_response(OperationResponse {
                    id: request.id,
                    generation: request.generation,
                    result: Ok(json!({"opened": true})),
                });
                continue;
            }
            if let Some(plugin_id) = request.operation.strip_suffix(".__webview.close") {
                if let Some(view_id) = request.arguments.get("view_id").and_then(Value::as_str) {
                    self.pending_webview_commands
                        .push(PluginWebViewCommand::Close {
                            plugin_id: plugin_id.to_owned(),
                            view_id: view_id.to_owned(),
                        });
                }
                let _ = self.shared.dispatch_plugin_response(OperationResponse {
                    id: request.id,
                    generation: request.generation,
                    result: Ok(json!({"closed": true})),
                });
                continue;
            }
            if let Some(result) = self
                .shared
                .workspace
                .backend
                .handle_plugin_operation(&request.operation, &request.arguments)
            {
                let _ = self.shared.dispatch_plugin_response(OperationResponse {
                    id: request.id,
                    generation: request.generation,
                    result: result.map_err(|error| OperationError::new("host", error)),
                });
                continue;
            }
            let Some((operation, args)) = legacy_plugin_call(&request) else {
                let _ = self.shared.dispatch_plugin_response(OperationResponse {
                    id: request.id,
                    generation: request.generation,
                    result: Err(OperationError::new(
                        "unsupported_operation",
                        request.operation,
                    )),
                });
                continue;
            };
            match fanplay_rgss::api_bridge::send_from_ui(operation, args) {
                Ok(bridge_id) => {
                    self.extension_bridge
                        .insert(bridge_id, (request.id, request.generation));
                }
                Err(error) => {
                    let _ = self.shared.dispatch_plugin_response(OperationResponse {
                        id: request.id,
                        generation: request.generation,
                        result: Err(OperationError::new("queue", error)),
                    });
                }
            }
        }

        let pending: Vec<_> = self.extension_bridge.keys().copied().collect();
        for bridge_id in pending {
            let Some(reply) = fanplay_rgss::api_bridge::take_ui_response(bridge_id) else {
                continue;
            };
            let (extension_id, generation) = self.extension_bridge.remove(&bridge_id).unwrap();
            let result = match reply {
                Ok(body) => serde_json::from_str(&body)
                    .map_err(|error| OperationError::new("invalid_response", error.to_string())),
                Err(error) => Err(OperationError::new("transport", error)),
            };
            let _ = self.shared.dispatch_plugin_response(OperationResponse {
                id: extension_id,
                generation,
                result,
            });
        }
    }

    fn process_plugin_webviews(&mut self, frame: &eframe::Frame) {
        let commands = std::mem::take(&mut self.pending_webview_commands);
        for command in commands {
            match command {
                PluginWebViewCommand::Open {
                    plugin_id,
                    view_id,
                    title,
                    html,
                } => {
                    self.plugin_webview = None;
                    self.plugin_webview_api_bridge.clear();
                    self.pending_webview_assets.clear();
                    match crate::webview::PluginWebView::open(frame, &title, &html) {
                        Ok(view) => self.plugin_webview = Some((plugin_id, view_id, view)),
                        Err(error) => {
                            if let Some(panel) = self.shared.workspace.session.as_mut() {
                                panel.error = Some(error);
                            }
                        }
                    }
                }
                PluginWebViewCommand::Close { plugin_id, view_id } => {
                    if self
                        .plugin_webview
                        .as_ref()
                        .is_some_and(|(owner, id, _)| owner == &plugin_id && id == &view_id)
                    {
                        self.plugin_webview = None;
                        self.plugin_webview_api_bridge.clear();
                        self.pending_webview_assets.clear();
                    }
                }
            }
        }
        let requests = self
            .plugin_webview
            .as_ref()
            .map(|(_, _, view)| view.take_requests())
            .unwrap_or_default();
        for request in requests {
            match request {
                crate::webview::PluginWebViewRequest::Api { id, method, target } => {
                    if method.eq_ignore_ascii_case("OPTIONS") {
                        if let Some((_, _, view)) = &self.plugin_webview {
                            let _ = view.respond(id, 200, "{}");
                        }
                        continue;
                    }
                    if method.eq_ignore_ascii_case("GET") && target == "/v1/extensions/routes" {
                        if let Some((_, _, view)) = &self.plugin_webview {
                            let body = fanplay_rgss::api_bridge::extension_routes_json();
                            let _ = view.respond(id, 200, &body);
                        }
                        continue;
                    }
                    match fanplay_rgss::api_bridge::route_http(&method.to_ascii_uppercase(), &target)
                        .and_then(|(operation, args)| {
                            fanplay_rgss::api_bridge::send_from_ui(&operation, args)
                                .map_err(|error| (500, error))
                        }) {
                        Ok(bridge_id) => {
                            self.plugin_webview_api_bridge.insert(bridge_id, id);
                        }
                        Err((status, error)) => {
                            if let Some((_, _, view)) = &self.plugin_webview {
                                let body = json!({"error": error}).to_string();
                                let _ = view.respond(id, status, &body);
                            }
                        }
                    }
                }
                crate::webview::PluginWebViewRequest::Asset { id, category, name } => {
                    self.pending_webview_assets.push((id, category, name, 0))
                }
            }
        }
        let pending_assets = std::mem::take(&mut self.pending_webview_assets);
        for (id, category, name, attempts) in pending_assets {
            let data_url = self.shared.plugin_asset_data_url(&category, &name);
            if let Some((_, _, view)) = &self.plugin_webview {
                if data_url.is_some() || attempts >= 30 {
                    let _ = view.respond_asset(id, data_url.as_deref());
                } else {
                    self.pending_webview_assets.push((
                        id,
                        category,
                        name,
                        attempts.saturating_add(1),
                    ));
                }
            }
        }
        let bridge_ids: Vec<_> = self.plugin_webview_api_bridge.keys().copied().collect();
        for bridge_id in bridge_ids {
            let Some(reply) = fanplay_rgss::api_bridge::take_ui_response(bridge_id) else {
                continue;
            };
            let Some(request_id) = self.plugin_webview_api_bridge.remove(&bridge_id) else {
                continue;
            };
            if let Some((_, _, view)) = &self.plugin_webview {
                match reply {
                    Ok(body) => {
                        let _ = view.respond(request_id, 200, &body);
                    }
                    Err(error) => {
                        let body = json!({"error": error}).to_string();
                        let _ = view.respond(request_id, 500, &body);
                    }
                }
            }
        }
        if let Some((_, _, view)) = &self.plugin_webview {
            view.resize(frame);
            view.pump();
        }
    }

    fn poll_session(&mut self, ctx: &egui::Context) {
        if let Some(task) = &self.save_task
            && let Ok(result) = task.try_recv()
        {
            let panel = self.shared.workspace.session.as_mut().unwrap();
            panel.busy = false;
            match result {
                Ok(message) => panel.status = message,
                Err(error) => panel.error = Some(error),
            }
            self.save_task = None;
        }
        self.poll_plugins();
        let Some(session) = &mut self.session else {
            return;
        };
        let mut state = session.exchange.lock().unwrap();
        self.shared.workspace.logs = state.logs.iter().cloned().collect();
        if let Some(frame) = state.frame.take() {
            self.logical_size = (frame.width(), frame.height());
            self.frame = frame;
            let panel = self.shared.workspace.session.as_mut().unwrap();
            panel.status = "Running".into();
        }
        if let Some(frame) = state.gpu_frame.take() {
            let mut gpu = self.gpu_state.lock().unwrap();
            gpu.logical_size = frame.size();
            self.logical_size = frame.size();
            if let Some(renderer) = &mut gpu.renderer {
                if let Err(error) = renderer.apply_remote_frame(frame) {
                    self.shared.workspace.session.as_mut().unwrap().error = Some(error);
                }
            }
        }
        self.gpu_view = state.gpu_view;
        self.text_input = state.text_input;
        self.fps = state.fps;
        self.frames = state.frames;
        if state.rate > 0 {
            fanplay_rgss::runtime_settings::set_current_rate(state.rate);
        }
        let old_api = state.control["api"].as_str().unwrap_or("").to_owned();
        let api = String::from_utf8_lossy(&fanplay_rgss::api_bridge::poll()).into_owned();
        let mut downs = state.control["downs"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut ups = state.control["ups"].as_array().cloned().unwrap_or_default();
        downs.extend(
            (-8..640)
                .filter(|key| ![58, 59, 60, 61, 67, 68].contains(key) && self.input.triggered(*key))
                .map(|key| json!(key)),
        );
        ups.extend(
            (-8..640)
                .filter(|key| ![58, 59, 60, 61, 67, 68].contains(key) && self.input.released(*key))
                .map(|key| json!(key)),
        );
        let pointer = ctx.input(|input| input.pointer.latest_pos());
        let rect = if self.touch_rect != egui::Rect::ZERO {
            self.touch_rect
        } else {
            self.game_rect
        };
        let source_size = if self.touch_rect != egui::Rect::ZERO {
            egui::vec2(256.0, 192.0)
        } else {
            egui::vec2(self.logical_size.0 as f32, self.logical_size.1 as f32)
        };
        if let Some(position) = pointer {
            let over_game = rect.contains(position)
                && (self.touch_rect != egui::Rect::ZERO || !ctx.egui_wants_pointer_input());
            let scale = (rect.width() / source_size.x)
                .min(rect.height() / source_size.y)
                .max(0.001);
            let origin = rect.center() - source_size * scale * 0.5;
            let game_position = (position - origin) / scale;
            self.input.mouse_x = game_position.x as i32;
            self.input.mouse_y = game_position.y as i32
                + if self.touch_rect != egui::Rect::ZERO {
                    192
                } else {
                    0
                };
            self.input.mouse_inside = over_game
                && game_position.x >= 0.0
                && game_position.y >= 0.0
                && game_position.x < self.logical_size.0 as f32
                && game_position.y < source_size.y;
        } else {
            self.input.mouse_inside = false;
        }
        let scale = (rect.width() / source_size.x)
            .min(rect.height() / source_size.y)
            .max(0.001);
        let mouse = pointer
            .map(|point| {
                (point - rect.center()) / scale
                    + source_size * 0.5
                    + egui::vec2(
                        0.0,
                        if self.touch_rect != egui::Rect::ZERO {
                            192.0
                        } else {
                            0.0
                        },
                    )
            })
            .unwrap_or(egui::Pos2::ZERO.to_vec2());
        let paused = self.shared.workspace.session.as_ref().unwrap().paused;
        if paused {
            self.shared.workspace.session.as_mut().unwrap().status = "Paused".into();
        }
        state.control = json!({"paused":paused,"performance":self.shared.workspace.performance_visible(),
            "downs":downs,"ups":ups,"keys":(-8..640).filter(|key| ![58,59,60,61,67,68].contains(key) && self.input.pressed(*key)).collect::<Vec<_>>(),
            "x":mouse.x as i32,"y":mouse.y as i32,"inside":self.input.mouse_inside,
            "scroll":self.input.scroll_v,"text":std::mem::take(&mut self.input.text),"fast":fanplay_rgss::runtime_settings::fast_forward(),
            "rate":fanplay_rgss::runtime_settings::rate_override(),"api":old_api+api.as_str(),
            "patches":fanplay_rgss::runtime_patch::patches().iter().map(|patch|json!([patch.id,fanplay_rgss::runtime_patch::is_enabled(&patch.id),fanplay_rgss::runtime_patch::value(&patch.id)])).collect::<Vec<_>>()});
        if let Some(error) = state.error.take() {
            self.shared.workspace.session.as_mut().unwrap().error = Some(error);
        }
        drop(state);
        if let Ok(Some(status)) = session.child.try_wait() {
            if session
                .output_threads
                .iter()
                .all(std::thread::JoinHandle::is_finished)
            {
                for thread in session.output_threads.drain(..) {
                    let _ = thread.join();
                }
                self.shared.workspace.logs = session
                    .exchange
                    .lock()
                    .unwrap()
                    .logs
                    .iter()
                    .cloned()
                    .collect();
                let panel = self.shared.workspace.session.as_mut().unwrap();
                panel.active = false;
                panel.status = format!("Stopped · {status}");
                panel.error = (!status.success())
                    .then(|| format!("Game exited: {status}. See Runtime output below."));
                self.session.take();
                fanplay_rgss::api_bridge::end_session();
                self.shared.reset_plugins();
                self.extension_bridge.clear();
                self.plugin_webview = None;
                self.pending_webview_commands.clear();
            }
        }
    }
}

impl eframe::App for DesktopApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.gpu_callback.is_none() {
            if let Some(state) = frame.wgpu_render_state() {
                let render_state = state.clone();
                match fanplay_rgss::gpu::GpuRenderer::new_with_device(
                    640,
                    480,
                    render_state.instance.clone(),
                    render_state.adapter.clone(),
                    render_state.device.clone(),
                    render_state.queue.clone(),
                ) {
                    Ok(renderer) => {
                        self.gpu_state.lock().unwrap().renderer = Some(renderer);
                        self.gpu_callback = Some(Arc::new(GameCallback::new(
                            &render_state.device,
                            self.gpu_state.clone(),
                            render_state.target_format,
                        )));
                    }
                    Err(error) => eprintln!("Could not initialize shared GPU compositor: {error}"),
                }
            }
        }
        self.collect_input(&ctx);
        self.process_control_actions(&ctx);
        if self.session.is_some() {
            self.controllers.poll(
                &mut self.input,
                !ctx.egui_wants_keyboard_input() && ctx.input(|i| i.focused),
            );
        }
        self.poll_session(&ctx);
        if let Some(args) = self.launch.take() {
            self.start_game(args, &ctx);
        }
        if self.session.is_none() {
            self.apply_window_aspect(&ctx, None, true);
            let panel = self.shared.workspace.session.as_ref().unwrap();
            let error = self.control_error.as_deref().or(panel.error.as_deref());
            if let Some(path) = self.launcher.draw(ui, &panel.recent, error) {
                match crate::library::discover(&path.to_string_lossy()) {
                    Ok(source) => {
                        self.launch = Some(vec![
                            "--game".into(),
                            source.path().to_string_lossy().into_owned(),
                        ])
                    }
                    Err(error) => {
                        self.shared.workspace.session.as_mut().unwrap().error = Some(error)
                    }
                }
                ctx.request_repaint();
            }
            return;
        }
        let info = fanplay_ui::WorkspaceInfo {
            core_id: self.session.as_ref().map_or("rgss", |session| session.core_id),
            logical_size: self.logical_size,
            fps: self.fps,
            frames: self.frames,
            global_fullscreen: ctx.input(|input| input.viewport().fullscreen.unwrap_or(false)),
            show_fullscreen: true,
            show_save_buttons: false,
        };
        self.shared.set_info(info);
        if self.gpu_view {
            if let Some(callback) = &self.gpu_callback {
                self.shared
                    .set_game_paint_callback(Some(callback.paint_callback(self.game_rect)));
            } else {
                self.shared.set_game_paint_callback(None);
                self.shared.set_bitmap(&ctx, &self.frame);
            }
        } else {
            self.shared.set_game_paint_callback(None);
            self.shared.set_bitmap(&ctx, &self.frame);
        }
        self.shared.draw(ui);
        if self.shared.take_plugin_import_request() {
            if let Some(path) = rfd::FileDialog::new()
                .set_title("Import Rhai plugin")
                .add_filter("Rhai plugin", &["rhai"])
                .pick_file()
            {
                if let Err(error) = self.shared.import_plugin(&path) {
                    if let Some(panel) = self.shared.workspace.session.as_mut() {
                        panel.error = Some(error);
                    }
                }
            }
            ctx.request_repaint();
        }
        self.process_plugin_webviews(frame);
        let output = self.shared.take_output();
        self.apply_window_aspect(&ctx, output.fit_aspect, self.frames > 0);
        self.game_rect = output.game_rect;
        self.touch_rect = output.touch_rect;
        let output_create_backup = output.create_backup;
        let output_restore_latest = output.restore_latest_backup;
        let output_open_game = output.open_game_folder;
        let output_open_save = output.open_save_folder;
        let output_open_patches = output.open_patches_folder;
        self.gpu_state.lock().unwrap().rect = self.game_rect;
        if self.session.is_some() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
        let actions = std::mem::take(&mut self.shared.workspace.session_actions);
        if let Some(port) = actions.start_api {
            self.start_control(port);
        }
        if actions.stop_api {
            self.stop_control();
        }
        if actions.browse {
            if let Some(path) = rfd::FileDialog::new()
                .set_title("Choose an RPG Maker game folder or GBA/DS ROM")
                .pick_folder()
            {
                let panel = self.shared.workspace.session.as_mut().unwrap();
                match super::library::discover(&path.to_string_lossy()) {
                    Ok(source) => {
                        panel.source = source.path().to_string_lossy().into_owned();
                        self.launch = Some(vec!["--game".into(), panel.source.clone()]);
                        panel.open = false;
                    }
                    Err(error) => panel.error = Some(error),
                }
            }
        }
        if actions.browse_game_file {
            if let Some(path) = rfd::FileDialog::new()
                .set_title("Choose Game.ini or a GBA/DS ROM")
                .add_filter("Games", &["ini", "gba", "nds"])
                .pick_file()
            {
                let panel = self.shared.workspace.session.as_mut().unwrap();
                match super::library::discover(&path.to_string_lossy()) {
                    Ok(source) => {
                        panel.source = source.path().to_string_lossy().into_owned();
                        self.launch = Some(vec!["--game".into(), panel.source.clone()]);
                        panel.open = false;
                    }
                    Err(error) => panel.error = Some(error),
                }
            }
        }
        if let Some(path) = actions.open {
            match super::library::discover(&path) {
                Ok(source) => {
                    self.launch = Some(vec![
                        "--game".into(),
                        source.path().to_string_lossy().into_owned(),
                    ])
                }
                Err(error) => self.shared.workspace.session.as_mut().unwrap().error = Some(error),
            }
        }
        if actions.stop {
            self.stop_session();
        }
        if actions.restart || fanplay_rgss::runtime_settings::reset_requested() {
            self.launch = Some(self.last_args.clone());
        }
        let create_backup = actions.create_backup || output_create_backup;
        let restore_latest = actions.restore_latest || output_restore_latest;
        if (create_backup || restore_latest)
            && self.save_task.is_none()
            && (self.session.is_none() || (create_backup && !restore_latest))
        {
            let panel = self.shared.workspace.session.as_ref().unwrap();
            let source = if !panel.source.is_empty() {
                panel.source.clone()
            } else {
                self.last_args
                    .windows(2)
                    .find(|pair| pair[0] == "--game")
                    .map(|pair| pair[1].clone())
                    .unwrap_or_default()
            };
            let configured_save = panel.save_directory.clone();
            let location = save_location(&self.root, &source);
            match location {
                Some((key, default_save)) => {
                    let save = if configured_save.is_empty() {
                        default_save
                    } else {
                        PathBuf::from(configured_save)
                    };
                    let root = self.root.clone();
                    let (sender, receiver) = std::sync::mpsc::channel();
                    self.shared.workspace.session.as_mut().unwrap().busy = true;
                    std::thread::spawn(move || {
                        let result = if restore_latest {
                            fanplay_provision::backup::list(&root, &key).and_then(|backups| {
                                let latest = backups
                                    .first()
                                    .ok_or_else(|| std::io::Error::other("no backups exist"))?;
                                fanplay_provision::backup::restore(&root, &key, &latest.name, &save)
                                    .map(|info| format!("Restored backup {}", info.name))
                            })
                        } else {
                            fanplay_provision::backup::create(&root, &key, &save, None)
                                .map(|info| format!("Backup {} created", info.name))
                        };
                        let _ = sender.send(result.map_err(|error| error.to_string()));
                    });
                    self.save_task = Some(receiver);
                }
                None if restore_latest || create_backup => {
                    if let Some(panel) = self.shared.workspace.session.as_mut() {
                        panel.error =
                            Some("Open or select a game before creating a save backup".into());
                    }
                }
                None => {}
            }
        } else if restore_latest && self.session.is_some() {
            self.shared.workspace.session.as_mut().unwrap().error =
                Some("Stop the game before restoring a save backup".into());
        }
        if output_open_game || output_open_save || output_open_patches {
            let panel = self.shared.workspace.session.as_ref().unwrap();
            let source = if !panel.source.is_empty() {
                panel.source.clone()
            } else {
                self.last_args
                    .windows(2)
                    .find(|pair| pair[0] == "--game")
                    .map(|pair| pair[1].clone())
                    .unwrap_or_default()
            };
            if source.is_empty() {
                if let Some(panel) = self.shared.workspace.session.as_mut() {
                    panel.error = Some("Select a game before opening its folders".into());
                }
            } else {
                let path = PathBuf::from(&source);
                let game = fanplay_provision::folders::game_directory(&path);
                let save = save_location(&self.root, &source).map(|(_, save)| save);
                if output_open_game {
                    let _ = fanplay_provision::folders::open(&game);
                }
                if output_open_save {
                    if let Some(save) = save {
                        let _ = fanplay_provision::folders::ensure(&save);
                        let _ = fanplay_provision::folders::open(&save);
                    }
                }
                if output_open_patches {
                    let patches = fanplay_provision::folders::patches_directory(&game);
                    let _ = fanplay_provision::folders::ensure(&patches);
                    let _ = fanplay_provision::folders::open(&patches);
                }
            }
        }
        for (restore, operation) in [(false, actions.backup), (true, actions.restore)] {
            if let Some((from, to)) = operation
                && self.save_task.is_none()
                && self.session.is_none()
            {
                let (sender, receiver) = std::sync::mpsc::channel();
                self.shared.workspace.session.as_mut().unwrap().busy = true;
                std::thread::spawn(move || {
                    let _ = sender.send(
                        super::saves::copy(&from, &to, restore)
                            .map(|()| "Save files copied".to_string())
                            .map_err(|error| error.to_string()),
                    );
                });
                self.save_task = Some(receiver);
            }
        }
        ctx.request_repaint_after(Duration::from_millis(16));
    }
}

fn save_location(root: &std::path::Path, source: &str) -> Option<(String, PathBuf)> {
    if source.is_empty() {
        return None;
    }
    let path = PathBuf::from(source);
    let (key, _) = fanplay_provision::settings::key_for_path(&path);
    let cartridge = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("gba") || extension.eq_ignore_ascii_case("nds")
        });
    let save = if cartridge {
        let id = fanplay_frontend::ContentId::of_file(&path).ok()?;
        root.join("saves")
            .join(
                path.extension()
                    .and_then(|extension| extension.to_str())
                    .unwrap_or("rom")
                    .to_ascii_lowercase(),
            )
            .join(id.hex())
    } else {
        fanplay_provision::folders::save_directory(root, &fanplay_provision::folders::title(&path))
    };
    Some((key, save))
}

/// Adapts the generic plugin request envelope to the RGSS bridge's positional
/// argument protocol. The host deliberately does not enumerate game domains;
/// operation declarations and their argument order belong to each plugin.
fn legacy_plugin_call(request: &OperationRequest) -> Option<(&str, Vec<String>)> {
    if request.operation.ends_with(".__ruby.inject") {
        let source = request.arguments.get("source")?.as_str()?;
        let name = request
            .arguments
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("plugin");
        let name = name
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '_' | '-') {
                    character
                } else {
                    '-'
                }
            })
            .collect::<String>();
        let patch_name = format!(
            "rhai-{}-{}",
            if name.is_empty() { "plugin" } else { &name },
            request.id
        );
        return Some((
            "ruby_inject",
            vec![patch_name, fanplay_rgss::api_bridge::hex_encode(source)],
        ));
    }
    if request.operation.ends_with(".__ruby.unload") {
        let handle = request.arguments.get("handle")?;
        let handle = handle.as_str().map(str::to_owned).or_else(|| {
            handle
                .as_i64()
                .filter(|value| *value > 0)
                .map(|value| format!("rhai-{value}"))
        })?;
        return Some(("ruby_unload", vec![handle]));
    }
    let operation = request.operation.rsplit_once('.')?.1;
    let arguments = request.arguments.as_object()?.values().map(|value| {
        value
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| value.to_string())
    });
    Some((operation, arguments.collect()))
}

fn egui_scancode(key: egui::Key) -> Option<i32> {
    use egui::Key::*;
    Some(match key {
        A => 4,
        B => 5,
        C => 6,
        D => 7,
        E => 8,
        F => 9,
        G => 10,
        H => 11,
        I => 12,
        J => 13,
        K => 14,
        L => 15,
        M => 16,
        N => 17,
        O => 18,
        P => 19,
        Q => 20,
        R => 21,
        S => 22,
        T => 23,
        U => 24,
        V => 25,
        W => 26,
        X => 27,
        Y => 28,
        Z => 29,
        Num1 => 30,
        Num2 => 31,
        Num3 => 32,
        Num4 => 33,
        Num5 => 34,
        Num6 => 35,
        Num7 => 36,
        Num8 => 37,
        Num9 => 38,
        Num0 => 39,
        Enter => 40,
        Escape => 41,
        Backspace => 42,
        Tab => 43,
        Space => 44,
        ArrowRight => 79,
        ArrowLeft => 80,
        ArrowDown => 81,
        ArrowUp => 82,
        _ => return None,
    })
}

/// Size a native window so its client area has `aspect` while preserving the
/// current height whenever possible. The result is clamped to the monitor so a
/// portrait DS window cannot grow taller than the screen.
fn fitted_window_size(current: egui::Vec2, aspect: f32, monitor: egui::Vec2) -> egui::Vec2 {
    let aspect = aspect.clamp(0.05, 20.0);
    let mut height = current.y.max(240.0);
    let mut width = height * aspect;
    // Keep the client area above the window's minimum size so a portrait DS
    // window is not clamped by the window manager into the wrong aspect.
    const MIN_WIDTH: f32 = 460.0;
    const MIN_HEIGHT: f32 = 340.0;
    if width < MIN_WIDTH {
        width = MIN_WIDTH;
        height = width / aspect;
    }
    if height < MIN_HEIGHT {
        height = MIN_HEIGHT;
        width = height * aspect;
    }
    let max = egui::vec2(
        (monitor.x.max(320.0) * 0.96).max(240.0),
        (monitor.y.max(240.0) * 0.92).max(180.0),
    );
    if width > max.x {
        width = max.x;
        height = width / aspect;
    }
    if height > max.y {
        height = max.y;
        width = height * aspect;
    }
    egui::vec2(width.round().max(1.0), height.round().max(1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_fit_preserves_aspect_for_rpg_maker_gba_and_ds() {
        let current = egui::vec2(1100.0, 740.0);
        let monitor = egui::vec2(1920.0, 1080.0);
        for aspect in [4.0 / 3.0, 3.0 / 2.0, 256.0 / 384.0] {
            let size = fitted_window_size(current, aspect, monitor);
            assert!(
                (size.x / size.y - aspect).abs() < 0.01,
                "aspect {aspect}: {size:?}"
            );
            assert!(size.x <= monitor.x && size.y <= monitor.y, "{size:?}");
        }
        // A portrait DS window keeps the current height and narrows the width.
        let ds = fitted_window_size(current, 256.0 / 384.0, monitor);
        assert!((ds.y - 740.0).abs() < 1.0);
        assert!((ds.x - 493.0).abs() < 1.0);
    }

    #[test]
    fn window_fit_clamps_tall_windows_to_the_monitor() {
        let size = fitted_window_size(
            egui::vec2(700.0, 2000.0),
            256.0 / 384.0,
            egui::vec2(800.0, 600.0),
        );
        assert!((size.x / size.y - 256.0 / 384.0).abs() < 0.01);
        assert!(size.y <= 600.0 * 0.93);
        assert!(size.x <= 800.0);
    }

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
