// SPDX-License-Identifier: GPL-2.0-or-later
//! One native event loop on Linux, Android and iOS. Ruby yields at frame
//! boundaries; neither pixel readbacks nor serialized IPC are used to display.
use crate::{
    engine,
    gpu_present::{GpuDisplay, Presentation},
    input::Controls,
    platform,
};
use rsxp::gpu::GpuRenderer;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::{ElementState, MouseButton, TouchPhase, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::PhysicalKey,
    window::{Window, WindowId},
};

#[derive(Default)]
struct Options {
    game: Option<String>,
    headless: bool,
    frames: Option<u64>,
    software: bool,
}
impl Options {
    fn parse() -> Result<Self, String> {
        let mut options = Self::default();
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--demo" => options.game = Some(".demo".into()),
                "--game" => {
                    options.game = Some(
                        args.next()
                            .ok_or("--game needs a directory or library name")?,
                    )
                }
                "--headless" => options.headless = true,
                "--frames" => {
                    options.frames = Some(
                        args.next()
                            .ok_or("--frames needs a count")?
                            .parse::<u64>()
                            .map_err(|e| e.to_string())?,
                    )
                }
                "--renderer" => {
                    options.software = match args.next().as_deref() {
                        Some("software") => true,
                        Some("wgpu") => false,
                        _ => return Err("--renderer must be wgpu or software".into()),
                    }
                }
                "--help" | "-h" => {
                    println!("RSXP native mobile frontend (egui + wgpu + native MRI bindings)\n\nrsxp-mobile-desktop [--demo | --game DIRECTORY] [--frames N]\nrsxp-mobile-desktop --headless --demo --frames 120 [--renderer wgpu|software]\n\nF1: tools. F11: uncapped. Arrow keys / Z / X or touch controls.\nThe software renderer is available only for headless compatibility tests.");
                    std::process::exit(0);
                }
                _ => return Err(format!("Unknown argument: {arg}")),
            }
        }
        if let Some(game) = options.game.as_mut() {
            if PathBuf::from(&*game).is_dir() {
                *game = std::fs::canonicalize(&*game)
                    .map_err(|e| e.to_string())?
                    .to_string_lossy()
                    .into_owned();
            }
        }
        if options.software && !options.headless {
            return Err("--renderer software is only available with --headless".into());
        }
        Ok(options)
    }
}

fn configure() -> Result<(), String> {
    engine::configure(platform::library_dir()?, Some(platform::ruby_runtime()?))
}
fn launch_game(value: &str) -> Result<serde_json::Value, String> {
    let path = PathBuf::from(value);
    if path.is_dir() {
        engine::initialize_path(&path)
    } else {
        engine::initialize(value)
    }
}

pub fn run() {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .try_init();
    let options = match Options::parse() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            return;
        }
    };
    if options.headless {
        if let Err(error) = headless(options) {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    run_event_loop(EventLoop::new().expect("native event loop"));
}

pub(crate) fn run_event_loop(event_loop: EventLoop<()>) {
    let result = (|| {
        let options = Options::parse()?;
        configure()?;
        let mut app = App::new(options);
        event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
        engine::stop();
        Ok::<_, String>(())
    })();
    if let Err(error) = result {
        eprintln!("RSXP: {error}");
    }
}

fn headless(options: Options) -> Result<(), String> {
    configure()?;
    if !options.software {
        engine::set_gpu(GpuRenderer::new(640, 480)?);
    }
    launch_game(options.game.as_deref().unwrap_or(".demo"))?;
    rsxp::runtime_settings::set_fast_forward(true);
    let start = Instant::now();
    let frames = options.frames.unwrap_or(120);
    while engine::stats().0 < frames && engine::step()? {}
    let count = engine::stats().0;
    let gpu_frame = engine::with_native_frame(|_, bitmap| bitmap.is_none()).unwrap_or(false);
    println!(
        "NATIVE_GAMEPLAY_OK frames={count} seconds={:.3} fps={:.1} gpu_frame={gpu_frame}",
        start.elapsed().as_secs_f64(),
        count as f64 / start.elapsed().as_secs_f64()
    );
    engine::stop();
    Ok(())
}

enum Action {
    Launch(String),
    Stop,
    Import,
}
struct Ui {
    controls: Controls,
    games: Vec<(String, String)>,
    running: bool,
    paused: bool,
    tools: bool,
    touch: bool,
    error: Option<String>,
    actions: Vec<Action>,
    workspace: rsxp::ui::Workspace,
}
impl Ui {
    fn new() -> Self {
        let mut ui = Self {
            controls: Controls::default(),
            games: vec![],
            running: false,
            paused: false,
            tools: false,
            touch: true,
            error: None,
            actions: vec![],
            workspace: rsxp::ui::Workspace::new(),
        };
        ui.refresh();
        ui
    }
    fn refresh(&mut self) {
        let response = engine::request(serde_json::json!({"op":"games"}));
        self.games = response["ok"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|game| Some((game["id"].as_str()?.into(), game["title"].as_str()?.into())))
            .collect();
    }
    fn draw(
        &mut self,
        ctx: &egui::Context,
        info: &rsxp::ui::WorkspaceInfo,
    ) -> rsxp::ui::WorkspaceOutput {
        ctx.root_ui(|root| {
            egui::Panel::top("mobile-toolbar").show(root, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.strong("RSXP");
                    if self.running {
                        if ui.button("Library").clicked() {
                            self.actions.push(Action::Stop);
                        }
                        if ui
                            .button(if self.paused { "Resume" } else { "Pause" })
                            .clicked()
                        {
                            self.paused = !self.paused;
                            self.controls.clear();
                        }
                        ui.toggle_value(&mut self.tools, "Tools");
                        if ui.toggle_value(&mut self.touch, "Controls").changed() {
                            self.controls.clear();
                        }
                        let mut uncapped = rsxp::runtime_settings::fast_forward();
                        if ui.checkbox(&mut uncapped, "Uncapped").changed() {
                            rsxp::runtime_settings::set_fast_forward(uncapped);
                        }
                        ui.label(format!("{:.0} fps", info.fps));
                    }
                });
                if let Some(error) = &self.error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
            });
        });
        self.controls.regions.clear();
        if !self.running {
            ctx.root_ui(|root| {
                egui::CentralPanel::default().show(root, |ui| {
                    ui.heading("Game library");
                    #[cfg(not(any(target_os = "android", target_os = "ios")))]
                    if ui.button("Import game folder…").clicked() {
                        self.actions.push(Action::Import);
                    }
                    #[cfg(any(target_os = "android", target_os = "ios"))]
                    ui.label("Copy game folders into the app's Games directory, then refresh.");
                    if ui.button("Refresh").clicked() {
                        self.refresh();
                    }
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for (id, title) in &self.games {
                            if ui
                                .add_sized([ui.available_width(), 48.], egui::Button::new(title))
                                .clicked()
                            {
                                self.actions.push(Action::Launch(id.clone()));
                            }
                        }
                    });
                });
            });
            return rsxp::ui::WorkspaceOutput::default();
        }
        if self.touch {
            ctx.root_ui(|root| {
                egui::Panel::bottom("touch-controls")
                    .exact_size(120.)
                    .show(root, |ui| {
                        let area = ui.max_rect();
                        let side = (area.width() / 7.).min(48.);
                        let origin = area.min + egui::vec2(8., 8.);
                        for (label, code, x, y) in [
                            ("↑", 82, 1., 0.),
                            ("←", 80, 0., 1.),
                            ("↓", 81, 1., 1.),
                            ("→", 79, 2., 1.),
                        ] {
                            self.control(
                                ui,
                                label,
                                code,
                                egui::Rect::from_min_size(
                                    origin + egui::vec2(x * side, y * side),
                                    egui::vec2(side - 4., side - 4.),
                                ),
                            );
                        }
                        let right = area.right() - side * 3.;
                        for (label, code, x, y) in
                            [("A", 40, 2., 0.), ("B", 41, 1., 1.), ("Shift", 225, 0., 0.)]
                        {
                            self.control(
                                ui,
                                label,
                                code,
                                egui::Rect::from_min_size(
                                    egui::pos2(right + x * side, origin.y + y * side),
                                    egui::vec2(side - 4., side - 4.),
                                ),
                            );
                        }
                    });
            });
        }
        if self.tools {
            self.workspace.set_controls_visible(true);
            self.workspace.draw(ctx, info)
        } else {
            let mut result = rsxp::ui::WorkspaceOutput::default();
            ctx.root_ui(|root| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(root, |ui| {
                        result.game_rect = ui.max_rect();
                    });
            });
            result
        }
    }
    fn control(&mut self, ui: &mut egui::Ui, label: &str, code: i32, rect: egui::Rect) {
        let held = self.controls.held(code);
        ui.painter().rect_filled(
            rect,
            8.,
            if held {
                egui::Color32::from_rgb(40, 130, 100)
            } else {
                egui::Color32::from_gray(50)
            },
        );
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(16.),
            egui::Color32::WHITE,
        );
        if !self.paused {
            self.controls.regions.push((rect, code));
        }
    }
}

struct App {
    options: Options,
    window: Option<Arc<Window>>,
    display: Option<GpuDisplay>,
    egui: egui::Context,
    platform: Option<egui_winit::State>,
    ui: Ui,
    active: bool,
    next_frame: Instant,
    next_redraw: Instant,
    fps_start: Instant,
    fps_frames: u64,
    fps: f64,
    cursor: egui::Pos2,
    mouse_down: bool,
    launched: bool,
    current_game: Option<String>,
}
impl App {
    fn new(options: Options) -> Self {
        Self {
            options,
            window: None,
            display: None,
            egui: egui::Context::default(),
            platform: None,
            ui: Ui::new(),
            active: false,
            next_frame: Instant::now(),
            next_redraw: Instant::now(),
            fps_start: Instant::now(),
            fps_frames: 0,
            fps: 0.,
            cursor: egui::Pos2::ZERO,
            mouse_down: false,
            launched: false,
            current_game: None,
        }
    }
    fn flush_input(&mut self) {
        for event in self.ui.controls.drain() {
            engine::input(event);
        }
    }
    fn release(&mut self) {
        self.ui.controls.clear();
        self.mouse_down = false;
        self.flush_input();
    }
    fn launch(&mut self, id: &str) {
        self.release();
        match launch_game(id) {
            Ok(_) => {
                self.current_game = Some(id.into());
                self.ui.running = true;
                self.ui.paused = false;
                self.ui.error = None;
                self.next_frame = Instant::now();
                self.fps_start = Instant::now();
                self.fps_frames = 0;
            }
            Err(error) => {
                engine::stop();
                self.ui.running = false;
                self.ui.error = Some(error);
            }
        }
    }
    fn redraw(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        self.flush_input();
        if rsxp::runtime_settings::reset_requested() {
            rsxp::runtime_settings::clear_reset();
            if let Some(game) = self.current_game.clone() {
                self.launch(&game);
            }
        }
        if self.ui.running && !self.ui.paused && Instant::now() >= self.next_frame {
            if !engine::step()? {
                self.ui.running = false;
                engine::stop();
            }
            let (frames, rate) = engine::stats();
            if self.options.frames.is_some_and(|limit| frames >= limit) {
                event_loop.exit();
                return Ok(());
            }
            let now = Instant::now();
            let period = Duration::from_secs_f64(1. / rate.max(1) as f64);
            self.next_frame = if rsxp::runtime_settings::fast_forward() {
                now
            } else {
                (self.next_frame + period).max(now)
            };
            if self.fps_start.elapsed() >= Duration::from_millis(500) {
                self.fps = frames.saturating_sub(self.fps_frames) as f64
                    / self.fps_start.elapsed().as_secs_f64();
                self.fps_frames = frames;
                self.fps_start = now;
            }
        }
        let Some(window) = &self.window else {
            return Ok(());
        };
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }
        let mut input = self.platform.as_mut().unwrap().take_egui_input(window);
        // F1 belongs to this shell's Tools toggle, not the nested workspace.
        input.events.retain(|event| {
            !matches!(
                event,
                egui::Event::Key {
                    key: egui::Key::F1,
                    ..
                }
            )
        });
        let display = self.display.as_mut().unwrap();
        display.set_game_metrics(self.fps, engine::stats().0);
        let output = engine::with_native_frame(|gpu, bitmap| {
            display.present_with_ui(
                gpu,
                bitmap,
                Presentation {
                    size: (size.width, size.height),
                    context: &self.egui,
                    input,
                    global_fullscreen: window.fullscreen().is_some(),
                },
                &mut |ctx, info| self.ui.draw(ctx, info),
            )
        })
        .ok_or("No native GPU available")??;
        self.platform
            .as_mut()
            .unwrap()
            .handle_platform_output(window, output);
        if let Some(fullscreen) = display.take_global_fullscreen_request() {
            window
                .set_fullscreen(fullscreen.then_some(winit::window::Fullscreen::Borderless(None)));
        }
        self.next_redraw = Instant::now() + Duration::from_millis(33);
        self.flush_input();
        for action in std::mem::take(&mut self.ui.actions) {
            match action {
                Action::Launch(id) => self.launch(&id),
                Action::Stop => {
                    self.release();
                    engine::stop();
                    self.ui.running = false;
                    self.ui.refresh();
                }
                Action::Import =>
                {
                    #[cfg(not(any(target_os = "android", target_os = "ios")))]
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        if let Err(error) = engine::import_directory(&path) {
                            self.ui.error = Some(error);
                        }
                        self.ui.refresh();
                    }
                }
            }
        }
        Ok(())
    }
}
impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            self.active = true;
            return;
        }
        let result = (|| {
            let window = Arc::new(
                event_loop
                    .create_window(
                        Window::default_attributes()
                            .with_title("RSXP · Native mobile")
                            .with_inner_size(LogicalSize::new(900., 760.)),
                    )
                    .map_err(|e| e.to_string())?,
            );
            let size = window.inner_size();
            let size = (size.width.max(1), size.height.max(1));
            let existing = engine::with_native_frame(|gpu, _| {
                let surface = gpu
                    .instance
                    .create_surface(window.clone())
                    .map_err(|e| e.to_string())?;
                GpuDisplay::new(gpu, surface, size)
            });
            let display = if let Some(display) = existing {
                display?
            } else {
                let instance =
                    wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
                let surface = instance
                    .create_surface(window.clone())
                    .map_err(|e| e.to_string())?;
                let mut gpu = GpuRenderer::new_for_surface(640, 480, instance, &surface)?;
                eprintln!("RSXP native GPU: {:?}", gpu.adapter.get_info());
                gpu.begin(640, 480)?;
                gpu.submit()?;
                let display = GpuDisplay::new(&gpu, surface, size)?;
                engine::set_gpu(gpu);
                display
            };
            self.platform = Some(egui_winit::State::new(
                self.egui.clone(),
                egui::ViewportId::ROOT,
                window.as_ref(),
                Some(window.scale_factor() as f32),
                window.theme(),
                None,
            ));
            self.window = Some(window);
            self.display = Some(display);
            self.active = true;
            if !self.launched {
                self.launched = true;
                if let Some(game) = self.options.game.clone() {
                    self.launch(&game);
                }
            }
            self.next_frame = Instant::now();
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            eprintln!("Native GPU setup failed: {error}");
            event_loop.exit();
        }
    }
    fn suspended(&mut self, _: &ActiveEventLoop) {
        self.active = false;
        self.release();
        // Android requires dropping the surface before suspended returns.
        self.display = None;
        self.platform = None;
        self.window = None;
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(window) = &self.window else {
            return;
        };
        if window.id() != id {
            return;
        }
        let consumed = self
            .platform
            .as_mut()
            .unwrap()
            .on_window_event(window, &event)
            .consumed;
        let scale = window.scale_factor() as f32;
        let redraw = matches!(event, WindowEvent::RedrawRequested);
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Focused(focused) => {
                self.active = focused;
                if !focused {
                    self.release();
                }
                self.next_frame = Instant::now();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(key) = event.physical_key {
                    if let Some(code) = crate::native_keys::scancode(key) {
                        let down = event.state == ElementState::Pressed;
                        if down && !event.repeat && code == 58 {
                            self.ui.tools = !self.ui.tools;
                        }
                        if !down || (!consumed && !self.ui.paused) {
                            self.ui.controls.key(code, down);
                        }
                    }
                }
            }
            WindowEvent::Touch(touch) => {
                let position = match touch.phase {
                    TouchPhase::Started | TouchPhase::Moved => Some(egui::pos2(
                        touch.location.x as f32 / scale,
                        touch.location.y as f32 / scale,
                    )),
                    _ => None,
                };
                self.ui.controls.pointer(touch.id, position);
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = egui::pos2(position.x as f32 / scale, position.y as f32 / scale);
                if self.mouse_down {
                    self.ui.controls.pointer(u64::MAX, Some(self.cursor));
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                self.mouse_down = state == ElementState::Pressed;
                self.ui
                    .controls
                    .pointer(u64::MAX, self.mouse_down.then_some(self.cursor));
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.redraw(event_loop) {
                    self.release();
                    engine::stop();
                    self.ui.running = false;
                    self.ui.error = Some(error);
                }
            }
            _ => {}
        }
        self.flush_input();
        if !redraw {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if !self.active
            || self.window.as_ref().is_none_or(|window| {
                let size = window.inner_size();
                size.width == 0 || size.height == 0
            })
        {
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        }
        let deadline = if self.ui.running && !self.ui.paused {
            self.next_frame.min(self.next_redraw)
        } else {
            self.next_redraw
        };
        event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
        if Instant::now() >= deadline {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }
}
