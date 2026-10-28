# frozen_string_literal: true

# rbwasm cannot load a Ruby extension at runtime. This extconf asks Cargo for
# the same Magnus bindings used by desktop, then leaves a static archive for
# rbwasm to link into the Ruby interpreter it is building.
require "mkmf"
require "rbconfig"
require "zlib"
require "fileutils"

unless RbConfig::CONFIG.fetch("host_os").include?("wasi")
  File.write("Makefile", dummy_makefile($srcdir).join)
  exit
end

$target = "fanplay_native"
minor = "#{RbConfig::CONFIG.fetch("MAJOR")}.#{RbConfig::CONFIG.fetch("MINOR")}"
prebuilt = "fanplay_native.prebuilt"

rust_dir = ENV["FANPLAY_RUST_DIR"]
unless rust_dir
  gz = File.join(__dir__, minor, "fanplay_native.a.gz")
  unless File.file?(gz)
    shipped = Dir[File.join(__dir__, "*", "fanplay_native.a.gz")]
      .map { |path| File.basename(File.dirname(path)) }
    abort "fanplay-rgss-wasm: no source directory or prebuilt extension for Ruby #{minor} " \
      "(this gem carries #{shipped.sort.join(", ")})"
  end
  File.binwrite(prebuilt, Zlib::GzipReader.open(gz, &:read))
else
  # rb-sys binds against the Ruby that rbwasm is currently compiling. Ruby is
  # not installed yet, so point it at rbwasm's source and generated headers.
  RbConfig::CONFIG.merge(
    "rubyhdrdir" => File.join(ENV.fetch("top_srcdir"), "include"),
    "rubyarchhdrdir" => File.join(ENV.fetch("extout"), "include", RbConfig::CONFIG.fetch("arch"))
  ).each { |key, value| ENV["RBCONFIG_#{key}"] = value.to_s }
  ENV["RBCONFIG_CROSS_COMPILING"] = "1"

  wasi_sdk = File.dirname(File.dirname(RbConfig::CONFIG.fetch("CC").split.first))
  ENV["BINDGEN_EXTRA_CLANG_ARGS"] = "--sysroot=#{wasi_sdk}/share/wasi-sysroot " \
    "-D_WASI_EMULATED_SIGNAL -D_WASI_EMULATED_PROCESS_CLOCKS -D_WASI_EMULATED_MMAN"
  ENV["CC_wasm32_wasip1"] = "#{wasi_sdk}/bin/clang"

  target_dir = File.expand_path(ENV.fetch("FANPLAY_WASM_TARGET_DIR", "target"))
  system(
    "cargo", "rustc",
    "--manifest-path", File.join(rust_dir, "Cargo.toml"),
    "--package", "fanplay-rgss-ruby",
    "--release",
    "--target", "wasm32-wasip1",
    "-Zbuild-std=std,panic_abort",
    "--crate-type", "staticlib",
    "--features", "ruby",
    "--target-dir", target_dir,
    exception: true
  )
  archive = File.join(target_dir, "wasm32-wasip1/release/libruby_rgss_core.a")

  # Ruby's WASI runtime and every Rust extension bring a few common symbols.
  # Keep this archive linkable beside wasi-vfs and future extensions by giving
  # the symbols owned names, exactly as HyperCast's wasm gem does.
  # wasm llvm-objcopy cannot rename symbols. Rewrite only equal-length names
  # in the archive bytes, preserving every archive/object offset and Rust's
  # v0 panic mangling exactly as HyperCast does.
  archive = File.binread(archive)
  {
    "rust_eh_personality" => "fanplay_personality",
    "ruby_abi_version" => "fanplay_abiverxx",
    "9panicking11EMPTY_PANIC" => "9panicking11FPLAY_PANIC"
  }.each { |from, to| archive.gsub!(from, to) }
  File.binwrite(prebuilt, archive)
  system("#{wasi_sdk}/bin/llvm-ranlib", prebuilt, exception: true)

  if (out = ENV["FANPLAY_WASM_OUT"])
    FileUtils.mkdir_p(File.join(out, minor))
    FileUtils.cp(prebuilt, File.join(out, minor, "fanplay_native.a"))
  end
end

# rbwasm's static path consumes this archive. Dynamic linking is intentionally
# rejected: the browser build is a wasm32-wasip1 interpreter, not a component.
File.write("Makefile", <<~MAKE)
  all: static
  static:
  	cp #{prebuilt} fanplay_native.a
  install-so:
  	@echo "fanplay-rgss-wasm links statically: build a wasm32-wasip1 interpreter" >&2; exit 1
  install-rb:
  	@true
  clean:
  	rm -f fanplay_native.a
  .PHONY: all static install-so install-rb clean
MAKE
