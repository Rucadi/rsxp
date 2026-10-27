// Run with Node and @ruby/wasm-wasi@2.10.1 plus
// @ruby/4.0-wasm-wasi@2.10.1 installed (or available through NODE_PATH).
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { test } = require('node:test');
const { runInNewContext } = require('node:vm');
const { DefaultRubyVM } = require('@ruby/wasm-wasi/dist/browser');

async function runtime() {
  const bytes = fs.readFileSync(path.join(
    path.dirname(require.resolve('@ruby/4.0-wasm-wasi')),
    '../ruby+stdlib.wasm',
  ));
  const context = {
    'ruby-wasm-wasi': {
      DefaultRubyVM: module => DefaultRubyVM(module, { consolePrint: false }),
    },
    fetch: async () => new Response(bytes, {
      headers: { 'Content-Type': 'application/wasm' },
    }),
    WebAssembly,
    Error,
  };
  runInNewContext(fs.readFileSync(path.join(__dirname, '../ruby_runtime.js'), 'utf8'), context);
  await context.__fanplayRubyReady;
  return context;
}

function start(host, source) {
  return host.fanplay_ruby_start(source, 3, 'Data/Scripts.rvdata2', 'Test');
}

test('completion and yielding never resume a dead fiber', async () => {
  const host = await runtime();
  start(host, 'nil');
  assert.equal(host.fanplay_ruby_step(), false);
  assert.equal(host.fanplay_ruby_step(), false);
  start(host, 'Fiber.yield; Fiber.yield; nil');
  assert.equal(host.fanplay_ruby_step(), true);
  assert.equal(host.fanplay_ruby_step(), false);
  assert.equal(host.fanplay_ruby_step(), false);
});

for (const [name, source, pattern] of [
  ['WASI exit during a step', 'Fiber.yield; exit!', /exit.*code 1/],
  ['WASI exit during startup', 'exit!', /exit.*code 1/],
  ['Ruby exception', 'Fiber.yield; raise "plugin failure"', /plugin failure/],
]) {
  test(name + ' preserves the first error and prevents VM re-entry', async () => {
    const host = await runtime();
    let original;
    try {
      start(host, source);
      host.fanplay_ruby_step();
    } catch (error) { original = error; }
    assert.ok(original);
    assert.match(original.message, pattern);
    for (const action of [
      () => host.fanplay_ruby_step(),
      () => host.fanplay_ruby_step(),
      () => host.fanplay_ruby_eval('1'),
      () => start(host, 'nil'),
    ]) {
      assert.throws(action, error => error === original);
    }
  });
}
