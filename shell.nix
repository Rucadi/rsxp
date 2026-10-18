{ pkgs ? import <nixpkgs> { } }:

pkgs.mkShell {
  inputsFrom = [ (import ./default.nix { inherit pkgs; }) ];
  packages = [
    (import ./ruby-profiler.nix { inherit pkgs; })
    pkgs.cargo
    pkgs.rustc
    pkgs.rustfmt
    pkgs.clippy
    pkgs.wasm-bindgen-cli
    pkgs.lld
    pkgs.nodejs
    pkgs.mold
  ];
  # Keep dynamically linked cargo binaries runnable outside the build phase.
  LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [
    pkgs.udev
    pkgs.alsa-lib
    pkgs.libx11
    pkgs.libxkbcommon
    pkgs.ruby
    pkgs.vulkan-loader
    pkgs.wayland
  ];
  # WebAssembly builds need a wasm-capable linker (rust-lld is not shipped here).
  CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_LINKER = "${pkgs.lld}/bin/wasm-ld";
  RUBYLIB = "${import ./ruby-profiler.nix { inherit pkgs; }}/${pkgs.ruby.gemPath}/gems/stackprof-0.2.28/lib";
}
