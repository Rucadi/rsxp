{ pkgs ? import <nixpkgs> { } }:

let
  linuxLibraries = pkgs.lib.optionals pkgs.stdenv.isLinux (with pkgs; [
    udev alsa-lib libx11 libxkbcommon vulkan-loader wayland
  ]);
  platformLibraries = pkgs.lib.optionals pkgs.stdenv.isDarwin (with pkgs; [
    darwin.apple_sdk.frameworks
  ]);
in

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
  buildInputs = linuxLibraries ++ platformLibraries;
  LD_LIBRARY_PATH = pkgs.lib.optionalString pkgs.stdenv.isLinux
    (pkgs.lib.makeLibraryPath (linuxLibraries ++ [ pkgs.ruby ]));
  # WebAssembly builds need a wasm-capable linker (rust-lld is not shipped here).
  CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_LINKER = "${pkgs.lld}/bin/wasm-ld";
  RUBYLIB = "${import ./ruby-profiler.nix { inherit pkgs; }}/${pkgs.ruby.gemPath}/gems/stackprof-0.2.28/lib";
}
