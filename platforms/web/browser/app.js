import { readStored } from './storage-compat.js';
// SPDX-License-Identifier: GPL-2.0-or-later
// Browser host for the fanplay engine.
//
// The Rust engine runs as WebAssembly and owns rendering, archives and input.
// The game's Ruby runs under ruby.wasm and talks to the engine through
// `globalThis.FANPLAY.native`, which carries base64-encoded native requests. Ruby
// executes inside a Fiber that yields on every Graphics.update, so the page can
// present frames and collect input between frames without a worker.
import init, * as engine from './pkg/fanplay_web.js';
import { createGamepadInput } from './gamepad.js';
import { PRELUDE } from './ruby-host.js';
import { createNativeDispatcher } from './native-bridge.js';
import { createGamePump, captureRubyOutput } from './game-pump.js';

const canvas = document.getElementById('screen');

// Plugin HTML views are DOM-owned overlays rather than a second rendering
// surface. This keeps them available in browsers where native webviews are
// unavailable, while the Rust engine still owns the game API queue.
const pluginWebviews = new Map();
const pluginHttpSources = new Map();
let nextPluginHttpRequest = 1;

function pluginViewKey(pluginId, viewId) {
  return `${pluginId}\u0000${viewId}`;
}

function pluginWebviewBootstrap() {
  return `<script>
(() => {
  const pending = new Map();
  let next = 1;
  const nativeFetch = window.fetch.bind(window);
  window.FANPLAY = window.FANPLAY || {};
  window.FANPLAY.api = (method, target, body) => new Promise((resolve, reject) => {
    const id = next++;
    pending.set(id, { resolve, reject });
    parent.postMessage({ type: 'fanplay-plugin-http', id, method, target, body: body || '' }, '*');
  });
  // Compatibility alias for plugins that used the original bridge name.
  window.FANPLAY.http = window.FANPLAY.api;
  window.FANPLAY.asset = (category, name) => new Promise((resolve, reject) => {
    const id = next++;
    pending.set(id, { kind: 'asset', resolve, reject });
    parent.postMessage({ type: 'fanplay-plugin-asset', id, category, name }, '*');
  });
  const requestTarget = input => {
    const raw = typeof input === 'string' ? input : input.url;
    if (raw.startsWith('/')) return raw;
    const base = location.protocol === 'about:' ? 'http://fanplay-plugin.invalid/' : location.href;
    const url = new URL(raw, base);
    return url.origin === 'http://fanplay-plugin.invalid' ? url.pathname + url.search : url.href;
  };
  window.addEventListener('message', event => {
    const message = event.data;
    if (!message || message.type !== 'fanplay-plugin-http-response') return;
    const request = pending.get(message.id);
    if (!request) return;
    pending.delete(message.id);
    const response = new Response(message.body || '', {
      status: message.status || 500,
      headers: { 'content-type': 'application/json' },
    });
    request.resolve(response);
  });
  window.addEventListener('message', event => {
    const message = event.data;
    if (!message || message.type !== 'fanplay-plugin-asset-response') return;
    const request = pending.get(message.id);
    if (!request || request.kind !== 'asset') return;
    pending.delete(message.id);
    request.resolve(message.src || null);
  });
  window.fetch = (input, init = {}) => {
    const target = requestTarget(input);
    if (!target.startsWith('/v1/')) return nativeFetch(input, init);
    const method = String(init.method || (typeof input === 'string' ? 'GET' : input.method) || 'GET').toUpperCase();
    return window.FANPLAY.api(method, target, init.body || '');
  };
})();
</script>`;
}

function closePluginWebview(pluginId, viewId) {
  const key = pluginViewKey(pluginId, viewId);
  const view = pluginWebviews.get(key);
  if (!view) return;
  view.root.remove();
  pluginWebviews.delete(key);
}

globalThis.__fanplay_openPluginWebview = (payload) => {
  const value = typeof payload === 'string' ? JSON.parse(payload) : payload;
  const key = pluginViewKey(value.plugin_id, value.view_id);
  closePluginWebview(value.plugin_id, value.view_id);

  const root = document.createElement('section');
  root.className = 'fanplay-plugin-webview';
  Object.assign(root.style, {
    position: 'fixed', left: 'min(320px, 5vw)', top: '5vh', width: 'min(720px, 90vw)',
    height: 'min(560px, 85vh)', zIndex: '10000', display: 'flex', flexDirection: 'column',
    background: '#1A1A2E', border: '1px solid rgba(255,0,110,.45)', borderRadius: '12px',
    boxShadow: '0 0 36px rgba(255,0,110,.22), 0 18px 48px rgba(0,0,0,.55)', overflow: 'hidden', resize: 'both',
    minWidth: '420px', minHeight: '240px', maxWidth: '95vw', maxHeight: '90vh',
  });
  const header = document.createElement('header');
  Object.assign(header.style, {
    display: 'flex', alignItems: 'center', justifyContent: 'space-between',
    minHeight: '38px', padding: '0 8px 0 14px', color: '#EEEEF0',
    background: '#12121F', borderBottom: '1px solid #2A2A3E',
    font: '12px/1.4 "Space Mono",ui-monospace,monospace', letterSpacing: '.04em',
    cursor: 'grab', userSelect: 'none', touchAction: 'none',
  });
  const label = document.createElement('span');
  label.textContent = String(value.title || value.view_id || 'Plugin');
  const close = document.createElement('button');
  close.type = 'button'; close.textContent = '×';
  Object.assign(close.style, { color: '#FF006E', background: 'transparent', border: '0', fontSize: '22px', cursor: 'pointer', lineHeight: '1' });
  close.addEventListener('click', () => closePluginWebview(value.plugin_id, value.view_id));
  header.append(label, close);

  // Browser plugin views are ordinary DOM overlays, so give them the same
  // basic window behavior as native plugin views. Keep the close button out
  // of the drag gesture and clamp the window inside the viewport.
  let dragging = false;
  let dragOffsetX = 0;
  let dragOffsetY = 0;
  const clamp = (number, minimum, maximum) => Math.min(Math.max(number, minimum), maximum);
  header.addEventListener('pointerdown', event => {
    if (event.button !== 0 || event.target.closest('button')) return;
    const bounds = root.getBoundingClientRect();
    dragging = true;
    dragOffsetX = event.clientX - bounds.left;
    dragOffsetY = event.clientY - bounds.top;
    header.style.cursor = 'grabbing';
    header.setPointerCapture?.(event.pointerId);
    event.preventDefault();
  });
  header.addEventListener('pointermove', event => {
    if (!dragging) return;
    const width = root.offsetWidth;
    const height = root.offsetHeight;
    root.style.left = `${clamp(event.clientX - dragOffsetX, 0, window.innerWidth - width)}px`;
    root.style.top = `${clamp(event.clientY - dragOffsetY, 0, window.innerHeight - height)}px`;
  });
  const stopDragging = event => {
    if (!dragging) return;
    dragging = false;
    header.style.cursor = 'grab';
    if (event?.pointerId !== undefined) header.releasePointerCapture?.(event.pointerId);
  };
  header.addEventListener('pointerup', stopDragging);
  header.addEventListener('pointercancel', stopDragging);

  const iframe = document.createElement('iframe');
  iframe.title = label.textContent;
  iframe.srcdoc = `<!doctype html><html><head><meta charset="utf-8"></head><body>${pluginWebviewBootstrap()}${value.html || ''}</body></html>`;
  Object.assign(iframe.style, { flex: '1', width: '100%', border: '0', background: '#fff' });
  root.append(header, iframe);
  document.body.appendChild(root);
  pluginWebviews.set(key, { root, iframe });
};

globalThis.__fanplay_closePluginWebview = (payload) => {
  const value = typeof payload === 'string' ? JSON.parse(payload) : payload;
  closePluginWebview(value.plugin_id, value.view_id);
};

globalThis.__fanplay_closeAllPluginWebviews = () => {
  for (const view of pluginWebviews.values()) view.root.remove();
  pluginWebviews.clear();
  pluginHttpSources.clear();
};

window.addEventListener('message', event => {
  const message = event.data;
  if (!message || message.type !== 'fanplay-plugin-http') return;
  const view = [...pluginWebviews.values()].find(candidate => candidate.iframe.contentWindow === event.source);
  if (!view) return;
  const id = nextPluginHttpRequest++;
  pluginHttpSources.set(id, { source: event.source, localId: Number(message.id) });
  try {
    engine.web_eframe_plugin_http(id, String(message.method || 'GET'), String(message.target || '/'), String(message.body || ''));
  } catch (error) {
    pluginHttpSources.delete(id);
    event.source.postMessage({ type: 'fanplay-plugin-http-response', id: Number(message.id), status: 500, body: JSON.stringify({ error: String(error) }) }, '*');
  }
});

window.addEventListener('message', event => {
  const message = event.data;
  if (!message || message.type !== 'fanplay-plugin-asset') return;
  const view = [...pluginWebviews.values()].find(candidate => candidate.iframe.contentWindow === event.source);
  if (!view) return;
  let src = '';
  try {
    src = engine.web_eframe_plugin_asset(String(message.category || ''), String(message.name || ''));
  } catch (_) { /* missing assets are represented by an empty source */ }
  event.source.postMessage({ type: 'fanplay-plugin-asset-response', id: Number(message.id), src }, '*');
});

function pumpPluginHttp() {
  if (engine.web_eframe_take_plugin_http_responses === undefined) return;
  let replies;
  try { replies = JSON.parse(engine.web_eframe_take_plugin_http_responses()); } catch { return; }
  for (const reply of replies) {
    const pending = pluginHttpSources.get(reply.id);
    pluginHttpSources.delete(reply.id);
    pending?.source.postMessage({
      type: 'fanplay-plugin-http-response', ...reply, id: pending.localId,
    }, '*');
  }
}

// The workspace UI (toolbar, panels, performance, runtime patches, save
// export/import) is drawn by egui inside the engine. The page only hosts the
// canvas and the Ruby VM, so there is no HTML chrome to keep in sync.
const status = (text) => { console.log(text); globalThis.FANPLAY_PLAYER?.status?.(text); };

function fallbackCopyText(text) {
  const field = document.createElement('textarea');
  field.value = text;
  field.setAttribute('readonly', '');
  Object.assign(field.style, { position: 'fixed', left: '-9999px', top: '0' });
  document.body.appendChild(field);
  field.select();
  let copied = false;
  try { copied = document.execCommand('copy'); } catch { /* clipboard denied */ }
  field.remove();
  return copied;
}

globalThis.__fanplay_copyText = (text) => {
  const value = String(text);
  if (navigator.clipboard?.writeText) {
    navigator.clipboard.writeText(value).then(
      () => status('Runtime output copied.'),
      () => status(fallbackCopyText(value) ? 'Runtime output copied.' : 'Could not access the clipboard.'),
    );
  } else {
    status(fallbackCopyText(value) ? 'Runtime output copied.' : 'Could not access the clipboard.');
  }
};
const setStatus = status;
const nextAnimationFrame = () => new Promise((resolve) => requestAnimationFrame(resolve));

const params = new URLSearchParams(location.search);
const player = globalThis.FANPLAY_PLAYER;
const localGame = player?.game;
const gameRoot = localGame?.storageId || params.get('game') || 'game/';
const assetList = params.get('assets') || 'assets.json';
// VM/machine stack for the Fiber the game runs in (and for threads). Long play
// sessions can nest deeply enough that ruby.wasm's small default stacks raise
// SystemStackError; a large VM stack is the important part. ruby.wasm traps if
// these exceed what its linear memory can reserve (128 MiB VM / 64 MiB machine
// fail), so keep headroom. Override with ?rubyStack=<bytes>.
const rubyVmStackBytes = Math.max(64 * 1024 * 1024, Number(params.get('rubyStack')) || 0);
const rubyMachineStackBytes = Math.max(16 * 1024 * 1024, Number(params.get('rubyMachineStack')) || 0);

// ---------------------------------------------------------------------------
// SDL scancode mapping (DOM KeyboardEvent.code -> SDL scancode).
// ---------------------------------------------------------------------------
function scancode(code) {
  if (/^Key[A-Z]$/.test(code)) return 4 + (code.charCodeAt(3) - 65);
  if (/^Digit[0-9]$/.test(code)) return 30 + (code.charCodeAt(5) - 48);
  if (/^F([1-9]|1[0-2])$/.test(code)) return 58 + (parseInt(code.slice(1), 10) - 1);
  return (
    {
      Enter: 40,
      Escape: 41,
      Backspace: 42,
      Tab: 43,
      Space: 44,
      Minus: 45,
      Equal: 46,
      BracketLeft: 47,
      BracketRight: 48,
      Backslash: 49,
      Semicolon: 51,
      Quote: 52,
      Backquote: 53,
      Comma: 54,
      Period: 55,
      Slash: 56,
      CapsLock: 57,
      ArrowRight: 79,
      ArrowLeft: 80,
      ArrowDown: 81,
      ArrowUp: 82,
      ShiftLeft: 225,
      ShiftRight: 229,
      ControlLeft: 224,
      ControlRight: 228,
      AltLeft: 226,
      AltRight: 230,
    }[code] ?? 0
  );
}

let textInput = false;
let engineReady = false;
let gameStarted = false;
// True once the WebGPU + egui workspace is active.
let gpuMode = false;
// GBA/DS sessions route keyboard, mouse and gamepad input to the shared core
// control plane instead of the RGSS runtime.
let cartridgeMode = false;

function sendKey(code, down) {
  if (cartridgeMode) engine.web_core_key(code, down);
  else engine.web_key(code, down);
}

function bytesToBase64(bytes) {
  let binary = '';
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode.apply(null, bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}

function encodeNativeResponse(response) {
  if (response === undefined) return '__FANPLAY_STOP__';
  if (response.length === 0) return '';
  try {
    return `U${new TextDecoder('utf-8', { fatal: true }).decode(response)}`;
  } catch {
    return `B${bytesToBase64(response)}`;
  }
}

// ---------------------------------------------------------------------------
// Save persistence. ruby.wasm's filesystem is in memory and is lost on reload,
// so saves are mirrored to localStorage. Export/Import move them between
// browsers as a small JSON bundle of the save files.
// ---------------------------------------------------------------------------
const saveStoreKey = `fanplay-saves:${gameRoot}`;
let saveStore = {};
try {
  saveStore = JSON.parse(readStored(localStorage, saveStoreKey) || '{}') || {};
} catch {
  saveStore = {};
}
const backupStoreKey = `fanplay-backups:${gameRoot}`;
let backupStore = {};
try {
  backupStore = JSON.parse(readStored(localStorage, backupStoreKey) || '{}') || {};
} catch {
  backupStore = {};
}
function writeBackupStore() {
  try {
    localStorage.setItem(backupStoreKey, JSON.stringify(backupStore));
  } catch (error) {
    status('Could not persist save backup: ' + error);
  }
}
function backupName() {
  return new Date().toISOString().replace(/[-:]/g, '').replace('T', '-').replace('Z', '');
}
function createSaveBackup(name = '') {
  if (!Object.keys(saveStore).length) {
    status('No save files exist yet. Save in the game first.');
    return null;
  }
  const chosen = String(name || '').trim() || backupName();
  backupStore[chosen] = { created: Date.now(), files: { ...saveStore } };
  writeBackupStore();
  status(`Save backup ${chosen} created.`);
  return chosen;
}
function latestBackupName() {
  return Object.keys(backupStore).sort().at(-1) || '';
}
function restoreLatestSaveBackup() {
  const name = latestBackupName();
  if (!name) {
    status('No save backups exist yet.');
    return;
  }
  const backup = backupStore[name];
  if (!backup?.files) {
    status(`Backup ${name} is invalid.`);
    return;
  }
  saveStore = { ...backup.files };
  writeSaveStore();
  status(`Restored save backup ${name}. Reloading…`);
  setTimeout(() => location.reload(), 300);
}
function writeSaveStore() {
  try {
    localStorage.setItem(saveStoreKey, JSON.stringify(saveStore));
  } catch (error) {
    status('Could not persist save: ' + error);
  }
}
function exportSaves() {
  const blob = new Blob([JSON.stringify(saveStore)], { type: 'application/json' });
  const link = document.createElement('a');
  link.href = URL.createObjectURL(blob);
  link.download = 'fanplay-saves.json';
  link.click();
  URL.revokeObjectURL(link.href);
}
function importSaves(file) {
  const reader = new FileReader();
  reader.onload = () => {
    try {
      const incoming = JSON.parse(String(reader.result));
      if (!incoming || typeof incoming !== 'object') throw new Error('not a save bundle');
      Object.assign(saveStore, incoming);
      writeSaveStore();
      status('Saves imported. Reloading to load them…');
      setTimeout(() => location.reload(), 300);
    } catch (error) {
      status('Import failed: ' + error);
    }
  };
  reader.readAsText(file);
}
// The egui workspace toolbar drives export/import through these globals; the
// file picker is created here so the page needs no HTML controls.
const importInput = document.createElement('input');
importInput.type = 'file';
importInput.accept = '.json,application/json';
importInput.hidden = true;
document.body.appendChild(importInput);
// Canvas key events cover ordinary controls, but browsers route IME, paste,
// and mobile text through editable elements. Keep a hidden textarea focused
// while the game explicitly requests text input.
const gameTextInput = document.createElement('textarea');
gameTextInput.setAttribute('aria-label', 'Game text input');
gameTextInput.setAttribute('autocomplete', 'off');
gameTextInput.setAttribute('autocapitalize', 'off');
gameTextInput.spellcheck = false;
gameTextInput.tabIndex = -1;
gameTextInput.inputMode = 'text';
Object.assign(gameTextInput.style, {
  position: 'fixed', left: '0', bottom: '0', width: '1px', height: '1px',
  padding: '0', border: '0', opacity: '0', overflow: 'hidden',
  pointerEvents: 'none', zIndex: '1',
});
document.body.appendChild(gameTextInput);
function setTextInputMode(enabled) {
  if (enabled) {
    gameTextInput.value = '';
    gameTextInput.focus({ preventScroll: true });
  } else if (document.activeElement === gameTextInput) {
    gameTextInput.blur();
    gameTextInput.value = '';
  }
}
gameTextInput.addEventListener('input', () => {
  if (!textInput || !gameTextInput.value) return;
  engine.web_text(gameTextInput.value);
  gameTextInput.value = '';
});
importInput.addEventListener('change', () => {
  if (importInput.files[0]) importSaves(importInput.files[0]);
  importInput.value = '';
});
globalThis.__fanplay_exportSaves = exportSaves;
globalThis.__fanplay_importSaves = () => importInput.click();
globalThis.__fanplay_createSaveBackup = createSaveBackup;
globalThis.__fanplay_restoreLatestSaveBackup = restoreLatestSaveBackup;

function pumpPlayerActions() {
  if (engine.web_eframe_player_actions === undefined) return;
  let actions;
  try { actions = JSON.parse(engine.web_eframe_player_actions() || '{}'); } catch { return; }
  if (actions.export_saves) exportSaves();
  if (actions.import_saves) importInput.click();
  if (actions.create_backup) createSaveBackup();
  if (actions.restore_latest) restoreLatestSaveBackup();
  if (actions.open_game_folder || actions.open_save_folder || actions.open_patches_folder) {
    status('Browsers cannot open native folders. Use Export saves or the desktop player for filesystem folders.');
  }
  if (actions.start_api || actions.stop_api) {
    status('The browser control API is always available as window.FANPLAY.control; it has no TCP listener.');
  }
}

const dispatchNative = createNativeDispatcher(engine, (enabled) => {
  textInput = enabled;
  setTextInputMode(enabled);
});

// The bridge Ruby calls. Returns base64, or null to signal a clean stop.
globalThis.FANPLAY = {
  source: '',
  manifestText: '',
  // Save files restored into ruby.wasm's filesystem at startup.
  savedSavesJson: JSON.stringify(saveStore),
  persistSave(path, base64) {
    saveStore[path] = base64;
    writeSaveStore();
  },
  startup_progress(steps) {
    setStatus(`Preparing game scripts… ${steps} startup steps`);
  },
  game_started() {
    if (!gameStarted) {
      gameStarted = true;
      setStatus('Running.');
    }
  },
  native(requestBase64) {
    const request = Uint8Array.from(atob(requestBase64), (c) => c.charCodeAt(0));
    const response = dispatchNative(request);
    if (response === undefined) return '__FANPLAY_STOP__';
    if (response.length === 0) return '';
    return bytesToBase64(response);
  },
  native_text(requestText) {
    const response = dispatchNative(new TextEncoder().encode(requestText));
    return encodeNativeResponse(response);
  },
};
globalThis.MKXP = globalThis.FANPLAY;

// ---------------------------------------------------------------------------
// Typed control bridge. A browser cannot bind a TCP listener, so the same
// operation catalog is exposed here. Discovery comes from the Rust catalog;
// browser-owned settings/saves use local storage, and game operations go
// through the shared Ruby bridge.
// ---------------------------------------------------------------------------
const browserSettingsKey = 'fanplay-settings';
function browserSettings() {
  try { return JSON.parse(readStored(localStorage, browserSettingsKey) || '{}') || {}; }
  catch { return {}; }
}
function saveBrowserSettings(settings) {
  localStorage.setItem(browserSettingsKey, JSON.stringify(settings));
}
function browserGameKey() {
  return localGame?.storageId || gameRoot;
}
function browserSettingsMerge(base = {}, overlay = {}) {
  return {
    ...base,
    ...overlay,
    patches: [...new Set([...(base.patches || []), ...(overlay.patches || [])])],
    patch_values: { ...(base.patch_values || {}), ...(overlay.patch_values || {}) },
    keyboard: { ...(base.keyboard || {}), ...(overlay.keyboard || {}) },
  };
}
function effectiveBrowserSettings() {
  const settings = browserSettings();
  const override = settings.games?.[browserGameKey()] || {};
  return browserSettingsMerge(settings.common || {}, override);
}
function controlError(status, message) {
  const error = new Error(String(message || 'control request failed'));
  error.status = status;
  throw error;
}
function browserBackupInfo(name) {
  const backup = backupStore[name];
  const files = backup?.files || {};
  const bytes = Object.values(files).reduce((total, value) => total + Math.ceil(String(value).length * 0.75), 0);
  return {
    name,
    created_at: Math.floor((backup?.created || 0) / 1000),
    created_at_utc: new Date(backup?.created || 0).toISOString(),
    files: Object.keys(files).length,
    bytes,
    path: `browser-storage:${name}`,
  };
}
async function controlRequest(method, target, body) {
  const upper = String(method || 'GET').toUpperCase();
  const url = new URL(String(target || '/'), 'http://fanplay.invalid');
  const path = url.pathname.replace(/\/+$/, '') || '/';
  const payload = body === undefined || body === null || body === ''
    ? {}
    : (typeof body === 'string' ? JSON.parse(body) : body);
  const settings = browserSettings();

  if (upper === 'GET' && (path === '/' || path === '/v1')) {
    const discovery = JSON.parse(engine.web_control_discovery());
    return { server: 'fanplay-control', api_version: discovery.api_version, discovery: 'browser://fanplay/v1/discovery', openapi: 'browser://fanplay/v1/openapi.json', status: 'browser://fanplay/v1/status' };
  }
  if (upper === 'GET' && path === '/v1/discovery') return JSON.parse(engine.web_control_discovery());
  if (upper === 'GET' && path === '/v1/openapi.json') return JSON.parse(engine.web_control_openapi());
  if (upper === 'GET' && path === '/v1/status') return JSON.parse(engine.web_control_discovery()).status;
  if (upper === 'GET' && path === '/v1/extensions/routes') return JSON.parse(engine.web_control_routes());

  if (path === '/v1/settings/common') {
    if (upper === 'GET') return settings.common || {};
    if (upper === 'PUT' || upper === 'PATCH') {
      settings.common = payload;
      saveBrowserSettings(settings);
      return settings.common;
    }
  }
  if (path.startsWith('/v1/settings/games/')) {
    const key = decodeURIComponent(path.slice('/v1/settings/games/'.length));
    if (upper === 'GET') {
      const override = settings.games?.[key] ?? null;
      return { key, override, settings: browserSettingsMerge(settings.common || {}, override || {}) };
    }
    if (upper === 'PUT' || upper === 'PATCH') {
      settings.games ||= {};
      if (payload.settings === null) delete settings.games[key];
      else settings.games[key] = payload.settings || payload;
      saveBrowserSettings(settings);
      return { key, override: settings.games[key] ?? null, settings: browserSettingsMerge(settings.common || {}, settings.games[key] || {}) };
    }
  }
  if (upper === 'GET' && path === '/v1/settings/effective') {
    const key = url.searchParams.get('game') || browserGameKey();
    const override = settings.games?.[key] ?? null;
    return browserSettingsMerge(settings.common || {}, override || {});
  }

  if (path === '/v1/saves' && upper === 'GET') {
    return {
      game: { key: browserGameKey(), save: gameRoot, title: localGame?.folder || '', cartridge: Boolean(localGame?.rom) },
      backups: Object.keys(backupStore).sort().reverse().map(browserBackupInfo),
      can_restore: !gameStarted,
      running: gameStarted,
    };
  }
  if (path === '/v1/saves/backup' && upper === 'POST') {
    const name = createSaveBackup(payload.name);
    if (!name) controlError(409, 'no save files to back up');
    return browserBackupInfo(name);
  }
  if (path === '/v1/saves/restore' && upper === 'POST') {
    if (gameStarted) controlError(409, 'reload the page before restoring a save snapshot');
    const name = String(payload.backup || '');
    const backup = backupStore[name];
    if (!backup?.files) controlError(404, `unknown backup: ${name}`);
    saveStore = { ...backup.files };
    writeSaveStore();
    status(`Restored save backup ${name}. Reload the page to load it.`);
    return { restored: browserBackupInfo(name), saved: true };
  }
  if (path.startsWith('/v1/saves/backups/') && upper === 'DELETE') {
    const name = decodeURIComponent(path.slice('/v1/saves/backups/'.length));
    if (!backupStore[name]) controlError(404, `unknown backup: ${name}`);
    delete backupStore[name];
    writeBackupStore();
    return { deleted: name };
  }

  if (path === '/v1/launcher/status' && upper === 'GET') {
    const status = JSON.parse(engine.web_control_discovery()).status;
    return { ...status, title: localGame?.folder || '', source: browserGameKey() };
  }
  if (path === '/v1/launcher/library' && upper === 'GET') {
    return { games: localGame ? [{ title: localGame.folder || '', key: browserGameKey(), installed: true, platform: localGame.rom ? 'gba' : 'rpg-maker' }] : [], count: localGame ? 1 : 0 };
  }
  if (path === '/v1/launcher/pause' && upper === 'POST') {
    const paused = Boolean(payload.paused);
    engine.web_eframe_set_paused(paused);
    return { paused };
  }
  if (path === '/v1/launcher/live' && upper === 'GET') {
    return {
      ...JSON.parse(engine.web_runtime_settings()),
      paused: engine.web_eframe_game_paused(),
      game: localGame?.folder || '',
      source: browserGameKey(),
    };
  }
  if (path === '/v1/launcher/live.update' && (upper === 'PATCH' || upper === 'POST')) {
    if (payload.paused !== undefined) engine.web_eframe_set_paused(Boolean(payload.paused));
    if (payload.fast_forward !== undefined) engine.web_runtime_fast_forward(Boolean(payload.fast_forward));
    if (payload.fps !== undefined) engine.web_runtime_set_rate(Number(payload.fps) || 0);
    return {
      ...JSON.parse(engine.web_runtime_settings()),
      paused: engine.web_eframe_game_paused(),
    };
  }
  if (path.startsWith('/v1/catalogs')) {
    return { catalogs: [], count: 0 };
  }
  if (path.startsWith('/v1/launcher/') || path.startsWith('/v1/settings')) {
    controlError(409, 'this launcher operation is not available in a browser; use the desktop launcher');
  }

  if (upper === 'OPTIONS') return {};
  // Game operations: append body fields to the query string understood by the
  // generic Rhai route matcher, then poll the shared bridge.
  const query = new URLSearchParams(url.search);
  for (const [key, value] of Object.entries(payload)) {
    if (query.has(key)) continue;
    query.set(key, value !== null && typeof value === 'object' ? JSON.stringify(value) : String(value));
  }
  const forward = `${path}${query.toString() ? `?${query}` : ''}`;
  const id = engine.web_control_request(upper, forward);
  for (;;) {
    const state = JSON.parse(engine.web_control_take_response(id));
    if (state.ready) {
      if (state.status >= 400) controlError(state.status, state.body?.error || 'game request failed');
      return state.body;
    }
    await new Promise(resolve => setTimeout(resolve, 16));
  }
}
globalThis.FANPLAY.control = {
  discovery: async () => JSON.parse(engine.web_control_discovery()),
  openapi: async () => JSON.parse(engine.web_control_openapi()),
  request: controlRequest,
  fetch: async (target, init = {}) => {
    const method = String(init.method || 'GET').toUpperCase();
    const body = init.body || '';
    try {
      const value = await controlRequest(method, target, body);
      return new Response(JSON.stringify(value), { status: 200, headers: { 'content-type': 'application/json' } });
    } catch (error) {
      return new Response(JSON.stringify({ error: String(error.message || error) }), { status: error.status || 500, headers: { 'content-type': 'application/json' } });
    }
  },
  backupName: () => createSaveBackup(),
  restoreLatest: () => restoreLatestSaveBackup(),
};


// On-demand asset loading. The engine decodes images/audio while it runs and
// cannot await, so this uses a synchronous request and caches the result.
const assetCache = new Map();
globalThis.FANPLAY.readFile = (path) => {
  if (localGame) return null; // All selected assets are preloaded into Rust.
  if (assetCache.has(path)) return assetCache.get(path);
  let bytes = null;
  try {
    const xhr = new XMLHttpRequest();
    xhr.open('GET', gameRoot + path, false);
    xhr.overrideMimeType('text/plain; charset=x-user-defined');
    xhr.send();
    if (xhr.status >= 200 && xhr.status < 300) {
      const text = xhr.responseText;
      const array = new Uint8Array(text.length);
      for (let i = 0; i < text.length; i += 1) array[i] = text.charCodeAt(i) & 0xff;
      bytes = array;
    }
  } catch {
    bytes = null;
  }
  assetCache.set(path, bytes);
  return bytes;
};

// ---------------------------------------------------------------------------
// Ruby prelude: bridge + frame yield.
// ---------------------------------------------------------------------------


async function fetchBytes(url) {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${response.status} ${url}`);
  return new Uint8Array(await response.arrayBuffer());
}

// Game files are cached in the browser so a reload does not download them
// again. `?nocache=1` bypasses the cache (useful while editing assets).
const gameCacheName = 'fanplay-assets-v1';
let gameCachePromise;
function openGameCache() {
  if (gameCachePromise === undefined) {
    gameCachePromise =
      typeof caches !== 'undefined' && !params.has('nocache')
        ? caches.open(gameCacheName)
        : null;
  }
  return gameCachePromise;
}
async function fetchCached(path) {
  if (localGame) return localGame.read(path);
  const url = gameRoot + path;
  const cache = await openGameCache();
  if (cache) {
    const hit = await cache.match(url);
    if (hit) return new Uint8Array(await hit.arrayBuffer());
  }
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${response.status} ${url}`);
  if (cache) {
    try {
      await cache.put(url, response.clone());
    } catch {
      /* quota or opaque response; the file still loads from the network */
    }
  }
  return new Uint8Array(await response.arrayBuffer());
}

function startControllerLoop() {
  let savedControls;
  try {
    savedControls = readStored(localStorage, 'fanplay.controls');
    if (savedControls) engine.web_controls_load(savedControls);
  } catch { /* Storage may be disabled. Controls still work for this run. */ }
  const pollGamepads = createGamepadInput({
    getGamepads: () => navigator.getGamepads?.() || [],
    readMapping: () => engine.web_controls(),
    writeKey: (key, down) => sendKey(key, down),
    connected: count => engine.web_controllers_connected(count),
  });
  function controllerFrame() {
    pollGamepads(document.hasFocus() && !document.hidden && !textInput);
    const encoded = engine.web_controls();
    if (encoded !== savedControls) {
      try { localStorage.setItem('fanplay.controls', encoded); } catch {}
      savedControls = encoded;
    }
    requestAnimationFrame(controllerFrame);
  }
  window.addEventListener('blur', () => pollGamepads(false));
  requestAnimationFrame(controllerFrame);
}

async function mainCartridge() {
  cartridgeMode = true;
  gpuMode = true;
  textInput = false;
  setTextInputMode(false);
  if (!player?.eframeInitialized) await engine.web_init_eframe(canvas);
  const rom = localGame.rom;
  status(`Loading ${rom.name}…`);
  engine.web_eframe_loading(0, 0);
  await nextAnimationFrame();
  // Loading can take a moment for large DS ROMs. Yield once so the workspace
  // paints the previous state before the core blocks the thread.
  engine.web_core_init(rom.name, await localGame.readRom());
  status(`Running ${rom.name}.`);
  startControllerLoop();
  let running = true;
  function frame() {
    if (!running) return;
    try {
      const messages = JSON.parse(engine.web_core_take_messages());
      for (const message of messages) status(message);
      if (!engine.web_eframe_game_paused()) engine.web_core_step();
      engine.web_core_present();
      pumpPluginHttp();
      pumpPlayerActions();
      if (engine.web_eframe_take_reset()) {
        running = false;
        engine.web_core_stop();
        location.reload();
        return;
      }
      requestAnimationFrame(frame);
    } catch (error) {
      running = false;
      console.error(error);
      status('Stopped: ' + error);
      engine.web_eframe_game_stopped(`Game stopped: ${error}`);
    }
  }
  requestAnimationFrame(frame);
}

async function main() {
  status('Starting engine…');
  if (!player?.engineInitialized) await init(player ? { module_or_path: player.engineBytes } : undefined);
  if (player) player.engineBytes = undefined;
  engineReady = true;
  engine.web_set_reader(globalThis.FANPLAY.readFile);
  if (localGame?.rom) return mainCartridge();

  // Read Game.ini first so the engine (and the WebGPU surface) has the right
  // size before the game itself is downloaded.
  let version = 1;
  let width = 640;
  let height = 480;
  let scriptPath = 'Data/Scripts.rxdata';
  let title = 'fanplay';
  try {
    const ini = new TextDecoder().decode(await fetchCached('Game.ini'));
    for (const line of ini.split(/\r?\n/)) {
      const match = line.match(/^\s*([^=]+)=(.*)$/);
      if (!match) continue;
      const key = match[1].trim().toLowerCase();
      const value = match[2].trim();
      if (key === 'scripts') scriptPath = value.replace(/\\/g, '/');
      if (key === 'title') title = value;
    }
    if (scriptPath.endsWith('.rvdata2')) version = 3;
    else if (scriptPath.endsWith('.rvdata')) version = 2;
  } catch (error) {
    console.warn('Game.ini unavailable', error);
  }
  if (version !== 1) {
    width = 544;
    height = 416;
  }
  if (localGame) {
    for (const archive of localGame.archives) engine.web_add_archive(await localGame.read(archive));
  }
  if (!player?.eframeInitialized) await engine.web_init_eframe(canvas);
  const browserConfig = effectiveBrowserSettings();
  const configuredRate = Number(browserConfig.fps) || 0;
  engine.web_init(
    width,
    height,
    version,
    configuredRate > 0 ? configuredRate : (version === 1 ? 40 : 60),
    null,
  );
  if (browserConfig.fast_forward) engine.web_runtime_fast_forward(true);
  startControllerLoop();

  // eframe owns browser presentation and canvas input; game frames are submitted
  // to the same SharedApp texture used by native hosts.
  status('Starting workspace…');
  gpuMode = true;
  status('Shared eframe workspace');

  // Preload the whole game (the warm manifest is also copied into ruby.wasm's
  // own filesystem for Dir/FileTest) so nothing streams during play. The egui
  // workspace draws a download progress screen while this runs.
  status('Loading file list…');
  let manifest = [];
  let preload;
  if (localGame) {
    manifest = localGame.rubyManifest(player.warmManifest, scriptPath);
    preload = localGame.paths.filter(path => !localGame.archives.includes(path));
  } else {
    try {
      manifest = await (await fetch(assetList)).json();
    } catch (error) {
      console.warn(`Could not load ${assetList}: ${error}`);
    }
    preload = manifest;
    try {
      preload = await (await fetch(`${gameRoot}files.json`)).json();
    } catch (error) {
      console.warn('full file list unavailable; preloading the warm manifest only', error);
    }
  }
  const total = preload.length;
  let loaded = 0;
  engine.web_eframe_loading(0, total);
  let preloadError;
  let preloading = true;
  const loadingFrame = () => {
    engine.web_eframe_present();
    pumpPluginHttp();
    pumpPlayerActions();
    if (preloading) requestAnimationFrame(loadingFrame);
  };
  requestAnimationFrame(loadingFrame);
  // Bounded concurrency so a large game does not open thousands of requests.
  const workers = Array.from({ length: 16 }, async () => {
    for (;;) {
      const index = loaded;
      if (index >= total) return;
      loaded += 1;
      const path = preload[index];
      try {
        const bytes = await fetchCached(path);
        engine.web_add_file(path, bytes);
        const patch = path.match(/^patches\/([a-zA-Z0-9_-]+)\.rb$/i);
        if (patch) engine.web_register_user_patch(patch[1], new TextDecoder('utf-8', { fatal: true }).decode(bytes));
      } catch (error) {
        console.warn('missing asset', path, error);
        if (localGame) preloadError ||= new Error(`Cannot load ${path}: ${error.message || error}`);
      }
      if (loaded % 64 === 0) engine.web_eframe_loading(loaded, total);
    }
  });
  await Promise.all(workers);
  engine.web_eframe_loading(loaded, total);
  await nextAnimationFrame();
  preloading = false;
  engine.web_eframe_loading(0, 0);
  await player?.releaseSource?.();
  if (preloadError) throw preloadError;
  status(`Preloaded ${loaded}/${total} files.`);

  status('Starting Ruby…');
  const rubyWasm = player ? await WebAssembly.compile(player.rubyBytes)
    : await WebAssembly.compileStreaming(fetch('ruby/ruby+stdlib.wasm'));
  if (player) player.rubyBytes = undefined;
  await nextAnimationFrame();
  const { DefaultRubyVM } = globalThis['ruby-wasm-wasi'];
  const { vm } = await captureRubyOutput(() => DefaultRubyVM(rubyWasm, {
    env: {
      FANPLAY_SCRIPTS: scriptPath,
      FANPLAY_GAME_TITLE: title,
      HOME: '/',
      USER: 'player',
      LANG: 'en_US.UTF-8',
      // The game runs inside a Fiber, and ruby.wasm's default Fiber stacks are
      // tiny. Long play sessions can nest deeply enough to raise
      // SystemStackError, so give the Fiber (and thread) stacks a large stack.
      // Override with ?rubyStack=<bytes> / ?rubyMachineStack=<bytes>.
      RUBY_FIBER_VM_STACK_SIZE: String(rubyVmStackBytes),
      RUBY_FIBER_MACHINE_STACK_SIZE: String(rubyMachineStackBytes),
      RUBY_THREAD_VM_STACK_SIZE: String(rubyVmStackBytes),
      RUBY_THREAD_MACHINE_STACK_SIZE: String(rubyMachineStackBytes),
    },
  }), text => engine.web_eframe_log(text));
  setStatus('Preparing Ruby runtime…');
  await nextAnimationFrame();

  globalThis.FANPLAY.source = player ? player.rgssSource : await (await fetch('rgss.rb')).text();
  globalThis.FANPLAY.manifestText = manifest.join('\n');
  vm.eval(PRELUDE);
  setStatus('Preparing game scripts…');
  await nextAnimationFrame();
  vm.eval('$fanplay_source = JS.global[:FANPLAY][:source].to_s');
  vm.eval('$fanplay_fiber = Fiber.new { eval($fanplay_source, TOPLEVEL_BINDING, "rgss.rb", 1) }');
  const gameFiber = vm.eval('$fanplay_fiber');
  await nextAnimationFrame();
  engine.web_eframe_activate_extensions();

  let gameRunning = true;
  const pump = createGamePump({
    isAlive: () => gameFiber.call('alive?').toString() === 'true',
    resume: () => gameFiber.call('resume'),
    stopped: ({ result, error }) => {
      gameRunning = false;
      textInput = false;
      setTextInputMode(false);
      const message = error ? `Stopped: ${error}`
        : result?.toString() === '0' ? 'Game ended.' : `Game exited with status ${result?.toString() || 'unknown'}`;
      if (error) console.error(error);
      status(message);
      engine.web_eframe_game_stopped(message);
    },
    present: () => {
      engine.web_eframe_present();
      pumpPluginHttp();
      pumpPlayerActions();
      if (!gameRunning && engine.web_eframe_take_reset()) location.reload();
    },
    requestFrame: requestAnimationFrame,
    shouldRun: () => !engine.web_eframe_game_paused(),
  });

  setStatus('Starting game scripts…');
  requestAnimationFrame(pump);
}

function canvasPoint(event) {
  const rect = canvas.getBoundingClientRect();
  return {
    x: ((event.clientX - rect.left) * canvas.width) / rect.width,
    y: ((event.clientY - rect.top) * canvas.height) / rect.height,
  };
}

// UI shortcuts belong to egui. F11 reaches the engine input path, which
// toggles pacing even while the workspace UI is hidden.
//
// eframe installs its own canvas listeners and stops propagation for most
// events. Capture on `window` runs before that, so the game still receives
// input while egui keeps working. Never stop propagation here: the workspace
// needs the same F1/F2/F3/F10 keys.
const workspaceKeys = new Set(['F1', 'F2', 'F3', 'F10']);
window.addEventListener('keydown', (event) => {
  if ((event.metaKey || event.ctrlKey) && event.code === 'KeyC' && !textInput) {
    event.preventDefault();
    engine.web_eframe_copy_runtime_output();
    return;
  }
  if (workspaceKeys.has(event.code)) {
    event.preventDefault();
    return;
  }
  if (engine.web_eframe_wants_keyboard === undefined || engine.web_eframe_wants_keyboard()) return;
  const code = scancode(event.code);
  const textKey = !cartridgeMode && textInput && Array.from(event.key).length === 1
    && !event.ctrlKey && !event.metaKey && !event.altKey;
  if (code) {
    sendKey(code, true);
    if (!textKey) event.preventDefault();
  }
}, true);
window.addEventListener('keyup', (event) => {
  if ((event.metaKey || event.ctrlKey) && event.code === 'KeyC' && !textInput) {
    event.preventDefault();
    return;
  }
  if (workspaceKeys.has(event.code)) {
    event.preventDefault();
    return;
  }
  const code = scancode(event.code);
  if (code) {
    sendKey(code, false);
    event.preventDefault();
  }
}, true);
function gamePoint(event) {
  const point = canvasPoint(event);
  const scale = window.devicePixelRatio || 1;
  const [gx0, gy0, gw0, gh0] = Array.from(engine.web_eframe_game_rect());
  const [gx, gy, gw, gh] = [gx0, gy0, gw0, gh0].map(value => value * scale);
  const inside = gw > 0 && gh > 0
    && point.x >= gx && point.x < gx + gw && point.y >= gy && point.y < gy + gh;
  return {
    x: inside ? Math.round(((point.x - gx) / gw) * engine.web_width()) : 0,
    y: inside ? Math.round(((point.y - gy) / gh) * engine.web_height()) : 0,
    coreX: inside ? Math.round(((point.x - gx) / gw) * (engine.web_core_size()[0] || 0)) : 0,
    coreY: inside ? Math.round(((point.y - gy) / gh) * (engine.web_core_size()[1] || 0)) : 0,
    inside,
  };
}
canvas.addEventListener('mousemove', (event) => {
  const point = canvasPoint(event);
  if (gpuMode) {
    const scale = window.devicePixelRatio || 1;
    const [gx0, gy0, gw0, gh0] = Array.from(engine.web_eframe_game_rect());
    const [gx, gy, gw, gh] = [gx0, gy0, gw0, gh0].map(value => value * scale);
    const inside = gw > 0 && gh > 0
      && point.x >= gx && point.x < gx + gw && point.y >= gy && point.y < gy + gh;
    if (cartridgeMode) {
      const [width, height] = engine.web_core_size();
      if (inside && width > 0 && height > 0) {
        engine.web_core_pointer(
          Math.round(((point.x - gx) / gw) * width),
          Math.round(((point.y - gy) / gh) * height),
          true,
        );
      } else {
        engine.web_core_pointer(0, 0, false);
      }
    } else if (inside) {
      engine.web_mouse(
        Math.round(((point.x - gx) / gw) * engine.web_width()),
        Math.round(((point.y - gy) / gh) * engine.web_height()),
        true,
      );
    } else {
      engine.web_mouse(0, 0, false);
    }
    return;
  }
  if (cartridgeMode) engine.web_core_pointer(Math.round(point.x), Math.round(point.y), true);
  else engine.web_mouse(Math.round(point.x), Math.round(point.y), true);
});
canvas.addEventListener('mousedown', (event) => {
  if (textInput && !cartridgeMode) {
    const point = canvasPoint(event);
    const scale = window.devicePixelRatio || 1;
    const gameRect = gpuMode
      ? Array.from(engine.web_eframe_game_rect()).map(value => value * scale)
      : [0, 0, engine.web_width(), engine.web_height()];
    const [x, y, width, height] = gameRect;
    if (width > 0 && height > 0 && point.x >= x && point.x < x + width
      && point.y >= y && point.y < y + height) {
      setTextInputMode(true);
    }
  }
  if (cartridgeMode) {
    const point = gamePoint(event);
    if (point.inside) engine.web_core_pointer(point.coreX, point.coreY, true);
    engine.web_core_pointer_button(true);
  }
});
canvas.addEventListener('mouseup', () => {
  if (cartridgeMode) engine.web_core_pointer_button(false);
});
canvas.addEventListener('mouseleave', () => {
  if (cartridgeMode) {
    engine.web_core_pointer(0, 0, false);
    engine.web_core_pointer_button(false);
  } else {
    engine.web_mouse(0, 0, false);
  }
});
canvas.addEventListener(
  'wheel',
  (event) => {
    if (!cartridgeMode) engine.web_scroll(event.deltaY < 0 ? 1 : -1);
    event.preventDefault();
  },
  { passive: false },
);

main().catch(async (error) => {
  console.error(error);
  await player?.releaseSource?.();
  status('Error: ' + error);
  if (gpuMode) {
    engine.web_eframe_game_stopped(`Startup failed: ${error}`);
    const repaint = () => {
      if (engine.web_eframe_take_reset()) location.reload();
      requestAnimationFrame(repaint);
    };
    requestAnimationFrame(repaint);
  }
});
