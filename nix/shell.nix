{ pkgs }:

let
  inherit (pkgs) lib;
  isLinux = pkgs.stdenv.hostPlatform.isLinux;
  runtimeLibraries = lib.optionals isLinux (
    with pkgs;
    [
      udev
      alsa-lib
      libx11
      libxkbcommon
      vulkan-loader
      wayland
    ]
  );
in
pkgs.mkShell {
  packages =
    with pkgs;
    [
      cargo
      rustc
      rustfmt
      clippy
      wasm-bindgen-cli
      binaryen
      lld
      nodejs
    ]
    ++ lib.optionals isLinux [ pkgs.mold ];

  nativeBuildInputs = [
    pkgs.pkg-config
    pkgs.ruby_4_0
    pkgs.rustPlatform.bindgenHook
  ];

  # Keep dynamically linked cargo binaries runnable outside the build phase.
  buildInputs = runtimeLibraries ++ [
    pkgs.ruby_4_0
    pkgs.zlib
    pkgs.libffi
    pkgs.webkitgtk_4_1

  ];
  LD_LIBRARY_PATH = lib.optionalString isLinux (
    lib.makeLibraryPath (runtimeLibraries ++ [ pkgs.ruby_4_0 ])
  );

  # Dust uses nightly-only language features. Keep the Nix-provided latest
  # rustc usable for the default DS-enabled build.
  RUSTC_BOOTSTRAP = "1";

  # WebAssembly builds need a wasm-capable linker (rust-lld is not shipped here).
  CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_LINKER = "${pkgs.lld}/bin/wasm-ld";
}
