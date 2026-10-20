
{ pkgs  ? import <nixpkgs> {}}:

let
  version = "4.0.6";
  release = "1";

  archive = pkgs.fetchurl {
    url = "https://mirror.msys2.org/mingw/mingw64/mingw-w64-x86_64-ruby-${version}-${release}-any.pkg.tar.zst";
    hash = "sha256-UykWV57A3yXC8HdHddh8g5S0CAJlCeWToE06sEP8rYQ=";
  };
in
pkgs.stdenvNoCC.mkDerivation {
  pname = "ruby-mingw64";
  inherit version;

  src = archive;

  nativeBuildInputs = [
    pkgs.zstd
    pkgs.gnutar
  ];

  dontUnpack = true;
  dontConfigure = true;
  dontBuild = true;
  dontFixup = true;

  installPhase = ''
    runHook preInstall

    mkdir -p "$out"

    tar --use-compress-program=unzstd \
      -xf "$src" \
      -C "$out" \
      --strip-components=1 \
      mingw64/

    test -d "$out/include"
    test -d "$out/lib"
    test -d "$out/bin"

    runHook postInstall
  '';

  meta = with pkgs.lib; {
    description = "Precompiled MSYS2 MinGW-w64 Ruby for Windows";
    homepage = "https://packages.msys2.org/packages/mingw-w64-x86_64-ruby";
    platforms = platforms.linux;
  };
}
