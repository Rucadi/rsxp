// SPDX-License-Identifier: GPL-2.0-or-later
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createLocalGame } from '../browser/local-game.js';
function file(path, content = '') {
  return {
    name: path.split('/').pop(), webkitRelativePath: path,
    async text() { return content; },
    async arrayBuffer() { return new TextEncoder().encode(content).buffer; },
  };
}

test('selected game keeps relative, case-insensitive assets without reading the player', async () => {
  const game = createLocalGame([
    file('My Game/Game.ini', '[Game]\nScripts=Data\\Custom.rxdata\nTitle=My Game'),
    file('My Game/Data/Custom.rxdata', 'scripts'),
    file('My Game/Graphics/Tilesets/Grass.PNG', 'pixels'),
    file('My Game/Fonts/Custom.ttf', 'font'),
    file('My Game/preload.rb', 'ruby'),
    file('My Game/player.html', 'large player'),
    file('My Game/.git/config', 'private'),
  ]);
  await game.validate();
  assert.equal(game.folder, 'My Game');
  assert.equal(game.storageId, 'folder:My Game');
  assert.equal(game.has('graphics\\tilesets\\grass.png'), true);
  assert.equal(new TextDecoder().decode(await game.read('./DATA/Custom.rxdata')), 'scripts');
  assert.deepEqual(game.rubyManifest([], 'Data/Custom.rxdata'), ['Game.ini', 'Data/Custom.rxdata', 'Fonts/Custom.ttf', 'preload.rb']);
  assert.equal(game.paths.length, 5);
  await assert.rejects(game.read('missing.dat'), /not found/);
});

test('folder validation explains missing scripts and accepts encrypted archives', async () => {
  await assert.rejects(createLocalGame([file('Bad/Game.ini')]).validate(), /Cannot find Data\/Scripts.rxdata/);
  const game = createLocalGame([file('Packed/Game.ini'), file('Packed/Game.rgssad')]);
  await game.validate();
  assert.deepEqual(game.archives, ['Game.rgssad']);
  assert.throws(() => createLocalGame([file('Game/Data/a'), file('Game/data/A')]), /Ambiguous/);
  assert.throws(() => createLocalGame([file('One/a'), file('Two/b')]), /single game folder/);
});
