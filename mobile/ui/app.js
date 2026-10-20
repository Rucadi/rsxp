// SPDX-License-Identifier: GPL-2.0-or-later
import { createTouchInput } from './input.js';
const $ = id => document.getElementById(id);
const decode = value => Uint8Array.from(atob(value), c => c.charCodeAt(0));
const encode = bytes => {
  let text = '';
  for (let offset = 0; offset < bytes.length; offset += 0x8000) {
    text += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000));
  }
  return btoa(text);
};
// cargo-mobile2 hosts the UI in a system WebView. Requests go over wry IPC and
// responses arrive asynchronously through `__rsxpResponse`.
const pending = new Map();
let nextRequest = 0;
globalThis.__rsxpResponse = (id, payload) => {
  const entry = pending.get(id);
  if (!entry) return;
  pending.delete(id);
  if (payload && payload.error) entry.reject(new Error(payload.error));
  else entry.resolve(payload ? payload.ok : null);
};
function native(request) {
  return new Promise((resolve, reject) => {
    if (!globalThis.ipc?.postMessage) {
      reject(new Error('Native Rust bridge is unavailable. Run the cargo-mobile2 app.'));
      return;
    }
    const id = ++nextRequest;
    pending.set(id, { resolve, reject });
    globalThis.ipc.postMessage(JSON.stringify({ id, request }));
  });
}
let running = false, paused = false, generation = 0;
let logs = [];
let nativePresentation = false;
let currentGame = null;
let hudVisible = true;
try { hudVisible = localStorage.getItem('rsxp.hud') !== '0'; } catch { /* Custom schemes may block storage. */ }
// Frame pacing lives in the shared UI because the mobile core runs Ruby fibers
// on demand. "default" follows the game's own rate, "uncapped" runs steps back
// to back, and a number pins an explicit cap.
let pacing = 'default';
try {
  const stored = localStorage.getItem('rsxp.rate');
  if (stored === 'default' || stored === 'uncapped' || /^\d+$/.test(stored || '')) pacing = stored;
} catch { /* Optional. */ }
// Native frame counters, sampled twice a second for the performance overlay.
const perf = { frames: 0, rate: 0, fps: 0, frameTime: 0, windowStart: 0, windowFrames: 0 };
let hudUpdated = 0;
function setPacing(value) {
  pacing = value;
  try { localStorage.setItem('rsxp.rate', value); } catch { /* Optional. */ }
  renderHud();
}
const touch = createTouchInput((code, down) => {
  if (running) native({ op: 'key', code, down }).catch(() => {});
});
function releaseControls() {
  touch.releaseAll();
  document.querySelectorAll('.held').forEach(button => button.classList.remove('held'));
}
function status(message) { $('status').textContent = message; $('status').hidden = !message; }
function failure(error) {
  console.error(error);
  stop();
  setFrameVisible(false);
  $('error-message').textContent = String(error.message || error) + (logs.length ? '\n\n' + logs.slice(-20).join('\n') : '');
  $('error').showModal();
}
async function stop() {
  releaseControls();
  running = false; paused = false; generation++; currentGame = null;
  perf.frames = 0; perf.fps = 0; perf.frameTime = 0; perf.windowStart = 0;
  if ($('game-menu').open) $('game-menu').close();
  setFrameVisible(true);
  try { await native({ op: 'stop' }); } catch { /* Initial bridge errors are reported separately. */ }
  $('player').hidden = true; $('library').hidden = false; status('');
}
async function refresh() {
  const games = await native({ op: 'games' });
  $('games').replaceChildren(...games.map(game => {
    const button = document.createElement('button'); button.className = 'game';
    const icon = document.createElement('span'); icon.className = 'game-icon'; icon.textContent = game.demo ? '✦' : '▧';
    const text = document.createElement('span');
    const title = document.createElement('strong'); title.textContent = game.title;
    const detail = document.createElement('small'); detail.textContent = game.demo ? 'Built in · Test the native engine' : 'Imported game · Tap to start';
    text.append(title, detail);
    const arrow = document.createElement('span'); arrow.className = 'arrow'; arrow.textContent = '›';
    button.append(icon, text, arrow); button.onclick = () => start(game).catch(failure); return button;
  }));
}
const nextFrame = () => new Promise(resolve => requestAnimationFrame(resolve));
// Sample the engine's completed-frame counter so FPS reflects real updates,
// not just how often the interface asked for one.
function samplePerformance(stats, now) {
  perf.frames = stats.frames; perf.rate = stats.rate;
  if (!perf.windowStart) { perf.windowStart = now; perf.windowFrames = stats.frames; }
  const elapsed = now - perf.windowStart;
  if (elapsed >= 500) {
    perf.fps = Math.max(0, (stats.frames - perf.windowFrames) * 1000 / elapsed);
    perf.frameTime = perf.fps > 0 ? 1000 / perf.fps : 0;
    perf.windowStart = now; perf.windowFrames = stats.frames;
  }
  if (now - hudUpdated >= 250) { hudUpdated = now; renderHud(); }
}
const rateLabel = rate => rate > 0 ? `${rate} Hz` : 'uncapped';
const effectiveRate = () => pacing === 'uncapped' ? 0 : pacing === 'default' ? perf.rate : Number(pacing);
function renderHud() {
  $('hud').hidden = !hudVisible;
  $('hud-fps').textContent = `${perf.fps.toFixed(perf.fps < 10 ? 1 : 0)} fps`;
  $('hud-frame').textContent = `frame ${perf.frames}`;
  $('hud-rate').textContent = rateLabel(effectiveRate());
  if ($('game-menu').open) fillMenuStats();
}
function fillMenuStats() {
  $('menu-fps').textContent = perf.fps.toFixed(1);
  $('menu-frame').textContent = perf.frames;
  $('menu-frame-time').textContent = perf.frameTime ? `${perf.frameTime.toFixed(1)} ms` : '–';
  $('menu-rate').textContent = rateLabel(effectiveRate());
  for (const button of document.querySelectorAll('#game-menu [data-rate]')) {
    button.classList.toggle('active', button.dataset.rate === pacing);
  }
}
function setHud(visible) {
  hudVisible = visible;
  $('menu-hud-toggle').checked = visible;
  try { localStorage.setItem('rsxp.hud', visible ? '1' : '0'); } catch { /* Optional. */ }
  renderHud();
}
async function start(game) {
  if (running) return;
  status('Starting native Rust engine…'); logs = [];
  await nextFrame();
  const config = await native({ op: 'init', id: game.id });
  status('Starting native Ruby scripts…'); await nextFrame();
  running = true; paused = false; currentGame = game; const current = ++generation;
  perf.frames = 0; perf.fps = 0; perf.frameTime = 0; perf.windowStart = 0; hudUpdated = 0;
  status('');
  $('title').textContent = config.title; $('library').hidden = true; $('player').hidden = false;
  $('menu-hud-toggle').checked = hudVisible;
  renderHud();
  const context = nativePresentation ? null : $('screen').getContext('2d', { alpha: false });
  if (nativePresentation) positionFrame();
  let deadline = 0;
  async function step(now) {
    try {
      const stats = await native({ op: 'step' });
      if (!stats.running) { stop(); return false; }
      samplePerformance(stats, now);
      if (!nativePresentation) {
        const frame = await native({ op: 'frame' });
        if ($('screen').width !== frame.width || $('screen').height !== frame.height) {
          $('screen').width = frame.width; $('screen').height = frame.height;
        }
        context.putImageData(new ImageData(new Uint8ClampedArray(decode(frame.rgba).buffer), frame.width, frame.height), 0, 0);
      }
    } catch (error) { failure(error); return false; }
    return true;
  }
  async function pump(now) {
    if (!running || current !== generation) return;
    try {
      if (!paused) {
        const target = pacing === 'uncapped' ? 0 : pacing === 'default' ? config.rate : Number(pacing);
        if (target > 0) {
          // Preserve the cadence: adding a period to `now` caps 40 fps games at
          // 30 fps on a 60 Hz display. Discard backlog after a pause or slow frame.
          if (now >= deadline) {
            if (!deadline) deadline = now;
            deadline = Math.max(deadline + 1000 / target, now);
            if (!(await step(now))) return;
          }
        } else {
          // Uncapped: run several updates per display refresh until the budget
          // is spent, so the engine can exceed the screen refresh rate.
          const start = performance.now();
          let steps = 0;
          do {
            if (!(await step(performance.now()))) return;
          } while (++steps < 64 && performance.now() - start < 12);
        }
      }
    } catch (error) { failure(error); return; }
    if (running && current === generation) requestAnimationFrame(pump);
  }
  requestAnimationFrame(pump);
}
// Tapping the game area confirms/advances, matching the "press Enter" message
// shown by RGSS title screens on devices without a hardware keyboard.
const viewport = $('viewport');
viewport.onpointerdown = event => {
  event.preventDefault(); if (!running || paused) return;
  try { viewport.setPointerCapture(event.pointerId); } catch { /* Best effort. */ }
  touch.press(event.pointerId, 29);
};
for (const type of ['pointerup', 'pointercancel', 'lostpointercapture']) viewport.addEventListener(type, event => {
  touch.release(event.pointerId);
});
for (const button of document.querySelectorAll('[data-key]')) {
  button.onpointerdown = event => {
    event.preventDefault(); if (!running || paused) return;
    // Pointer capture is an optimisation; some WebKit versions reject it and
    // it must not prevent the key press itself.
    try { button.setPointerCapture(event.pointerId); } catch { /* Best effort. */ }
    button.classList.add('held');
    touch.press(event.pointerId, Number(button.dataset.key));
  };
  for (const type of ['pointerup', 'pointercancel', 'lostpointercapture']) button.addEventListener(type, event => {
    touch.release(event.pointerId); button.classList.remove('held');
  });
}
document.addEventListener('visibilitychange', () => {
  if (!running) return;
  if (document.hidden) { releaseControls(); paused = true; }
  // Resume after returning from the app switcher. Keep the pause if the user
  // deliberately left the in-game menu open.
  else if (!$('game-menu').open) paused = false;
});
window.addEventListener('blur', releaseControls);
const keys = { ArrowUp: 82, ArrowDown: 81, ArrowLeft: 80, ArrowRight: 79, KeyZ: 29, Enter: 40, KeyX: 27, Escape: 41, ShiftLeft: 225 };
window.addEventListener('keydown', event => { if (keys[event.code] && running && !paused) { event.preventDefault(); touch.press(event.code, keys[event.code]); } });
window.addEventListener('keyup', event => touch.release(event.code));
function setFrameVisible(visible) {
  // The native game surface is layered above the WebView on mobile. Hide it
  // while a dialog covers the screen so the interface stays readable.
  if (!nativePresentation) return;
  native({ op: 'frameVisibility', visible }).catch(() => {});
}
function openMenu() {
  if (!running) return;
  releaseControls(); paused = true;
  fillMenuStats();
  setFrameVisible(false);
  $('game-menu').showModal();
}
function closeMenu() { if ($('game-menu').open) $('game-menu').close(); }
$('menu').onclick = openMenu;
$('game-menu').addEventListener('close', () => { if (running) { paused = false; setFrameVisible(true); } });
$('menu-resume').onclick = closeMenu;
$('menu-restart').onclick = () => {
  const game = currentGame; closeMenu(); if (!game) return;
  stop().then(() => start(game)).catch(failure);
};
$('menu-library').onclick = () => { closeMenu(); stop(); };
$('menu-hud-toggle').onchange = event => setHud(event.target.checked);
for (const button of document.querySelectorAll('#game-menu [data-rate]')) {
  button.onclick = () => setPacing(button.dataset.rate);
}
$('back').onclick = () => { stop(); };
$('dismiss').onclick = () => { $('error').close(); refresh().catch(failure); };
$('refresh').onclick = () => { refresh().catch(failure); };

// Folder import uses the WebView's native directory picker. Each file is
// streamed to Rust in chunks so large games never sit in one IPC message.
const picker = $('import-picker');
async function importFiles(files) {
  if (!files || !files.length) return;
  const first = (files[0].webkitRelativePath || '').replaceAll('\\', '/');
  const name = first.split('/')[0];
  if (!name || !first.includes('/')) throw new Error('Choose a game folder, not individual files.');
  status(`Importing ${name}…`);
  try {
    await native({ op: 'importBegin', name });
    for (let index = 0; index < files.length; index++) {
      const file = files[index];
      const path = (file.webkitRelativePath || '').replaceAll('\\', '/').split('/').slice(1).join('/');
      if (!path) continue;
      const bytes = new Uint8Array(await file.arrayBuffer());
      const chunk = 512 * 1024;
      if (bytes.length === 0) {
        await native({ op: 'importChunk', path, offset: 0, data: '' });
      } else {
        for (let offset = 0; offset < bytes.length; offset += chunk) {
          await native({ op: 'importChunk', path, offset, data: encode(bytes.subarray(offset, offset + chunk)) });
        }
      }
      status(`Importing ${name}… ${index + 1}/${files.length}`);
    }
    await native({ op: 'importEnd' });
    status('');
    await refresh();
  } catch (error) {
    try { await native({ op: 'importCancel' }); } catch { /* Best effort. */ }
    throw error;
  }
}
$('import').onclick = async () => {
  try {
    const info = await native({ op: 'info' });
    if (info.platform === 'linux') {
      const imported = await native({ op: 'chooseFolder' });
      if (imported) await refresh();
    } else {
      picker.value = '';
      picker.click();
    }
  } catch (error) { failure(error); }
};
picker.onchange = () => { importFiles(picker.files).catch(failure); };

function positionFrame() {
  if (!nativePresentation) return;
  const r = $('viewport').getBoundingClientRect();
  native({ op: 'viewport', x: r.x, y: r.y, width: r.width, height: r.height }).catch(() => {});
}
window.addEventListener('resize', () => { if (running) requestAnimationFrame(positionFrame); });

(async () => {
  const info = await native({ op: 'info' });
  nativePresentation = info.nativePresentation;
  $('platform').textContent = `${info.engine} · ${info.platform} / ${info.arch}`;
  await refresh();
  const parameters = new URL(location.href).searchParams;
  if (parameters.has('game')) {
    start({ id: parameters.get('game') }).catch(failure);
  } else if (parameters.get('demo') === '1') {
    start({ id: '.demo' }).catch(failure);
  }
})().catch(failure);
