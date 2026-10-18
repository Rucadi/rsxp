{
  description = "Native Rust engine for RPG Maker RGSS games";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      linuxSystems = [ "x86_64-linux" "aarch64-linux" ];
      windowsSystems = [ "x86_64-windows" ];
      eachSystem = systems: nixpkgs.lib.genAttrs systems;

      mkLinux = system:
        let pkgs = import nixpkgs { inherit system; };
        in pkgs.rustPlatform.buildRustPackage {
          pname = "rsxp";
          version = "0.1.0";
          src = self;
          cargoLock.lockFile = ./Cargo.lock;
          dontStrip = true;

          nativeBuildInputs = with pkgs; [ pkg-config rustPlatform.bindgenHook makeWrapper ];
          buildInputs = with pkgs; [ udev alsa-lib libx11 libxkbcommon ruby vulkan-loader wayland ];

          postFixup = ''
            wrapProgram $out/bin/rsxp --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath [
              pkgs.udev pkgs.alsa-lib pkgs.libx11 pkgs.libxkbcommon
              pkgs.vulkan-loader pkgs.wayland
            ]}
          '';

          meta = {
            description = "Native Rust engine for RPG Maker RGSS games";
            license = pkgs.lib.licenses.gpl2Plus;
            mainProgram = "rsxp";
            platforms = linuxSystems;
          };
        };

      mkWindows = system:
        let
          pkgs = import nixpkgs { localSystem = "x86_64-linux"; crossSystem = system; };
        in pkgs.rustPlatform.buildRustPackage {
          pname = "rsxp";
          version = "0.1.0";
          src = self;
          cargoLock.lockFile = ./Cargo.lock;
          cargoBuildFlags = [ "--target" "x86_64-pc-windows-gnu" ];
          cargoCheckFlags = [ "--target" "x86_64-pc-windows-gnu" ];
          doCheck = false;
          nativeBuildInputs = with pkgs.buildPackages; [ pkg-config rustPlatform.bindgenHook ];
          buildInputs = with pkgs; [ windows.pthreads ];
          CARGO_BUILD_TARGET = "x86_64-pc-windows-gnu";
          postInstall = ''
            mkdir -p $out/bin
            cp target/x86_64-pc-windows-gnu/release/rsxp.exe $out/bin/
          '';
          meta = {
            description = "Native Rust engine for RPG Maker RGSS games";
            license = pkgs.lib.licenses.gpl2Plus;
            platforms = [ "x86_64-windows" ];
          };
        };
    in {
      packages = eachSystem linuxSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
          app = mkLinux system;
        in {
          default = app;
          rsxp = app;
          appimage = pkgs.appimageTools.wrapType2 {
            pname = "rsxp";
            version = "0.1.0";
            src = app;
            extraInstallCommands = ''
              mkdir -p $out/share/applications
              cat > $out/share/applications/rsxp.desktop <<'EOF'
              [Desktop Entry]
              Type=Application
              Name=RSXP
              Exec=rsxp
              Terminal=true
              Categories=Game;Development;
              EOF
            '';
          };
        }) // {
          x86_64-windows = {
            default = mkWindows "x86_64-windows";
            rsxp = mkWindows "x86_64-windows";
          };
        };

      devShells = eachSystem linuxSystems (system: {
        default = import ./shell.nix { pkgs = import nixpkgs { inherit system; }; };
      });

      formatter = eachSystem linuxSystems (system: nixpkgs.legacyPackages.${system}.nixfmt);
    };
}
