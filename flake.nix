{
  description = "Fanplay fangame player for RPG Maker, Game Boy, GBA and Nintendo DS";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  inputs.nix-appimage.url = "github:ralismark/nix-appimage";
  inputs.nix-appimage.inputs.nixpkgs.follows = "nixpkgs";

  outputs =
    { self, nixpkgs, nix-appimage }:
    let
      lib = nixpkgs.lib;
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
      forAllSystems = lib.genAttrs systems;
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          package = import ./nix/package.nix {
            inherit pkgs;
            doCheck = pkgs.stdenv.hostPlatform.isLinux;
          };
        in
        {
          default = package;
          fanplay = package;
        }
        // lib.optionalAttrs (system == "x86_64-linux") {
          appimage = nix-appimage.bundlers.${system}.default package;
          windows = import ./nix/windows.nix {
            pkgs = nixpkgs.legacyPackages.x86_64-linux;
          };
        }
      );

      devShells = forAllSystems (system: {
        default = import ./nix/shell.nix {
          pkgs = nixpkgs.legacyPackages.${system};
        };
      });

      formatter = forAllSystems (system: nixpkgs.legacyPackages.${system}.nixfmt);
    };
}
