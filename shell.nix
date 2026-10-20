{
  pkgs ? import <nixpkgs> { },
}:

import ./nix/shell.nix { inherit pkgs; }
