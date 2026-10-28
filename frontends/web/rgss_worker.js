/* RGSS owns its Ruby VM and engine in this worker. UI and audio stay on the
 * window thread. The frame channel allows one unacknowledged frame and keeps
 * only the newest pending frame, so a busy UI cannot build a video backlog. */
let engine;
let stopped = false;
let paused = false;
let fast = false;
let deadline = 0;
let inFlight = false;
let pendingFrame = null;
let timer;

function sendFrame(frame) {
  inFlight = true;
  postMessage(frame, [frame.pixels.buffer]);
}
globalThis.__fanplay_worker_output = (message) => {
  if (message.type === "frame") {
    if (inFlight) pendingFrame = message;
    else sendFrame(message);
  } else if (message.data instanceof Uint8Array) {
    postMessage(message, [message.data.buffer]);
  } else {
    postMessage(message);
  }
};
function fail(error) {
  stopped = true;
  clearTimeout(timer);
  postMessage({ type: "error", message: String(error?.stack || error) });
}
function pump() {
  if (stopped) return;
  try {
    if (paused) {
      deadline = 0;
      timer = setTimeout(pump, 16);
      return;
    }
    if (!deadline) deadline = performance.now();
    const result = engine.fanplay_worker_step();
    if (!result.alive) {
      stopped = true;
      postMessage({ type: "stopped" });
      return;
    }
    const now = performance.now();
    // Startup checkpoints yield without presenting a frame. They must not
    // each consume a whole game-frame period while loading script bundles.
    const period = fast || !result.presented ? 0 : 1000 / Math.max(1, result.rate);
    // Advance once per task. Drop old deadlines instead of running a catch-up
    // loop, and always yield to messages before starting the next Ruby slice.
    deadline = Math.max(deadline + period, now);
    timer = setTimeout(pump, Math.max(0, deadline - now));
  } catch (error) { fail(error); }
}
onmessage = async ({ data }) => {
  try {
    if (data.type === "load") {
      globalThis.FANPLAY_RUBY_WASM_URL = data.rubyWasm;
      const loader = await import(data.rubyLoader);
      globalThis["ruby-wasm-wasi"] ||= loader.default || loader;
      await import(data.rubyRuntime);
      engine = await import(data.bindings);
      await engine.default({ module_or_path: data.module });
      await globalThis.__fanplayRubyReady;
      engine.fanplay_worker_load(data.entry, data.label, data.paths, data.buffers);
      postMessage({ type: "ready" });
      pump();
    } else if (data.type === "control") {
      paused = data.paused;
      fast = data.fast;
      engine?.fanplay_worker_control(JSON.stringify(data));
    } else if (data.type === "stop") {
      stopped = true;
      clearTimeout(timer);
      postMessage({ type: "stopped" });
      close();
    } else if (data.type === "frame_ack") {
      inFlight = false;
      if (pendingFrame) {
        const frame = pendingFrame;
        pendingFrame = null;
        sendFrame(frame);
      }
    }
  } catch (error) { fail(error); }
};
