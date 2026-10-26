// SPDX-License-Identifier: GPL-2.0-or-later
import init, * as engine from './pkg/fanplay_web.js';
import { createLauncher } from './launcher-controller.js';

// The page supplies browser I/O. SessionPanel owns the launcher UI on both hosts.
const bundle = globalThis.FANPLAY_BUNDLE;
const canvas = document.getElementById('screen');
const picker = document.getElementById('game-folder');
const zipPicker = document.getElementById('game-zip');
const romPicker = document.getElementById('game-rom');
const status = document.getElementById('launch-status');
const events = new AbortController();
const on = (target, name, callback, options = {}) => target.addEventListener(name, callback, { ...options, signal: events.signal });
let active = true;
await init({ module_or_path: bundle.decode(bundle.payload.engine) });
delete bundle.payload.engine;
function resize() {
  const scale = window.devicePixelRatio || 1;
  canvas.width = Math.max(1, Math.floor(window.innerWidth * scale));
  canvas.height = Math.max(1, Math.floor(window.innerHeight * scale));
}
resize();
await engine.web_init_eframe(canvas);
globalThis.FANPLAY = globalThis.FANPLAY || {};
globalThis.FANPLAY.control = {
  discovery: async () => JSON.parse(engine.web_control_discovery()),
  openapi: async () => JSON.parse(engine.web_control_openapi()),
  request: async () => {
    throw Object.assign(new Error('Start a game first; the browser game bridge becomes active after Play.'), { status: 409 });
  },
  fetch: async () => new Response(JSON.stringify({ error: 'Start a game first; the browser game bridge becomes active after Play.' }), {
    status: 409, headers: { 'content-type': 'application/json' },
  }),
};
status.hidden = true;
const launcher = createLauncher({
  bundle, supplied: globalThis.FANPLAY_GAME, document, location, fetch,
  update({ title, status: message, busy }) { engine.web_eframe_launcher(title, message, busy); },
  pickFolder() { picker.value = ''; picker.click(); },
  pickZip() { zipPicker.value = ''; zipPicker.click(); },
  pickRom() { romPicker.value = ''; romPicker.click(); },
  async launch({ game, releaseSource }) {
    const cartridge = Boolean(game.rom);
    // A cartridge session never starts ruby.wasm, so leave the interpreter
    // module and RGSS bundle untouched (and avoid parsing ~30 MiB of Ruby).
    if (!cartridge) await bundle.load('ruby/browser.umd.js');
    // Keep the renderer and WASM instance alive across the launcher/game boundary.
    globalThis.FANPLAY_PLAYER = {
      game, releaseSource, engineInitialized: true, eframeInitialized: true,
      rubyBytes: cartridge ? undefined : bundle.decode(bundle.payload.ruby),
      rgssSource: cartridge ? '' : bundle.payload.rgss,
      warmManifest: bundle.payload.manifest,
      status(text) { console.log(text); },
    };
    active = false;
    engine.web_eframe_finish_launcher();
    try {
      await bundle.load('app.js');
    } catch (error) {
      active = true;
      requestAnimationFrame(tick);
      throw error;
    }
    events.abort();
    delete bundle.payload.ruby;
    document.getElementById('fanplay-payload')?.remove();
  },
});
on(picker, 'change', () => launcher.folder(picker.files));
on(zipPicker, 'change', () => launcher.zip(zipPicker.files[0]));
on(romPicker, 'change', () => launcher.rom(romPicker.files[0]));
on(window, 'resize', resize);
on(canvas, 'dragover', event => event.preventDefault());
on(canvas, 'drop', event => {
  event.preventDefault();
  if (event.dataTransfer.files.length !== 1) return;
  const file = event.dataTransfer.files[0];
  if (/\.(gb|gbc|gba|nds)$/i.test(file.name)) launcher.rom(file);
  else launcher.zip(file);
});
function paint() {
  if (!active) return;
  Promise.resolve(launcher.action(JSON.parse(engine.web_eframe_launcher_actions()))).catch(error => {
    engine.web_eframe_launcher('', String(error.message || error), false);
  });
}
function tick() {
  if (!active) return;
  try { paint(); requestAnimationFrame(tick); }
  catch (error) { status.hidden = false; status.textContent = String(error); }
}
tick();
