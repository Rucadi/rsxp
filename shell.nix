{ pkgs ? import <nixpkgs> { } }:

let
  linuxLibraries = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux (with pkgs; [
    udev alsa-lib libx11 libxkbcommon vulkan-loader wayland
  ]);
in

pkgs.mkShell {
  inputsFrom = [ (import ./default.nix { inherit pkgs; }) ];
  packages = [
    pkgs.cargo
    pkgs.rustc
    pkgs.rustfmt
    pkgs.clippy
    pkgs.rustfmt
    pkgs.wasm-bindgen-cli
    pkgs.binaryen
    pkgs.lld
    pkgs.nodejs
  ] ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.mold ];
  # Keep dynamically linked cargo binaries runnable outside the build phase.
  buildInputs = linuxLibraries ++ [ pkgs.zlib pkgs.libffi ];
  LD_LIBRARY_PATH = pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isLinux
    (pkgs.lib.makeLibraryPath (linuxLibraries ++ [ pkgs.ruby ]));
  # WebAssembly builds need a wasm-capable linker (rust-lld is not shipped here).
  CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_LINKER = "${pkgs.lld}/bin/wasm-ld";
}
