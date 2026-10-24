import init, * as wasm from '../dist/pkg/rsxp_web.js';
import fs from 'fs';

const bytes = fs.readFileSync(new URL('../dist/pkg/rsxp_web_bg.wasm', import.meta.url));
await init(bytes);

const enc = (s) => new TextEncoder().encode(s + '\n');
const dec = (b) => (b ? new TextDecoder().decode(b) : '<null>');
const call = (s) => dec(wasm.web_native_call(enc(s)));

wasm.web_add_file('hello.txt', new TextEncoder().encode('hello world'));
console.log('file_count', wasm.web_file_count());
wasm.web_init(64, 48, 1, 40, null);

console.log('version =', call('version'));
console.log('graphics_size =', call('graphics_size'));

// Create a bitmap, fill it, then draw it as a sprite and render one frame.
const id = Number(call('bitmap_new\t8\t8'));
wasm.web_native_call(enc(`bitmap_fill\t${id}\t0\t0\t8\t8\t255\t0\t0\t255`));
const sprite = Number(call('sprite_new'));
const sync = [
  'sprite_sync', sprite, id,
  0, 0, 8, 8,          // src rect
  0, 0, 0, 0,          // x, y, ox, oy
  1, 1, 0, 0,          // zoom_x, zoom_y, angle, mirror
  255, 1,              // opacity, shown
  0, 0, 64, 48,        // clip
  0,                   // z
  0,                   // blend_type
  0, 0, 0, 0,          // color
  0, 0, 0, 0,          // tone
  0,                   // invert
  0, 0, 0, 255,        // pattern, pattern_blend, pattern_tile, pattern_opacity
  0, 0,                // pattern scroll
  1, 1,                // pattern zoom
  0,                   // viewport
];
call(sync.join('\t'));
console.log('graphics_update =', call('graphics_update'));

const fb = wasm.web_framebuffer();
console.log('framebuffer', wasm.web_width(), 'x', wasm.web_height(), 'bytes', fb.length);
// Check a pixel inside the sprite (top-left 8x8 should be red, alpha 255).
const idx = (2 * wasm.web_width() + 2) * 4;
console.log('pixel(2,2) =', fb[idx], fb[idx + 1], fb[idx + 2], fb[idx + 3]);
console.log('frame_count =', call('graphics_frame'));
console.log('OK');

// The browser inspector uses the shared game-thread queue without a listener.
const { strict: assert } = await import('node:assert');
assert.equal(call('api_supported'), '1');
// The game-specific Ruby API is now installed by the Rhai extension package.
// The legacy endpoint remains available for compatibility, but intentionally
// has no bundled module to avoid loading the old in-core bridge twice.
assert.equal(call('api_game_bridge'), '');
const requestId = wasm.web_game_request('game_status', []);
assert.equal(wasm.web_game_response(requestId), undefined);
assert.equal(call('api_poll'), `${requestId}\tgame_status\n`);
wasm.web_native_call(new TextEncoder().encode(`api_respond\t${requestId}\n{"map_id":3}`));
assert.deepEqual(JSON.parse(wasm.web_game_response(requestId)), { map_id: 3 });
assert.equal(wasm.web_game_response(requestId), undefined);
assert.throws(() => wasm.web_game_request('unknown_operation', []));
console.log('shared game API queue OK');

assert.equal(wasm.web_register_user_patch('menu_fix', 'fix source'), true);
assert.equal(wasm.web_register_user_patch('../outside', 'invalid'), false);
assert.equal(call('runtime_patch_source\tuser/menu_fix'), 'fix source');
assert.equal(wasm.web_register_user_patch('menu_fix', 'updated source'), true);
assert.equal(call('runtime_patch_source\tuser/menu_fix'), 'updated source');
console.log('virtual game patch catalog OK');
