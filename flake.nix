{
  description = "Native Rust engine for RPG Maker RGSS games";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  inputs.nix-appimage.url = "github:ralismark/nix-appimage";
  inputs.nix-appimage.inputs.nixpkgs.follows = "nixpkgs";

  outputs = { self, nixpkgs, nix-appimage }:
    let
      linuxSystems = [ "x86_64-linux" "aarch64-linux" ];
      darwinSystems = [ "x86_64-darwin" "aarch64-darwin" ];
      windowsSystem = "x86_64-windows";
      eachSystem = systems: nixpkgs.lib.genAttrs systems;

      mkLinux = system:
        let pkgs = import nixpkgs { inherit system; };
        in import ./default.nix { inherit pkgs; };

    in {
      packages = eachSystem linuxSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
          app = mkLinux system;
        in {
          default = app;
          rsxp = app;
          appimage = nix-appimage.bundlers.${system}.default app;
        }) // eachSystem darwinSystems (system:
          let
            pkgs = import nixpkgs { inherit system; };
            app = import ./default.nix { inherit pkgs; doCheck = false; };
          in {
            default = app;
            rsxp = app;
          }) // {
          ${windowsSystem} = let
              pkgs = import nixpkgs { system = "x86_64-linux"; };
              app = pkgs.pkgsCross.mingwW64.rustPlatform.buildRustPackage {
                pname = "rsxp";
                version = "0.1.0";
                src = pkgs.lib.cleanSourceWith {
                  src = ./.;
                  filter = path: type:
                    let relative = pkgs.lib.removePrefix (toString ./. + "/") (toString path);
                    in relative == "rust" || pkgs.lib.hasPrefix "rust/" relative
                      || relative == "assets" || relative == "assets/liberation.ttf"
                      || relative == "api" || builtins.elem relative [ "api/game_bridge.rb" "api/openapi.json" ]
                      || relative == "binding" || builtins.elem relative [
                        "binding/module_rpg1.rb.xxd" "binding/module_rpg2.rb.xxd" "binding/module_rpg3.rb.xxd"
                      ] || builtins.elem relative [ "Cargo.toml" "Cargo.lock" "build.rs" "COPYING" ];
                };
                cargoLock.lockFile = ./Cargo.lock;
                cargoBuildFlags = [ "--target" "x86_64-pc-windows-gnu" ];
                doCheck = false;
                CARGO_BUILD_TARGET = "x86_64-pc-windows-gnu";
                buildInputs = with pkgs.pkgsCross.mingwW64; [ windows.pthreads ];
                nativeBuildInputs = with pkgs.buildPackages; [ pkg-config rustPlatform.bindgenHook ];
                postInstall = ''
                  mkdir -p $out/bin
                  cp target/x86_64-pc-windows-gnu/release/rsxp.exe $out/bin/
                '';
                meta.platforms = [ "x86_64-windows" ];
              };
            in { default = app; rsxp = app; };
          };

      devShells = eachSystem (linuxSystems ++ darwinSystems) (system:
        let pkgs = import nixpkgs { inherit system; };
        in {
          default = import ./shell.nix { inherit pkgs; };
        });

      formatter = eachSystem (linuxSystems ++ darwinSystems) (system: nixpkgs.legacyPackages.${system}.nixfmt);
    };
}
