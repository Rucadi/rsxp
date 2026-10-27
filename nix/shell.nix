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
      rustc
      rustfmt
      clippy
      lld
      nodejs
      trunk
    ]
    ++ lib.optionals isLinux [ pkgs.mold ];

  nativeBuildInputs = [
    pkgs.pkg-config
    pkgs.rustPlatform.bindgenHook
  ];

  # Keep dynamically linked cargo binaries runnable outside the build phase.
  buildInputs = runtimeLibraries ++ [
    pkgs.zlib
    pkgs.libffi
  ];
  LD_LIBRARY_PATH = lib.optionalString isLinux (
    lib.makeLibraryPath (runtimeLibraries)
  );

  # Dust uses nightly-only language features. Keep the Nix-provided latest
  # rustc usable for the default DS-enabled build.
  RUSTC_BOOTSTRAP = "1";

}
