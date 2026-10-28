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
let diagnostics = false;

function trace(event, details = {}) {
  if (diagnostics) console.info("[FANPLAY RGSS worker]", event, details);
}

function traceError(event, error, details = {}) {
  console.error("[FANPLAY RGSS worker]", event, String(error?.stack || error), details);
}

function sendFrame(frame) {
  inFlight = true;
  postMessage(frame, [frame.pixels.buffer]);
}
globalThis.__fanplay_worker_output = (message) => {
  if (message.type === "frame") {
    if (inFlight) pendingFrame = message;
    else sendFrame(message);
  } else if (message.type === "plugin_response") {
    const response = String(message.response || "");
    trace("plugin response", {
      responseLength:response.length,
      responsePreview:response.slice(0, 500),
    });
    postMessage(message);
  } else if (message.data instanceof Uint8Array) {
    postMessage(message, [message.data.buffer]);
  } else {
    postMessage(message);
  }
};
function fail(error) {
  traceError("fatal", error);
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
      diagnostics = data.nativeDiagnostics === true || data.webviewDiagnostics === true;
      trace("load", {label:data.label, entry:data.entry, paths:data.paths?.length || 0});
      globalThis.FANPLAY_RUBY_WASM_URL = data.rubyWasm;
      globalThis.FANPLAY_NATIVE_DIAGNOSTICS = data.nativeDiagnostics ? "1" : "0";
      const loader = await import(data.rubyLoader);
      globalThis["ruby-wasm-wasi"] ||= loader.default || loader;
      await import(data.rubyRuntime);
      engine = await import(data.bindings);
      await engine.default({ module_or_path: data.module });
      await globalThis.__fanplayRubyReady;
      engine.fanplay_worker_load(
        data.entry,
        data.label,
        data.paths,
        data.buffers,
        data.save || new Uint8Array()
      );
      postMessage({ type: "ready" });
      trace("ready");
      pump();
    } else if (data.type === "control") {
      paused = data.paused;
      fast = data.fast;
      engine?.fanplay_worker_control(data);
    } else if (data.type === "save_export") {
      const encoded = engine?.fanplay_ruby_export_save?.();
      if (!encoded) {
        postMessage({ type: "save_export", missing: true });
      } else {
        const binary = atob(String(encoded));
        const bytes = new Uint8Array(binary.length);
        for (let index = 0; index < binary.length; index++) {
          bytes[index] = binary.charCodeAt(index);
        }
        postMessage({ type: "save_export", data: bytes }, [bytes.buffer]);
      }
    } else if (data.type === "plugin_request") {
      trace("plugin request", {
        request_id:String(data.request_id),
        generation:String(data.generation),
        operation:String(data.operation || ""),
      });
      engine?.fanplay_worker_plugin_request?.(
        // wasm-bindgen exposes Rust u64 parameters as JavaScript BigInt.
        // Passing Number here throws before Ruby can process the first plugin
        // request, aborting the game during plugin activation.
        BigInt(String(data.request_id)),
        BigInt(String(data.generation)),
        String(data.operation || ""),
        JSON.stringify(data.arguments || {})
      );
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
  } catch (error) {
    traceError("message failed", error, {type:data?.type, request_id:String(data?.request_id || "")});
    fail(error);
  }
};
