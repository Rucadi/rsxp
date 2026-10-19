# Android development shell for the RSXP mobile port.
#
# Usage:
#   nix-shell shell-droid.nix
#   python3 mobile/build.py android
#
# The Android SDK, NDK, emulator, Gradle, JDK and Android Studio all come from
# Nix. Unfree packages and the Android SDK licences are accepted here, so the
# shell can be entered non-interactively. android-nixpkgs is pinned to a known
# revision; bump it deliberately when you want a newer SDK channel.
{
  pkgs ? import <nixpkgs> {
    config = {
      allowUnfree = true;
      android_sdk.accept_license = true;
    };
  },
}:

let
  android-nixpkgs = import (builtins.fetchGit {
    url = "https://github.com/tadfisher/android-nixpkgs.git";
    rev = "764a9063ee85af9e2afc7efc2f3457c0807c92d6";
  }) {
    inherit pkgs;
    channel = "stable";
  };

  # Keep these attrs in sync with mobile/android/app/build.gradle.
  sdkPackages = s: with s; [
    cmdline-tools-latest
    platform-tools
    build-tools-35-0-0
    platforms-android-35
    ndk-28-1-13356709
    cmake-3-22-1
    emulator
    system-images-android-35-google-apis-arm64-v8a
  ];

  android-sdk = android-nixpkgs.sdk sdkPackages;
  sdkRoot = "${android-sdk}/share/android-sdk";
  ndkRoot = "${sdkRoot}/ndk/28.1.13356709";
in
pkgs.mkShell {
  packages = [
    android-sdk
    pkgs.android-studio
    pkgs.jdk17
    pkgs.gradle_8
    pkgs.cmake
    pkgs.ninja
    pkgs.pkg-config
    pkgs.patchelf
    pkgs.llvm
    pkgs.rustup
    pkgs.cargo
    pkgs.rustc
    pkgs.rustfmt
    pkgs.clippy
    pkgs.cargo-ndk
    pkgs.ruby
    pkgs.autoconf
    pkgs.bison
    pkgs.gnumake
    pkgs.perl
    pkgs.curl
    pkgs.unzip
    pkgs.llvmPackages.libclang
    pkgs.python3
  ];

  ANDROID_HOME = sdkRoot;
  ANDROID_SDK_ROOT = sdkRoot;
  ANDROID_NDK_HOME = ndkRoot;
  ANDROID_NDK_ROOT = ndkRoot;
  JAVA_HOME = "${pkgs.jdk17.home}";
  # Use the SDK CMake and build-tools without making the Nix wrappers win.
  GRADLE_OPTS = "-Dorg.gradle.java.home=${pkgs.jdk17.home}";

  shellHook = ''
    export PATH="$JAVA_HOME/bin:$ANDROID_HOME/cmdline-tools/latest/bin:$ANDROID_HOME/platform-tools:$ANDROID_HOME/emulator:$PATH"
    echo "Android SDK: $ANDROID_HOME"
    echo "Android NDK: $ANDROID_NDK_HOME"
    echo "Build with: python3 mobile/build.py android"
  '';
}
