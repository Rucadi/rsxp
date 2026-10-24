// SPDX-License-Identifier: GPL-2.0-or-later
import init, * as engine from './pkg/rsxp_web.js';
import { createLauncher } from './launcher-controller.js';

// The page supplies browser I/O. SessionPanel owns the launcher UI on both hosts.
const bundle = globalThis.RSXP_BUNDLE;
const canvas = document.getElementById('screen');
const picker = document.getElementById('game-folder');
const zipPicker = document.getElementById('game-zip');
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
status.hidden = true;
const launcher = createLauncher({
  bundle, supplied: globalThis.RSXP_GAME, document, location, fetch,
  update({ title, status: message, busy }) { engine.web_eframe_launcher(title, message, busy); },
  pickFolder() { picker.value = ''; picker.click(); },
  pickZip() { zipPicker.value = ''; zipPicker.click(); },
  async launch({ game, releaseSource }) {
    // Keep the renderer and WASM instance alive across the launcher/game boundary.
    globalThis.RSXP_PLAYER = {
      game, releaseSource, engineInitialized: true, eframeInitialized: true,
      rubyBytes: bundle.decode(bundle.payload.ruby),
      rgssSource: bundle.payload.rgss,
      warmManifest: bundle.payload.manifest,
      status(text) { console.log(text); },
    };
    await bundle.load('ruby/browser.umd.js');
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
    document.getElementById('rsxp-payload')?.remove();
  },
});
on(picker, 'change', () => launcher.folder(picker.files));
on(zipPicker, 'change', () => launcher.zip(zipPicker.files[0]));
on(window, 'resize', resize);
on(canvas, 'dragover', event => event.preventDefault());
on(canvas, 'drop', event => {
  event.preventDefault();
  if (event.dataTransfer.files.length === 1) launcher.zip(event.dataTransfer.files[0]);
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
