// SPDX-License-Identifier: GPL-2.0-or-later
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createLocalGame } from '../browser/local-game.js';
const source = readFileSync(new URL('../browser/player-launcher.js', import.meta.url), 'utf8').replace(/^import .*;$/gm, '');
for (const embedded of [true, false, 'chunks']) test(`supplied game launcher (${embedded ? 'embedded' : 'served'})`, async () => {
  const elements = new Map();
  function element(id) { if (!elements.has(id)) elements.set(id, { addEventListener(event, fn) { this[event] = fn; }, remove() {}, querySelector: element }); return elements.get(id); }
  const ini = '[Game]\nScripts=Data/Scripts.rxdata';
  const assets = new Map([['Game.ini', ini], ['Data/Scripts.rxdata', 'scripts']]);
  if (embedded === 'chunks') {
    for (const [path, value] of assets) element(path).textContent = Buffer.from(value).toString('base64');
  }
  const loads = []; const globals = {
    RSXP_GAME: { folder: 'Fixture', files: [...assets].map(([path, value]) => [path, embedded === 'chunks' ? { chunks: [path] } : embedded ? Buffer.from(value).toString('base64') : null]) },
    RSXP_BUNDLE: { payload: { engine: 'AA==', ruby: 'AA==', rgss: 'shim', manifest: [] }, decode: text => Uint8Array.from(Buffer.from(text, 'base64')), async load(name) { loads.push(name); } },
  };
  const launch = new Function('createLocalGame', 'document', 'globalThis', 'location', 'fetch', source);
  launch(createLocalGame, { getElementById: element }, globals, { pathname: '/player.html' }, async url => ({ ok: true, arrayBuffer: async () => Uint8Array.from(Buffer.from(assets.get(decodeURIComponent(url.slice(6))))).buffer }));
  await element('choose-game').click();
  // Click listener returns the startup promise.
  assert.deepEqual(loads, ['ruby/browser.umd.js', 'app.js']);
  assert.equal(new TextDecoder().decode(await globals.RSXP_PLAYER.game.read('game.ini')), ini);
  assert.deepEqual(globals.RSXP_PLAYER.game.paths, [...assets.keys()]);
});

function launcherHarness() {
  const elements = new Map();
  function element(id) {
    if (!elements.has(id)) elements.set(id, {
      addEventListener(event, fn) { this[event] = fn; }, remove() {}, querySelector: element,
      classList: { add() {}, remove() {} },
    });
    return elements.get(id);
  }
  const game = { folder: 'ZIP fixture', async validate() {} };
  const globals = { zip: {}, RSXP_BUNDLE: {
    payload: {engine:'AA==',ruby:'AA==',manifest:[],rgss:'shim'},
    decode: text => Uint8Array.from(Buffer.from(text,'base64')),
    async load(name) { return name === 'vendor/webtorrent.js' ? {default:class {}} : {}; },
  }};
  const launch = new Function('createLocalGame','document','globalThis','location','fetch','openZip','downloadTorrent',source);
  let disposed = false;
  launch(createLocalGame,{getElementById:element},globals,{pathname:'/player.html'},undefined,
    async () => ({game,dispose:async()=>{disposed=true;}}),
    async (_url,{signal}) => new Promise((_resolve,reject) => signal.addEventListener('abort',()=>reject(signal.reason))));
  return {element,globals,game,isDisposed:()=>disposed};
}
for (const kind of ['picker','drop']) test(`ZIP ${kind} prepares a game and Start uses the existing player`,async()=>{
  const {element,globals,game,isDisposed}=launcherHarness();
  const file={name:'game.zip'};
  if(kind==='picker') {
    element('game-zip').files=[file];await element('game-zip').change();
  } else {
    let prevented=false;
    await element('drop-zone').drop({preventDefault(){prevented=true;},dataTransfer:{files:[file]}});
    assert.ok(prevented);
  }
  assert.equal(element('choose-game').textContent,'Start game');
  await element('choose-game').click();assert.equal(globals.RSXP_PLAYER.game,game);
  await globals.RSXP_PLAYER.releaseSource();assert.ok(isDisposed());
});
test('download Cancel restores the launcher controls',async()=>{
  const {element}=launcherHarness();
  element('torrent-url').value='https://example.com/game.torrent';
  const pending=element('download-torrent').click();
  await new Promise(resolve=>setImmediate(resolve));
  element('cancel-download').click();await pending;
  assert.equal(element('launch-status').textContent,'Cancelled.');
  assert.equal(element('choose-game').disabled,false);
});
