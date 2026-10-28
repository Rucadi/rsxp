#![cfg(target_arch = "wasm32")]

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use eframe::{App, Frame, Renderer, WebOptions, WebRunner};
use egui::{Context, Ui};
use fanplay_core::audio::{
    AudioChannel, AudioConfig, AudioSink, AudioStream, PcmBuffer, PcmRing, PcmRingConsumer,
    PcmRingProducer,
};
use fanplay_core::{
    Button, ButtonSet, Content, ContentProbe, Core, CoreCapabilities, CoreCommand, CoreContext,
    CoreControl, CoreEvent, CoreInfo, EventSink, ExitStatus, InputEvent, PixelFormat, Platform,
    PlatformError, PlatformResult, RunSliceStatus, SessionConfig, VideoFrame, VideoSink,
};
use fanplay_extension_api::{OperationError, OperationRequest, OperationResponse};
use fanplay_frontend::{video::convert_rgba, ContentId};
use fanplay_ui::{
    session::{SessionCapabilities, SessionPanel},
    ui::backend::Backend,
    SharedApp, StateSlot, WorkspaceInfo,
};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use wasm_bindgen::{closure::Closure, prelude::*, JsCast};
use wasm_bindgen_futures::JsFuture;
use zip::ZipArchive;

mod game_input;
mod wgpu_frame;
mod worker;

#[wasm_bindgen(inline_js = r#"
function state() {
  if (globalThis.__fanplay_webview_state) return globalThis.__fanplay_webview_state;
  const value = {
    build: 'webview-bridge-v4',
    next: 1,
    queue: [],
    pending: new Map(),
    overlay: null,
    diagnostics() {
      return globalThis.__fanplay_webview_diagnostics === true
        || new URLSearchParams(globalThis.location?.search || '').get('fanplay_webview_diagnostics') === '1';
    },
    trace(event, payload = {}) {
      if (value.diagnostics()) console.info('[FANPLAY webview]', event, payload);
    },
    take() {
      const messages = value.queue;
      value.queue = [];
      return messages;
    },
    respond(id, status, body) {
      const request = value.pending.get(id);
      const text = String(body || '');
      value.trace('response', {id, status:Number(status) || 500, bodyLength:text.length, body:text.slice(0, 500)});
      if (!request) {
        if (value.diagnostics()) console.warn('[FANPLAY webview] response without pending request', id);
        return;
      }
      value.pending.delete(id);
      request.resolve({status: Number(status) || 500, body:text});
    },
    close() {
      value.trace('close');
      value.overlay?.remove();
      value.overlay = null;
    },
    open(viewId, title, html) {
      value.close();
      value.trace('open', {viewId, title, htmlLength:String(html || '').length});
      const document = globalThis.document;
      if (!document) return;
      const overlay = document.createElement('div');
      overlay.id = 'fanplay-plugin-webview';
      // The plugin window is a modeless tool window: keep the dashboard and
      // game fully visible instead of dimming everything below the webview.
      overlay.style.cssText = 'position:fixed;inset:0;z-index:2147483000;display:flex;align-items:center;justify-content:center;background:transparent;padding:3vh 3vw;pointer-events:none;';
      const panel = document.createElement('div');
      panel.style.cssText = 'width:min(1180px,96vw);height:min(860px,94vh);display:flex;flex-direction:column;overflow:hidden;background:#101220;border:1px solid #48506d;border-radius:12px;box-shadow:0 20px 80px rgba(0,0,0,.55);';
      const header = document.createElement('div');
      header.style.cssText = 'display:flex;align-items:center;justify-content:space-between;gap:12px;padding:10px 14px;background:#171a2c;color:#f3f4ff;font:600 14px/1.2 system-ui,sans-serif;cursor:grab;touch-action:none;user-select:none;';
      const heading = document.createElement('span');
      heading.textContent = String(title || 'Plugin');
      const close = document.createElement('button');
      close.type = 'button';
      close.textContent = '×';
      close.setAttribute('aria-label', 'Close plugin window');
      close.style.cssText = 'border:1px solid #48506d;border-radius:7px;background:#20243a;color:#f3f4ff;font-size:20px;line-height:1;padding:2px 10px;cursor:pointer;';
      close.onclick = () => value.close();
      header.append(heading, close);
      let drag = null;
      const stopDrag = () => {
        drag = null;
        header.style.cursor = 'grab';
        header.removeEventListener('pointermove', moveDrag);
      };
      const moveDrag = event => {
        if (!drag) return;
        const left = Math.min(Math.max(8, drag.left + event.clientX - drag.x), Math.max(8, innerWidth - drag.width - 8));
        const top = Math.min(Math.max(8, drag.top + event.clientY - drag.y), Math.max(8, innerHeight - drag.height - 8));
        panel.style.left = `${left}px`;
        panel.style.top = `${top}px`;
      };
      header.addEventListener('pointerdown', event => {
        if (event.button !== 0 || event.target.closest('button')) return;
        const rect = panel.getBoundingClientRect();
        panel.style.position = 'fixed';
        panel.style.left = `${rect.left}px`;
        panel.style.top = `${rect.top}px`;
        panel.style.width = `${rect.width}px`;
        panel.style.height = `${rect.height}px`;
        drag = {x:event.clientX, y:event.clientY, left:rect.left, top:rect.top, width:rect.width, height:rect.height};
        header.style.cursor = 'grabbing';
        header.setPointerCapture?.(event.pointerId);
        header.addEventListener('pointermove', moveDrag);
        header.addEventListener('pointerup', stopDrag, {once:true});
        header.addEventListener('pointercancel', stopDrag, {once:true});
      });
      const frame = document.createElement('iframe');
      frame.title = String(title || 'Plugin');
      frame.style.cssText = 'border:0;display:block;flex:1;width:100%;background:#0c0d15;';
      const literal = JSON.stringify(String(viewId || 'plugin')).replaceAll('<', '\\u003c');
      const bridge = `<script>(()=>{const viewId=${literal};const host=()=>window.parent.__fanplay_webview_state;const request=(kind,payload)=>new Promise((resolve,reject)=>{const state=host();if(!state){reject(new Error('FANPLAY local bridge host is unavailable'));return;}const id=state.next++;const info={id,kind,view_id:viewId};if(kind==='api'){info.method=payload.method;info.target=payload.target;info.bodyLength=String(payload.body||'').length;}else{info.category=payload.category;info.name=payload.name;}state.trace('request',info);const timer=setTimeout(()=>{state.pending.delete(id);state.trace('timeout',info);reject(new Error('FANPLAY local bridge timed out ('+kind+' '+id+')'));},15000);state.pending.set(id,{resolve:value=>{clearTimeout(timer);resolve(value);},reject});state.queue.push(Object.assign({kind,id,view_id:viewId},payload));});window.FANPLAY=window.FANPLAY||{};window.FANPLAY.api=(method,target,body='')=>request('api',{method:String(method||'GET').toUpperCase(),target:String(target||'/'),body:String(body||'')}).then(value=>new Response(value.body||'',{status:value.status||500,headers:{'content-type':'application/json'}}));window.FANPLAY.http=window.FANPLAY.api;window.FANPLAY.asset=(category,name)=>request('asset',{category:String(category||''),name:String(name||'')}).then(value=>{try{return JSON.parse(value.body||'{}').src||null;}catch(error){console.error('[FANPLAY webview] invalid asset response',error);return null;}});const report=(kind,error)=>{const message=String(error?.stack||error);console.error('[FANPLAY webview] iframe '+kind,message);};window.addEventListener('error',event=>report('error',event.error||event.message));window.addEventListener('unhandledrejection',event=>report('unhandled rejection',event.reason));const nativeFetch=window.fetch.bind(window);window.fetch=(input,init={})=>{const raw=typeof input==='string'?input:input.url;if(!raw.startsWith('/v1/'))return nativeFetch(input,init);const method=String(init.method||(typeof input==='string'?'GET':input.method)||'GET').toUpperCase();return window.FANPLAY.api(method,raw,init.body||'');};})();</script>`;
      frame.srcdoc = bridge + String(html || '');
      panel.append(header, frame);
      panel.style.pointerEvents = 'auto';
      overlay.append(panel);
      document.body.append(overlay);
      value.overlay = overlay;
    }
  };
  globalThis.__fanplay_webview_state = value;
  return value;
}
export function fanplay_webview_install() { state(); }
export function fanplay_webview_open(viewId, title, html) { state().open(viewId, title, html); }
export function fanplay_webview_close() { state().close(); }
export function fanplay_webview_take() { return state().take(); }
export function fanplay_webview_respond(id, status, body) { state().respond(Number(id), Number(status), String(body || '')); }
export function fanplay_webview_diagnostics() { return state().diagnostics(); }
"#)]
extern "C" {
    fn fanplay_webview_install();
    fn fanplay_webview_open(view_id: &str, title: &str, html: &str);
    fn fanplay_webview_close();
    fn fanplay_webview_take() -> JsValue;
    fn fanplay_webview_respond(id: u64, status: u16, body: &str);
    fn fanplay_webview_diagnostics() -> bool;
}

fn webview_trace(message: impl AsRef<str>) {
    if fanplay_webview_diagnostics() {
        web_sys::console::info_1(&JsValue::from_str(&format!(
            "[FANPLAY webview] {}",
            message.as_ref()
        )));
    }
}

fn webview_trace_error(message: impl AsRef<str>) {
    web_sys::console::error_1(&JsValue::from_str(&format!(
        "[FANPLAY webview] {}",
        message.as_ref()
    )));
}

fn normalize_browser_path(path: &str) -> String {
    path.replace('\\', "/")
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>()
        .join("/")
        .to_ascii_lowercase()
}

#[derive(Clone)]
struct BrowserGame {
    label: String,
    entry: String,
    files: Arc<HashMap<String, Vec<u8>>>,
}

impl BrowserGame {
    fn from_single(name: String, bytes: Vec<u8>) -> Result<Self, String> {
        if name.to_ascii_lowercase().ends_with(".zip") {
            return Self::from_zip(name, &bytes);
        }
        let entry = normalize_browser_path(&name);
        if entry.is_empty() {
            return Err("selected game file has no usable name".into());
        }
        let mut files = HashMap::new();
        files.insert(entry.clone(), bytes);
        let label = Path::new(&name)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(&name)
            .to_owned();
        if matches!(
            Path::new(&entry).extension().and_then(|ext| ext.to_str()),
            Some("rgssad" | "rgss2a" | "rgss3a" | "rxdata" | "rvdata" | "rvdata2")
        ) {
            Self::from_files(label, files)
        } else {
            Ok(Self {
                label,
                entry,
                files: Arc::new(files),
            })
        }
    }

    fn from_zip(name: String, bytes: &[u8]) -> Result<Self, String> {
        let mut archive = ZipArchive::new(Cursor::new(bytes))
            .map_err(|error| format!("could not open {name} as ZIP: {error}"))?;
        let mut files = HashMap::new();
        for index in 0..archive.len() {
            let mut file = archive
                .by_index(index)
                .map_err(|error| format!("could not read ZIP entry {index}: {error}"))?;
            if file.is_dir() {
                continue;
            }
            let path = normalize_browser_path(file.name());
            if path.is_empty() || path.split('/').any(|part| part == "..") {
                continue;
            }
            let mut data = Vec::new();
            file.read_to_end(&mut data)
                .map_err(|error| format!("could not extract ZIP entry {path}: {error}"))?;
            files.insert(path, data);
        }
        Self::from_files(
            Path::new(&name)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("game")
                .to_owned(),
            files,
        )
    }

    fn from_files(label: String, mut files: HashMap<String, Vec<u8>>) -> Result<Self, String> {
        let root = files
            .keys()
            .next()
            .and_then(|path| path.split('/').next())
            .map(str::to_owned);
        let strip_root = root.as_deref().is_some_and(|root| {
            files.keys().all(|path| {
                path.starts_with(root) && path.as_bytes().get(root.len()) == Some(&b'/')
            })
        });
        if strip_root {
            let root = root.unwrap_or_default();
            files = files
                .into_iter()
                .filter_map(|(path, bytes)| {
                    let path = path.strip_prefix(&root)?.strip_prefix('/')?;
                    Some((path.to_owned(), bytes))
                })
                .collect();
        }
        let entry = ["game.rgss3a", "game.rgss2a", "game.rgssad"]
            .iter()
            .find_map(|candidate| files.keys().find(|path| path.ends_with(candidate)))
            .or_else(|| {
                files.keys().find(|path| {
                    matches!(
                        Path::new(path).extension().and_then(|ext| ext.to_str()),
                        Some("rgssad" | "rgss2a" | "rgss3a")
                    )
                })
            })
            .or_else(|| {
                [
                    "data/scripts.rvdata2",
                    "data/scripts.rvdata",
                    "data/scripts.rxdata",
                ]
                .iter()
                .find_map(|candidate| files.keys().find(|path| path == candidate))
            })
            .cloned()
            .ok_or_else(|| {
                "folder or ZIP does not contain a Game.rgss* archive or Data/Scripts.*".to_owned()
            })?;
        Ok(Self {
            label,
            entry,
            files: Arc::new(files),
        })
    }

    fn from_dropped_files(label: String, entries: Vec<(String, Vec<u8>)>) -> Result<Self, String> {
        let files = entries
            .into_iter()
            .map(|(path, bytes)| (normalize_browser_path(&path), bytes))
            .collect();
        Self::from_files(label, files)
    }
}

#[derive(Default)]
struct BrowserPlatform {
    files: Arc<HashMap<String, Vec<u8>>>,
    saves: Mutex<BTreeMap<String, Vec<u8>>>,
    content_id: String,
    content_name: String,
}

impl BrowserPlatform {
    const SYSTEM_STORAGE_VERSION: &str = "v2";

    fn with_files(
        files: Arc<HashMap<String, Vec<u8>>>,
        content_id: String,
        content_name: String,
    ) -> Self {
        Self {
            files,
            saves: Mutex::new(BTreeMap::new()),
            content_id,
            content_name,
        }
    }

    fn storage() -> Option<web_sys::Storage> {
        web_sys::window()?.local_storage().ok().flatten()
    }

    fn save_key(&self, name: &str) -> String {
        format!("fanplay.save.{}.{}", self.content_id, name)
    }

    fn backup_key(&self, name: &str) -> String {
        format!("fanplay.backup.{}.{}", self.content_id, name)
    }

    fn system_key(name: &str) -> String {
        format!("fanplay.system.{}.{}", Self::SYSTEM_STORAGE_VERSION, name)
    }

    fn write_system(name: &str, bytes: &[u8]) -> PlatformResult<()> {
        let Some(storage) = Self::storage() else {
            return Err(PlatformError::unsupported("browser storage is unavailable"));
        };
        let name = Path::new(name)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(name)
            .to_ascii_lowercase();
        storage
            .set_item(&Self::system_key(&name), &BASE64.encode(bytes))
            .map_err(|error| {
                PlatformError::io(format!("could not persist browser system file: {error:?}"))
            })
    }

    fn create_backup(&self, name: &str) -> PlatformResult<()> {
        let Some(bytes) = self.read_save(name)? else {
            return Ok(());
        };
        let Some(storage) = Self::storage() else {
            return Err(PlatformError::unsupported("browser storage is unavailable"));
        };
        storage
            .set_item(&self.backup_key(name), &BASE64.encode(bytes))
            .map_err(|error| {
                PlatformError::io(format!("could not create browser backup: {error:?}"))
            })
    }

    fn restore_backup(&self, name: &str) -> PlatformResult<bool> {
        let Some(storage) = Self::storage() else {
            return Err(PlatformError::unsupported("browser storage is unavailable"));
        };
        let Some(encoded) = storage.get_item(&self.backup_key(name)).map_err(|error| {
            PlatformError::io(format!("could not read browser backup: {error:?}"))
        })?
        else {
            return Ok(false);
        };
        let bytes = BASE64
            .decode(encoded)
            .map_err(|error| PlatformError::io(format!("browser backup is corrupt: {error}")))?;
        self.write_save(name, &bytes)?;
        Ok(true)
    }
}

impl Platform for BrowserPlatform {
    fn read_content(&self, path: &str) -> PlatformResult<Vec<u8>> {
        let requested = normalize_browser_path(path);
        let content_name = normalize_browser_path(&self.content_name);
        let content_extension = Path::new(&content_name)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default();
        let default_script = match content_extension {
            "rxdata" => "data/scripts.rxdata",
            "rvdata" => "data/scripts.rvdata",
            "rvdata2" => "data/scripts.rvdata2",
            _ => "",
        };
        let lookup = if requested.is_empty() {
            Some(content_name.as_str())
        } else if requested == content_name {
            Some(content_name.as_str())
        } else if !default_script.is_empty() && requested == default_script {
            Some(content_name.as_str())
        } else {
            Some(requested.as_str())
        };
        self.files
            .get(lookup.unwrap_or_default())
            .cloned()
            .ok_or_else(|| PlatformError::not_found(path))
    }

    fn read_system_file(&self, path: &str) -> PlatformResult<Option<Vec<u8>>> {
        let Some(name) = Path::new(path).file_name().and_then(|name| name.to_str()) else {
            return Ok(None);
        };
        if !matches!(
            name.to_ascii_lowercase().as_str(),
            "bios7.bin" | "bios9.bin" | "firmware.bin" | "gba_bios.bin" | "bios.bin"
        ) {
            return Ok(None);
        }
        let Some(storage) = Self::storage() else {
            return Ok(None);
        };
        let Some(encoded) = storage.get_item(&Self::system_key(name)).map_err(|error| {
            PlatformError::io(format!("could not read browser system file: {error:?}"))
        })?
        else {
            return Ok(None);
        };
        BASE64
            .decode(encoded)
            .map(Some)
            .map_err(|error| PlatformError::io(format!("browser system file is corrupt: {error}")))
    }

    fn read_save(&self, name: &str) -> PlatformResult<Option<Vec<u8>>> {
        if let Some(bytes) = self
            .saves
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(name)
            .cloned()
        {
            return Ok(Some(bytes));
        }
        let Some(storage) = Self::storage() else {
            return Ok(None);
        };
        let Some(encoded) = storage.get_item(&self.save_key(name)).map_err(|error| {
            PlatformError::io(format!("could not read browser save: {error:?}"))
        })?
        else {
            return Ok(None);
        };
        let bytes = BASE64
            .decode(encoded)
            .map_err(|error| PlatformError::io(format!("browser save is corrupt: {error}")))?;
        self.saves
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(name.to_owned(), bytes.clone());
        Ok(Some(bytes))
    }

    fn write_save(&self, name: &str, bytes: &[u8]) -> PlatformResult<()> {
        self.saves
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(name.to_owned(), bytes.to_vec());
        if let Some(storage) = Self::storage() {
            storage
                .set_item(&self.save_key(name), &BASE64.encode(bytes))
                .map_err(|error| {
                    PlatformError::io(format!("could not persist browser save: {error:?}"))
                })?;
        }
        Ok(())
    }

    fn save_dir(&self) -> PathBuf {
        PathBuf::new()
    }

    fn now(&self) -> f64 {
        worker::now()
    }

    fn log(&self, level: fanplay_core::LogLevel, message: &str) {
        let value = JsValue::from_str(message);
        match level {
            fanplay_core::LogLevel::Error => web_sys::console::error_1(&value),
            fanplay_core::LogLevel::Warn => web_sys::console::warn_1(&value),
            fanplay_core::LogLevel::Info | fanplay_core::LogLevel::Debug => {
                web_sys::console::log_1(&value)
            }
        }
    }
}

struct PresentedFrame {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    format: PixelFormat,
    revision: u64,
    game_frames: u64,
    text_input: bool,
}

impl Default for PresentedFrame {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            pixels: Vec::new(),
            format: PixelFormat::Rgba8,
            revision: 0,
            game_frames: 0,
            text_input: false,
        }
    }
}

struct BrowserVideo {
    frame: Arc<Mutex<PresentedFrame>>,
}

impl VideoSink for BrowserVideo {
    fn present(&mut self, frame: VideoFrame<'_>) {
        let mut presented = self.frame.lock().unwrap_or_else(|error| error.into_inner());
        // RGSS presents the software bitmap as tightly packed BGRA on little
        // endian targets. Preserve that layout until the egui upload instead
        // of converting BGRA -> RGBA here and then RGBA -> Color32 later.
        if frame.validate().is_ok()
            && matches!(frame.format, PixelFormat::Rgba8 | PixelFormat::Bgra8)
            && frame.stride == frame.width as usize * 4
        {
            let row_len = frame.width as usize * 4;
            let byte_len = row_len * frame.height as usize;
            presented.pixels.resize(byte_len, 0);
            presented.pixels.copy_from_slice(&frame.pixels[..byte_len]);
            presented.format = frame.format;
        } else if convert_rgba(&frame, &mut presented.pixels).is_ok() {
            presented.format = PixelFormat::Rgba8;
        } else {
            return;
        }
        presented.width = frame.width;
        presented.height = frame.height;
        presented.revision = presented.revision.wrapping_add(1);
    }
}

struct BrowserEvents {
    events: Arc<Mutex<Vec<CoreEvent>>>,
}

impl EventSink for BrowserEvents {
    fn emit(&mut self, event: CoreEvent) {
        self.events
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(event);
    }
}

// RGSS audio arrives here as an encoded asset stream.  The PCM ring below is
// still used by the GBA/GB/NDS cores, but feeding RGSS bytes through that ring
// would require a second decoder and would make the browser behave
// differently from the desktop backend.  Keep the browser decoder in the
// browser: HTMLAudioElement supports the formats accepted by the game and can
// handle looping, seeking and playback-rate changes without blocking the core.
#[wasm_bindgen(inline_js = r#"
const fanplayAudioChannels = new Map();
const fanplayAudioVolumes = new Map([[0, 100], [1, 100], [2, 100], [3, 100]]);

function fanplayAudioClamp(value, low, high) {
  return Math.min(high, Math.max(low, Number(value) || 0));
}

function fanplayAudioMime(path) {
  const extension = String(path).split(/[./\\]/).pop().toLowerCase();
  switch (extension) {
    case "ogg": return "audio/ogg";
    case "mp3": return "audio/mpeg";
    case "wav": return "audio/wav";
    case "m4a": return "audio/mp4";
    case "aac": return "audio/aac";
    case "flac": return "audio/flac";
    default: return "application/octet-stream";
  }
}

function fanplayAudioSingleChannel(channel) {
  // RGSS's SE channel is allowed to overlap; the other channels replace the
  // currently playing voice when a new file is selected.
  return channel !== 3;
}

function fanplayAudioItems(channel) {
  return fanplayAudioChannels.get(channel) || [];
}

function fanplayAudioRemove(item) {
  const items = fanplayAudioItems(item.channel);
  const remaining = items.filter((candidate) => candidate !== item);
  if (remaining.length === 0) {
    fanplayAudioChannels.delete(item.channel);
  } else {
    fanplayAudioChannels.set(item.channel, remaining);
  }
  item.audio.pause();
  item.audio.removeAttribute("src");
  item.audio.load();
  URL.revokeObjectURL(item.url);
}

function fanplayAudioStopItem(item) {
  item.audio.pause();
  item.audio.currentTime = 0;
  fanplayAudioRemove(item);
}

function fanplayAudioApplyVolume(item) {
  const master = fanplayAudioVolumes.get(item.channel) ?? 100;
  item.audio.volume = fanplayAudioClamp(item.volume * master / 10000, 0, 1);
}

export function fanplay_audio_play(bytes, channel, path, volume, pitch, position, looping) {
  channel = channel | 0;
  if (fanplayAudioSingleChannel(channel)) {
    fanplay_audio_stop(channel);
  }
  const blob = new Blob([bytes], { type: fanplayAudioMime(path) });
  const audio = new Audio();
  const item = {
    audio,
    channel,
    url: URL.createObjectURL(blob),
    volume: fanplayAudioClamp(volume, 0, 100),
  };
  audio.preload = "auto";
  audio.loop = Boolean(looping);
  audio.playbackRate = fanplayAudioClamp(pitch / 100, 0.01, 10);
  audio.src = item.url;
  audio.addEventListener("ended", () => {
    if (!audio.loop) fanplayAudioRemove(item);
  }, { once: false });
  fanplayAudioApplyVolume(item);
  fanplayAudioChannels.set(channel, [...fanplayAudioItems(channel), item]);

  const start = () => {
    const offset = Math.max(0, Number(position) || 0);
    if (offset > 0) {
      try { audio.currentTime = offset; } catch (_) { /* metadata is late */ }
    }
    const promise = audio.play();
    if (promise && typeof promise.catch === "function") {
      promise.catch((error) => console.warn("FANPLAY audio could not start", error));
    }
  };
  if (audio.readyState >= 1) {
    start();
  } else {
    audio.addEventListener("loadedmetadata", start, { once: true });
  }
}

export function fanplay_audio_stop(channel) {
  channel = channel | 0;
  for (const item of [...fanplayAudioItems(channel)]) fanplayAudioStopItem(item);
}

export function fanplay_audio_fade(channel, milliseconds) {
  channel = channel | 0;
  const duration = Math.max(0, Number(milliseconds) || 0);
  const items = [...fanplayAudioItems(channel)];
  if (duration === 0) {
    for (const item of items) fanplayAudioStopItem(item);
    return;
  }
  const started = performance.now();
  for (const item of items) {
    const initial = item.audio.volume;
    const step = (now) => {
      if (!fanplayAudioItems(channel).includes(item)) return;
      const progress = Math.min(1, (now - started) / duration);
      item.audio.volume = initial * (1 - progress);
      if (progress < 1) requestAnimationFrame(step);
      else fanplayAudioStopItem(item);
    };
    requestAnimationFrame(step);
  }
}

export function fanplay_audio_position(channel) {
  const item = fanplayAudioItems(channel | 0)[0];
  return item ? (Number(item.audio.currentTime) || 0) : 0;
}

export function fanplay_audio_set_volume(channel, volume) {
  channel = channel | 0;
  const value = fanplayAudioClamp(volume, 0, 100);
  fanplayAudioVolumes.set(channel, value);
  for (const item of fanplayAudioItems(channel)) fanplayAudioApplyVolume(item);
}

export function fanplay_audio_volume(channel) {
  return fanplayAudioVolumes.get(channel | 0) ?? 100;
}

export function fanplay_audio_pause(paused) {
  for (const items of fanplayAudioChannels.values()) {
    for (const item of items) {
      if (paused) {
        item.audio.pause();
      } else {
        const promise = item.audio.play();
        if (promise && typeof promise.catch === "function") promise.catch(() => {});
      }
    }
  }
}
"#)]
extern "C" {
    fn fanplay_audio_play(
        bytes: &[u8],
        channel: u8,
        path: &str,
        volume: i32,
        pitch: f64,
        position: f64,
        looping: bool,
    );
    fn fanplay_audio_stop(channel: u8);
    fn fanplay_audio_fade(channel: u8, milliseconds: i32);
    fn fanplay_audio_position(channel: u8) -> f64;
    fn fanplay_audio_set_volume(channel: u8, volume: i32);
    fn fanplay_audio_volume(channel: u8) -> i32;
    fn fanplay_audio_pause(paused: bool);
}

#[derive(Clone)]
struct BrowserAudioSink {
    producer: PcmRingProducer,
    consumer: Arc<Mutex<PcmRingConsumer>>,
    config: AudioConfig,
    paused: Arc<AtomicBool>,
}

impl AudioSink for BrowserAudioSink {
    fn play_stream(&mut self, stream: AudioStream<'_>) {
        if self.paused.load(Ordering::Acquire) || stream.data.is_empty() {
            return;
        }
        fanplay_audio_play(
            stream.data,
            stream.channel.0,
            stream.path,
            stream.volume,
            stream.pitch,
            stream.position,
            stream.looping,
        );
    }

    fn stop(&mut self, channel: AudioChannel) {
        fanplay_audio_stop(channel.0);
    }

    fn fade(&mut self, channel: AudioChannel, milliseconds: i32) {
        fanplay_audio_fade(channel.0, milliseconds);
    }

    fn position(&self, channel: AudioChannel) -> f64 {
        fanplay_audio_position(channel.0)
    }

    fn set_volume(&mut self, channel: AudioChannel, volume: i32) {
        fanplay_audio_set_volume(channel.0, volume);
    }

    fn volume(&self, channel: AudioChannel) -> i32 {
        fanplay_audio_volume(channel.0)
    }

    fn queue_pcm(&mut self, pcm: PcmBuffer<'_>) {
        if self.paused.load(Ordering::Acquire) || pcm.samples.is_empty() {
            return;
        }
        if pcm.channels == self.config.channels {
            self.producer.push(pcm.samples);
        } else if pcm.channels == 1 && self.config.channels == 2 {
            let mut stereo = Vec::with_capacity(pcm.samples.len() * 2);
            for sample in pcm.samples {
                stereo.extend_from_slice(&[*sample, *sample]);
            }
            self.producer.push(&stereo);
        }
    }

    fn pause(&mut self, paused: bool) {
        self.paused.store(paused, Ordering::Release);
        fanplay_audio_pause(paused);
    }

    fn output_config(&self) -> Option<AudioConfig> {
        Some(self.config)
    }

    fn reset_pcm(&mut self) {
        if let Ok(consumer) = self.consumer.lock() {
            consumer.reset();
        }
    }

    fn pcm_status(&self) -> fanplay_core::PcmStatus {
        self.consumer.lock().map_or_else(
            |_| fanplay_core::PcmStatus::default(),
            |consumer| consumer.status(2),
        )
    }
}

#[derive(Default)]
struct BrowserRuntimeState {
    control: Option<Arc<CoreControl>>,
    core_id: Option<&'static str>,
    files: Option<Arc<HashMap<String, Vec<u8>>>>,
    asset_cache: HashMap<String, Option<Arc<image::RgbaImage>>>,
}

#[derive(Clone, Default)]
struct BrowserBackend {
    state: Arc<Mutex<BrowserRuntimeState>>,
}

impl BrowserBackend {
    fn attach(
        &self,
        control: Arc<CoreControl>,
        core_id: &'static str,
        files: Arc<HashMap<String, Vec<u8>>>,
    ) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.control = Some(control);
        state.core_id = Some(core_id);
        state.files = Some(files);
        state.asset_cache.clear();
    }

    fn control(&self) -> Option<Arc<CoreControl>> {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .control
            .clone()
    }
}

impl Backend for BrowserBackend {
    fn fast_forward(&self) -> bool {
        self.control().is_some_and(|control| control.fast_forward())
    }

    fn set_fast_forward(&self, enabled: bool) {
        if let Some(control) = self.control() {
            control.set_fast_forward(enabled);
        }
    }

    fn supports_speed_control(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .core_id
            .is_none_or(|core_id| core_id != "gbc")
    }

    fn current_rate(&self) -> u32 {
        self.control()
            .and_then(|control| control.target_fps())
            .unwrap_or_default()
    }

    fn set_rate_override(&self, rate: u32) {
        if let Some(control) = self.control() {
            control.set_target_fps((rate > 0).then_some(rate));
        }
    }

    fn request_reset(&self) {
        if let Some(control) = self.control() {
            let _ = control.send(CoreCommand::Reset);
        }
    }

    fn patch_enabled(&self, id: &str) -> bool {
        fanplay_rgss::runtime_patch::is_enabled(id)
    }

    fn set_patch_enabled(&self, id: &str, enabled: bool) -> bool {
        fanplay_rgss::runtime_patch::set_enabled(id, enabled)
    }

    fn patch_value(&self, id: &str) -> Option<f64> {
        fanplay_rgss::runtime_patch::value(id)
    }

    fn set_patch_value(&self, id: &str, value: f64) -> bool {
        fanplay_rgss::runtime_patch::set_value(id, value)
    }

    fn asset_image(&self, category: &str, name: &str) -> Option<Arc<image::RgbaImage>> {
        use std::path::{Component, Path};
        if name.is_empty()
            || Path::new(name).components().count() != 1
            || !Path::new(name)
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
            || !Path::new(category)
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
        {
            return None;
        }
        let key = format!("{category}/{name}").to_ascii_lowercase();
        let files = {
            let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if let Some(cached) = state.asset_cache.get(&key) {
                return cached.clone();
            }
            state.files.clone()?
        };
        let base = normalize_browser_path(&format!("Graphics/{category}/{name}"));
        let image = [
            base.clone(),
            format!("{base}.png"),
            format!("{base}.bmp"),
            format!("{base}.gif"),
            format!("{base}.jpg"),
            format!("{base}.jpeg"),
        ]
        .iter()
        .find_map(|path| files.get(path))
        .and_then(|bytes| image::load_from_memory(bytes).ok())
        .map(|image| Arc::new(image.to_rgba8()));
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .asset_cache
            .insert(key, image.clone());
        image
    }

    fn handle_plugin_operation(
        &self,
        operation: &str,
        arguments: &Value,
    ) -> Option<Result<Value, String>> {
        // The Online Play panel's relay/status operations are host services,
        // not Ruby game API calls. Keep them on the window side just like the
        // desktop backend; forwarding them into Ruby would report an
        // unrelated "GameAPI is not active" error in the plugin panel.
        fanplay_rgss::cable_club::handle_plugin_operation(operation, arguments, None)
    }
}

struct BrowserAudioDriver {
    context: web_sys::AudioContext,
    _node: web_sys::AudioNode,
    _callback: Closure<dyn FnMut(web_sys::AudioProcessingEvent)>,
}

struct BrowserAudio {
    sink: BrowserAudioSink,
    driver: Option<BrowserAudioDriver>,
}

impl BrowserAudio {
    fn new() -> Self {
        let (producer, consumer) = PcmRing::new(48_000 * 2 * 2);
        let consumer = Arc::new(Mutex::new(consumer));
        let paused = Arc::new(AtomicBool::new(false));
        let (config, driver) = match web_sys::AudioContext::new() {
            Ok(context) => {
                let sample_rate = context.sample_rate().round().max(1.0) as u32;
                let processor = context
                    .create_script_processor_with_buffer_size_and_number_of_input_channels_and_number_of_output_channels(
                        2048,
                        0,
                        2,
                    );
                match processor {
                    Ok(processor) => {
                        let callback_consumer = Arc::clone(&consumer);
                        let callback_paused = Arc::clone(&paused);
                        let callback =
                            Closure::wrap(Box::new(move |event: web_sys::AudioProcessingEvent| {
                                let Ok(output) = event.output_buffer() else {
                                    return;
                                };
                                let length = output.length() as usize;
                                let mut samples = vec![0_i16; length * 2];
                                if !callback_paused.load(Ordering::Acquire)
                                    && let Ok(consumer) = callback_consumer.lock()
                                {
                                    consumer.pop_fill(&mut samples);
                                }
                                let mut left = Vec::with_capacity(length);
                                let mut right = Vec::with_capacity(length);
                                for pair in samples.chunks_exact(2) {
                                    left.push(f32::from(pair[0]) / 32_768.0);
                                    right.push(f32::from(pair[1]) / 32_768.0);
                                }
                                let _ = output.copy_to_channel(&left, 0);
                                let _ = output.copy_to_channel(&right, 1);
                            })
                                as Box<dyn FnMut(web_sys::AudioProcessingEvent)>);
                        processor.set_onaudioprocess(Some(callback.as_ref().unchecked_ref()));
                        let node = processor.unchecked_into::<web_sys::AudioNode>();
                        let destination =
                            context.destination().unchecked_into::<web_sys::AudioNode>();
                        let connected = node.connect_with_audio_node(&destination).is_ok();
                        if connected {
                            (
                                AudioConfig::new(sample_rate, 2),
                                Some(BrowserAudioDriver {
                                    context,
                                    _node: node,
                                    _callback: callback,
                                }),
                            )
                        } else {
                            (AudioConfig::new(sample_rate, 2), None)
                        }
                    }
                    Err(_) => (AudioConfig::new(sample_rate, 2), None),
                }
            }
            Err(_) => (AudioConfig::new(48_000, 2), None),
        };
        Self {
            sink: BrowserAudioSink {
                producer,
                consumer,
                config,
                paused,
            },
            driver,
        }
    }

    fn sink(&self) -> BrowserAudioSink {
        self.sink.clone()
    }

    fn resume(&self) {
        if let Some(driver) = &self.driver {
            let _ = driver.context.resume();
        }
    }
}

struct WebSession {
    core: Box<dyn Core>,
    control: Arc<CoreControl>,
    platform: Arc<BrowserPlatform>,
    frame: Arc<Mutex<PresentedFrame>>,
    events: Arc<Mutex<Vec<CoreEvent>>>,
    plugin_responses: Vec<OperationResponse>,
    info: CoreInfo,
    save_name: String,
    state_slots: BTreeMap<String, Vec<u8>>,
    native_rate: u32,
    pacing_time: f64,
    frame_credit: f64,
    frames: u64,
    presented_frames: u64,
    uploaded_revision: u64,
    running: bool,
    remote: bool,
    fps_time: f64,
    fps_frames: u64,
    measured_fps: f64,
    native_status: Option<String>,
    remote_handle: Option<worker::RemoteHandle>,
}

impl WebSession {
    fn load(
        game: BrowserGame,
        audio: Box<dyn AudioSink>,
        frame: Option<Arc<Mutex<PresentedFrame>>>,
        initial_save: Option<Vec<u8>>,
    ) -> Result<Self, String> {
        let BrowserGame {
            label: name,
            entry,
            files,
        } = game;
        let bytes = files
            .get(&entry)
            .cloned()
            .ok_or_else(|| format!("selected RGSS entry {entry} is missing"))?;
        let path = PathBuf::from(&entry);
        let header_len = bytes.len().min(64);
        let probe = ContentProbe::new(&path, false, &bytes[..header_len]);
        let catalog = fanplay_cores::catalog();
        let registration = catalog
            .route_probe(&probe)
            .map_err(|error| error.to_string())?;
        let info = registration.info;
        let native_rate = if info.id == "rgss" {
            match path.extension().and_then(|extension| extension.to_str()) {
                Some(extension)
                    if matches!(extension.to_ascii_lowercase().as_str(), "rxdata" | "rgssad") =>
                {
                    40
                }
                _ => 60,
            }
        } else {
            60
        };
        let content_id = ContentId::of_bytes(&bytes).hex();
        let save_name = if info.id == "rgss" {
            // This is the actual file written by SaveData in the Ruby VFS.
            "Game.rxdata".into()
        } else {
            format!(
                "{}.sav",
                Path::new(&name)
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or("game")
            )
        };
        let platform = Arc::new(BrowserPlatform::with_files(
            files,
            content_id.clone(),
            entry.clone(),
        ));
        let state_slots = Self::load_state_slots(&content_id);
        let frame = frame.unwrap_or_else(|| Arc::new(Mutex::new(PresentedFrame::default())));
        {
            let mut presented = frame.lock().unwrap_or_else(|error| error.into_inner());
            presented.width = 0;
            presented.height = 0;
            presented.pixels.clear();
            presented.format = PixelFormat::Rgba8;
            presented.revision = 0;
            presented.game_frames = 0;
            presented.text_input = false;
        }
        let events = Arc::new(Mutex::new(Vec::new()));
        let control = Arc::new(CoreControl::new());
        // RGSS1 games run at 40 Hz by default. Forcing 60 adds 50% more Ruby
        // and software-rendering work and changes the game's timing.
        if info.id != "rgss" {
            control.set_target_fps(Some(60));
        }
        let context = CoreContext::new(
            Arc::clone(&platform) as Arc<dyn Platform>,
            Box::new(BrowserVideo {
                frame: Arc::clone(&frame),
            }),
            audio,
            Box::new(BrowserEvents {
                events: Arc::clone(&events),
            }),
            Arc::clone(&control),
        )
        .with_session(SessionConfig::headless());
        let remote = info.id == "rgss" && web_sys::window().is_some();
        let (mut core, remote_handle): (Box<dyn Core>, Option<worker::RemoteHandle>) = if remote {
            let remote_core = worker::RemoteCore::new(
                info,
                Arc::clone(&platform),
                Arc::clone(&frame),
                initial_save,
            );
            let handle = remote_core.handle();
            (Box::new(remote_core), Some(handle))
        } else {
            ((registration.create)(), None)
        };
        core.load(Content::file(path).with_label(name.clone()), context)
            .map_err(|error| error.to_string())?;

        Ok(Self {
            core,
            control,
            platform,
            frame,
            events,
            plugin_responses: Vec::new(),
            info,
            save_name,
            state_slots,
            native_rate,
            pacing_time: 0.0,
            frame_credit: 1.0,
            frames: 0,
            presented_frames: 0,
            uploaded_revision: 0,
            running: true,
            remote,
            fps_time: 0.0,
            fps_frames: 0,
            measured_fps: 0.0,
            native_status: None,
            remote_handle,
        })
    }

    /// Returns the number of Ruby/engine frames that are due for this browser
    /// repaint. BrowserPlatform::sleep is intentionally non-blocking, so the
    /// cooperative frontend owns the pacing instead of letting Ruby run once
    /// for every repaint (which is too fast for RGSS1's 40 Hz clock).
    fn tick_budget(&mut self) -> usize {
        // This only exchanges worker messages; it never executes Ruby. Keep
        // processing controls, errors and audio even while the game is paused.
        if self.remote {
            let now = self.platform.now();
            if self.pacing_time > now {
                return 0;
            }
            self.pacing_time = now + 1.0 / f64::from(self.effective_rate());
            return 1;
        }
        if self.control.paused() {
            self.pacing_time = self.platform.now();
            self.frame_credit = 0.0;
            return 0;
        }
        if self.control.fast_forward() {
            self.pacing_time = self.platform.now();
            self.frame_credit = 0.0;
            return 8;
        }

        let now = self.platform.now();
        if self.pacing_time > 0.0 && now >= self.pacing_time {
            let elapsed = (now - self.pacing_time).min(0.25);
            let rate = f64::from(self.effective_rate());
            // Drop old catch-up debt after a slow frame or background tab.
            // Replaying it before drawing the workspace causes UI starvation.
            self.frame_credit = (self.frame_credit + elapsed * rate).min(2.0);
        }
        self.pacing_time = now;
        let budget = self.frame_credit.floor().min(2.0) as usize;
        self.frame_credit -= budget as f64;
        budget
    }

    fn effective_rate(&self) -> u32 {
        self.control
            .target_fps()
            .unwrap_or_else(|| {
                if self.info.id == "rgss" {
                    fanplay_rgss::engine::with_runtime(|runtime| runtime.rate())
                        .unwrap_or(self.native_rate)
                } else {
                    self.native_rate
                }
            })
            .max(1)
    }

    fn measured_rate(&mut self) -> f64 {
        let now = self.platform.now();
        // The engine frame counter is useful for diagnostics, but a few Ruby
        // startup paths do not expose it until after the first scene. Count
        // frames that actually reached the frontend so the performance panel
        // still reports a real rate during that period.
        let presented_frames = self.presented_frames;
        if self.fps_time == 0.0 {
            self.fps_time = now;
            self.fps_frames = presented_frames;
        } else if now - self.fps_time >= 0.5 {
            self.measured_fps =
                presented_frames.saturating_sub(self.fps_frames) as f64 / (now - self.fps_time);
            self.fps_time = now;
            self.fps_frames = presented_frames;
        }
        self.measured_fps
    }

    fn tick(&mut self) -> Result<(), String> {
        if !self.running {
            return Ok(());
        }
        let paused = self.control.paused();
        let status = match self.core.run_slice().map_err(|error| error.to_string()) {
            Ok(status) => status,
            Err(error) => {
                // Do not keep requesting browser repaints after a Ruby/native
                // failure. Re-entering the same failed VM turns one useful
                // diagnostic into a runaway loop and eventually a white page.
                self.running = false;
                self.control.stop();
                return Err(error);
            }
        };
        match status {
            RunSliceStatus::Continue => {}
            RunSliceStatus::Exited(_status) => self.running = false,
        }
        if self.remote {
            self.frames = self
                .frame
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .game_frames;
        } else if !paused {
            self.frames = self.frames.saturating_add(1);
        }
        self.process_events()?;
        Ok(())
    }

    fn process_events(&mut self) -> Result<(), String> {
        let events = std::mem::take(
            &mut *self
                .events
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        );
        for event in events {
            match event {
                CoreEvent::SaveRam { data } if !self.remote => self
                    .platform
                    .write_save(&self.save_name, &data)
                    .map_err(|error| error.to_string())?,
                // RGSS saves live in the Ruby WASI VFS. They are exported on
                // demand instead of being mirrored into browser battery RAM.
                CoreEvent::SaveRam { .. } => {}
                CoreEvent::SaveState { slot, data } => {
                    self.persist_state(&slot, &data)?;
                    self.state_slots.insert(slot, data);
                }
                CoreEvent::Error(message) => return Err(message),
                CoreEvent::Exited(ExitStatus { .. }) => self.running = false,
                CoreEvent::Status(status) => self.native_status = Some(status),
                CoreEvent::Loaded(_)
                | CoreEvent::Window(_)
                | CoreEvent::Log(_, _)
                | CoreEvent::Screenshot { .. } => {}
                CoreEvent::CommandResult {
                    response: Some(response),
                    ..
                } => {
                    if let Some(response) = decode_plugin_response(&response) {
                        self.plugin_responses.push(response);
                    }
                }
                CoreEvent::CommandResult { response: None, .. } => {}
            }
        }
        Ok(())
    }

    fn upload_frame(&mut self, shared: &mut SharedApp, context: &Context, direct_gl: bool) {
        let frame = self.frame.lock().unwrap_or_else(|error| error.into_inner());
        if frame.revision == 0 || frame.revision == self.uploaded_revision {
            return;
        }
        self.presented_frames = self.presented_frames.saturating_add(1);
        if direct_gl {
            // The app schedules the next repaint at the core's effective rate
            // after drawing. Avoid requesting a second immediate repaint for
            // the same frame; that made the dashboard run at display refresh
            // rate even when RGSS1 was producing 40 Hz frames.
        } else {
            match frame.format {
                PixelFormat::Bgra8 => {
                    shared.set_bgra_frame(context, frame.width, frame.height, &frame.pixels)
                }
                _ => shared.set_rgba_frame(context, frame.width, frame.height, &frame.pixels),
            }
        }
        self.uploaded_revision = frame.revision;
    }

    fn request_save_export(&self) -> bool {
        let Some(handle) = &self.remote_handle else {
            return false;
        };
        handle.request_save_export();
        true
    }

    fn request_plugin(&self, request: OperationRequest) -> bool {
        let Some(handle) = &self.remote_handle else {
            return false;
        };
        handle.request_plugin(request);
        true
    }

    fn take_exported_save(&self) -> Option<Result<Vec<u8>, String>> {
        self.remote_handle
            .as_ref()
            .and_then(worker::RemoteHandle::take_exported_save)
    }

    fn stop(&self) {
        self.control.stop();
    }

    fn load_state_slots(content_id: &str) -> BTreeMap<String, Vec<u8>> {
        let mut slots = BTreeMap::new();
        let Some(storage) = BrowserPlatform::storage() else {
            return slots;
        };
        let prefix = format!("fanplay.state.{content_id}.");
        let length = storage.length().unwrap_or(0);
        for index in 0..length {
            let Some(key) = storage.key(index).ok().flatten() else {
                continue;
            };
            let Some(slot) = key.strip_prefix(&prefix) else {
                continue;
            };
            let Ok(Some(encoded)) = storage.get_item(&key) else {
                continue;
            };
            if let Ok(data) = BASE64.decode(encoded) {
                slots.insert(slot.to_owned(), data);
            }
        }
        slots
    }

    fn persist_state(&self, slot: &str, data: &[u8]) -> Result<(), String> {
        let Some(storage) = BrowserPlatform::storage() else {
            return Err("browser storage is unavailable".into());
        };
        storage
            .set_item(
                &format!("fanplay.state.{}.{}", self.platform.content_id, slot),
                &BASE64.encode(data),
            )
            .map_err(|error| format!("could not persist save state: {error:?}"))
    }

    fn state_slots(&self) -> Vec<StateSlot> {
        (1..=4)
            .map(|slot| {
                let slot = slot.to_string();
                let data = self.state_slots.get(&slot);
                StateSlot {
                    slot,
                    saved_at: data.map(|_| "saved this session".into()),
                    bytes: data.map_or(0, Vec::len),
                }
            })
            .collect()
    }

    fn save_state(&self, slot: String) {
        let _ = self.control.send(CoreCommand::SaveState { slot });
    }

    fn load_state(&self, slot: String) {
        let Some(data) = self.state_slots.get(&slot).cloned() else {
            return;
        };
        let _ = self.control.send(CoreCommand::LoadState { slot, data });
    }

    fn take_plugin_responses(&mut self) -> Vec<OperationResponse> {
        std::mem::take(&mut self.plugin_responses)
    }
}

enum PendingFile {
    Game(Result<BrowserGame, String>),
    Save(String, Result<Vec<u8>, String>),
    Plugin(String, Result<Vec<u8>, String>),
    System(String, Result<Vec<u8>, String>),
}

#[derive(Clone, Copy)]
enum FileKind {
    Game,
    Save,
    Plugin,
    System,
}

type PendingFiles = Arc<Mutex<Vec<PendingFile>>>;
type UploadStatus = Arc<Mutex<Option<String>>>;
type FileChange = Closure<dyn FnMut(web_sys::Event)>;
type KeyboardGuard = Closure<dyn FnMut(web_sys::KeyboardEvent)>;

struct WebApp {
    shared: SharedApp,
    backend: BrowserBackend,
    session: Option<WebSession>,
    audio: BrowserAudio,
    library: BTreeMap<String, BrowserGame>,
    pending_files: PendingFiles,
    upload_status: UploadStatus,
    file_input: Option<(web_sys::HtmlInputElement, FileChange)>,
    held_buttons: ButtonSet,
    game_keyboard: game_input::GameKeyboard,
    game_ime: bool,
    game_frame: Option<Arc<Mutex<PresentedFrame>>>,
    webview_api_bridge: HashMap<u64, u64>,
    next_webview_request: u64,
    _keyboard_guard: KeyboardGuard,
}

// Webview IDs cross the window/worker boundary through JSON and must remain
// exactly representable by JavaScript's Number type. The plugin host uses
// ordinary small request IDs, so this separate range is collision-resistant
// without exceeding Number.MAX_SAFE_INTEGER (2^53 - 1).
const WEBVIEW_REQUEST_START: u64 = 1_u64 << 40;

impl WebApp {
    fn new(
        keyboard_guard: KeyboardGuard,
        wgpu_render_state: Option<eframe::egui_wgpu::RenderState>,
    ) -> Self {
        fanplay_webview_install();
        webview_trace("build webview-bridge-v4 ruby-request-base=1099511627776");
        let backend = BrowserBackend::default();
        let mut shared = SharedApp::default();
        let mut game_frame = None;
        if let Some(render_state) = wgpu_render_state {
            let adapter_info = render_state.adapter.get_info();
            let frame = Arc::new(Mutex::new(PresentedFrame::default()));
            shared.set_game_paint_callback(Some(wgpu_frame::callback(
                Arc::clone(&frame),
                render_state.target_format,
            )));
            game_frame = Some(frame);
            web_sys::console::log_1(&JsValue::from_str(&format!(
                "FANPLAY WGPU renderer enabled (backend={:?}, adapter={}; dashboard + game frame)",
                adapter_info.backend, adapter_info.name
            )));
        }
        shared.set_backend(Box::new(backend.clone()));
        shared.set_info(WorkspaceInfo {
            core_id: "gbc",
            logical_size: (160, 144),
            fps: 0.0,
            base_fps: 60,
            frames: 0,
            supports_save_states: true,
            state_slots: Vec::<StateSlot>::new(),
            global_fullscreen: false,
            show_fullscreen: true,
            show_save_buttons: true,
        });
        let mut session = SessionPanel::default();
        session.capabilities = SessionCapabilities::WEB;
        session.status = "Drop a GB, GBC, GBA, NDS ROM, or RGSS archive to begin".into();
        shared.workspace.session = Some(session);
        Self {
            shared,
            backend,
            session: None,
            audio: BrowserAudio::new(),
            library: BTreeMap::new(),
            pending_files: Arc::new(Mutex::new(Vec::new())),
            upload_status: Arc::new(Mutex::new(None)),
            file_input: None,
            held_buttons: ButtonSet::empty(),
            game_keyboard: game_input::GameKeyboard::default(),
            game_ime: false,
            game_frame,
            webview_api_bridge: HashMap::new(),
            // Keep webview request IDs above the plugin host's small counter,
            // but below JavaScript's exact-integer limit. These IDs cross the
            // window/worker boundary as JSON numbers; values at 1 << 63 are
            // rounded by JS and the returned response no longer matches the
            // `webview_api_bridge` key.
            next_webview_request: WEBVIEW_REQUEST_START,
            _keyboard_guard: keyboard_guard,
        }
    }

    fn collect_input(&mut self, context: &Context) {
        let Some(session) = &self.session else {
            return;
        };
        let control = Arc::clone(&session.control);
        let events = context.input(|input| input.events.clone());
        let rgss = session.info.id == "rgss";
        if rgss {
            let clicked_game = !egui::Popup::is_any_open(context)
                && events.iter().any(|event| {
                    matches!(event, egui::Event::PointerButton {pos, pressed: true, ..}
                    if self.shared.game_rect().contains(*pos))
                });
            if clicked_game {
                context.memory_mut(|memory| {
                    if let Some(id) = memory.focused() {
                        memory.surrender_focus(id);
                    }
                });
            }
            let allowed = !context.egui_wants_keyboard_input()
                && !egui::Popup::is_any_open(context)
                && context.input(|input| input.focused);
            let text_input = session
                .frame
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .text_input;
            self.game_keyboard.route(
                &control,
                &events,
                context.input(|input| input.modifiers),
                allowed,
                text_input,
            );
            self.game_ime = allowed && text_input;
            if allowed {
                // Game arrows/Enter must not also navigate or activate egui
                // controls. Workspace function-key shortcuts remain available.
                context
                    .input_mut(|input| input.events.retain(|event| !game_input::game_event(event)));
            }
        } else if context.egui_wants_keyboard_input() {
            if self.held_buttons.any(u32::MAX) {
                control
                    .input()
                    .set_button_bits(self.held_buttons.bits(), false);
                self.held_buttons = ButtonSet::empty();
            }
            return;
        }
        let touch_rect = self.shared.touch_rect();
        for event in events {
            match event {
                egui::Event::Key {
                    key,
                    physical_key,
                    pressed,
                    repeat: false,
                    ..
                } if !rgss => {
                    let Some((button, code)) = keyboard_button(physical_key.unwrap_or(key)) else {
                        continue;
                    };
                    control.input().set_button(button, pressed);
                    // RGSS consumes its keyboard edge events, while the
                    // generic cartridge cores consume the button snapshot.
                    // Publish both representations so browser keyboard input
                    // reaches either kind of core.
                    let _ = control.input().push_event(InputEvent::Key {
                        code,
                        down: pressed,
                        repeat: false,
                    });
                    self.held_buttons.set(button, pressed);
                }
                egui::Event::PointerMoved(position) => {
                    update_touch_input(&control, touch_rect, position, false);
                }
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    ..
                } => {
                    update_touch_input(&control, touch_rect, pos, pressed);
                }
                _ => {}
            }
        }
    }

    fn collect_dropped_files(&self, context: &Context) {
        let upload_status = Arc::clone(&self.upload_status);
        let files = context.input(|input| input.raw.dropped_files.clone());
        for file in files {
            let name = file.path().to_string_lossy().into_owned();
            let kind = if is_system_file_name(&name) {
                FileKind::System
            } else {
                FileKind::Game
            };
            let pending = Arc::clone(&self.pending_files);
            let upload_status = Arc::clone(&upload_status);
            let repaint = context.clone();
            wasm_bindgen_futures::spawn_local(async move {
                *upload_status
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) = Some(format!("Reading {name}…"));
                repaint.request_repaint();
                let pending_file = match kind {
                    FileKind::Game => {
                        let result = file
                            .bytes_async()
                            .await
                            .and_then(|bytes| BrowserGame::from_single(name.clone(), bytes));
                        repaint.request_repaint();
                        PendingFile::Game(result)
                    }
                    FileKind::System => PendingFile::System(name, file.bytes_async().await),
                    FileKind::Save | FileKind::Plugin => unreachable!(),
                };
                pending
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push(pending_file);
            });
        }
    }

    fn start_pending_files(&mut self) {
        if let Some(status) = self
            .upload_status
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            if let Some(panel) = self.shared.workspace.session.as_mut() {
                panel.status = status;
                panel.busy = true;
            }
        }
        let files = std::mem::take(
            &mut *self
                .pending_files
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        );
        if !files.is_empty() {
            if let Some(panel) = self.shared.workspace.session.as_mut() {
                panel.busy = false;
            }
        }
        for file in files {
            match file {
                PendingFile::Game(Ok(game)) => self.start_game(game),
                PendingFile::Game(Err(error)) => self.report_error(error),
                PendingFile::Save(_, Err(error))
                | PendingFile::Plugin(_, Err(error))
                | PendingFile::System(_, Err(error)) => self.report_session_error(error),
                PendingFile::Save(name, Ok(bytes)) => {
                    let rgss_game = self
                        .session
                        .as_ref()
                        .filter(|session| session.info.id == "rgss")
                        .and_then(|_| self.shared.workspace.session.as_ref())
                        .and_then(|panel| self.library.get(&panel.title))
                        .cloned();
                    if let Some(game) = rgss_game {
                        // RGSS owns its save file in Ruby's VFS. Pass the
                        // selected file only to the replacement worker; it is
                        // written there before the title scene starts.
                        self.start_game_with_save(game, Some(bytes));
                        if let Some(panel) = self.shared.workspace.session.as_mut() {
                            if panel.error.is_none() {
                                panel.status = format!("Imported {name}; game restarted");
                            }
                        }
                    } else if let Some(session) = &self.session {
                        match session.platform.write_save(&session.save_name, &bytes) {
                            Ok(()) => {
                                if let Some(panel) = self.shared.workspace.session.as_mut() {
                                    panel.status = format!("Imported {name}");
                                }
                            }
                            Err(error) => {
                                self.report_session_error(error.to_string());
                            }
                        }
                    }
                }
                PendingFile::Plugin(name, Ok(bytes)) => {
                    let id = Path::new(&name)
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .filter(|stem| !stem.is_empty())
                        .unwrap_or("browser-plugin")
                        .to_owned();
                    match String::from_utf8(bytes) {
                        Ok(source) => {
                            if let Err(error) = self.shared.paste_plugin(id, source) {
                                self.report_session_error(error);
                            } else if let Some(panel) = self.shared.workspace.session.as_mut() {
                                panel.status = format!("Loaded plugin {name}");
                            }
                        }
                        Err(error) => self
                            .report_session_error(format!("plugin {name} is not UTF-8: {error}")),
                    }
                }
                PendingFile::System(name, Ok(bytes)) => {
                    if let Err(error) = BrowserPlatform::write_system(&name, &bytes) {
                        self.report_session_error(error.to_string());
                    } else if let Some(panel) = self.shared.workspace.session.as_mut() {
                        panel.status = format!("Installed browser system file {name}");
                    }
                }
            }
        }
    }

    fn start_game(&mut self, game: BrowserGame) {
        self.start_game_with_save(game, None);
    }

    fn start_game_with_save(&mut self, game: BrowserGame, initial_save: Option<Vec<u8>>) {
        let name = game.label.clone();
        self.webview_api_bridge.clear();
        fanplay_webview_close();
        if let Some(session) = &self.session {
            session.stop();
        }
        self.audio.resume();
        self.library.insert(name.clone(), game.clone());
        match WebSession::load(
            game,
            Box::new(self.audio.sink()),
            self.game_frame.as_ref().map(Arc::clone),
            initial_save,
        ) {
            Ok(session) => {
                let info = session.info;
                self.backend.attach(
                    Arc::clone(&session.control),
                    info.id,
                    Arc::clone(&session.platform.files),
                );
                let logical_size = if info.id == "nds" {
                    (256, 384)
                } else if info.id == "rgss" {
                    (544, 416)
                } else {
                    info.screens
                        .first()
                        .map_or((160, 144), |screen| (screen.width, screen.height))
                };
                let state_slots = session.state_slots();
                self.shared.set_info(WorkspaceInfo {
                    core_id: info.id,
                    logical_size,
                    fps: 0.0,
                    base_fps: 60,
                    frames: 0,
                    supports_save_states: info.capabilities.contains(CoreCapabilities::SAVE_STATES),
                    state_slots,
                    global_fullscreen: false,
                    show_fullscreen: true,
                    show_save_buttons: true,
                });
                if let Some(panel) = self.shared.workspace.session.as_mut() {
                    panel.active = true;
                    panel.busy = false;
                    panel.error = None;
                    panel.title = name.clone();
                    panel.source = info.id.into();
                    panel.status = "Running".into();
                    panel.recent.retain(|recent| recent != &name);
                    panel.recent.insert(0, name);
                    panel.recent.truncate(24);
                }
                // Keep the browser dashboard visible when a game starts. F1
                // still toggles the clean game-only view for play sessions.
                self.shared.show_workspace_controls();
                self.shared.reset_plugins();
                if let Err(error) = self.shared.activate_plugins_for_core(info.id) {
                    self.report_error(error);
                }
                self.session = Some(session);
            }
            Err(error) => self.report_error(error),
        }
    }

    fn report_error(&mut self, error: String) {
        if let Some(panel) = self.shared.workspace.session.as_mut() {
            panel.active = false;
            panel.busy = false;
            panel.error = Some(error);
            panel.status = "Could not load game".into();
        }
    }

    /// Reports a non-fatal browser operation failure without hiding a running
    /// game. Save export/import and plugin uploads are toolbar actions; their
    /// errors must stay in the dashboard instead of switching back to the
    /// library while the RGSS worker continues in the background.
    fn report_session_error(&mut self, error: String) {
        if self.session.as_ref().is_some_and(|session| session.running) {
            if let Some(panel) = self.shared.workspace.session.as_mut() {
                panel.error = None;
                panel.status = error;
            }
        } else {
            self.report_error(error);
        }
    }

    fn open_file_picker(&mut self, context: &Context) -> Result<(), String> {
        self.open_picker(
            ".gb,.gbc,.gba,.nds,.bin,.zip,.rgssad,.rgss2a,.rgss3a,.rxdata,.rvdata,.rvdata2",
            FileKind::Game,
            false,
            context,
        )
    }

    fn open_folder_picker(&mut self, context: &Context) -> Result<(), String> {
        self.open_picker("", FileKind::Game, true, context)
    }

    fn open_save_picker(&mut self, context: &Context) -> Result<(), String> {
        self.open_picker(
            ".sav,.srm,.dat,.rxdata,.rvdata,.rvdata2",
            FileKind::Save,
            false,
            context,
        )
    }

    fn open_plugin_picker(&mut self, context: &Context) -> Result<(), String> {
        self.open_picker(".rhai", FileKind::Plugin, false, context)
    }

    fn open_picker(
        &mut self,
        accept: &str,
        kind: FileKind,
        directory: bool,
        context: &Context,
    ) -> Result<(), String> {
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| "browser document is unavailable".to_owned())?;
        let input = document
            .create_element("input")
            .map_err(|error| format!("could not create file input: {error:?}"))?
            .dyn_into::<web_sys::HtmlInputElement>()
            .map_err(|_| "created file input has the wrong type".to_owned())?;
        input.set_type("file");
        if !accept.is_empty() {
            input.set_accept(accept);
        }
        if directory {
            input.set_multiple(true);
            input
                .set_attribute("webkitdirectory", "")
                .map_err(|error| format!("could not enable folder selection: {error:?}"))?;
        }
        input
            .style()
            .set_property("display", "none")
            .map_err(|error| format!("could not hide file input: {error:?}"))?;
        let input_for_callback = input.clone();
        let pending = Arc::clone(&self.pending_files);
        let upload_status = Arc::clone(&self.upload_status);
        let repaint = context.clone();
        *upload_status
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(if directory {
            "Preparing folder upload…".to_owned()
        } else {
            "Preparing file upload…".to_owned()
        });
        context.request_repaint();
        let callback = Closure::wrap(Box::new(move |_event: web_sys::Event| {
            let Some(file_list) = input_for_callback.files() else {
                return;
            };
            let pending = Arc::clone(&pending);
            let upload_status = Arc::clone(&upload_status);
            let repaint = repaint.clone();
            let selected = (0..file_list.length())
                .filter_map(|index| file_list.get(index))
                .collect::<Vec<_>>();
            wasm_bindgen_futures::spawn_local(async move {
                let pending_file = if directory {
                    let total = selected.len();
                    let label = selected
                        .first()
                        .map(relative_browser_file_path)
                        .and_then(|path| path.split('/').next().map(str::to_owned))
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| "game".to_owned())
                        .to_owned();
                    let mut entries = Vec::with_capacity(selected.len());
                    let mut error = None;
                    let mut offset = 0;
                    while offset < selected.len() {
                        let end = (offset + 16).min(selected.len());
                        let promises = js_sys::Array::new();
                        for file in &selected[offset..end] {
                            promises.push(&file.array_buffer());
                        }
                        let buffers = match JsFuture::from(js_sys::Promise::all(&promises)).await {
                            Ok(buffers) => buffers,
                            Err(value) => {
                                error = Some(format!("could not read folder entry: {value:?}"));
                                break;
                            }
                        };
                        let buffers = match buffers.dyn_into::<js_sys::Array>() {
                            Ok(buffers) => buffers,
                            Err(value) => {
                                error = Some(format!("invalid folder read result: {value:?}"));
                                break;
                            }
                        };
                        for (batch_index, buffer) in buffers.iter().enumerate() {
                            let path = relative_browser_file_path(&selected[offset + batch_index]);
                            entries.push((path, js_sys::Uint8Array::new(&buffer).to_vec()));
                            let index = offset + batch_index;
                            let percent = if total == 0 {
                                100
                            } else {
                                ((index + 1) * 100) / total
                            };
                            *upload_status
                                .lock()
                                .unwrap_or_else(|error| error.into_inner()) = Some(format!(
                                "Reading RGSS folder: {}/{} files ({}%)",
                                index + 1,
                                total,
                                percent
                            ));
                            repaint.request_repaint();
                        }
                        if error.is_some() {
                            break;
                        }
                        offset = end;
                    }
                    *upload_status
                        .lock()
                        .unwrap_or_else(|error| error.into_inner()) =
                        Some("Indexing RGSS folder…".to_owned());
                    repaint.request_repaint();
                    PendingFile::Game(
                        error.map_or_else(|| BrowserGame::from_dropped_files(label, entries), Err),
                    )
                } else {
                    let Some(file) = selected.into_iter().next() else {
                        return;
                    };
                    let name = file.name();
                    let size = file.size() as u64;
                    *upload_status
                        .lock()
                        .unwrap_or_else(|error| error.into_inner()) =
                        Some(format!("Reading {name} ({})…", format_bytes(size)));
                    repaint.request_repaint();
                    let result = file
                        .array_buffer()
                        .await
                        .map(|buffer| js_sys::Uint8Array::new(&buffer).to_vec())
                        .map_err(|error| format!("could not read {name}: {error:?}"));
                    let kind = if matches!(kind, FileKind::Game) && is_system_file_name(&name) {
                        FileKind::System
                    } else {
                        kind
                    };
                    match kind {
                        FileKind::Game => PendingFile::Game(result.and_then(|bytes| {
                            if name.to_ascii_lowercase().ends_with(".zip") {
                                *upload_status
                                    .lock()
                                    .unwrap_or_else(|error| error.into_inner()) =
                                    Some(format!("Extracting {name}…"));
                                repaint.request_repaint();
                            }
                            BrowserGame::from_single(name, bytes)
                        })),
                        FileKind::Save => PendingFile::Save(name, result),
                        FileKind::Plugin => PendingFile::Plugin(name, result),
                        FileKind::System => PendingFile::System(name, result),
                    }
                };
                pending
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push(pending_file);
            });
        }) as Box<dyn FnMut(web_sys::Event)>);
        input.set_onchange(Some(callback.as_ref().unchecked_ref()));
        document
            .body()
            .ok_or_else(|| "browser document has no body".to_owned())?
            .append_child(&input)
            .map_err(|error| format!("could not attach file input: {error:?}"))?;
        input.click();
        self.file_input = Some((input, callback));
        Ok(())
    }

    fn handle_session_actions(&mut self, context: &Context) {
        let actions = std::mem::take(&mut self.shared.workspace.session_actions);
        if self.shared.take_plugin_import_request() {
            if let Err(error) = self.open_plugin_picker(context) {
                self.report_error(error);
            }
        }
        if let Some(name) = actions.open {
            if let Some(game) = self.library.get(&name).cloned() {
                self.start_game(game);
            } else {
                self.report_error(format!("game {name:?} is no longer available in this tab"));
            }
        }
        if actions.browse {
            if let Err(error) = self.open_folder_picker(context) {
                self.report_error(error);
            }
        }
        if actions.browse_game_file {
            if let Err(error) = self.open_file_picker(context) {
                self.report_error(error);
            }
        }
        if actions.stop {
            if let Some(session) = &self.session {
                session.stop();
            }
        }
        if actions.restart {
            if let Some(session) = &self.session {
                session.control.set_paused(false);
                let _ = session.control.send(CoreCommand::Reset);
                if let Some(panel) = self.shared.workspace.session.as_mut() {
                    panel.active = true;
                    panel.paused = false;
                    panel.status = "Running".into();
                }
            }
        }
        let (output_export, output_import, output_backup, output_restore) =
            self.shared.take_save_actions();
        if actions.export_saves || output_export {
            if self
                .session
                .as_ref()
                .is_some_and(|session| session.info.id == "rgss")
            {
                if self
                    .session
                    .as_ref()
                    .is_some_and(WebSession::request_save_export)
                {
                    if let Some(panel) = self.shared.workspace.session.as_mut() {
                        panel.status = "Reading RGSS save from the game VFS…".into();
                    }
                }
            } else if let Some(session) = &self.session {
                match session.platform.read_save(&session.save_name) {
                    Ok(Some(bytes)) => {
                        if let Err(error) = download_bytes(&session.save_name, &bytes) {
                            self.report_session_error(error);
                        }
                    }
                    Ok(None) => self.report_session_error(
                        "there is no battery save to export; save in-game first".into(),
                    ),
                    Err(error) => self.report_session_error(error.to_string()),
                }
            }
        }
        if actions.import_saves || output_import {
            if let Err(error) = self.open_save_picker(context) {
                self.report_session_error(error);
            }
        }
        if actions.create_backup || output_backup {
            if self
                .session
                .as_ref()
                .is_some_and(|session| session.info.id == "rgss")
            {
                if let Some(panel) = self.shared.workspace.session.as_mut() {
                    panel.status =
                        "RGSS saves are files; use Export save instead of a browser backup".into();
                }
            } else if let Some(session) = &self.session {
                match session.platform.create_backup(&session.save_name) {
                    Ok(()) => {
                        if let Some(panel) = self.shared.workspace.session.as_mut() {
                            panel.status = "Browser save backup created".into();
                        }
                    }
                    Err(error) => self.report_session_error(error.to_string()),
                }
            }
        }
        if actions.restore_latest || output_restore {
            if self
                .session
                .as_ref()
                .is_some_and(|session| session.info.id == "rgss")
            {
                if let Some(panel) = self.shared.workspace.session.as_mut() {
                    panel.status =
                        "RGSS saves are files; use Import save instead of a browser backup".into();
                }
            } else if let Some(session) = &self.session {
                if session.running {
                    self.report_session_error(
                        "stop the game before restoring a browser save backup".into(),
                    );
                    return;
                }
                match session.platform.restore_backup(&session.save_name) {
                    Ok(true) => {
                        if let Some(panel) = self.shared.workspace.session.as_mut() {
                            panel.status = "Browser save backup restored".into();
                        }
                    }
                    Ok(false) => self.report_session_error("no browser save backup exists".into()),
                    Err(error) => self.report_session_error(error.to_string()),
                }
            }
        }
        let (save_slot, load_slot) = self.shared.take_state_actions();
        if let Some(session) = &self.session {
            if let Some(slot) = save_slot {
                session.save_state(slot);
            }
            if let Some(slot) = load_slot {
                session.load_state(slot);
            }
        }
    }

    fn poll_plugin_webview_messages(&mut self) {
        let messages = js_sys::Array::from(&fanplay_webview_take());
        for index in 0..messages.length() {
            let message = messages.get(index);
            let get_string = |name: &str| {
                js_sys::Reflect::get(&message, &JsValue::from_str(name))
                    .ok()
                    .and_then(|value| value.as_string())
                    .unwrap_or_default()
            };
            let id = js_sys::Reflect::get(&message, &JsValue::from_str("id"))
                .ok()
                .and_then(|value| value.as_f64())
                .map(|value| value as u64)
                .unwrap_or_default();
            let kind = get_string("kind");
            if id == 0 {
                webview_trace_error("discarding webview message with id=0");
                continue;
            }
            webview_trace(format!("request id={id} kind={kind}"));
            if kind == "asset" {
                let category = get_string("category");
                let name = get_string("name");
                // Decode only on demand, then return the same one-shot PNG data
                // URL used by the desktop webview bridge. The bytes stay in the
                // browser VFS; no HTTP server or worker round-trip is needed.
                let src = self.shared.plugin_asset_data_url(&category, &name);
                webview_trace(format!(
                    "asset id={id} category={category:?} name={name:?} found={}",
                    src.is_some()
                ));
                fanplay_webview_respond(id, 200, &serde_json::json!({"src": src}).to_string());
                continue;
            }
            if kind != "api" {
                webview_trace_error(format!("unsupported webview message kind={kind:?} id={id}"));
                fanplay_webview_respond(
                    id,
                    400,
                    &serde_json::json!({"error": format!("unsupported webview message kind: {kind}")}).to_string(),
                );
                continue;
            }
            let method = get_string("method").to_ascii_uppercase();
            let target = get_string("target");
            let body = get_string("body");
            webview_trace(format!(
                "api id={id} method={method} target={target:?} body_bytes={}",
                body.len()
            ));
            if method == "OPTIONS" {
                webview_trace(format!("CORS preflight id={id} target={target:?}"));
                fanplay_webview_respond(id, 200, "{}");
                continue;
            }
            if method == "GET" && target == "/v1/extensions/routes" {
                webview_trace(format!("route discovery id={id}"));
                fanplay_webview_respond(
                    id,
                    200,
                    &fanplay_rgss::api_bridge::extension_routes_json(),
                );
                continue;
            }
            let Some(session) = self.session.as_ref() else {
                webview_trace_error(format!("no active game session for webview_id={id}"));
                fanplay_webview_respond(id, 503, r#"{"error":"no active game session"}"#);
                continue;
            };
            let (operation, args) = match fanplay_rgss::api_bridge::route_http_with_body(
                &method, &target, &body,
            ) {
                Ok(route) => route,
                Err((status, error)) => {
                    webview_trace_error(format!(
                            "route failed id={id} method={method} target={target:?} status={status}: {error}"
                        ));
                    fanplay_webview_respond(
                        id,
                        status,
                        &serde_json::json!({"error": error}).to_string(),
                    );
                    continue;
                }
            };
            let operation = fanplay_rgss::api_bridge::operation_for_rgss_worker(&operation);
            if !session.remote || session.info.id != "rgss" {
                webview_trace_error(format!(
                    "non-RGSS session for webview_id={id}: core={} remote={}",
                    session.info.id, session.remote
                ));
                fanplay_webview_respond(id, 503, r#"{"error":"RGSS worker is unavailable"}"#);
                continue;
            }
            let request_id = self.next_webview_request;
            self.next_webview_request = self.next_webview_request.wrapping_add(1);
            let operation_name = operation.clone();
            webview_trace(format!(
                "queue Ruby request webview_id={id} request_id={request_id} operation={operation} args={}",
                args.len()
            ));
            let request = OperationRequest {
                id: request_id,
                generation: 0,
                operation,
                arguments: Value::Array(args.into_iter().map(Value::String).collect()),
            };
            if session.request_plugin(request) {
                self.webview_api_bridge.insert(request_id, id);
            } else {
                webview_trace_error(format!(
                    "worker unavailable webview_id={id} request_id={request_id} operation={operation_name}"
                ));
                fanplay_webview_respond(id, 503, r#"{"error":"RGSS worker is unavailable"}"#);
            }
        }
    }

    fn poll_plugins(&mut self) {
        self.poll_plugin_webview_messages();
        for request in self.shared.take_plugin_requests() {
            if let Some(plugin_id) = request.operation.strip_suffix(".__webview.open") {
                let view_id = request
                    .arguments
                    .get("view_id")
                    .and_then(Value::as_str)
                    .unwrap_or(plugin_id);
                let title = request
                    .arguments
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("Plugin");
                let html = request
                    .arguments
                    .get("html")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                fanplay_webview_open(view_id, title, html);
                let _ = self.shared.dispatch_plugin_response(OperationResponse {
                    id: request.id,
                    generation: request.generation,
                    result: Ok(serde_json::json!({"opened": true})),
                });
                continue;
            }
            if request.operation.ends_with(".__webview.close") {
                fanplay_webview_close();
                let _ = self.shared.dispatch_plugin_response(OperationResponse {
                    id: request.id,
                    generation: request.generation,
                    result: Ok(serde_json::json!({"closed": true})),
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
            // RGSS runs in its own browser worker and owns the Ruby extension
            // bridge there. The window only forwards the generic operation
            // envelope; the worker adapts it to Ruby's positional protocol.
            if self
                .session
                .as_ref()
                .is_some_and(|session| session.info.id == "rgss" && session.remote)
            {
                if let Some(session) = &self.session {
                    session.request_plugin(request);
                }
                continue;
            }
            if let Some((core, operation)) =
                request.operation.split_once(".__").and_then(|(_, suffix)| {
                    let (core, operation) = suffix.split_once('.')?;
                    matches!(core, "gba" | "nds").then_some((core, operation))
                })
            {
                let Some(session) = &self.session else {
                    let _ = self.shared.dispatch_plugin_response(OperationResponse {
                        id: request.id,
                        generation: request.generation,
                        result: Err(OperationError::new(
                            "no_session",
                            "no active cartridge session",
                        )),
                    });
                    continue;
                };
                let payload = serde_json::json!({
                    "request_id": request.id,
                    "generation": request.generation,
                    "operation": operation,
                    "arguments": request.arguments,
                });
                let command = CoreCommand::Extension {
                    id: format!("{core}.rhai"),
                    payload: match serde_json::to_vec(&payload) {
                        Ok(payload) => payload,
                        Err(error) => {
                            let _ = self.shared.dispatch_plugin_response(OperationResponse {
                                id: request.id,
                                generation: request.generation,
                                result: Err(OperationError::new("encode", error.to_string())),
                            });
                            continue;
                        }
                    },
                };
                if let Err(error) = session.control.send_request(command) {
                    let _ = self.shared.dispatch_plugin_response(OperationResponse {
                        id: request.id,
                        generation: request.generation,
                        result: Err(OperationError::new("queue", error.to_string())),
                    });
                }
                continue;
            }

            let _ = self.shared.dispatch_plugin_response(OperationResponse {
                id: request.id,
                generation: request.generation,
                result: Err(OperationError::new(
                    "unsupported_operation",
                    request.operation,
                )),
            });
        }

        if let Some(session) = &mut self.session {
            for response in session.take_plugin_responses() {
                if let Some(webview_id) = self.webview_api_bridge.remove(&response.id) {
                    match response.result {
                        Ok(value) => {
                            webview_trace(format!(
                                "Ruby response request_id={} webview_id={} status=200 body_bytes={}",
                                response.id,
                                webview_id,
                                value.to_string().len()
                            ));
                            fanplay_webview_respond(webview_id, 200, &value.to_string());
                        }
                        Err(error) => {
                            webview_trace_error(format!(
                                "Ruby response request_id={} webview_id={} status=500 code={} message={}",
                                response.id, webview_id, error.code, error.message
                            ));
                            fanplay_webview_respond(
                                webview_id,
                                500,
                                &serde_json::json!({"error": error.message}).to_string(),
                            );
                        }
                    }
                } else {
                    // Most Ruby responses belong to the dashboard/plugin host,
                    // not to an iframe webview. They intentionally have the
                    // host's small request IDs (for example 1_000_007), so
                    // absence from `webview_api_bridge` is normal. Only report
                    // a dispatch failure as an error.
                    let response_id = response.id;
                    let generation = response.generation;
                    match self.shared.dispatch_plugin_response(response) {
                        Ok(()) => webview_trace(format!(
                            "forwarded Ruby response request_id={response_id} generation={generation} to plugin host"
                        )),
                        Err(error) => webview_trace_error(format!(
                            "plugin host rejected Ruby response request_id={response_id} generation={generation}: {error}"
                        )),
                    }
                }
            }
        }
    }
}

impl App for WebApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut Frame) {
        let context = ui.ctx().clone();
        self.collect_dropped_files(&context);
        self.start_pending_files();
        self.collect_input(&context);

        let paused = self
            .shared
            .workspace
            .session
            .as_ref()
            .is_some_and(|panel| panel.paused);
        let mut session_error = None;
        let mut session_status = None;
        let mut save_export = None;
        if let Some(session) = &mut self.session {
            session.control.set_paused(paused);
            if session.running {
                let budget = session.tick_budget();
                let slice_start = session.platform.now();
                for _ in 0..budget {
                    if let Err(error) = session.tick() {
                        session_error = Some(error);
                        break;
                    }
                    if !session.running {
                        break;
                    }
                    // A Ruby frame is cooperative and cannot be interrupted;
                    // don't start another once this repaint's time is spent.
                    if session.platform.now() - slice_start >= 0.008 {
                        break;
                    }
                }
                session.upload_frame(&mut self.shared, &context, self.game_frame.is_some());
                self.shared
                    .set_performance(session.measured_rate(), session.frames);
                self.shared.set_state_slots(session.state_slots());
            } else if let Some(panel) = self.shared.workspace.session.as_mut() {
                panel.active = false;
            }
            session_status = session.native_status.take();
            save_export = session
                .take_exported_save()
                .map(|result| result.map(|bytes| (session.save_name.clone(), bytes)));
        }
        if let Some(status) = session_status
            && let Some(panel) = self.shared.workspace.session.as_mut()
        {
            panel.status = status;
        }
        if let Some(error) = session_error {
            self.report_error(error);
        }
        if let Some(result) = save_export {
            match result {
                Ok((name, bytes)) => {
                    if let Err(error) = download_bytes(&name, &bytes) {
                        self.report_session_error(error);
                    } else if let Some(panel) = self.shared.workspace.session.as_mut() {
                        panel.status = format!("Exported {name}");
                    }
                }
                Err(error) => self.report_session_error(error),
            }
        }

        self.poll_plugins();
        self.shared.draw(ui);
        if self.game_ime && !context.egui_wants_keyboard_input() {
            let rect = self.shared.game_rect();
            context.output_mut(|output| {
                output.ime = Some(egui::output::IMEOutput {
                    purpose: egui::IMEPurpose::Normal,
                    rect,
                    cursor_rect: rect,
                    should_interrupt_composition: false,
                })
            });
        }
        self.handle_session_actions(&context);
        if let Some(session) = self.session.as_ref().filter(|session| session.running) {
            let rate = f64::from(session.effective_rate().max(1));
            context.request_repaint_after(Duration::from_secs_f64(1.0 / rate));
        }
    }
}

fn keyboard_button(key: egui::Key) -> Option<(Button, u32)> {
    use egui::Key;
    Some(match key {
        Key::ArrowUp => (Button::Up, 82),
        Key::ArrowDown => (Button::Down, 81),
        Key::ArrowLeft => (Button::Left, 80),
        Key::ArrowRight => (Button::Right, 79),
        Key::Z => (Button::South, 29),
        Key::X => (Button::East, 27),
        Key::Q => (Button::LeftShoulder, 20),
        Key::W => (Button::RightShoulder, 26),
        Key::Enter => (Button::Start, 40),
        Key::Backspace => (Button::Select, 42),
        _ => return None,
    })
}

fn is_system_file_name(name: &str) -> bool {
    matches!(
        Path::new(name)
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.to_ascii_lowercase()),
        Some(name)
            if matches!(
                name.as_str(),
                "bios7.bin" | "bios9.bin" | "firmware.bin" | "gba_bios.bin" | "bios.bin"
            )
    )
}

fn relative_browser_file_path(file: &web_sys::File) -> String {
    let path = js_sys::Reflect::get(file.as_ref(), &JsValue::from_str("webkitRelativePath"))
        .ok()
        .and_then(|value| value.as_string())
        .unwrap_or_default();
    if path.is_empty() {
        file.name()
    } else {
        path
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

fn install_browser_key_guard() -> Result<KeyboardGuard, JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("browser window is missing"))?;
    let callback = Closure::wrap(Box::new(move |event: web_sys::KeyboardEvent| {
        let key = event.key();
        let function_key = key
            .strip_prefix('F')
            .and_then(|number| number.parse::<u8>().ok())
            .is_some_and(|number| (1..=12).contains(&number));
        let text_target = event
            .target()
            .and_then(|target| target.dyn_into::<web_sys::HtmlElement>().ok())
            .is_some_and(|element| matches!(element.tag_name().as_str(), "INPUT" | "TEXTAREA"));
        let game_key = matches!(
            key.as_str(),
            "ArrowUp"
                | "ArrowDown"
                | "ArrowLeft"
                | "ArrowRight"
                | "Enter"
                | "Backspace"
                | "z"
                | "x"
                | "q"
                | "w"
        );
        if function_key || (game_key && !text_target) {
            event.prevent_default();
        }
    }) as Box<dyn FnMut(web_sys::KeyboardEvent)>);
    window.add_event_listener_with_callback("keydown", callback.as_ref().unchecked_ref())?;
    Ok(callback)
}

fn update_touch_input(
    control: &CoreControl,
    rect: egui::Rect,
    position: egui::Pos2,
    pressed: bool,
) {
    if rect != egui::Rect::ZERO && rect.contains(position) {
        let x = ((position.x - rect.min.x) / rect.width() * 256.0)
            .floor()
            .clamp(0.0, 255.0) as i32;
        let y = 192
            + ((position.y - rect.min.y) / rect.height() * 192.0)
                .floor()
                .clamp(0.0, 191.0) as i32;
        control.input().set_pointer(x, y, true);
        control.input().set_button(Button::Extra0, pressed);
    } else if !pressed {
        control.input().set_pointer(0, 0, false);
        control.input().set_button(Button::Extra0, false);
    }
}

fn download_bytes(name: &str, bytes: &[u8]) -> Result<(), String> {
    let document = web_sys::window()
        .and_then(|window| window.document())
        .ok_or_else(|| "browser document is unavailable".to_owned())?;
    let parts = js_sys::Array::new();
    parts.push(&js_sys::Uint8Array::from(bytes).into());
    let blob = web_sys::Blob::new_with_u8_array_sequence(&parts.into())
        .map_err(|error| format!("could not create download: {error:?}"))?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)
        .map_err(|error| format!("could not create download URL: {error:?}"))?;
    let anchor = document
        .create_element("a")
        .map_err(|error| format!("could not create download link: {error:?}"))?
        .dyn_into::<web_sys::HtmlAnchorElement>()
        .map_err(|_| "download link has the wrong type".to_owned())?;
    anchor.set_href(&url);
    anchor.set_download(name);
    web_sys::HtmlElement::click(&anchor);
    let _ = web_sys::Url::revoke_object_url(&url);
    Ok(())
}

fn decode_plugin_response(bytes: &[u8]) -> Option<OperationResponse> {
    let value = serde_json::from_slice::<Value>(bytes).ok()?;
    let id = value.get("request_id")?.as_u64()?;
    let generation = value.get("generation")?.as_u64()?;
    let result = if let Some(result) = value.get("result") {
        Ok(result.clone())
    } else {
        let error = value.get("error")?;
        Err(OperationError::new(
            error.get("code").and_then(Value::as_str).unwrap_or("core"),
            error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("core operation failed"),
        ))
    };
    Some(OperationResponse {
        id,
        generation,
        result,
    })
}

#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    // The worker imports the same compiled module, but owns no DOM or egui UI.
    if web_sys::window().is_none() {
        return Ok(());
    }
    let document = web_sys::window()
        .and_then(|window| window.document())
        .ok_or_else(|| JsValue::from_str("browser document is unavailable"))?;
    let canvas = document
        .get_element_by_id("fanplay-canvas")
        .ok_or_else(|| JsValue::from_str("fanplay canvas is missing"))?
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .map_err(|_| JsValue::from_str("fanplay canvas has the wrong type"))?;
    let keyboard_guard = install_browser_key_guard()?;
    let runner = WebRunner::new();
    wasm_bindgen_futures::spawn_local(async move {
        let ready =
            js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("__fanplayRubyReady"));
        match ready {
            Ok(value) if value.is_instance_of::<js_sys::Promise>() => {
                if let Err(error) = JsFuture::from(js_sys::Promise::from(value)).await {
                    web_sys::console::error_1(&error);
                    return;
                }
            }
            Ok(_) => {}
            Err(error) => {
                web_sys::console::error_1(&error);
                return;
            }
        }
        let options = WebOptions {
            renderer: Renderer::Wgpu,
            max_fps: Some(60),
            ..Default::default()
        };
        if let Err(error) = runner
            .start(
                canvas,
                options,
                Box::new(move |creation_context| {
                    Ok(Box::new(WebApp::new(
                        keyboard_guard,
                        creation_context.wgpu_render_state.clone(),
                    )))
                }),
            )
            .await
        {
            web_sys::console::error_1(&error);
        }
    });
    Ok(())
}
