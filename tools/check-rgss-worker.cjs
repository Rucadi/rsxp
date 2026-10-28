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
  let beats = 0, lastBeat = performance.now(), longestGap = 0, frames = 0, audio = 0;
  const heartbeat = setInterval(() => { const now=performance.now(); longestGap=Math.max(longestGap,now-lastBeat); lastBeat=now; beats++; }, 10);
  const timeout = setTimeout(() => { worker.terminate(); throw new Error('RGSS worker smoke timed out'); }, 90000);
  await new Promise((resolve, reject) => {
    worker.on('error', reject);
    worker.on('message', message => {
      if (message.type === 'error') reject(new Error(message.message));
      if (message.type === 'audio') audio++;
      if (message.type === 'frame') {
        frames++;
        assert(message.pixels instanceof Uint8Array);
        assert.equal(message.pixels.length, message.width * message.height * 4);
        worker.postMessage({type:'frame_ack'});
        if (frames >= 20) resolve();
      }
      if (message.type === 'ready') worker.postMessage({type:'control', paused:false, fast:false, rate:null, events:[], positions:[0,0,0,0], volumes:[100,100,100,100]});
    });
    worker.postMessage({type:'load', module, entry:'data/scripts.rxdata', label:'Pokemon Anil', paths, buffers,
      bindings:url(path.join(dist, bindings)), rubyWasm:url(path.join(dist,'ruby.wasm')),
      rubyRuntime:url(path.join(dist,runtime)),
      rubyLoader:url(require.resolve('@ruby/wasm-wasi/dist/browser'))}, buffers.map(bytes=>bytes.buffer));
  }).finally(async () => {clearInterval(heartbeat);clearTimeout(timeout);await worker.terminate();});
  assert(beats > 20, 'window thread heartbeat did not run');
  assert(longestGap < 250, `window thread blocked for ${longestGap.toFixed(1)} ms`);
  console.log(JSON.stringify({frames, audio_messages:audio, window_heartbeats:beats, longest_window_gap_ms:longestGap},null,2));
})().catch(error => {console.error(error);process.exitCode=1;});
