// SPDX-License-Identifier: GPL-2.0-or-later
// Browser host for the rsxp engine.
//
// The Rust engine runs as WebAssembly and owns rendering, archives and input.
// The game's Ruby runs under ruby.wasm and talks to the engine through
// `globalThis.RSXP.native`, which carries base64-encoded native requests. Ruby
// executes inside a Fiber that yields on every Graphics.update, so the page can
// present frames and collect input between frames without a worker.
import init, * as engine from './pkg/rsxp_web.js';
import { createGamepadInput } from './gamepad.js';
import { PRELUDE } from './ruby-host.js';
import { createNativeDispatcher } from './native-bridge.js';
import { createGamePump, captureRubyOutput } from './game-pump.js';

const canvas = document.getElementById('screen');

// The workspace UI (toolbar, panels, performance, runtime patches, save
// export/import) is drawn by egui inside the engine. The page only hosts the
// canvas and the Ruby VM, so there is no HTML chrome to keep in sync.
const status = (text) => { console.log(text); globalThis.RSXP_PLAYER?.status?.(text); };

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

globalThis.__rsxp_copyText = (text) => {
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
const player = globalThis.RSXP_PLAYER;
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

function bytesToBase64(bytes) {
  let binary = '';
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode.apply(null, bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}

function encodeNativeResponse(response) {
  if (response === undefined) return '__RSXP_STOP__';
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
const saveStoreKey = `rsxp-saves:${gameRoot}`;
let saveStore = {};
try {
  saveStore = JSON.parse(localStorage.getItem(saveStoreKey) || '{}') || {};
} catch {
  saveStore = {};
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
  link.download = 'rsxp-saves.json';
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
globalThis.__rsxp_exportSaves = exportSaves;
globalThis.__rsxp_importSaves = () => importInput.click();

const dispatchNative = createNativeDispatcher(engine, (enabled) => {
  textInput = enabled;
  setTextInputMode(enabled);
});

// The bridge Ruby calls. Returns base64, or null to signal a clean stop.
globalThis.RSXP = {
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
    if (response === undefined) return '__RSXP_STOP__';
    if (response.length === 0) return '';
    return bytesToBase64(response);
  },
  native_text(requestText) {
    const response = dispatchNative(new TextEncoder().encode(requestText));
    return encodeNativeResponse(response);
  },
};
globalThis.MKXP = globalThis.RSXP;

// On-demand asset loading. The engine decodes images/audio while it runs and
// cannot await, so this uses a synchronous request and caches the result.
const assetCache = new Map();
globalThis.RSXP.readFile = (path) => {
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
const gameCacheName = 'rsxp-assets-v1';
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

async function main() {
  status('Starting engine…');
  if (!player?.engineInitialized) await init(player ? { module_or_path: player.engineBytes } : undefined);
  if (player) player.engineBytes = undefined;
  engineReady = true;
  engine.web_set_reader(globalThis.RSXP.readFile);

  // Read Game.ini first so the engine (and the WebGPU surface) has the right
  // size before the game itself is downloaded.
  let version = 1;
  let width = 640;
  let height = 480;
  let scriptPath = 'Data/Scripts.rxdata';
  let title = 'rsxp';
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
  engine.web_init(width, height, version, version === 1 ? 40 : 60, null);
  let savedControls;
  try {
    savedControls = localStorage.getItem('rsxp.controls');
    if (savedControls) engine.web_controls_load(savedControls);
  } catch { /* Storage may be disabled. Controls still work for this run. */ }
  const pollGamepads = createGamepadInput({
    getGamepads: () => navigator.getGamepads?.() || [],
    readMapping: () => engine.web_controls(),
    writeKey: (key, down) => engine.web_key(key, down),
    connected: count => engine.web_controllers_connected(count),
  });
  function controllerFrame() {
    pollGamepads(document.hasFocus() && !document.hidden && !textInput);
    const encoded = engine.web_controls();
    if (encoded !== savedControls) {
      try { localStorage.setItem('rsxp.controls', encoded); } catch {}
      savedControls = encoded;
    }
    requestAnimationFrame(controllerFrame);
  }
  window.addEventListener('blur', () => pollGamepads(false));
  requestAnimationFrame(controllerFrame);

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
      RSXP_SCRIPTS: scriptPath,
      RSXP_GAME_TITLE: title,
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

  globalThis.RSXP.source = player ? player.rgssSource : await (await fetch('rgss.rb')).text();
  globalThis.RSXP.manifestText = manifest.join('\n');
  vm.eval(PRELUDE);
  setStatus('Preparing game scripts…');
  await nextAnimationFrame();
  vm.eval('$rsxp_source = JS.global[:RSXP][:source].to_s');
  vm.eval('$rsxp_fiber = Fiber.new { eval($rsxp_source, TOPLEVEL_BINDING, "rgss.rb", 1) }');
  const gameFiber = vm.eval('$rsxp_fiber');
  await nextAnimationFrame();

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
      if (!gameRunning && engine.web_eframe_take_reset()) location.reload();
    },
    requestFrame: requestAnimationFrame,
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
  const code = scancode(event.code);
  const textKey = textInput && Array.from(event.key).length === 1
    && !event.ctrlKey && !event.metaKey && !event.altKey;
  if (code) {
    engine.web_key(code, true);
    if (!textKey) event.preventDefault();
  }
});
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
    engine.web_key(code, false);
    event.preventDefault();
  }
});
canvas.addEventListener('mousemove', (event) => {
  const point = canvasPoint(event);
  if (gpuMode) {
    const scale = window.devicePixelRatio || 1;
    const [gx0, gy0, gw0, gh0] = Array.from(engine.web_eframe_game_rect());
    const [gx, gy, gw, gh] = [gx0, gy0, gw0, gh0].map(value => value * scale);
    if (gw > 0 && gh > 0 && point.x >= gx && point.x < gx + gw && point.y >= gy && point.y < gy + gh) {
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
  engine.web_mouse(Math.round(point.x), Math.round(point.y), true);
});
canvas.addEventListener('mousedown', (event) => {
  if (textInput) {
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
});
canvas.addEventListener('mouseleave', () => engine.web_mouse(0, 0, false));
canvas.addEventListener(
  'wheel',
  (event) => {
    engine.web_scroll(event.deltaY < 0 ? 1 : -1);
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
