{ pkgs ? import <nixpkgs> { }, doCheck ? true }:

let
  isLinux = pkgs.stdenv.hostPlatform.isLinux;
  isDarwin = pkgs.stdenv.hostPlatform.isDarwin;
  runtimeLibraries = pkgs.lib.optionals isLinux (with pkgs; [
    udev alsa-lib libx11 libxkbcommon vulkan-loader wayland
  ]);
  rubyPath = "${pkgs.ruby}/${pkgs.ruby.gemPath}/gems/stackprof-0.2.28/lib";
in

pkgs.rustPlatform.buildRustPackage {
  pname = "rsxp";
  version = "0.1.0";
  src = pkgs.lib.cleanSourceWith {
    src = ./.;
    filter = path: type:
      let
        relative = pkgs.lib.removePrefix (toString ./. + "/") (toString path);
      in
      relative == "rust" || pkgs.lib.hasPrefix "rust/" relative
      || relative == "assets" || relative == "assets/liberation.ttf"
      || relative == "api" || builtins.elem relative [ "api/game_bridge.rb" "api/openapi.json" ]
      || relative == "binding" || builtins.elem relative [
        "binding/module_rpg1.rb.xxd" "binding/module_rpg2.rb.xxd" "binding/module_rpg3.rb.xxd"
      ]
      || builtins.elem relative [ "Cargo.toml" "Cargo.lock" "build.rs" "COPYING" ];
  };

  cargoLock.lockFile = ./Cargo.lock;
  # A few relay tests share process-global state and assume serial execution.
  cargoTestThreads = 1;
  # Cargo emits release debuginfo for perf; preserve it during Nix fixup too.
  dontStrip = true;
  buildInputs = runtimeLibraries ++ [ pkgs.ruby ];
  nativeBuildInputs = [ pkgs.pkg-config pkgs.ruby pkgs.rustPlatform.bindgenHook ]
    ++ pkgs.lib.optionals isLinux [ pkgs.makeWrapper ];
  postFixup = pkgs.lib.optionalString isLinux ''
    wrapProgram $out/bin/rsxp --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath runtimeLibraries} \
      --prefix RUBYLIB : ${rubyPath}
  '';
  doCheck = doCheck;

  postInstall = ''
    install -Dm644 ${./COPYING} $out/share/licenses/rsxp/COPYING
    install -Dm644 ${./docs/rust-port.md} $out/share/doc/rsxp/rust-port.md
    install -Dm644 ${./docs/ruby-profiling.md} $out/share/doc/rsxp/ruby-profiling.md
    install -Dm644 ${./rust/testdata/rgss_demo.rb} $out/share/doc/rsxp/rgss_demo.rb
  '';

  doInstallCheck = true;
  installCheckPhase = ''
    runHook preInstallCheck
    $out/bin/rsxp --version
    $out/bin/rsxp --headless --frames 2 --output smoke.ppm
    test -s smoke.ppm
    $out/bin/rsxp run --script ${./rust/testdata}/rgss_smoke.rb --headless
    $out/bin/rsxp run --script ${./rust/testdata}/rgss_smoke.rb --headless \
      --ruby-profile "$PWD/ruby-smoke.dump" --ruby-profile-start-frame 1 --ruby-profile-frames 2
    test -s ruby-smoke.dump
    test -s ruby-smoke.dump.txt
    test -s ruby-smoke.dump.json
    runHook postInstallCheck
  '';

  meta = {
    description = "Native Rust engine for RPG Maker RGSS games";
    license = pkgs.lib.licenses.gpl2Plus;
    mainProgram = "rsxp";
    platforms = with pkgs.lib.platforms; linux ++ darwin ++ windows;
  };
}
