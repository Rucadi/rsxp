// SPDX-License-Identifier: GPL-2.0-or-later
// Run with @ruby/wasm-wasi 2.10.1 on NODE_PATH, after build-ruby-wasm.sh.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { DefaultRubyVM } = require("@ruby/wasm-wasi/dist/browser");

async function main() {
  const root = path.resolve(__dirname, "..");
  const module = await WebAssembly.compile(
    fs.readFileSync(path.join(root, "frontends/web/ruby.wasm")),
  );
  const { vm } = await DefaultRubyVM(module);
  vm.eval('require "/bundle/setup"; require "fanplay_native"');
  vm.eval(fs.readFileSync(path.join(root, "crates/fanplay-rgss/src/ruby/rgss/table.rb"), "utf8"));
  assert.equal(vm.eval("Table.new(2).xsize").toString(), "2");
  // Native Ruby methods are installed; host calls must reach the outer engine.
  assert.equal(vm.eval("Audio.method(:bgm_play).source_location.nil?").toString(), "true");
  assert.equal(vm.eval("respond_to?(:__fanplay_native, true)").toString(), "false");
  for (const hook of ["__fanplay_sprite_sync", "__fanplay_scroll_tiles",
    "__fanplay_scroll_rgss_tiles", "__fanplay_apply_rgss_tiles", "__fanplay_world_camera",
    "__fanplay_world_release", "__fanplay_world_metadata_changed", "__fanplay_world_unbind",
    "__fanplay_install_scalar_setters", "__fanplay_retained_source", "__fanplay_touch",
    "__fanplay_watch", "__fanplay_unwatch", "__fanplay_forget", "__fanplay_volatile",
    "__fanplay_dirty_resources", "__fanplay_take_render_commands"]) {
    assert.equal(vm.eval(`respond_to?(:${hook}, true)`).toString(), "true", hook);
  }
  assert.equal(vm.eval("defined?(FANPLAYRetainedMap)").toString(), "constant");
  assert.equal(vm.eval("defined?(Win32API)").toString(), "constant");

  const requests = [];
  globalThis.__fanplay_native_b64 = (encoded) => {
    const request = Buffer.from(String(encoded), "base64");
    requests.push(request.toString("utf8"));
    const operation = request.toString("utf8").split(/[\t\n]/)[0];
    const response = { version: "1", audio_pos: "1.25", audio_volume: "75" };
    return Buffer.from(response[operation] || "").toString("base64");
  };
  vm.eval("ENV['FANPLAY_WEB']='1'");
  for (const unit of ["core/bootstrap.rb", "web_bridge.rb", "rgss/audio.rb"]) {
    vm.eval(fs.readFileSync(path.join(root, "crates/fanplay-rgss/src/ruby", unit), "utf8"));
  }
  vm.eval(`
    Audio.bgm_play('Audio/BGM/Título.ogg', 80, 100, 1.5, nil)
    Audio.bgs_play('Audio/BGS/Rain', 60, 90)
    Audio.me_play('Audio/ME/Victory', 70, 100)
    Audio.se_play('Audio/SE/Select', 50, 110)
    raise 'position query' unless Audio.bgm_pos == 1.25
    raise 'volume query' unless Audio.bgm_volume == 75
    Audio.bgm_set_volume(75)
    Audio.bgm_fade(200)
    Audio.__reset__
  `);
  assert(requests.includes("audio_play\tbgm\t80\t100\t1.5\nAudio/BGM/Título.ogg"));
  for (const channel of ["bgm", "bgs", "me", "se"]) {
    assert(requests.some((request) => request.startsWith(`audio_play\t${channel}\t`)));
    assert(requests.includes(`audio_stop\t${channel}\n`));
  }
  assert(requests.includes("audio_fade\tbgm\t200\n"));
  assert(requests.includes("audio_set_volume\tbgm\t75\n"));

  // Optional local game: execute Añil's actual method from the reported trace.
  if (process.argv[2]) {
    const scripts = fs.readFileSync(path.join(process.argv[2], "Data/Scripts.rxdata"));
    vm.eval(`
      source = Marshal.load('${scripts.toString("base64")}'.unpack1('m0'))
      script = source.find { |entry| entry[1] == 'Game_System' }
      raise 'Game_System script missing' unless script
      eval(Zlib.inflate(script[2]).force_encoding('UTF-8'), TOPLEVEL_BINDING, 'Game_System')
      $PokemonSystem = Struct.new(:bgmvolume).new(75)
      Game_System.allocate.bgm_play_internal2('Audio/BGM/Titulo', 80, 100, 0, nil)
    `);
    assert(requests.includes("audio_play\tbgm\t60\t100\t0\nAudio/BGM/Titulo"));
    console.log("Pokémon Añil Game_System#bgm_play_internal2 passes in ruby.wasm");
  }
  console.log("Ruby WASM bindings and all four audio channels pass");
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
