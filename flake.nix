{
  description = "Native Rust engine for RPG Maker RGSS games";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
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
          rsxp = package;
        }
        // lib.optionalAttrs (system == "x86_64-linux") {
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
