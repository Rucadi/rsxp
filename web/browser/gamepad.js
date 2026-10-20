// SPDX-License-Identifier: GPL-2.0-or-later
// Poll snapshots, rather than connection events: browsers can hide controllers
// until the user presses a button. Aggregate pads before emitting key edges.
export function createGamepadInput({ getGamepads, readMapping, writeKey, connected }) {
  let held = new Set();
  return function poll(active = true) {
    const [zone, encoded] = readMapping().split(';');
    const deadzone = Number(zone);
    const mapping = encoded.split(',').map(Number);
    const buttons = Array(20).fill(false);
    let count = 0;
    for (const pad of getGamepads() || []) {
      if (!pad?.connected || pad.mapping !== 'standard') continue;
      count++;
      if (!active) continue;
      for (let i = 0; i < 16; i++) buttons[i] ||= !!pad.buttons[i]?.pressed;
      const [x = 0, y = 0] = pad.axes;
      buttons[16] ||= x < -deadzone;
      buttons[17] ||= x > deadzone;
      buttons[18] ||= y < -deadzone;
      buttons[19] ||= y > deadzone;
    }
    connected(count);
    const keys = new Set();
    for (let i = 0; i < Math.min(count, 16); i++) keys.add(620 + i);
    buttons.forEach((pressed, i) => {
      if (pressed && mapping[i] > 0) keys.add(599 + mapping[i]);
    });
    for (const key of held) if (!keys.has(key)) writeKey(key, false);
    for (const key of keys) if (!held.has(key)) writeKey(key, true);
    held = keys;
  };
}
