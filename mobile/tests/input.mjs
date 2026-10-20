import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createTouchInput } from '../ui/input.js';
test('multitouch supports diagonals and action buttons; cancellation releases all keys', () => {
  const events = []; const input = createTouchInput((...event) => events.push(event));
  input.press(1,82); input.press(2,79); input.press(3,29); input.release(2); input.releaseAll();
  assert.deepEqual(events,[[82,true],[79,true],[29,true],[79,false],[82,false],[29,false]]);
});
test('two pointers mapped to the same key do not release one another', () => {
  const events=[]; const input=createTouchInput((...event)=>events.push(event));
  input.press(1,27); input.press(2,27); input.release(1); assert.deepEqual(events,[[27,true]]);
  input.release(2); input.release(2); assert.deepEqual(events,[[27,true],[27,false]]);
});
