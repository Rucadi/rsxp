// SPDX-License-Identifier: GPL-2.0-or-later
// Exercise the production browser worker in a Node worker with browser WASI.
// The window-thread heartbeat continues while the worker loads and runs Ruby.
const fs = require('node:fs');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const { Worker } = require('node:worker_threads');
const assert = require('node:assert/strict');
const root = path.resolve(__dirname, '..');
const dist = path.join(root, 'target/trunk-dist');
const url = p => pathToFileURL(p).href;
const game = path.resolve(process.argv[2] || path.join(root, 'ericlostie'));
(async () => {
  const html = fs.readFileSync(path.join(dist, 'index.html'), 'utf8');
  const bindings = html.match(/import init, \* as bindings from '([^']+)'/)[1].split('/').pop();
  const wasm = bindings.replace(/\.js$/, '_bg.wasm');
  const runtime = html.match(/src="([^"]*ruby_runtime[^"]*\.js)"/)[1].split('/').pop();
  const manifest = new Set(JSON.parse(fs.readFileSync(path.join(root, 'target/ruby-wasm/anil-manifest.json'))));
  for (const name of ['Game.ini', 'mkxp.json', 'preload.rb']) manifest.add(name);
  for (const directory of ['Graphics/Titles', 'Audio/BGM']) {
    for (const name of fs.readdirSync(path.join(game, directory))) {
      if (fs.statSync(path.join(game, directory, name)).isFile()) manifest.add(`${directory}/${name}`);
    }
  }
  const paths = [], buffers = [];
  for (const name of manifest) {
    paths.push(name.replaceAll('\\', '/').toLowerCase());
    buffers.push(new Uint8Array(fs.readFileSync(path.join(game, name))));
  }
  const module = await WebAssembly.compile(fs.readFileSync(path.join(dist, wasm)));
  const gameApiRuby = fs.readFileSync(path.join(root, 'extensions/essentials-game-api/game_api.rb'), 'utf8');
  const gameApiPatch = `{ apply: proc { ${gameApiRuby} }, revert: proc { FANPLAY.send(:remove_const, :GameAPI) if defined?(FANPLAY::GameAPI) } }`;
  const pokedexSource = fs.readFileSync(path.join(root, 'extensions/essentials-pokedex.rhai'), 'utf8');
  const pokedexRuby = pokedexSource.match(/\/\* fanplay:text ruby-begin\n([\s\S]*?)\nfanplay:text ruby-end \*\//)[1];
  const pokedexPatch = `{ apply: proc { ${pokedexRuby} }, revert: proc { FANPLAY::PokedexPlugin.uninstall if defined?(FANPLAY::PokedexPlugin) } }`;
  const bootstrap = `
    import { parentPort } from 'node:worker_threads';
    import fs from 'node:fs/promises';
    const nativeFetch = globalThis.fetch;
    globalThis.fetch = async (url, options) => String(url).startsWith('file:')
      ? new Response(await fs.readFile(new URL(url)), {headers:{'Content-Type':'application/wasm'}})
      : nativeFetch(url, options);
    globalThis.postMessage = (message, transfer) => parentPort.postMessage(message, transfer);
    globalThis.close = () => parentPort.close();
    globalThis.onmessage = null;
    await import(${JSON.stringify(url(path.join(dist, 'rgss_worker.js')))});
    parentPort.on('message', data => globalThis.onmessage({data}));
  `;
  const worker = new Worker(new URL(`data:text/javascript,${encodeURIComponent(bootstrap)}`));
  const webviewRequestId = 2 ** 40;
  let pokedexResponse = false, highIdResponse = false;
  let beats = 0, lastBeat = performance.now(), longestGap = 0, frames = 0, audio = 0, pluginResponses = 0, pluginErrors = [];
  const heartbeat = setInterval(() => { const now=performance.now(); longestGap=Math.max(longestGap,now-lastBeat); lastBeat=now; beats++; }, 10);
  const timeout = setTimeout(() => { worker.terminate(); throw new Error('RGSS worker smoke timed out'); }, 90000);
  await new Promise((resolve, reject) => {
    worker.on('error', reject);
    worker.on('message', message => {
      if (message.type === 'error') reject(new Error(message.message));
      if (message.type === 'audio') audio++;
      if (message.type === 'plugin_response') {
        pluginResponses++;
        const response = JSON.parse(message.response);
        if (response.error) pluginErrors.push(response.error.message);
        if (response.request_id === 2 && !response.error) {
          worker.postMessage({type:'plugin_request', request_id:3, generation:0,
            operation:'essentials-pokedex.__ruby.inject',
            arguments:{name:'worker-pokedex', source:pokedexPatch}});
        }
        if (response.request_id === 3 && !response.error) {
          worker.postMessage({type:'plugin_request', request_id:4, generation:0,
            operation:'essentials-pokedex.pokemon_essentials_pokedex', arguments:[]});
        }
        if (response.request_id === 4 && !response.error) {
          pokedexResponse = true;
          // Webview requests use a high but Number-safe ID. If this comes back
          // unchanged, the JSON/Number/BigInt boundary preserved the key that
          // the window-side webview bridge uses to resolve its Promise.
          worker.postMessage({type:'plugin_request', request_id:webviewRequestId, generation:0,
            operation:'essentials-pokedex.pokemon_essentials_pokedex', arguments:[]});
        }
        if (response.request_id === webviewRequestId) {
          highIdResponse = true;
          if (frames >= 20) resolve();
        }
      }
      if (message.type === 'frame') {
        frames++;
        assert(message.pixels instanceof Uint8Array);
        assert.equal(message.pixels.length, message.width * message.height * 4);
        worker.postMessage({type:'frame_ack'});
        if (frames >= 20 && highIdResponse) resolve();
      }
      if (message.type === 'ready') {
        worker.postMessage({type:'control', paused:false, fast:false, rate:null, events:[], positions:[0,0,0,0], volumes:[100,100,100,100]});
        // Exercise the u64 -> BigInt wasm-bindgen boundary used by dashboard
        // plugin activation. Number arguments throw before Ruby sees a request.
        worker.postMessage({type:'plugin_request', request_id:2, generation:0,
          operation:'essentials-game-api.__ruby.inject',
          arguments:{name:'worker-game-api', source:gameApiPatch}});
      }
    });
    worker.postMessage({type:'load', module, entry:'data/scripts.rxdata', label:'Pokemon Anil', paths, buffers,
      bindings:url(path.join(dist, bindings)), rubyWasm:url(path.join(dist,'ruby.wasm')),
      rubyRuntime:url(path.join(dist,runtime)),
      rubyLoader:url(require.resolve('@ruby/wasm-wasi/dist/browser'))}, buffers.map(bytes=>bytes.buffer));
  }).finally(async () => {clearInterval(heartbeat);clearTimeout(timeout);await worker.terminate();});
  assert(beats > 20, 'window thread heartbeat did not run');
  assert(longestGap < 250, `window thread blocked for ${longestGap.toFixed(1)} ms`);
  assert(pluginResponses > 0, 'plugin request did not receive a response');
  assert(pokedexResponse, 'Pokedex plugin request did not receive a successful response');
  assert(highIdResponse, 'Number-safe webview request ID did not round-trip');
  assert.deepEqual(pluginErrors, [], `plugin request failed: ${pluginErrors.join('; ')}`);
  console.log(JSON.stringify({frames, audio_messages:audio, plugin_responses:pluginResponses, window_heartbeats:beats, longest_window_gap_ms:longestGap},null,2));
})().catch(error => {console.error(error);process.exitCode=1;});
