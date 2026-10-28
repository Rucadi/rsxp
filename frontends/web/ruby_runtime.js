/* Official ruby.wasm browser adapter. The Rust RGSS core remains the owner of
 * the game engine; this file only bootstraps CRuby and resumes one Fiber step
 * at a time so eframe keeps control of the browser event loop. */

(function () {
  "use strict";

  // RGSS executes in its dedicated worker. Do not instantiate a second Ruby
  // VM on the UI thread when this asset is included by the HTML entrypoint.
  if (typeof document !== "undefined") return;

  let vm = null;
  let gameFiber = null;
  let pendingStart = null;
  let ready = false;
  let terminalError = null;

  function rubyError(error) {
    if (error instanceof Error) return error;
    return new Error(String(error));
  }

  const rubyWasm = globalThis["ruby-wasm-wasi"];
  const rubyUrl =
    globalThis.FANPLAY_RUBY_WASM_URL ||
    new URL("ruby.wasm", globalThis.location.href).href;
  // The official ruby.wasm loader initializes the custom interpreter built by
  // tools/build-ruby-wasm.sh. That interpreter already contains the Magnus
  // RGSS gem; browser code must not try to dlopen it after startup.
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
    const loaded = await rubyWasm.DefaultRubyVM(module, {
      consolePrint: true,
      env: {
        HOME: "/",
        USER: "fanplay",
        LANG: "en_US.UTF-8",
        FANPLAY_WEB: "1",
        FANPLAY_NATIVE_DIAGNOSTICS:
          globalThis.FANPLAY_NATIVE_DIAGNOSTICS === "1" ? "1" : "0",
        FANPLAY_NATIVE_STATE: "1",
        FANPLAY_NATIVE_TILEMAP: "1",
        FANPLAY_SPRITE_SYNC: "native",
        RUBY_FIBER_VM_STACK_SIZE: String(64 * 1024 * 1024),
        RUBY_FIBER_MACHINE_STACK_SIZE: String(16 * 1024 * 1024),
        RUBY_THREAD_VM_STACK_SIZE: String(64 * 1024 * 1024),
        RUBY_THREAD_MACHINE_STACK_SIZE: String(16 * 1024 * 1024),
      },
    });
    vm = loaded.vm;
    vm.eval('require "/bundle/setup"');
    // web_bridge.rb calls this after the RGSS Ruby shim and the static
    // fanplay_native extension have installed their bindings. Keeping the
    // signal on the worker makes it visible to the UI without touching the
    // game/engine bridge protocol.
    globalThis.__fanplay_native_bindings_loaded = function (status) {
      console.info(String(status));
      if (typeof globalThis.__fanplay_worker_output === "function") {
        globalThis.__fanplay_worker_output({
          type: "native_bindings",
          status: String(status),
        });
      }
    };
    ready = true;
    if (pendingStart) {
      beginGame(pendingStart);
      pendingStart = null;
    }
    return vm;
  })().catch(function (error) {
    terminalError = rubyError(error);
    throw terminalError;
  });

  function requireVm() {
    // WASI exit! can escape without unwinding Ruby's current Fiber. Never
    // re-enter a failed VM; retain the first error instead of masking it.
    if (terminalError) throw terminalError;
    if (!vm) throw new Error("ruby.wasm is not ready");
    return vm;
  }

  function resumeGame() {
    if (terminalError) throw terminalError;
    if (!vm) return pendingStart !== null;
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

  function beginGame(config) {
    const ruby = requireVm();
    const env =
      "ENV['FANPLAY_VERSION']=" + rubyLiteral(config.version) + ";" +
      "ENV['FANPLAY_SCRIPTS']=" + rubyLiteral(config.scripts) + ";" +
      "ENV['FANPLAY_GAME_TITLE']=" + rubyLiteral(config.title) + ";" +
      "ENV['FANPLAY_WEB']='1';" +
      "ENV['FANPLAY_NATIVE_DIAGNOSTICS']=" +
        rubyLiteral(globalThis.FANPLAY_NATIVE_DIAGNOSTICS || "0") +
        ";" +
      "ENV['FANPLAY_NATIVE_STATE']='1';" +
      "ENV['FANPLAY_NATIVE_TILEMAP']='1';" +
      "ENV['FANPLAY_SPRITE_SYNC']='native';";
    ruby.eval(env);
    const initialSave = globalThis.__fanplay_initial_save;
    // Seed the VFS before the RGSS fiber starts. SaveData checks the path while
    // booting the title scene, so importing after the first resume is too late.
    if (initialSave && initialSave.length) {
      fanplay_ruby_import_save(initialSave);
      globalThis.__fanplay_initial_save = null;
    }
    ruby.eval(
      "$fanplay_fiber=Fiber.new{eval(" +
        rubyLiteral(config.source) +
        ",TOPLEVEL_BINDING,'rgss.rb',1)}"
    );
    gameFiber = ruby.eval("$fanplay_fiber");
    resumeGame();
  }

  globalThis.fanplay_ruby_start = function (source, version, scripts, title) {
    if (terminalError) throw terminalError;
    if (gameFiber || pendingStart) return true;
    const config = { source, version, scripts, title };
    if (ready) {
      beginGame(config);
    } else {
      // The official ruby.wasm loader is asynchronous. Keep the core alive
      // until CRuby is ready; the next synchronous `step` observes either the
      // started Fiber or the original initialization error.
      pendingStart = config;
    }
    return true;
  };

  globalThis.fanplay_ruby_step = function () {
    return resumeGame();
  };

  globalThis.fanplay_ruby_import_save = function (bytes) {
    const array = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes || []);
    let binary = "";
    for (let index = 0; index < array.length; index++) {
      binary += String.fromCharCode(array[index]);
    }
    const encoded = btoa(binary);
    return requireVm().eval(
      "FANPLAY.__fanplay_import_save(" + rubyLiteral(encoded) + ")"
    );
  };

  globalThis.fanplay_ruby_export_save = function () {
    return requireVm().eval("FANPLAY.__fanplay_export_save");
  };

  globalThis.fanplay_ruby_eval = function (source) {
    return requireVm().eval(String(source));
  };
})();
