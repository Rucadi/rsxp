// SPDX-License-Identifier: GPL-2.0-or-later
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createGamePump, captureRubyOutput } from '../browser/game-pump.js';

for (const mode of ['exception', 'exit']) {
  test(`workspace keeps ticking after Ruby ${mode}`, () => {
    let resumes = 0, presentations = 0, checks = 0, next;
    const stops = [];
    const pump = createGamePump({
      isAlive: () => { checks += 1; return resumes < 2; },
      resume: () => {
        resumes += 1;
        if (mode === 'exception' && resumes === 2) throw new Error('Ruby crashed');
        return 1;
      },
      present: () => presentations += 1,
      stopped: reason => stops.push(reason),
      requestFrame: callback => next = callback,
    });
    pump();
    for (let frame = 0; frame < 5; frame += 1) next();
    assert.equal(resumes, 2);
    assert.equal(presentations, 6);
    assert.equal(stops.length, 1);
    assert.equal(checks, mode === 'exception' ? 2 : 3);
    if (mode === 'exception') assert.match(stops[0].error.message, /Ruby crashed/);
    else assert.equal(stops[0].result, 1);
  });
}

test('VM output capture restores the page console even when initialization fails', async () => {
  const original = console.warn;
  let printer;
  const captured = [];
  await assert.rejects(captureRubyOutput(async () => {
    printer = console.warn;
    throw new Error('VM setup failed');
  }, text => captured.push(text)), /VM setup failed/);
  assert.equal(console.warn, original);
  printer('ruby.rb:7: crash');
  assert.deepEqual(captured, ['ruby.rb:7: crash']);
});
