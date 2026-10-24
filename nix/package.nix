{
  pkgs,
  doCheck ? true,
}:

let
  inherit (pkgs) lib;
  root = ../.;
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
      glib
      webkitgtk_4_1
    ]
  );
in
pkgs.rustPlatform.buildRustPackage {
  pname = "rsxp";
  version = "0.1.0";
  src = root;

  cargoLock = {
    lockFile = root + "/Cargo.lock";
    # Same git repository as the three RustBoyAdvance-NG workspace crates.
    # Pinned in Cargo.lock and Cargo.toml.
    outputHashes = {
      "arm-decoder-0.0.0" = "sha256-HG8Gh5H7ez1ad6VtWNCKcQ6ywA1yguwrMeFL5ZbKyIE=";
      "dust-core-0.0.0" = "sha256-BT/AofCPtDt22l0G/yW2bvDG+QqP2Po/8Q17Lp3lwIs=";
      "arm7tdmi-0.1.0" = "sha256-wRsghc7uRlv+fUh7hpibQ7ajw+3kasEqdHTV+1N2QnI=";
      "rustboyadvance-core-0.1.0" = "sha256-wRsghc7uRlv+fUh7hpibQ7ajw+3kasEqdHTV+1N2QnI=";
      "rustboyadvance-utils-0.1.0" = "sha256-wRsghc7uRlv+fUh7hpibQ7ajw+3kasEqdHTV+1N2QnI=";
    };
  };
  cargoBuildFlags = [
    "-p"
    "rsxp-desktop"
    "--bin"
    "rsxp"
  ];
  cargoInstallFlags = [
    "-p"
    "rsxp-desktop"
    "--bin"
    "rsxp"
  ];

  # Dust uses nightly-only language features and is in the desktop defaults.
  RUSTC_BOOTSTRAP = "1";

  # A few relay tests share process-global state and assume serial execution.
  cargoTestThreads = 1;
  # Cargo emits release debuginfo for perf; preserve it during Nix fixup too.
  dontStrip = true;

  buildInputs = runtimeLibraries ++ [ pkgs.ruby_4_0 ];
  nativeBuildInputs = [
    pkgs.pkg-config
    pkgs.ruby_4_0
    pkgs.rustPlatform.bindgenHook
  ]
  ++ lib.optionals isLinux [ pkgs.makeWrapper ];
  postFixup = lib.optionalString isLinux ''
    wrapProgram $out/bin/rsxp --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath runtimeLibraries}
  '';
  inherit doCheck;

  doInstallCheck = true;
  installCheckPhase = ''
    runHook preInstallCheck
    $out/bin/rsxp --version
    $out/bin/rsxp --headless --frames 2 --output smoke.ppm
    test -s smoke.ppm
    $out/bin/rsxp run --script ${root}/crates/rsxp-rgss/testdata/rgss_smoke.rb --headless
    runHook postInstallCheck
  '';

  meta = with lib; {
    description = "Native Rust engine for RPG Maker RGSS games";
    license = licenses.gpl2Plus;
    mainProgram = "rsxp";
    platforms = platforms.linux ++ platforms.darwin;
  };
}
