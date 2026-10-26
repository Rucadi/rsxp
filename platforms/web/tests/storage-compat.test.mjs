import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readStored } from '../browser/storage-compat.js';
function storage(entries) {
  const values = new Map(entries);
  return { getItem: key => values.get(key) ?? null, setItem: (key, value) => values.set(key, value) };
}
test('migrates saves, backups, settings and controls without deleting old data', () => {
  for (const key of ['saves:game', 'backups:game', 'settings', '.controls']) {
    const suffix = key.startsWith('.') ? key : `-${key}`;
    const db = storage([[`rsxp${suffix}`, '{"save":"kept"}']]);
    assert.equal(readStored(db, `fanplay${suffix}`), '{"save":"kept"}');
    assert.equal(db.getItem(`fanplay${suffix}`), db.getItem(`rsxp${suffix}`));
  }
});
test('new values win, including empty strings', () => {
  const db = storage([['fanplay-settings', ''], ['rsxp-settings', 'old']]);
  assert.equal(readStored(db, 'fanplay-settings'), '');
  assert.equal(readStored(db, 'other'), null);
});
test('old saves remain readable when storage quota blocks copying', () => {
  const db = storage([['rsxp-saves:game', 'kept']]);
  db.setItem = () => { throw new Error('quota'); };
  assert.equal(readStored(db, 'fanplay-saves:game'), 'kept');
});
