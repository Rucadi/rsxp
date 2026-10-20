{
  pkgs ? import <nixpkgs> { },
  doCheck ? true,
}:

import ./nix/package.nix { inherit pkgs doCheck; }
