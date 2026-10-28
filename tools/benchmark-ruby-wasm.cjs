// SPDX-License-Identifier: GPL-2.0-or-later
// Ruby synchronization benchmark; FANPLAY_WEB_ENGINE enables real WASM rendering.
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const { createHash } = require('node:crypto');
const enginePath = process.env.FANPLAY_WEB_ENGINE || path.resolve(__dirname, '../target/ruby-wasm/check-engine/web_check.js');
const engine = require(path.resolve(enginePath));
const { DefaultRubyVM } = require('@ruby/wasm-wasi/dist/browser');
const root = path.resolve(__dirname, '..');
const gameRoot = process.env.FANPLAY_WEB_GAME && path.resolve(process.env.FANPLAY_WEB_GAME);
const assetCache = new Map();
globalThis.__fanplay_check_read = name => {
  if (!gameRoot) return undefined;
  name = String(name).replaceAll('\\', '/');
  if (assetCache.has(name)) return assetCache.get(name);
  let current = gameRoot;
  for (const part of name.split('/').filter(Boolean)) {
    if (part === '..') return undefined;
    try {
      const entry = fs.readdirSync(current).find(entry => entry.toLowerCase() === part.toLowerCase());
      if (!entry) return undefined;
      current = path.join(current, entry);
    } catch { return undefined; }
  }
  try {
    const bytes = fs.readFileSync(current);
    assetCache.set(name, bytes);
    return bytes;
  } catch { return undefined; }
};
async function run(mode, module) {
  if (engine) engine.init();
  const { vm } = await DefaultRubyVM(module, { env: {
    HOME: '/', USER: 'fanplay', LANG: 'en_US.UTF-8',
    RUBY_FIBER_VM_STACK_SIZE: String(64 * 1024 * 1024),
    RUBY_FIBER_MACHINE_STACK_SIZE: String(16 * 1024 * 1024),
    RUBY_THREAD_VM_STACK_SIZE: String(64 * 1024 * 1024),
    RUBY_THREAD_MACHINE_STACK_SIZE: String(16 * 1024 * 1024),
    FANPLAY_NATIVE_STATE: mode === 'native' ? '1' : '0',
    FANPLAY_NATIVE_TILEMAP: mode === 'native' ? '1' : '0',
    FANPLAY_SPRITE_SYNC: mode === 'native' ? 'native' : 'ruby',
  } });
  let calls = 0, requestBytes = 0;
  globalThis.__fanplay_native_b64 = encoded => {
    calls++;
    const bytes = Buffer.from(String(encoded), 'base64');
    requestBytes += bytes.length;
    const result = engine.native_call(bytes);
    return result === undefined ? '__FANPLAY_STOP__' : Buffer.from(result).toString('base64');
  };
  vm.eval('require "/bundle/setup"; require "fanplay_native"');
  // Supports recording the previous table-only interpreter as a baseline.
  vm.eval('def __fanplay_take_render_commands; nil; end unless respond_to?(:__fanplay_take_render_commands, true)');
  const declaration = fs.readFileSync(path.join(root, 'crates/fanplay-rgss/src/ruby.rs'), 'utf8');
  const bundle = declaration.match(/pub const RGSS_WEB_SOURCE: &str = concat!\(([\s\S]*?)\);/)[1];
  let source = [...bundle.matchAll(/include_str!\("([^"]+)"\)/g)].map(m =>
    fs.readFileSync(path.join(root, 'crates/fanplay-rgss/src', m[1]), 'utf8')).join('\n');
  if (gameRoot) {
    assert(engine, 'game replay requires FANPLAY_WEB_ENGINE');
    vm.eval("ENV['FANPLAY_WEB']='1'; ENV['FANPLAY_VERSION']='1'; ENV['FANPLAY_SCRIPTS']='Data/Scripts.rxdata'; ENV['FANPLAY_GAME_TITLE']='Pokemon Anil v4.14'");
    const encoded = Buffer.from(source).toString('base64');
    vm.eval("$replay=Fiber.new { eval('"+encoded+"'.unpack1('m0'),TOPLEVEL_BINDING,'rgss.rb') }");
    const frames = Number(process.env.FANPLAY_WEB_FRAMES || 300);
    for (let i=0; i<frames; i++) {
      if (vm.eval('$replay.alive?').toString() !== 'true') throw new Error('Game stopped before replay finished');
      vm.eval('$replay.resume');
    }
    if (process.env.FANPLAY_WEB_MANIFEST_OUT) fs.writeFileSync(process.env.FANPLAY_WEB_MANIFEST_OUT, JSON.stringify([...assetCache.keys()]));
    const bytes = engine.framebuffer();
    assert(bytes.some(byte => byte !== 0 && byte !== 255), 'game frame is blank');
    return { mode, resumes: frames, framebuffer_sha256: createHash('sha256').update(bytes).digest('hex') };
  }
  source = source.slice(0, source.lastIndexOf('\nbegin\n'));
  vm.eval(source);
  if (process.env.FANPLAY_WEB_FIXTURE) {
    assert(engine, 'fixtures require FANPLAY_WEB_ENGINE');
    console.error('Fixture mode:', mode);
    vm.eval('$check_output=StringIO.new; $stdout=$check_output');
    vm.eval(fs.readFileSync(process.env.FANPLAY_WEB_FIXTURE, 'utf8'));
    const fixture_output = vm.eval('$stdout=STDOUT; $check_output.string').toString();
    return { mode, fixture_output, framebuffer_sha256: createHash('sha256').update(engine.framebuffer()).digest('hex') };
  }
  vm.eval(`
    $bitmap=Bitmap.new(8,8)
    $bitmap.fill_rect($bitmap.rect,Color.new(80,170,230,190))
    $sprites=Array.new(256) do |i|
      s=Sprite.new
      s.bitmap=$bitmap
      s.x=i%160
      s.y=i%120
      s
    end
    10.times { Sprite.__sync_all; FANPLAY.flush_sync; FANPLAY.call('graphics_frame') }
  `);
  const results = [];
  for (const workload of ['static', 'changing', ...(engine ? ['render'] : [])]) {
    const change = workload !== 'static';
    calls = 0; requestBytes = 0;
    vm.eval('GC.start; $alloc=GC.stat(:total_allocated_objects)');
    const started = performance.now();
    vm.eval(`80.times do |frame|
      ${change ? '$sprites.each_with_index { |s,i| s.x=(i+frame)%160 if i%4==0 }' : ''}
      ${workload === 'render' ? 'Graphics.update' : "Sprite.__sync_all; FANPLAY.flush_sync; FANPLAY.call('graphics_frame')"}
    end`);
    results.push({ workload, ms_per_frame: (performance.now()-started)/80,
      allocations_per_frame: Number(vm.eval('GC.stat(:total_allocated_objects)-$alloc').toString())/80,
      bridge_calls: calls, request_bytes: requestBytes });
  }
  return { mode, results, ...(engine ? { framebuffer_sha256: createHash('sha256').update(engine.framebuffer()).digest('hex') } : {}) };
}
(async () => {
  const module = await WebAssembly.compile(fs.readFileSync(process.argv[2] || path.join(root, 'frontends/web/ruby.wasm')));
  const modes = process.argv[3] ? [process.argv[3]] : ['portable', 'native'];
  const results = [];
  for (const mode of modes) results.push(await run(mode, module));
  if (results.length === 2) {
    // Fixtures emit digests of their rendered frames. Some intentionally end
    // with different states to test path-specific error handling.
    if (process.env.FANPLAY_WEB_FIXTURE && results[0].fixture_output) {
      assert.equal(results[0].fixture_output, results[1].fixture_output);
    } else {
      assert.equal(results[0].framebuffer_sha256, results[1].framebuffer_sha256);
    }
  }
  console.log(JSON.stringify(results, null, 2));
})().catch(error => { console.error(error); process.exitCode=1; });
