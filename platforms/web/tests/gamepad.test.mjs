import test from 'node:test';
import assert from 'node:assert/strict';
import { createGamepadInput } from '../browser/gamepad.js';

function fixture() {
  let pads = [];
  let mapping = [7,6,8,9,11,12,5,10,6,7,0,0,4,1,2,3,2,3,4,1];
  const events = [];
  let count;
  const poll = createGamepadInput({
    getGamepads: () => pads,
    readMapping: () => `0.25;${mapping.join(',')}`,
    writeKey: (key, down) => events.push([key, down]),
    connected: value => { count = value; },
  });
  return { poll, events, mapping, pads: value => { pads=value; }, count: () => count };
}
const pad = (buttons = [], axes = [0,0]) => ({ connected:true, mapping:'standard', buttons: Array.from({length:16}, (_,i)=>({pressed:buttons.includes(i)})), axes });

test('buttons aggregate across pads and release on last disconnect', () => {
  const f=fixture(); f.pads([pad([0]), pad([0])]); f.poll();
  assert.equal(f.count(),2);
  assert.ok(f.events.some(([key,down])=>key===606 && down));
  f.events.length=0; f.pads([pad([0])]); f.poll();
  assert.deepEqual(f.events,[[621,false]]);
  f.events.length=0; f.pads([]); f.poll();
  assert.ok(f.events.some(([key,down])=>key===606 && !down));
});

test('stick deadzone, remapping while held, and focus loss', () => {
  const f=fixture(); f.pads([pad([], [0.2,0])]); f.poll();
  assert.deepEqual(f.events,[[620,true]]);
  f.events.length=0; f.pads([pad([], [0.6,-0.7])]); f.poll();
  assert.deepEqual(f.events,[[602,true],[603,true]]);
  f.events.length=0; f.mapping[17]=6; f.poll();
  assert.deepEqual(f.events,[[602,false],[605,true]]);
  f.events.length=0; f.poll(false);
  assert.deepEqual(f.events,[[605,false],[603,false]]);
  f.events.length=0; f.poll();
  assert.deepEqual(f.events,[[605,true],[603,true]]);
});

test('unmapped and missing devices remain neutral', () => {
  const f=fixture(); f.pads([null,{...pad([0]),mapping:''}]); f.poll();
  assert.equal(f.count(),0); assert.deepEqual(f.events,[]);
  f.mapping[0]=0; f.pads([pad([0])]); f.poll();
  assert.deepEqual(f.events,[[620,true]]);
});
