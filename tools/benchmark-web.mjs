// SPDX-License-Identifier: GPL-2.0-or-later
// Actual ruby.wasm -> JavaScript -> Rust WASM workload without assets or a DOM.
// Run after cargo xtask web: node tools/benchmark-web.mjs [engine-pkg-dir] [rgss.rb]
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { DefaultRubyVM } from '../web/node_modules/@ruby/wasm-wasi/dist/esm/browser.js';

const root = resolve(import.meta.dirname, '..');
const pkg = resolve(process.argv[2] || `${root}/web/dist/pkg`);
const shim = readFileSync(process.argv[3] || `${root}/rust/rgss.rb`, 'utf8');
const engine = await import(pathToFileURL(`${pkg}/rsxp_web.js`));
await engine.default({ module_or_path: readFileSync(`${pkg}/rsxp_web_bg.wasm`) });
engine.web_init(160, 120, 1, 40, undefined);
const encoder = new TextEncoder();
const decoder = new TextDecoder('utf-8', { fatal: true });
let calls = 0;
globalThis.RSXP = {
  native_text(request) {
    calls += 1;
    const result = engine.web_native_call(encoder.encode(request));
    if (result === undefined) return '__RSXP_STOP__';
    if (!result.length) return '';
    try { return `U${decoder.decode(result)}`; }
    catch { return `B${Buffer.from(result).toString('base64')}`; }
  },
};
globalThis.MKXP = globalThis.RSXP;
const { vm } = await DefaultRubyVM(await WebAssembly.compile(
  readFileSync(`${root}/web/dist/ruby/ruby+stdlib.wasm`),
), { env: {
  HOME: '/', LANG: 'en_US.UTF-8',
  ...Object.fromEntries(['RSXP_SPRITE_CACHE']
    .filter(name => process.env[name] !== undefined).map(name => [name, process.env[name]])),
} });
vm.eval(`
require 'js'
def __rsxp_native(request)
  text=JS.global[:RSXP].native_text(request.force_encoding(Encoding::UTF_8)).to_s
  return nil if text=='__RSXP_STOP__'
  return ''.b if text.empty?
  text.getbyte(0)==85 ? text.byteslice(1..-1).b : text.byteslice(1..-1).unpack1('m0')
end
def __mkxp_native(request) = __rsxp_native(request)
`);
// Load only the RGSS API; the final begin/rescue block launches game scripts.
const boundary = shim.lastIndexOf('\nbegin\n');
assert(boundary > 0, 'RGSS game entry block missing');
vm.eval(shim.slice(0, boundary));
if (process.env.RSXP_WEB_FIXTURE) {
  vm.eval(readFileSync(process.env.RSXP_WEB_FIXTURE, 'utf8'));
  console.log('WASM_FIXTURE_OK');
  process.exit(0);
}
vm.eval(`
$bitmap=Bitmap.new(8,8)
$bitmap.fill_rect($bitmap.rect,Color.new(80,170,230,190))
$viewport=Viewport.new(0,0,160,120)
$viewport.tone=Tone.new(10,-5,0,25)
$sprites=Array.new(256) do |i|
  s=Sprite.new($viewport)
  s.bitmap=$bitmap
  s.x=(i*13)%160
  s.y=(i*7)%120
  s
end
10.times { Graphics.update }
`);
const results = [];
for (const mode of ['static-sync', 'changing-sync', 'render']) {
  const frames = 80;
  const change = mode === 'static-sync' ? '' : '$sprites.each_with_index { |s,i| s.x=(i*13+frame)%160 if i%4==0 }';
  const update = mode === 'render' ? 'Graphics.update' : 'Sprite.__sync_all; RSXP.flush_sync';
  calls = 0;
  vm.eval('GC.start; $bench_allocations=GC.stat(:total_allocated_objects)');
  const start = performance.now();
  vm.eval(`${frames}.times do |frame|; ${change}; ${update}; end`);
  const ms = performance.now() - start;
  const allocations = Number(vm.eval('GC.stat(:total_allocated_objects)-$bench_allocations').toString());
  results.push({ mode, frames, ms_per_frame: ms / frames, allocations_per_frame: allocations / frames, bridge_calls: calls });
}
const hash = createHash('sha256').update(engine.web_framebuffer()).digest('hex');
console.log(JSON.stringify({ results, framebuffer_sha256: hash }));
