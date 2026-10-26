// SPDX-License-Identifier: GPL-2.0-or-later
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createLauncher } from '../browser/launcher-controller.js';

const ini = '[Game]\nScripts=Data/Scripts.rxdata';
const entries = () => [
  { name: 'Game.ini', webkitRelativePath: 'Fixture/Game.ini', async text() { return ini; }, async arrayBuffer() { return new TextEncoder().encode(ini).buffer; } },
  { name: 'Scripts.rxdata', webkitRelativePath: 'Fixture/Data/Scripts.rxdata', async text() { return 'scripts'; }, async arrayBuffer() { return new TextEncoder().encode('scripts').buffer; } },
];
function harness({ supplied, launch = async () => {} } = {}) {
  const updates = []; const picks = [];
  return {
    updates, picks,
    launcher: createLauncher({
      bundle: { decode: text => Uint8Array.from(Buffer.from(text, 'base64')), async load() { return {}; } },
      supplied, document: { getElementById() { return { textContent: '' }; } },
      location: { pathname: '/player.html' }, fetch: async () => ({ ok: false }),
      update: state => updates.push(state), launch,
      pickFolder: () => picks.push('folder'), pickZip: () => picks.push('zip'),
      pickRom: () => picks.push('rom'),
    }),
  };
}

test('supplied games start through the shared library action', async () => {
  const supplied = { folder: 'Fixture', files: [['Game.ini', Buffer.from(ini).toString('base64')], ['Data/Scripts.rxdata', Buffer.from('scripts').toString('base64')]] };
  let started;
  const { launcher } = harness({ supplied, launch: async game => { started = game; } });
  await launcher.action({ open: 'Fixture' });
  assert.equal(started.game.folder, 'Fixture');
  assert.equal(new TextDecoder().decode(await started.game.read('Game.ini')), ini);
});

test('folder selection prepares a game and shared Play launches it', async () => {
  let started;
  const { launcher, updates, picks } = harness({ launch: async game => { started = game; } });
  launcher.action({ browse: true });
  assert.deepEqual(picks, ['folder']);
  await launcher.folder(entries());
  assert.equal(updates.at(-1).title, 'Fixture');
  await launcher.action({ open: 'Fixture' });
  assert.equal(started.game.folder, 'Fixture');
});

test('ZIP import intent opens the host picker used by the shared library', () => {
  const { launcher, picks } = harness();
  launcher.action({ zip: true });
  assert.deepEqual(picks, ['zip']);
});

test('cartridge ROM intent opens the ROM picker and launches a cartridge session', async () => {
  let started;
  const { launcher, picks, updates } = harness({ launch: async game => { started = game; } });
  launcher.action({ browse_game_file: true });
  assert.deepEqual(picks, ['rom']);
  const bytes = Uint8Array.from([0, 1, 2, 3]);
  await launcher.rom({
    name: 'Pokemon.gbc',
    size: bytes.length,
    async arrayBuffer() { return bytes.buffer; },
  });
  assert.equal(updates.at(-1).title, 'Pokemon');
  await launcher.action({ open: 'Pokemon' });
  assert.equal(started.game.rom.name, 'Pokemon.gbc');
  assert.deepEqual(await started.game.readRom(), bytes);
});

test('invalid folders leave the shared launcher idle and report the error', async () => {
  const { launcher, updates } = harness();
  await launcher.folder([{ name: 'wrong.txt', webkitRelativePath: 'Fixture/wrong.txt' }]);
  assert.equal(updates.at(-1).busy, false);
  assert.match(updates.at(-1).status, /game|Game.ini/i);
});
