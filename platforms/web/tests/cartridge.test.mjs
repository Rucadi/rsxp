// SPDX-License-Identifier: GPL-2.0-or-later
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import init, * as wasm from '../dist/pkg/rsxp_web.js';

const bytes = readFileSync(new URL('../dist/pkg/rsxp_web_bg.wasm', import.meta.url));
await init(bytes);

// Same tiny mode-3 ROM the native GBA adapter test uses. It has no commercial
// content and exercises the real cartridge loader, frame loop and CPU.
function gbaRom() {
  const program = [
    0xE3A00003, 0xE3800B01, 0xE3A01301, 0xE5810000,
    0xE3A02406, 0xE3A03402, 0xE5934000, 0xE2844001,
    0xE5834000, 0xE1C240B0, 0xEAFFFFFA,
  ];
  const rom = new Uint8Array(0x200);
  const view = new DataView(rom.buffer);
  program.forEach((value, index) => view.setUint32(index * 4, value, true));
  rom.fill(0x41, 0xa0, 0xac);
  return rom;
}

// A minimal homebrew DS image with valid ARM9/ARM7 headers and infinite loops.
function ndsRom() {
  const rom = new Uint8Array(0x8000);
  const view = new DataView(rom.buffer);
  rom.set([0x52, 0x53, 0x58, 0x50], 0);
  for (const [offset, entry, size] of [[0x20, 0x02000000, 0x200], [0x30, 0x02380000, 0x210]]) {
    view.setUint32(offset, size, true);
    view.setUint32(offset + 4, entry, true);
    view.setUint32(offset + 8, entry, true);
    view.setUint32(offset + 12, 4, true);
  }
  view.setUint32(0x200, 0xEAFFFFFE, true);
  view.setUint32(0x210, 0xEAFFFFFE, true);
  return rom;
}

test('cartridge loader accepts GBA and advances frames through input', () => {
  assert.equal(wasm.web_core_init('fixture.gba', gbaRom()), 'gba');
  for (let frame = 0; frame < 3; frame += 1) assert.doesNotThrow(() => wasm.web_core_step());
  assert.deepEqual(Array.from(wasm.web_core_size()), [240, 160]);
  const messages = JSON.parse(wasm.web_core_take_messages());
  assert.ok(messages.some(message => /240x160/.test(message)));
  wasm.web_core_key(82, true);
  wasm.web_core_key(82, false);
  assert.doesNotThrow(() => wasm.web_core_step());
  wasm.web_core_stop();
});

test('cartridge loader accepts NDS and presents both screens', () => {
  assert.equal(wasm.web_core_init('fixture.nds', ndsRom()), 'nds');
  for (let frame = 0; frame < 2; frame += 1) assert.doesNotThrow(() => wasm.web_core_step());
  assert.deepEqual(Array.from(wasm.web_core_size()), [256, 384]);
  assert.doesNotThrow(() => wasm.web_core_pointer(128, 300, true));
  assert.doesNotThrow(() => wasm.web_core_pointer_button(true));
  assert.doesNotThrow(() => wasm.web_core_pointer_button(false));
  wasm.web_core_stop();
});

test('unsupported or empty cartridge files fail with a useful message', () => {
  assert.throws(() => wasm.web_core_init('fixture.zip', new Uint8Array([1, 2, 3])), /\.gba|\.nds|Unsupported/);
});
