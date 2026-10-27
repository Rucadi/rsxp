/* Official ruby.wasm browser adapter. The Rust RGSS core remains the owner of
 * the game engine; this file only bootstraps CRuby and resumes one Fiber step
 * at a time so eframe keeps control of the browser event loop. */
(function () {
  "use strict";

  const rubyWasm = globalThis["ruby-wasm-wasi"];
  const rubyUrl =
    "https://cdn.jsdelivr.net/npm/@ruby/4.0-wasm-wasi@2.10.1/dist/ruby+stdlib.wasm";
  let vm = null;
  let gameFiber = null;
  let terminalError = null;

  function rubyError(error) {
    if (error instanceof Error) return error;
    return new Error(String(error));
  }

  globalThis.__fanplayRubyReady = (async function () {
    if (!rubyWasm || typeof rubyWasm.DefaultRubyVM !== "function") {
      throw new Error("ruby.wasm browser loader is unavailable");
    }
    const response = await fetch(rubyUrl);
    if (!response.ok) {
      throw new Error("could not download ruby.wasm: HTTP " + response.status);
    }
    let module;
    try {
      module = await WebAssembly.compileStreaming(response.clone());
    } catch (_) {
      module = await WebAssembly.compile(await response.arrayBuffer());
    }
    const loaded = await rubyWasm.DefaultRubyVM(module, { consolePrint: true });
    vm = loaded.vm;
    return vm;
  })().catch(function (error) {
    throw rubyError(error);
  });

  function requireVm() {
    // WASI exit! can escape without unwinding Ruby's current Fiber. Never
    // re-enter a failed VM; retain the first error instead of masking it.
    if (terminalError) throw terminalError;
    if (!vm) throw new Error("ruby.wasm is not ready");
    return vm;
  }

  function resumeGame() {
    requireVm();
    if (!gameFiber) return false;
    try {
      if (gameFiber.call("alive?").toString() !== "true") return false;
      gameFiber.call("resume");
      return gameFiber.call("alive?").toString() === "true";
    } catch (error) {
      terminalError = rubyError(error);
      throw terminalError;
    }
  }

  // A Ruby double-quoted literal would interpolate #{...} found in ordinary
  // game scripts. Single-quoted Ruby literals only need backslash/apostrophe
  // escaping and are safe for arbitrary embedded RGSS source.
  function rubyLiteral(value) {
    return (
      "'" +
      String(value).replace(/\\/g, "\\\\").replace(/'/g, "\\'") +
      "'"
    );
  }

  globalThis.fanplay_ruby_start = function (source, version, scripts, title) {
    const ruby = requireVm();
    const env =
      "ENV['FANPLAY_VERSION']=" + rubyLiteral(version) + ";" +
      "ENV['FANPLAY_SCRIPTS']=" + rubyLiteral(scripts) + ";" +
      "ENV['FANPLAY_GAME_TITLE']=" + rubyLiteral(title) + ";";
    ruby.eval(env);
    ruby.eval(
      "$fanplay_fiber=Fiber.new{eval(" +
        rubyLiteral(source) +
        ",TOPLEVEL_BINDING,'rgss.rb',1)}"
    );
    gameFiber = ruby.eval("$fanplay_fiber");
    resumeGame();
    return true;
  };

  globalThis.fanplay_ruby_step = function () {
    return resumeGame();
  };

  globalThis.fanplay_ruby_eval = function (source) {
    return requireVm().eval(String(source));
  };
})();
