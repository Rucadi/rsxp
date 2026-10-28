{ pkgs }:

let
  inherit (pkgs) lib;
  isLinux = pkgs.stdenv.hostPlatform.isLinux;
  rustcBase = pkgs.rustPlatform.rust.rustc;
  rustSysroot = pkgs.runCommand "fanplay-rust-sysroot" {} ''
    mkdir -p $out/lib/rustlib
    for path in ${rustcBase.unwrapped}/lib/rustlib/*; do
      ln -s "$path" "$out/lib/rustlib/$(basename "$path")"
    done
    mkdir -p $out/lib/rustlib/src/rust
    ln -s ${pkgs.rustPlatform.rustLibSrc} $out/lib/rustlib/src/rust/library
  '';
  rustcWithSources = rustcBase.override { sysroot = rustSysroot; };
  runtimeLibraries = lib.optionals isLinux (
    with pkgs;
    [
      udev
      alsa-lib
      libx11
      libxkbcommon
      vulkan-loader
      wayland
      pkgs.webkitgtk_4_1
    ]
  );
in
pkgs.mkShell {
  packages =
    with pkgs;
    [
      cargo
      curl
      unzip
      rustcWithSources
      rustfmt
      clippy
      lld
      nodejs
      trunk
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
  ];
  LD_LIBRARY_PATH = lib.optionalString isLinux (
    lib.makeLibraryPath (runtimeLibraries ++ [ pkgs.ruby_4_0 ])
  );

  # Dust uses nightly-only language features. Keep the Nix-provided latest
  # rustc usable for the default DS-enabled build.
  RUSTC_BOOTSTRAP = "1";

}
