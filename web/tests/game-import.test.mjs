// SPDX-License-Identifier: GPL-2.0-or-later
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { gameFromEntries, openZip, downloadTorrent, torrentID, TorrentStore } from '../browser/game-import.js';
// The browser UMD distribution exposes its API on globalThis.
await import('../browser/vendor/zip.js');
const zip = globalThis.zip;
zip.configure({ useWebWorkers: false });
const encode = text => new TextEncoder().encode(text);
const ini = '[Game]\nScripts=Data/Custom.rxdata';
const entries = () => [
  { path: 'Wrapper/My Game/Game.ini', read: async () => encode(ini).buffer },
  { path: 'Wrapper/My Game/Data/Custom.rxdata', read: async () => encode('scripts').buffer },
  { path: 'Wrapper/My Game/PBS/metadata.txt', read: async () => encode('Pokémon').buffer },
];
test('download wrapper directories resolve to the real game root', async () => {
  const game = gameFromEntries(entries()); await game.validate();
  assert.equal(game.folder, 'My Game');
  assert.equal(new TextDecoder().decode(await game.read('pbs/METADATA.TXT')), 'Pokémon');
  for (const path of ['../outside', '/absolute', 'C:\\outside', 'Game/../outside']) assert.throws(() => gameFromEntries([...entries(), { path }]), /Invalid game path/);
  assert.throws(() => gameFromEntries([...entries(), {path:'Other/Game.ini'}]), /multiple games/);
});
test('real ZIP adapter reads compressed wrapped games lazily and validates corrupt ZIPs', async () => {
  const writer = new zip.ZipWriter(new zip.BlobWriter('application/zip'));
  for (const entry of entries()) await writer.add(entry.path, new zip.Uint8ArrayReader(new Uint8Array(await entry.read())));
  const blob = await writer.close();
  const source = await openZip(blob, {zip, name:'game.zip'});
  assert.equal(new TextDecoder().decode(await source.game.read('Data/Custom.rxdata')), 'scripts');
  await source.dispose();
  await assert.rejects(source.game.read('Game.ini'), /not found/);
  await assert.rejects(openZip(new Blob(['broken']), {zip}));
});
test('magnet and HTTP torrent identifiers accept only browser-safe inputs', () => {
  assert.equal(torrentID(' https://example.com/game.torrent '), 'https://example.com/game.torrent');
  assert.match(torrentID('magnet:?xt=urn:btih:0123456789012345678901234567890123456789'), /^magnet:/);
  for (const value of ['javascript:alert(1)', 'file:///tmp/game.torrent', 'magnet:?dn=nohash']) assert.throws(() => torrentID(value));
});
class FakeWebTorrent extends EventEmitter {
  static clients = [];
  static WEBRTC_SUPPORT = true;
  constructor() { super(); FakeWebTorrent.clients.push(this); }
  add(id, options, callback) {
    this.torrent = Object.assign(new EventEmitter(), {
      name:'My Game', progress:0, downloaded:0, downloadSpeed:0, numPeers:1, done:false,
      files: entries().map(entry => ({path:entry.path, name:entry.path.split('/').pop(), arrayBuffer:entry.read})),
    });
    queueMicrotask(() => {
      callback(this.torrent);
      if (!this.stall) { this.torrent.done=true; this.torrent.emit('done'); }
    });
    return this.torrent;
  }
  destroy(callback) { this.destroyed = true; callback(); }
}
test('torrent downloads become the same game adapter and release the client after preload', async () => {
  const source = await downloadTorrent('https://example.com/game.torrent', {WebTorrent:FakeWebTorrent,zip});
  await source.game.validate();
  assert.equal(new TextDecoder().decode(await source.game.read('Data/Custom.rxdata')), 'scripts');
  const client=FakeWebTorrent.clients.at(-1);
  assert.ok(!client.destroyed);
  await source.dispose(); assert.ok(client.destroyed);
});
test('torrent cancel rejects and tears down peers; unsupported browsers give a useful error', async () => {
  class Stalled extends FakeWebTorrent { constructor() {super();this.stall=true;} }
  const controller=new AbortController();
  const loading=downloadTorrent('https://example.com/game.torrent',{WebTorrent:Stalled,signal:controller.signal});
  controller.abort();
  await assert.rejects(loading,{name:'AbortError'});
  assert.ok(FakeWebTorrent.clients.at(-1).destroyed);
  await assert.rejects(downloadTorrent('https://example.com/game.torrent',{WebTorrent:{WEBRTC_SUPPORT:false}}), /WebRTC/);
});
test('session torrent store supports partial reads and frees pieces', async () => {
  const store=new TorrentStore();
  await new Promise(resolve => store.put(0,encode('abcdef'),resolve));
  const bytes=await new Promise((resolve,reject)=>store.get(0,{offset:2,length:3},(error,value)=>error?reject(error):resolve(value)));
  assert.equal(new TextDecoder().decode(bytes),'cde');
  await new Promise(resolve=>store.destroy(resolve));assert.equal(store.chunks.size,0);
});
test('vendored WebTorrent module imports in the browser global environment', async () => {
  const before=globalThis.self; globalThis.self=globalThis;
  try {const {default:WebTorrent}=await import('../browser/vendor/webtorrent.js'); assert.equal(typeof WebTorrent,'function');}
  finally {if(before===undefined)delete globalThis.self;else globalThis.self=before;}
});
