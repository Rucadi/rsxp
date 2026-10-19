{ pkgs ? import <nixpkgs> { } }:

let
  sdk = pkgs.androidenv.composeAndroidPackages {
    platformVersions = [ "35" ];
    buildToolsVersions = [ "35.0.0" ];
    includeEmulator = false;
    includeSystemImages = false;
    includeNDK = true;
    ndkVersion = "27.2.12479018";
    includeCmake = false;
  };
in
pkgs.mkShell {
  packages = [
    pkgs.jdk17
    pkgs.gradle_8
    pkgs.cargo-ndk
    pkgs.rustup
    pkgs.cargo
    pkgs.rustc
    pkgs.rustfmt
    pkgs.ruby
    pkgs.autoconf
    pkgs.bison
    pkgs.gnumake
    pkgs.perl
    pkgs.curl
    pkgs.unzip
    pkgs.llvmPackages.libclang
    sdk.androidsdk
  ];

  ANDROID_HOME = "${sdk.androidsdk}/libexec/android-sdk";
  ANDROID_SDK_ROOT = "${sdk.androidsdk}/libexec/android-sdk";
  ANDROID_NDK_HOME = "${sdk.androidsdk}/libexec/android-sdk/ndk/27.2.12479018";
  JAVA_HOME = "${pkgs.jdk17}";
  LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
}
