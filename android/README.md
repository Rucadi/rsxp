# Android APK build

Requirements: JDK 17+, Android SDK Platform 35, NDK 27+, Rust targets
`aarch64-linux-android` and `x86_64-linux-android`, `cargo-ndk`, Ruby build
tools, and network access to fetch the CRuby source archive on the first build.

From the repository root, enter the Nix shell and build:

```sh
NIXPKGS_ALLOW_UNFREE=1 NIXPKGS_ACCEPT_ANDROID_SDK_LICENSE=1 nix-shell shell-droid.nix
./tools/build-android.sh
```

Nix requires explicit acceptance of Google's Android SDK license. Review the
terms and set `NIXPKGS_ACCEPT_ANDROID_SDK_LICENSE=1` for the shell command to
accept them.

The APK is written to `android/app/build/outputs/apk/debug/app-debug.apk`.
Set `ANDROID_HOME` and `ANDROID_NDK_HOME` if the SDK is installed outside the
usual locations. The script accepts `ANDROID_ABIS="arm64-v8a x86_64"`.

The build cross-compiles CRuby 3.4.7 for each ABI and links its static library
into the NativeActivity library. No WebAssembly runtime is used. The launcher
uses Android's Storage Access Framework to retain access to the selected game
folder and reads game files on demand; it does not copy the game folder. Game
saves and writable files are kept in per-game app-private storage.
