// SPDX-License-Identifier: GPL-2.0-or-later
// Stop resuming a dead Ruby VM, but keep presenting and processing workspace UI.
export function createGamePump({ isAlive, resume, present, stopped, requestFrame }) {
  let running = true;
  let result;
  return function pump() {
    if (running) {
      try {
        if (isAlive()) result = resume();
        else { running = false; stopped({ result }); }
      } catch (error) {
        running = false;
        stopped({ error });
      }
    }
    present();
    requestFrame(pump);
  };
}

// ruby.wasm captures console callbacks while constructing its WASI printer.
// Restore the page console afterwards; only this VM keeps the log callbacks.
export async function captureRubyOutput(createVM, output) {
  const original = { log: console.log, warn: console.warn };
  for (const level of ['log', 'warn']) {
    console[level] = (...args) => { original[level](...args); output(args.map(String).join(' ')); };
  }
  try { return await createVM(); }
  finally { Object.assign(console, original); }
}
