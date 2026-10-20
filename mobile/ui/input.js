// SPDX-License-Identifier: GPL-2.0-or-later
// Reference counts let two fingers hold the same mapped key without early release.
export function createTouchInput(writeKey) {
  const pointers = new Map();
  const held = new Map();
  function release(pointer) {
    const key = pointers.get(pointer);
    if (key === undefined) return;
    pointers.delete(pointer);
    const count = (held.get(key) || 1) - 1;
    if (count) held.set(key, count);
    else { held.delete(key); writeKey(key, false); }
  }
  return {
    press(pointer, key) {
      release(pointer);
      pointers.set(pointer, key);
      if (!held.has(key)) writeKey(key, true);
      held.set(key, (held.get(key) || 0) + 1);
    },
    release,
    releaseAll() { for (const key of held.keys()) writeKey(key, false); pointers.clear(); held.clear(); },
  };
}
