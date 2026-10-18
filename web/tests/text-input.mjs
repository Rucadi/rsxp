// SPDX-License-Identifier: GPL-2.0-or-later
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import init, * as engine from '../dist/pkg/rsxp_web.js';
import { DefaultRubyVM } from '../node_modules/@ruby/wasm-wasi/dist/esm/browser.js';
import { PRELUDE } from '../browser/ruby-host.js';
import { createGamePump, captureRubyOutput } from '../browser/game-pump.js';
import { createNativeDispatcher } from '../browser/native-bridge.js';

await init({ module_or_path: readFileSync(new URL('../dist/pkg/rsxp_web_bg.wasm', import.meta.url)) });
engine.web_init(64, 48, 1, 40, undefined);
const modes = [];
const dispatch = createNativeDispatcher(engine, enabled => modes.push(enabled));
const encoder = new TextEncoder();
globalThis.RSXP = {
  native_text(request) {
    const response = dispatch(encoder.encode(request));
    return response === undefined ? '__RSXP_STOP__' : `U${new TextDecoder().decode(response)}`;
  },
  native(request) {
    const response = dispatch(Buffer.from(request, 'base64'));
    return response === undefined ? '__RSXP_STOP__' : Buffer.from(response).toString('base64');
  },
};
const output = [];
const { vm } = await captureRubyOutput(async () => DefaultRubyVM(await WebAssembly.compile(
  readFileSync(new URL('../dist/ruby/ruby+stdlib.wasm', import.meta.url)),
), { env: { HOME: '/', LANG: 'en_US.UTF-8' } }), text => output.push(text));
vm.eval(PRELUDE);
const shim = readFileSync(new URL('../../rust/rgss.rb', import.meta.url), 'utf8');
vm.eval(shim.slice(0, shim.lastIndexOf('\nbegin\n')));
// The real Ruby setter chooses native_text for this UTF-8 command.
vm.eval('Input.text_input = true');
assert.deepEqual(modes, [true], 'text bridge must activate the browser text field');
engine.web_text('Lucía');
// DOM events are applied at the next Graphics update before Input.update.
dispatch(encoder.encode('graphics_update\n'));
vm.eval('Input.update');
assert.equal(vm.eval('Input.gets').toString(), 'Lucía');
assert.equal(vm.eval('Input.gets').toString(), '', 'text is consumed once');
vm.eval('Input.text_input = false');
assert.deepEqual(modes, [true, false]);
// Legacy binary bridge calls use exactly the same dispatch and mode callback.
globalThis.RSXP.native(Buffer.from('input_text_mode_set\t1\n').toString('base64'));
globalThis.RSXP.native(Buffer.from('input_text_mode_set\t0\n').toString('base64'));
assert.deepEqual(modes, [true, false, true, false]);
assert.throws(() => dispatch(encoder.encode('input_text_mode_set\tinvalid\n')));
assert.equal(modes.length, 4, 'rejected commands must not change browser focus');
console.log('TEXT_INPUT_BRIDGES_OK');

// A real Ruby error returns status 1 through the RGSS rescue path; the UI must
// continue drawing without touching the dead Fiber again.
vm.eval(`$crash_fiber = Fiber.new do
  Graphics.update
  Fiber.yield
  begin
    raise 'intentional browser crash'
  rescue Exception => error
    STDERR.write(error.full_message(highlight: false))
    1
  end
end`);
const fiber = vm.eval('$crash_fiber');
let draws = 0, next, stopped;
const pump = createGamePump({
  isAlive: () => fiber.call('alive?').toString() === 'true',
  resume: () => fiber.call('resume'),
  present: () => draws += 1,
  stopped: value => stopped = value,
  requestFrame: callback => next = callback,
});
pump();
for (let i = 0; i < 5; i += 1) next();
assert.equal(draws, 6);
assert.equal(stopped.result.toString(), '1');
assert.ok(output.some(text => text.includes('intentional browser crash')));
assert.equal(engine.web_ui_take_reset(), false);
console.log('RUBY_CRASH_UI_PUMP_OK');

assert.equal(engine.web_register_user_patch('web_fixture', '{ apply: -> { $web_patch_active = true }, revert: -> { $web_patch_active = false } }'), true);
engine.web_native_call(encoder.encode('runtime_patch_set\tuser/web_fixture\t1\n'));
vm.eval('Input.update');
assert.equal(vm.eval('$web_patch_active').toString(), 'true');
engine.web_native_call(encoder.encode('runtime_patch_set\tuser/web_fixture\t0\n'));
vm.eval('Input.update');
assert.equal(vm.eval('$web_patch_active').toString(), 'false');
console.log('RUNTIME_PATCH_WASM_OK');
