// SPDX-License-Identifier: GPL-2.0-or-later
// Headless end-to-end check of the browser pipeline: engine wasm + ruby.wasm +
// the real game scripts, driven by the same Fiber frame pump the page uses.
// Run from the repository: node platforms/web/tests/game-e2e.mjs
import init, * as engine from '../dist/pkg/rsxp_web.js';
import { PRELUDE } from '../browser/ruby-host.js';
import { DefaultRubyVM } from '../node_modules/@ruby/wasm-wasi/dist/esm/browser.js';
import { readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { rubySource } from '../../tools/ruby-sources.mjs';

const gameRoot = resolve(import.meta.dirname, '..', '..', 'ericlostie');
const manifest = JSON.parse(readFileSync(resolve(import.meta.dirname, '../browser/assets.json'), 'utf8'));

await init(readFileSync(resolve(import.meta.dirname, '../dist/pkg/rsxp_web_bg.wasm')));

let loaded = 0;
for (const path of manifest) {
  try {
    engine.web_add_file(path, readFileSync(resolve(gameRoot, path)));
    loaded += 1;
  } catch {
    /* missing */
  }
}
console.log(`loaded ${loaded}/${manifest.length} assets`);

const ini = new TextDecoder().decode(engine.web_read_file('Game.ini'));
let scriptPath = 'Data/Scripts.rxdata';
let title = 'rsxp';
for (const line of ini.split(/\r?\n/)) {
  const m = line.match(/^\s*([^=]+)=(.*)$/);
  if (!m) continue;
  if (m[1].trim().toLowerCase() === 'scripts') scriptPath = m[2].trim().replace(/\\/g, '/');
  if (m[1].trim().toLowerCase() === 'title') title = m[2].trim();
}
let version = 1;
if (scriptPath.endsWith('.rvdata2')) version = 3;
else if (scriptPath.endsWith('.rvdata')) version = 2;
const [width, height] = version === 1 ? [640, 480] : [544, 416];
engine.web_init(width, height, version, version === 1 ? 40 : 60, null);

globalThis.RSXP = {
  source: rubySource('RGSS_SOURCE'),
  manifestText: manifest.join('\n'),
  // Saves are kept in memory for the run (the page uses localStorage instead).
  savedSavesJson: '{}',
  persistSave() {},
  startup_progress() {},
  game_started() {},
  native(requestBase64) {
    const request = Buffer.from(requestBase64, 'base64');
    const response = engine.web_native_call(request);
    if (response === undefined) return '__RSXP_STOP__';
    if (response.length === 0) return '';
    return Buffer.from(response).toString('base64');
  },
  native_text(requestText) {
    const response = engine.web_native_call(new TextEncoder().encode(requestText));
    if (response === undefined) return '__RSXP_STOP__';
    if (response.length === 0) return '';
    try {
      return `U${new TextDecoder('utf-8', { fatal: true }).decode(response)}`;
    } catch {
      return `B${Buffer.from(response).toString('base64')}`;
    }
  },
  readFile(path) {
    try {
      return new Uint8Array(readFileSync(resolve(gameRoot, path)));
    } catch {
      return null;
    }
  },
};
globalThis.MKXP = globalThis.RSXP;
engine.web_set_reader(globalThis.RSXP.readFile);



const rubyWasm = readFileSync(
  resolve(import.meta.dirname, '../dist/ruby/ruby+stdlib.wasm'),
);
const { vm } = await DefaultRubyVM(await WebAssembly.compile(rubyWasm), {
  env: {
    RSXP_SCRIPTS: scriptPath,
    RSXP_GAME_TITLE: title,
    HOME: '/',
    USER: 'player',
    LANG: 'en_US.UTF-8',
    // The game runs inside a Fiber; give it a large stack so long play
    // sessions with deeply nested scripts do not raise SystemStackError.
    // (128 MiB VM / 64 MiB machine trap ruby.wasm's linear memory.)
    RUBY_FIBER_VM_STACK_SIZE: '67108864',
    RUBY_FIBER_MACHINE_STACK_SIZE: '16777216',
    RUBY_THREAD_VM_STACK_SIZE: '67108864',
    RUBY_THREAD_MACHINE_STACK_SIZE: '16777216',
  },
});

vm.eval(PRELUDE);
const binaryReader = vm.eval(`
  io = StringIO.new([0x78, 0x56, 0x34, 0x12, 0xCD, 0xAB, 0x89].pack('C*'))
  [io.fgetb, io.fgetw, io.fgetdw].inspect
`).toString();
if (binaryReader !== '[120, 13398, 2309737746]') {
  throw new Error(`StringIO RGSS binary reader mismatch: ${binaryReader}`);
}
vm.eval('$rsxp_source = JS.global[:RSXP][:source].to_s');
vm.eval('$rsxp_fiber = Fiber.new { eval($rsxp_source, TOPLEVEL_BINDING, "rgss.rb", 1) }');
const gameFiber = vm.eval('$rsxp_fiber');

// Simulate scripted input: press Return every 40 frames to advance the game.
let frames = 0;
const limit = Number(process.argv[2] || 400);
while (frames < limit) {
  if (gameFiber.call('alive?').toString() !== 'true') break;
  if (frames % 40 === 0) engine.web_key(40, true);
  else if (frames % 40 === 3) engine.web_key(40, false);
  try {
    const result = gameFiber.call('resume');
    if (gameFiber.call('alive?').toString() !== 'true' && result.toString() !== '0') {
      throw new Error(`Ruby game exited with status ${result.toString()}; see its backtrace above`);
    }
  } catch (error) {
    throw new Error(`Browser game failed after ${frames} Fiber resumes`, { cause: error });
  }
  frames += 1;
}

const fb = engine.web_framebuffer();
const fw = engine.web_width();
const fh = engine.web_height();
const rgb = Buffer.alloc(fw * fh * 3);
for (let i = 0; i < fw * fh; i += 1) {
  rgb[i * 3] = fb[i * 4];
  rgb[i * 3 + 1] = fb[i * 4 + 1];
  rgb[i * 3 + 2] = fb[i * 4 + 2];
}
writeFileSync(
  process.argv[3] || '/tmp/opencode/web-frame.ppm',
  Buffer.concat([Buffer.from(`P6\n${fw} ${fh}\n255\n`), rgb]),
);
let nonBlack = 0;
for (let i = 0; i < fb.length; i += 4) {
  if (fb[i] || fb[i + 1] || fb[i + 2]) nonBlack += 1;
}
console.log(
  `frames=${frames} frame_count=${new TextDecoder().decode(engine.web_native_call(new TextEncoder().encode('graphics_frame\n')))} ` +
    `size=${engine.web_width()}x${engine.web_height()} nonblack=${nonBlack}`,
);
console.log(nonBlack > 0 ? 'E2E_OK' : 'E2E_BLANK');

// Exercise the same Ruby adapter used by desktop tools against real game data.
// Poll explicitly while the game Fiber is paused so this also works at a title.
const { strict: assert } = await import('node:assert');
engine.web_add_file('PBS/rsxp-virtual-check.txt', new TextEncoder().encode('Pueblo español\nOutdoor=true\n'));
vm.eval(`
path = File.expand_path('PBS/rsxp-virtual-check.txt')
raise 'fixture unexpectedly mirrored into Ruby' if File.__rsxp_real_file?(path)
raise 'virtual File.file? failed' unless File.file?(path) && FileTest.file?(path)
raise 'directory treated as file' if File.file?('PBS')
lines = File.foreach(path, encoding: 'UTF-8').to_a
raise 'virtual foreach encoding/lines failed' unless lines == ["Pueblo español\\n", "Outdoor=true\\n"]
seen = []
result = File.foreach(path, encoding: 'UTF-8') { |line| seen << line }
raise 'virtual foreach block semantics failed' unless result.nil? && seen == lines
`);
for (const [operation, args] of [['game_status', []], ['map_preview', ['3']], ['world_map', []]]) {
  const id = engine.web_game_request(operation, args);
  vm.eval('RSXP::GameAPI.poll');
  const reply = JSON.parse(engine.web_game_response(id));
  assert.equal(reply.error, undefined, JSON.stringify(reply));
  if (operation === 'map_preview') {
    assert.equal(reply.map_id, 3);
    assert.ok(reply.width > 0 && reply.height > 0);
    assert.equal(reply.layers.length, 3);
    assert.equal(reply.layers[0].length, reply.height);
    assert.equal(reply.layers[0][0].length, reply.width);
  } else if (operation === 'world_map') {
    assert.ok(reply.maps.some(map => map.id === 3), 'World map omitted a map outside the warm manifest');
    assert.ok(reply.maps.length > 4, `World map is missing maps: ${reply.maps.length}`);
    assert.ok(reply.maps.some(map => map.outdoor), 'World map has no outdoor maps');
    console.log(`WORLD_MAP_OK maps=${reply.maps.length} outdoors=${reply.maps.filter(map => map.outdoor).length}`);
  } else {
    assert.equal(reply.resolution.width, engine.web_width());
  }
}
console.log('E2E_SHARED_INSPECTOR_OK');

assert.equal(vm.eval('Kernel.respond_to?(:pbShowCommands)').toString(), 'true', 'game command helper must be public on Kernel');
// Reproduce the reported menu failure with the real game's helper. Nil
// commands return immediately, so this checks dispatch without opening a menu.
vm.eval(`begin
  Kernel.singleton_class.send(:private, :pbShowCommands)
  raise 'private command helper failed' unless Kernel.pbShowCommands(nil, nil) == 0
ensure
  Kernel.singleton_class.send(:public, :pbShowCommands)
end`);
console.log('E2E_KERNEL_COMMAND_HELPER_OK');
