// SPDX-License-Identifier: GPL-2.0-or-later
import { createLocalGame } from './local-game.js';

function safePath(path) {
  path = path.replaceAll('\\', '/');
  if (path.startsWith('/') || /^[a-z]:/i.test(path) || /[\0-\x1f]/.test(path) || path.split('/').includes('..')) {
    throw new Error(`Invalid game path: ${path}`);
  }
  return path.replace(/^(\.\/)+/, '');
}

// ZIPs/torrents may contain a wrapper directory. Locate the actual game root,
// while keeping each asset lazy: no second uncompressed copy of the whole game.
export function gameFromEntries(entries, name = 'Game') {
  entries = entries.map(entry => ({ ...entry, path: safePath(entry.path) }))
    .filter(entry => entry.path && !entry.path.split('/').some(part => part.startsWith('.') || part === '__MACOSX'));
  let roots = entries.filter(({ path }) => /(^|\/)game\.ini$/i.test(path)).map(({ path }) => path.slice(0, -8));
  if (!roots.length) {
    roots = entries.flatMap(({ path }) => {
      const match = path.match(/^(.*?)(?:Data\/Scripts\.(?:rxdata|rvdata2?)|Game\.rgss(?:ad|2a|3a))$/i);
      return match ? [match[1]] : [];
    });
  }
  roots = [...new Set(roots)];
  if (!roots.length) throw new Error('No game found. The archive must contain Game.ini, Data/Scripts.rxdata, or an RGSS archive.');
  if (roots.length > 1) throw new Error('This download contains multiple games. Choose a ZIP or torrent containing one game.');
  const root = roots[0];
  const folder = (root.split('/').filter(Boolean).pop() || name.replace(/\.zip$/i, '') || 'Game').replaceAll(/[\/\\]/g, '_');
  const files = entries.filter(({ path }) => path.startsWith(root)).map(entry => {
    const path = entry.path.slice(root.length);
    return {
      name: path.split('/').pop(), webkitRelativePath: `${folder}/${path}`,
      arrayBuffer: () => entry.read(),
      async text() { return new TextDecoder().decode(await entry.read()); },
    };
  });
  return createLocalGame(files);
}

export async function openZip(blob, { zip, signal, name = blob.name || 'Game' } = {}) {
  signal?.throwIfAborted();
  // Use embedded codecs on the main thread: no worker URL or server required.
  let reader = new zip.ZipReader(new zip.BlobReader(blob), { useWebWorkers: false });
  try {
    const entries = await reader.getEntries({ signal });
    const game = gameFromEntries(entries.filter(entry => !entry.directory).map(entry => ({
      path: entry.filename,
      read: () => entry.getData(new zip.Uint8ArrayWriter(), { signal, useWebWorkers: false, checkSignature: true }).then(bytes => bytes.buffer),
    })), name);
    await game.validate();
    return { game, async dispose() {
      game.releaseSource();
      const source = reader; reader = undefined;
      await source?.close();
    } };
  } catch (error) { await reader.close(); throw error; }
}

export function torrentID(input) {
  const value = input.trim();
  if (/^magnet:\?/i.test(value)) {
    const url = new URL(value);
    if (!url.searchParams.getAll('xt').some(xt => /^urn:bt(?:ih|mh):/i.test(xt))) throw new Error('The magnet link is missing its torrent hash.');
    return value;
  }
  const url = new URL(value);
  if (!['http:', 'https:'].includes(url.protocol)) throw new Error('Use a magnet link or an HTTP(S) URL to a .torrent file.');
  return url.href;
}

// A session-only store works on file:// too. It is released after preload.
export class TorrentStore {
  constructor() { this.chunks = new Map(); }
  put(index, bytes, callback) { this.chunks.set(index, bytes); queueMicrotask(() => callback(null)); }
  get(index, options, callback) {
    if (typeof options === 'function') { callback = options; options = {}; }
    const bytes = this.chunks.get(index);
    queueMicrotask(() => {
      if (!bytes) return callback(new Error('Torrent piece is not downloaded yet.'));
      const start = options.offset || 0;
      callback(null, bytes.subarray(start, options.length === undefined ? undefined : start + options.length));
    });
  }
  close(callback) { queueMicrotask(() => callback?.(null)); }
  destroy(callback) { this.chunks.clear(); this.close(callback); }
}

export async function downloadTorrent(input, { WebTorrent, signal, onProgress = () => {}, zip } = {}) {
  const id = torrentID(input);
  signal?.throwIfAborted();
  if (WebTorrent.WEBRTC_SUPPORT === false) throw new Error('This browser does not support WebRTC torrents. Use a ZIP file instead.');
  const client = new WebTorrent({ dht: false, lsd: false });
  let disposed;
  const dispose = () => disposed ||= new Promise(resolve => client.destroy(() => resolve()));
  let cleanup;
  try {
    const torrent = await new Promise((resolve, reject) => {
      let active;
      const abort = () => reject(signal.reason || new DOMException('Cancelled', 'AbortError'));
      const timeout = setTimeout(() => reject(new Error('No torrent metadata received. Check the link and WebRTC-compatible seeders.')), 90_000);
      const progress = () => onProgress({ progress: active?.progress || 0, downloaded: active?.downloaded || 0, speed: active?.downloadSpeed || 0, peers: active?.numPeers || 0 });
      const timer = setInterval(progress, 500);
      cleanup = () => { clearTimeout(timeout); clearInterval(timer); signal?.removeEventListener('abort', abort); };
      signal?.addEventListener('abort', abort, { once: true });
      client.on('error', reject);
      active = client.add(id, { store: TorrentStore, destroyStoreOnDestroy: true }, torrent => {
        clearTimeout(timeout);
        torrent.on('error', reject);
        torrent.on('noPeers', () => onProgress({ progress: torrent.progress, downloaded: torrent.downloaded, speed: torrent.downloadSpeed, peers: 0, waiting: true }));
        if (torrent.done) resolve(torrent);
        else torrent.once('done', () => resolve(torrent));
      });
    });
    cleanup();
    signal?.throwIfAborted();
    const files = torrent.files.filter(file => !file.path.split('/').some(part => part.startsWith('.') || part === '__MACOSX'));
    if (files.length === 1 && /\.zip$/i.test(files[0].name)) {
      const source = await openZip(new Blob([await files[0].arrayBuffer()]), { zip, signal, name: files[0].name });
      await dispose();
      return source;
    }
    const game = gameFromEntries(files.map(file => ({ path: file.path, read: () => file.arrayBuffer() })), torrent.name);
    await game.validate();
    return { game, async dispose() { game.releaseSource(); await dispose(); } };
  } catch (error) { cleanup?.(); await dispose(); throw error; }
}
