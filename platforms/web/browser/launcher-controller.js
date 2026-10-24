// SPDX-License-Identifier: GPL-2.0-or-later
// Platform services for the shared Rust SessionPanel. No widgets or layout here.
import { createLocalGame, createRomGame, isRom } from './local-game.js';
import { openZip, downloadTorrent } from './game-import.js';

export function createLauncher({ bundle, supplied, document, location, fetch, update, launch, pickFolder, pickZip, pickRom }) {
  let prepared;
  let controller;
  let starting = false;
  let title = supplied?.folder || '';
  const report = (status, busy = Boolean(controller || starting)) => update({ title, status, busy });
  async function zipLibrary() {
    await bundle.load('vendor/zip.js');
    return globalThis.zip;
  }
  async function prepare(factory) {
    if (controller || starting) return;
    controller = new AbortController();
    const signal = controller.signal;
    report('Preparing game…');
    try {
      await prepared?.dispose();
      prepared = undefined;
      title = '';
      prepared = await factory(signal);
      signal.throwIfAborted();
      title = prepared.game.folder;
      report('Ready. Press Play game.', false);
    } catch (error) {
      await prepared?.dispose();
      prepared = undefined;
      title = supplied?.folder || '';
      report(error.name === 'AbortError' ? 'Cancelled.' : String(error.message || error), false);
    } finally {
      controller = undefined;
    }
  }
  function suppliedFiles() {
    return supplied.files.map(([path, encoded]) => ({
      name: path.split('/').pop(),
      webkitRelativePath: `${supplied.folder}/${path}`,
      async arrayBuffer() {
        if (encoded !== null) {
          if (typeof encoded === 'string') return bundle.decode(encoded).buffer;
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
  async function start() {
    if (starting || controller || (!prepared && !supplied)) return;
    starting = true;
    report('Checking game folder…');
    try {
      const source = prepared;
      const playerName = decodeURIComponent(location.pathname.split('/').pop() || 'player.html');
      const game = source?.game || createLocalGame(suppliedFiles(), { playerName });
      await game.validate();
      await launch({ game, releaseSource: () => { prepared = undefined; return source?.dispose(); } });
    } catch (error) {
      starting = false;
      report(String(error.message || error), false);
    }
  }
  report('Choose a game to begin.', false);
  return {
    folder(files) {
      if (!files?.length) return;
      return prepare(async () => {
        const playerName = decodeURIComponent(location.pathname.split('/').pop() || 'player.html');
        const game = createLocalGame(files, { playerName });
        await game.validate();
        return { game, async dispose() {} };
      });
    },
    zip(file) {
      if (!file) return;
      if (!/\.zip$/i.test(file.name)) { report('Choose a ZIP file containing one game.'); return; }
      return prepare(async signal => openZip(file, { zip: await zipLibrary(), signal }));
    },
    rom(file) {
      if (!file) return;
      if (!isRom(file.name)) { report('Choose a .gba or .nds ROM file.'); return; }
      return prepare(async () => {
        const game = createRomGame(file, {
          playerName: decodeURIComponent(location.pathname.split('/').pop() || 'player.html'),
        });
        await game.validate();
        return { game, async dispose() {} };
      });
    },
    action(action) {
      if (action.cancel) controller?.abort();
      if (controller || starting) return;
      if (action.browse) pickFolder();
      if (action.zip) pickZip();
      if (action.browse_game_file) pickRom();
      if (action.open) return start();
      if (action.download) return prepare(async signal => {
        const [{ default: WebTorrent }, zip] = await Promise.all([bundle.load('vendor/webtorrent.js'), zipLibrary()]);
        return downloadTorrent(action.download, {
          WebTorrent, zip, signal,
          onProgress({ progress, downloaded, speed, peers, waiting }) {
            report(`${waiting ? 'Waiting for browser-compatible peers… ' : ''}${(progress * 100).toFixed(1)}% · ${(downloaded / 1048576).toFixed(1)} MiB · ${(speed / 1048576).toFixed(1)} MiB/s · ${peers} peers`);
          },
        });
      });
    },
  };
}
