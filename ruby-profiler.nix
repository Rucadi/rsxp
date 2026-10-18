{ pkgs ? import <nixpkgs> { } }:

pkgs.buildRubyGem {
  gemName = "stackprof";
  version = "0.2.28";
  source.sha256 = "014s1zxlxcw35shislar3y1i3mqa0c6gh3m21js14q1q5zharhjf";
}
