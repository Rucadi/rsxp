# Reproducible Android development shell for the cargo-mobile2 app in ./mobile.
#
#   nix-shell android-shell.nix
#   cd mobile && cargo mobile init --skip-targets-install
#   cargo android run
#
# The shell accepts the Android SDK license and allows the unfree SDK/NDK
# packages as required by Google's redistribution terms. Rust Android target
# standard libraries are managed by rustup in the invoking user's profile.
{
  pkgs ?
    import <nixpkgs> {
      config = {
        allowUnfree = true;
        android_sdk.accept_license = true;
      };
    },
}:

let
  inherit (pkgs) lib;

  androidPackages = pkgs.androidenv.composeAndroidPackages {
    platformVersions = [ "37.0" ];
    buildToolsVersions = [ "37.0.0" ];
    platformToolsVersion = "37.0.1";
    # r27 keeps the shell noticeably smaller than r28/r29 while satisfying
    # cargo-mobile2's minimum NDK and AGP 9's toolchain expectations.
    includeNDK = true;
    ndkVersion = "27.0.12077973";
    includeCmake = false;
    includeEmulator = false;
    includeSystemImages = false;
  };

  androidSdk = "${androidPackages.androidsdk}/libexec/android-sdk";
  ndk = "${androidSdk}/ndk/27.0.12077973";
  ndkHost = if pkgs.stdenv.hostPlatform.isDarwin then "darwin-x86_64" else "linux-x86_64";
  ndkToolchain = "${ndk}/toolchains/llvm/prebuilt/${ndkHost}/bin";
  api = "26"; # Keep in sync with [android].min-sdk-version in mobile/mobile.toml.

  androidRustTargets = [
    "aarch64-linux-android"
    "armv7-linux-androideabi"
    "i686-linux-android"
    "x86_64-linux-android"
  ];

  # cargo-mobile2 invokes cargo with an explicit environment, and Gradle's
  # Rust plugin invokes `cargo android build` through PATH. These names match
  # both the Cargo target triples and the cc-rs cross-compilation convention.
  targetEnv = {
    "CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER" = "${ndkToolchain}/aarch64-linux-android${api}-clang";
    "CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_LINKER" = "${ndkToolchain}/armv7a-linux-androideabi${api}-clang";
    "CARGO_TARGET_I686_LINUX_ANDROID_LINKER" = "${ndkToolchain}/i686-linux-android${api}-clang";
    "CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER" = "${ndkToolchain}/x86_64-linux-android${api}-clang";

    "CC_aarch64_linux_android" = "${ndkToolchain}/aarch64-linux-android${api}-clang";
    "CC_armv7_linux_androideabi" = "${ndkToolchain}/armv7a-linux-androideabi${api}-clang";
    "CC_i686_linux_android" = "${ndkToolchain}/i686-linux-android${api}-clang";
    "CC_x86_64_linux_android" = "${ndkToolchain}/x86_64-linux-android${api}-clang";

    "CXX_aarch64_linux_android" = "${ndkToolchain}/aarch64-linux-android${api}-clang++";
    "CXX_armv7_linux_androideabi" = "${ndkToolchain}/armv7a-linux-androideabi${api}-clang++";
    "CXX_i686_linux_android" = "${ndkToolchain}/i686-linux-android${api}-clang++";
    "CXX_x86_64_linux_android" = "${ndkToolchain}/x86_64-linux-android${api}-clang++";

    "AR_aarch64_linux_android" = "${ndkToolchain}/llvm-ar";
    "AR_armv7_linux_androideabi" = "${ndkToolchain}/llvm-ar";
    "AR_i686_linux_android" = "${ndkToolchain}/llvm-ar";
    "AR_x86_64_linux_android" = "${ndkToolchain}/llvm-ar";
  };
in
pkgs.mkShell (
  {
    packages =
      with pkgs;
      [
        androidPackages.androidsdk
        cargo-mobile2
        jdk17
        cargo
        rustc
        rustfmt
        clippy
        rustup
        pkg-config
        git
        curl
        unzip
        zip
        which
      ]
      ++ lib.optionals pkgs.stdenv.hostPlatform.isLinux [
        # Gradle and the Android tools expect a normal Linux userland.
        glibcLocales
      ];

    ANDROID_HOME = androidSdk;
    ANDROID_SDK_ROOT = androidSdk;
    ANDROID_NDK_HOME = ndk;
    ANDROID_NDK_ROOT = ndk;
    NDK_HOME = ndk;
    JAVA_HOME = "${pkgs.jdk17}";

    # cargo-mobile2 also reads these from the generated .cargo/config.toml,
    # but exporting them keeps direct `cargo build --target ...` usable.
    # (The `// targetEnv` below spreads linker/cc settings into the shell.)

    shellHook = ''
      export PATH="${androidSdk}/platform-tools:${androidSdk}/build-tools/37.0.0:${androidSdk}/cmdline-tools/latest/bin:$HOME/.cargo/bin:$PATH"

      # Prefer a rustup-managed toolchain: Nix's rustc only ships the host
      # target, while Android builds need target standard libraries.
      if [ "''${RSXP_ANDROID_SKIP_RUSTUP:-0}" != "1" ]; then
        if ! rustup toolchain list 2>/dev/null | grep -q '^stable'; then
          echo "android-shell: installing stable Rust toolchain via rustup..."
          rustup toolchain install stable --profile minimal
        fi
        if ! rustup target list --installed --toolchain stable 2>/dev/null | grep -qx 'aarch64-linux-android'; then
          echo "android-shell: installing Rust Android targets..."
          rustup target add --toolchain stable ${lib.concatStringsSep " " androidRustTargets}
        fi
      fi

      echo "RSXP Android shell"
      echo "  ANDROID_HOME=$ANDROID_HOME"
      echo "  NDK_HOME=$NDK_HOME"
      echo "  JAVA_HOME=$JAVA_HOME"
      echo "  cargo: $(command -v cargo || echo missing)"
      echo "  cargo-mobile: $(command -v cargo-mobile || echo missing)"
      echo "  Next: cd mobile && cargo mobile init --skip-targets-install"
    '';
  }
  // targetEnv
)
