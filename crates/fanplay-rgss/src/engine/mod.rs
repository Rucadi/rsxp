// SPDX-License-Identifier: GPL-2.0-or-later
//! Platform-independent RGSS engine: bitmaps, sprites, planes, viewports,
//! input and the native command dispatcher. Platform services (windowing,
//! image/font decoding, files) come from a [`Host`].
//!
//! This module owns runtime state and the public bridge entry points. Internal
//! modules separate asset resolution (`assets`), serialized protocol handling
//! (`commands`), frame lifecycle (`frame`), retained order (`scene`) and
//! composition (`render_software` / `render_gpu`); `retained_map`,
//! `map_chunks` and `world_map` provide Ruby-free tiling/camera helpers.
//!
//! One [`Runtime`] is stored per thread in the `ENGINE` thread-local; the Ruby
//! host reaches it through `native_call_bytes`, which dispatches a request
//! under `catch_unwind` and maps the `__FANPLAY_STOP__` error to `Ok(None)` (clean
//! stop). Ruby wrappers hold numeric IDs into the runtime's resource maps,
//! never references. `FANPLAY_INPUT` feeds deterministic headless key presses via
//! [`parse_scripted`].
use crate::graphics::{Bitmap, Color, Rect, Tone};
use crate::host::{Host, InputEvent};
use crate::input::Input;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

mod assets;
mod commands;
mod frame;
pub mod host;
pub mod input;
pub mod map_chunks;
#[cfg(feature = "gpu")]
mod render_gpu;
mod render_software;
pub mod retained_map;
mod scene;
#[cfg(test)]
mod tests;
pub mod world_map;

thread_local! { static ENGINE: RefCell<Option<Runtime>> = const { RefCell::new(None) }; }

/// Runs the native command dispatcher against the active runtime. `Ok(None)`
/// signals a clean stop (`__FANPLAY_STOP__`).
pub fn native_call_bytes(request: &[u8]) -> Result<Option<Vec<u8>>, String> {
    let result = std::panic::catch_unwind(|| {
        ENGINE.with(|slot| {
            slot.borrow_mut()
                .as_mut()
                .ok_or_else(|| "runtime is closed".to_string())?
                .dispatch(request)
        })
    });
    match result {
        Ok(Ok(bytes)) => Ok(Some(bytes)),
        Ok(Err(message)) if message == "__FANPLAY_STOP__" => Ok(None),
        Ok(Err(message)) => Err(message),
        Err(_) => Err("internal Rust runtime error".to_string()),
    }
}

/// Installs the active runtime.
pub fn set_runtime(runtime: Runtime) {
    ENGINE.with(|slot| *slot.borrow_mut() = Some(runtime));
}

/// Removes and returns the active runtime.
pub fn take_runtime() -> Option<Runtime> {
    ENGINE.with(|slot| slot.borrow_mut().take())
}

/// Runs `f` with a shared reference to the active runtime, if any.
pub fn with_runtime<R>(f: impl FnOnce(&Runtime) -> R) -> Option<R> {
    ENGINE.with(|slot| slot.borrow().as_ref().map(f))
}

/// Runs `f` with a mutable reference to the active runtime, if any.
pub fn with_runtime_mut<R>(f: impl FnOnce(&mut Runtime) -> R) -> Option<R> {
    ENGINE.with(|slot| slot.borrow_mut().as_mut().map(f))
}

/// Fully converted sprite snapshot. Host bindings must finish Ruby conversions
/// before submitting this value, so GC cannot reenter a borrowed runtime.
#[derive(Clone, Default)]
pub struct SpriteState {
    pub clip: Rect,
    pub bitmap: u64,
    pub src: Rect,
    pub x: f64,
    pub y: f64,
    pub ox: f64,
    pub oy: f64,
    pub zoom_x: f64,
    pub zoom_y: f64,
    pub angle: f64,
    pub mirror: bool,
    pub z: i32,
    pub opacity: u8,
    pub visible: bool,
    pub blend_type: i32,
    pub color: Color,
    pub tone: Tone,
    pub invert: bool,
    pub bush_depth: i32,
    pub bush_opacity: u8,
    pub pattern: u64,
    pub pattern_blend: i32,
    pub pattern_tile: bool,
    pub pattern_opacity: u8,
    pub pattern_scroll_x: f64,
    pub pattern_scroll_y: f64,
    pub pattern_zoom_x: f64,
    pub pattern_zoom_y: f64,
    pub viewport: u64,
}
#[derive(Clone)]
struct RuntimeSprite {
    #[cfg(feature = "gpu")]
    native_tile: bool,
    state: SpriteState,
    world: Option<world_map::WorldTile>,
    order: u64,
}
impl RuntimeSprite {
    fn position(&self, cameras: &HashMap<u64, world_map::MapCamera>) -> (f64, f64, i32) {
        self.world
            .and_then(|world| {
                cameras.get(&world.group).map(|&camera| {
                    let (x, y, z) = world.project(camera);
                    (x as f64, y as f64, z)
                })
            })
            .unwrap_or((self.state.x, self.state.y, self.state.z))
    }
    fn resolved(&self, cameras: &HashMap<u64, world_map::MapCamera>) -> Self {
        let mut sprite = self.clone();
        let (x, y, z) = self.position(cameras);
        sprite.state.x = x;
        sprite.state.y = y;
        sprite.state.z = z;
        sprite.world = None;
        sprite
    }
}
impl std::ops::Deref for RuntimeSprite {
    type Target = SpriteState;
    fn deref(&self) -> &SpriteState {
        &self.state
    }
}
impl std::ops::DerefMut for RuntimeSprite {
    fn deref_mut(&mut self) -> &mut SpriteState {
        &mut self.state
    }
}

#[derive(Clone)]
struct RuntimePlane {
    bitmap: u64,
    viewport: u64,
    clip: Rect,
    ox: i32,
    oy: i32,
    zoom_x: f64,
    zoom_y: f64,
    opacity: u8,
    blend_type: i32,
    color: Color,
    tone: Tone,
    visible: bool,
    z: i32,
    order: u64,
}

struct RuntimeViewport {
    rect: Rect,
    z: i32,
    ox: i32,
    oy: i32,
    visible: bool,
    color: Color,
    tone: Tone,
    order: u64,
    buffer: Option<Bitmap>,
}

enum Element {
    Sprite(RuntimeSprite),
    Plane(RuntimePlane),
    Viewport(u64),
}

#[cfg(feature = "gpu")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SceneDraw {
    Sprite(u64),
    Plane(u64),
    BeginViewport(u64),
    EndViewport(u64),
}

enum DrawItem {
    Sprite(usize),
    Plane(usize),
}

/// A scripted headless key press, used by automated tests and headless runs.
/// Parsed from `FANPLAY_INPUT` as `FRAME:SCANCODE[:HOLD]` entries.
pub struct ScriptedKey {
    pub start: u64,
    pub key: i32,
    pub hold: u64,
}

/// Everything needed to build a [`Runtime`].
pub struct RuntimeOptions {
    pub width: u32,
    pub height: u32,
    pub version: u8,
    pub headless: bool,
    pub frame_limit: Option<u64>,
    pub rate: u32,
    /// Start with frame pacing disabled (fast-forward).
    pub fast_forward: bool,
    pub archives: Vec<Vec<u8>>,
    pub scripted: Vec<ScriptedKey>,
    pub host: Box<dyn Host>,
}

/// Shared RGSS state and resource ownership.
///
/// Internal modules implement asset resolution, command decoding, frame
/// orchestration and scene composition against this single owner. Frontends
/// supply platform services through [`Host`].
pub struct Runtime {
    // Composition state and retained GPU scene order.
    gpu_frame: bool,
    #[cfg(feature = "gpu")]
    scene_plan: Vec<SceneDraw>,
    #[cfg(feature = "gpu")]
    scene_dirty: bool,
    #[cfg(feature = "gpu")]
    scene_cache: bool,
    next_frame_deadline: Option<f64>,
    fullscreen: bool,
    scale: f64,
    frozen: Option<Bitmap>,
    transitioning: Option<(u32, u32)>,
    brightness: u8,
    target: Bitmap,

    // Native resources. Ruby wrappers keep IDs, never references into these maps.
    bitmaps: HashMap<u64, Bitmap>,
    font_cache: HashMap<String, Option<Arc<Vec<u8>>>>,
    sprites: HashMap<u64, RuntimeSprite>,
    map_cameras: HashMap<u64, world_map::MapCamera>,
    planes: HashMap<u64, RuntimePlane>,
    viewports: HashMap<u64, RuntimeViewport>,
    movies: HashMap<u64, crate::media::movie::Movie>,
    movie_start: HashMap<u64, f64>,
    next_id: u64,

    // Platform services and input accumulated between Graphics/Input updates.
    host: Box<dyn Host>,
    headless: bool,
    input: Input,
    pending: Input,
    queued: Vec<InputEvent>,
    text_buffer: String,
    text_input: bool,

    // RGSS frame counters, pacing and FPS display state.
    frame: u64,
    updates: u64,
    frame_limit: Option<u64>,
    rate: u32,
    fast_forward: bool,
    show_counter: bool,
    fps: f64,
    fps_window_start: f64,
    fps_window_frames: u32,
    /// Host clock at the last animation tick, for animated GIF bitmaps.
    animation_time: f64,

    // Game configuration and deterministic headless input.
    version: u8,
    archives: Vec<Vec<u8>>,
    scripted: Vec<ScriptedKey>,
    scripted_held: Vec<i32>,
}

/// Parses `FANPLAY_INPUT`, a comma-separated list of `FRAME:SCANCODE[:HOLD]`
/// entries that press a key for `HOLD` frames (default 1) in headless runs.
pub fn parse_scripted(value: &str) -> Result<Vec<ScriptedKey>, String> {
    let mut keys = Vec::new();
    for entry in value.split(',').filter(|entry| !entry.trim().is_empty()) {
        let parts: Vec<&str> = entry.split(':').collect();
        if parts.len() < 2 || parts.len() > 3 {
            return Err(format!("invalid FANPLAY_INPUT entry: {entry}"));
        }
        let parse = |part: &str| {
            part.trim()
                .parse::<u64>()
                .map_err(|_| format!("invalid number in FANPLAY_INPUT entry: {entry}"))
        };
        let start = parse(parts[0])?;
        let key = parts[1]
            .trim()
            .parse::<i32>()
            .map_err(|_| format!("invalid scancode in FANPLAY_INPUT entry: {entry}"))?;
        let hold = if parts.len() == 3 {
            parse(parts[2])?
        } else {
            1
        };
        if hold == 0 {
            return Err(format!("FANPLAY_INPUT hold must be positive: {entry}"));
        }
        keys.push(ScriptedKey { start, key, hold });
    }
    Ok(keys)
}

/// Validated, engine-owned RGSS1 tile properties for one bulk refresh.
pub struct NativeTileUpdate {
    pub id: u64,
    pub bitmap: u64,
    pub src: Rect,
    pub x: f64,
    pub y: f64,
    pub z: i32,
    pub visible: bool,
    pub viewport: u64,
    pub clip: Rect,
    pub color: Color,
    pub tone: Tone,
}
/// Geometry-only changes prepared without borrowing the engine during Ruby
/// access or conversion. Other retained sprite properties survive.
pub struct SpriteMotion {
    pub id: u64,
    pub position: Option<(f64, f64)>,
    pub z: Option<i32>,
}

impl Runtime {
    pub fn update_native_tiles(&mut self, updates: &[NativeTileUpdate], hidden: &[u64]) {
        for tile in updates {
            if let Some(sprite) = self.sprites.get_mut(&tile.id) {
                sprite.state.bitmap = tile.bitmap;
                sprite.state.src = tile.src;
                sprite.state.x = tile.x;
                sprite.state.y = tile.y;
                sprite.state.z = tile.z;
                sprite.state.visible = tile.visible;
                sprite.state.viewport = tile.viewport;
                sprite.state.clip = tile.clip;
                sprite.state.color = tile.color;
                sprite.state.tone = tile.tone;
            }
        }
        for id in hidden {
            if let Some(sprite) = self.sprites.get_mut(id) {
                sprite.state.visible = false;
            }
        }
        self.invalidate_scene();
    }
    pub fn move_sprites(&mut self, motions: &[SpriteMotion]) {
        let mut order_changed = false;
        for motion in motions {
            if let Some(sprite) = self.sprites.get_mut(&motion.id) {
                if let Some((x, y)) = motion.position {
                    sprite.state.x = x;
                    sprite.state.y = y;
                }
                if let Some(z) = motion.z {
                    order_changed |= sprite.visible && sprite.z != z;
                    sprite.state.z = z;
                }
            }
        }
        if order_changed {
            self.invalidate_scene();
        }
    }
    pub fn sync_sprite(&mut self, id: u64, state: SpriteState) -> Result<(), String> {
        let sprite = self.sprites.get_mut(&id).ok_or("disposed sprite")?;
        // Draw order depends on z, viewport membership and visibility only; y is
        // not part of the sort key, so moving a sprite does not reorder it.
        let order_changed = sprite.visible != state.visible
            || (state.visible && (sprite.viewport != state.viewport || sprite.z != state.z));
        sprite.state = state;
        if order_changed {
            self.invalidate_scene();
        }
        Ok(())
    }
    pub fn new(options: RuntimeOptions) -> Result<Self, String> {
        let target = Bitmap::new(options.width, options.height).map_err(|e| e.to_string())?;
        Ok(Self {
            gpu_frame: false,
            #[cfg(feature = "gpu")]
            scene_plan: Vec::new(),
            #[cfg(feature = "gpu")]
            scene_dirty: true,
            #[cfg(feature = "gpu")]
            scene_cache: std::env::var("FANPLAY_SCENE_CACHE").as_deref() != Ok("0"),
            next_frame_deadline: None,
            font_cache: HashMap::new(),
            fullscreen: false,
            scale: 1.0,
            frozen: None,
            transitioning: None,
            brightness: 255,
            target,
            bitmaps: HashMap::new(),
            sprites: HashMap::new(),
            map_cameras: HashMap::new(),
            planes: HashMap::new(),
            viewports: HashMap::new(),
            next_id: 1,
            movies: HashMap::new(),
            movie_start: HashMap::new(),
            host: options.host,
            headless: options.headless,
            input: Input::default(),
            pending: Input::default(),
            queued: Vec::new(),
            text_buffer: String::new(),
            text_input: false,
            frame: 0,
            updates: 0,
            frame_limit: options.frame_limit,
            rate: options.rate,
            fast_forward: options.fast_forward,
            show_counter: false,
            fps: 0.0,
            fps_window_start: 0.0,
            fps_window_frames: 0,
            animation_time: 0.0,
            version: options.version,
            archives: options.archives,
            scripted: options.scripted,
            scripted_held: Vec::new(),
        })
    }

    /// The current framebuffer, for hosts that present it themselves.
    pub fn target(&self) -> &Bitmap {
        &self.target
    }
    /// Completed RGSS frames, for host-side performance displays.
    pub fn frame_count(&self) -> u64 {
        self.frame
    }
    /// The frame rate currently in effect.
    pub fn rate(&self) -> u32 {
        self.rate
    }
    /// Whether frame pacing is disabled in the engine.
    pub fn is_fast_forward(&self) -> bool {
        self.fast_forward
    }
    /// Synchronize only for explicit CPU reads (screenshots, freeze, snap).
    pub fn capture(&mut self) -> Result<&Bitmap, String> {
        #[cfg(feature = "gpu")]
        if self.gpu_frame {
            self.target = self
                .host
                .gpu()
                .ok_or("GPU frame has no owning device")?
                .readback()?;
            self.gpu_frame = false;
        }
        Ok(&self.target)
    }
    /// Queues a host input event, applied at the start of the next update.
    pub fn queue_input(&mut self, event: InputEvent) {
        self.queued.push(event);
    }

    fn bitmap(&self, id: u64) -> Result<&Bitmap, String> {
        self.bitmaps
            .get(&id)
            .ok_or_else(|| "disposed bitmap".into())
    }
    fn bitmap_mut(&mut self, id: u64) -> Result<&mut Bitmap, String> {
        self.bitmaps
            .get_mut(&id)
            .ok_or_else(|| "disposed bitmap".into())
    }
    fn id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
    fn apply_queued(&mut self) {
        for event in self.queued.drain(..) {
            match event {
                InputEvent::KeyDown(key) => self.pending.key_down(key),
                InputEvent::KeyUp(key) => self.pending.key_up(key),
                InputEvent::Text(text) => self.pending.push_text(&text),
                InputEvent::Mouse { x, y, inside } => {
                    self.pending.mouse_x = x;
                    self.pending.mouse_y = y;
                    self.pending.mouse_inside = inside;
                }
                InputEvent::Scroll(delta) => self.pending.scroll_v += delta,
            }
        }
    }
    /// Applies scripted headless input for the current frame.
    fn inject_scripted(&mut self) {
        if self.scripted.is_empty() {
            return;
        }
        let frame = self.updates;
        let mut desired = Vec::new();
        for entry in &self.scripted {
            if frame >= entry.start && frame < entry.start + entry.hold {
                desired.push(entry.key);
            }
        }
        for &key in &self.scripted_held {
            if !desired.contains(&key) {
                self.pending.key_up(key);
            }
        }
        for &key in &desired {
            if !self.scripted_held.contains(&key) {
                self.pending.key_down(key);
            }
        }
        self.scripted_held = desired;
    }
}
