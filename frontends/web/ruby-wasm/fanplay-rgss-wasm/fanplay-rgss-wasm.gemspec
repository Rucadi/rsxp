# frozen_string_literal: true

Gem::Specification.new do |spec|
  spec.name = "fanplay-rgss-wasm"
  spec.version = "0.1.0"
  spec.summary = "FANPLAY RGSS Magnus bindings for ruby.wasm"
  spec.description = <<~DESCRIPTION
    The FANPLAY RGSS Ruby extension compiled as a wasm32-wasip1 static archive
    and linked into a Ruby interpreter by rbwasm.
  DESCRIPTION
  spec.license = "GPL-2.0-or-later"
  spec.authors = ["FANPLAY contributors"]
  spec.email = ["devnull@example.invalid"]
  spec.homepage = "https://github.com/Rucadi/FANPLAY"
  spec.files = Dir["lib/**/*", "ext/**/*", "*.gemspec"]
  spec.extensions = ["ext/fanplay_native/extconf.rb"]
  spec.require_paths = ["lib"]
  spec.required_ruby_version = ">= 3.3"
end
