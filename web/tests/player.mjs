// SPDX-License-Identifier: GPL-2.0-or-later
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

test('Rust-packaged player includes both runtimes and current browser sources', () => {
  const html = readFileSync(new URL('../dist/player.html', import.meta.url), 'utf8');
  const payload = JSON.parse(html.match(/<script id="rsxp-payload" type="application\/json">([\s\S]*?)<\/script>/)[1]);
  for (const key of ['engine', 'ruby']) assert.equal(Buffer.from(payload[key], 'base64').subarray(0, 4).toString('hex'), '0061736d');
  for (const name of ['app.js', 'player-launcher.js', 'local-game.js']) assert.equal(payload.modules[name], readFileSync(new URL(`../browser/${name}`, import.meta.url), 'utf8'));
  assert.equal(payload.rgss, readFileSync(new URL('../../core/src/rgss.rb', import.meta.url), 'utf8'));
  assert.ok(payload.modules['ruby/browser.umd.js'].includes('DefaultRubyVM'));
  assert.equal((html.match(/<script/g) || []).length, 2);
  assert.doesNotMatch(html, /<script\s+src=/);
});
