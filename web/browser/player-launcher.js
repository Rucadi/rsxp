// SPDX-License-Identifier: GPL-2.0-or-later
import { createLocalGame } from './local-game.js';
import { openZip, downloadTorrent } from './game-import.js';

const panel = document.getElementById('launcher');
const button = document.getElementById('choose-game');
const picker = document.getElementById('game-folder');
const message = document.getElementById('launch-status');
const bundle = globalThis.RSXP_BUNDLE;
const supplied = globalThis.RSXP_GAME;
const zipPicker = document.getElementById('game-zip');
const torrentInput = document.getElementById('torrent-url');
const downloadButton = document.getElementById('download-torrent');
const cancelButton = document.getElementById('cancel-download');
const dropZone = document.getElementById('drop-zone');
let prepared;
let controller;
let starting = false;
function busy(value) {
  for (const control of [button, picker, zipPicker, torrentInput, downloadButton]) control.disabled = value;
  cancelButton.hidden = !value;
}
button.addEventListener('click', () => {
  if (prepared) return startGame(null, prepared);
  return supplied ? startGame(suppliedFiles()) : picker.click();
});
async function zipLibrary() {
  await bundle.load('vendor/zip.js');
  return globalThis.zip;
}
async function prepare(factory) {
  if (controller || starting) return;
  await prepared?.dispose();
  prepared = undefined;
  controller = new AbortController();
  busy(true);
  try {
    prepared = await factory(controller.signal);
    controller.signal.throwIfAborted();
    button.textContent = 'Start game';
    message.textContent = `Ready: ${prepared.game.folder}. Press Start game.`;
  } catch (error) {
    await prepared?.dispose();
    prepared = undefined;
    button.textContent = 'Choose game folder';
    message.textContent = error.name === 'AbortError' ? 'Cancelled.' : String(error.message || error);
  } finally {
    controller = undefined;
    busy(false);
    zipPicker.value = '';
  }
}
function chooseZip(file) {
  if (!file) return;
  if (!/\.zip$/i.test(file.name)) { message.textContent = 'Choose a ZIP file containing one game.'; return; }
  message.textContent = 'Reading ZIP directory…';
  return prepare(async signal => openZip(file, { zip: await zipLibrary(), signal }));
}
zipPicker.addEventListener('change', () => chooseZip(zipPicker.files[0]));
for (const event of ['dragenter', 'dragover']) dropZone.addEventListener(event, e => {
  e.preventDefault(); dropZone.classList.add('dragover');
});
dropZone.addEventListener('dragleave', () => dropZone.classList.remove('dragover'));
dropZone.addEventListener('drop', event => {
  event.preventDefault(); dropZone.classList.remove('dragover');
  if (event.dataTransfer.files.length !== 1) { message.textContent = 'Drop one game ZIP at a time.'; return; }
  return chooseZip(event.dataTransfer.files[0]);
});
cancelButton.addEventListener('click', () => controller?.abort());
downloadButton.addEventListener('click', () => {
  message.textContent = 'Connecting to WebTorrent…';
  return prepare(async signal => {
    const [{ default: WebTorrent }, zip] = await Promise.all([bundle.load('vendor/webtorrent.js'), zipLibrary()]);
    return downloadTorrent(torrentInput.value, {
      WebTorrent, zip, signal,
      onProgress({ progress, downloaded, speed, peers, waiting }) {
        message.textContent = `${waiting ? 'Waiting for browser-compatible peers… ' : ''}${(progress * 100).toFixed(1)}% · ${(downloaded / 1048576).toFixed(1)} MiB · ${(speed / 1048576).toFixed(1)} MiB/s · ${peers} peers`;
      },
    });
  });
});
function suppliedFiles() {
  return supplied.files.map(([path, encoded]) => ({
    name: path.split('/').pop(),
    webkitRelativePath: `${supplied.folder}/${path}`,
    async arrayBuffer() {
      if (encoded !== null) {
        if (typeof encoded === 'string') return bundle.decode(encoded).buffer;
        // Each block is small enough to parse independently, even for GB games.
        const chunks = encoded.chunks.map(id => bundle.decode(document.getElementById(id).textContent));
        return new Blob(chunks).arrayBuffer();
      }
      const response = await fetch('/game/' + path.split('/').map(encodeURIComponent).join('/'));
      if (!response.ok) throw new Error(`Cannot read ${path}: ${response.status}`);
      return response.arrayBuffer();
    },
    async text() { return new TextDecoder().decode(await this.arrayBuffer()); },
  }));
}
if (supplied) {
  document.getElementById('source-options').hidden = true;
  button.textContent = 'Start game';
  panel.querySelector('h1').textContent = supplied.folder;
  panel.querySelector('p').textContent = 'Start the game in your browser.';
  message.textContent = 'Ready.';
}
picker.addEventListener('change', () => {
  if (picker.files.length) startGame(picker.files);
});
async function startGame(files, source) {
  if (starting || controller) return;
  starting = true;
  busy(true);
  cancelButton.hidden = true;
  message.textContent = 'Checking game folder…';
  try {
    const playerName = decodeURIComponent(location.pathname.split('/').pop() || 'player.html');
    const game = source?.game || createLocalGame(files, { playerName });
    await game.validate();
    message.textContent = `Starting ${game.folder}…`;
    const engineBytes = bundle.decode(bundle.payload.engine);
    const rubyBytes = bundle.decode(bundle.payload.ruby);
    delete bundle.payload.engine;
    delete bundle.payload.ruby;
    document.getElementById('rsxp-payload').remove();
    globalThis.RSXP_PLAYER = {
      game, engineBytes, rubyBytes,
      releaseSource: () => { prepared = undefined; return source?.dispose(); },
      rgssSource: bundle.payload.rgss,
      warmManifest: bundle.payload.manifest,
      status(text) {
        message.textContent = text;
        if (text.startsWith('WebGPU workspace')) panel.hidden = true;
        if (text.startsWith('Error:')) { panel.hidden = false; message.textContent = `${text} Reload this file to try again.`; }
      },
    };
    await bundle.load('ruby/browser.umd.js');
    await bundle.load('app.js');
  } catch (error) {
    message.textContent = String(error.message || error);
    await source?.dispose();
    prepared = undefined;
    starting = false;
    busy(false);
    picker.value = '';
  }
}
