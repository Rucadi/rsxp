// SPDX-License-Identifier: GPL-2.0-or-later
// Regression check for the browser File/FileTest shim. Game plugins commonly
// replace File.exist? and FileTest.exist? with wrappers that probe encrypted
// archives through each other's saved originals (Compresor-Fixes'
// EncryptedFileExists is one). If the saved FileTest probe itself dispatches
// through File.exist?, those wrappers recurse until the wasm VM traps with
// "RuntimeError: unreachable". Run after cargo xtask web:
//   node platforms/web/tests/file-exist.mjs
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { DefaultRubyVM } from '../node_modules/@ruby/wasm-wasi/dist/esm/browser.js';
import { PRELUDE } from '../browser/ruby-host.js';
import { rubySource } from '../../../tools/ruby-sources.mjs';

const root = resolve(import.meta.dirname, '..', '..');
const wasm = readFileSync(resolve(import.meta.dirname, '../dist/ruby/ruby+stdlib.wasm'));
const { vm } = await DefaultRubyVM(await WebAssembly.compile(wasm), {
  env: { HOME: '/', LANG: 'en_US.UTF-8' },
});

vm.eval(PRELUDE);
// No engine is initialized here, so asset probes report the file as missing.
vm.eval(`def __rsxp_native(_request) = ''.b`);
const shim = rubySource('RGSS_SOURCE');
const entry = shim.lastIndexOf('\nbegin\n');
assert.ok(entry > 0, 'RGSS game entry block missing');
vm.eval(shim.slice(0, entry));

const outcome = vm.eval(`
$orig_file_exist = File.method(:exist?)
$orig_filetest_exist = FileTest.method(:exist?)
$encrypted = nil
def archive_exists?(_path) = false
def encrypted?
  $encrypted ||= $orig_filetest_exist.call('./Game.rgssad')
end
class << File
  def exist?(path)
    return true if $orig_file_exist.call(path)
    encrypted? && archive_exists?(path)
  end
end
module FileTest
  class << self
    def exist?(path)
      return true if $orig_filetest_exist.call(path)
      encrypted? && archive_exists?(path)
    end
  end
end
begin
  32.times { FileTest.exist?('Graphics/Pictures/missing') }
  'ok'
rescue SystemStackError => error
  "stack overflow: #{error.message}"
end
`).toString();

assert.equal(outcome, 'ok', 'FileTest.exist? recursed through the patched File.exist?');
console.log('BROWSER_FILE_EXIST_OK');
