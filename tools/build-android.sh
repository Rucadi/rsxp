#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
ndk="${ANDROID_NDK_HOME:-}"
if [[ -z "$sdk" || -z "$ndk" ]]; then
  echo "Set ANDROID_HOME and ANDROID_NDK_HOME to an installed Android SDK/NDK." >&2
  exit 2
fi
export ANDROID_HOME="$sdk" ANDROID_NDK_HOME="$ndk"
command -v cargo-ndk >/dev/null || { echo "Install cargo-ndk: cargo install cargo-ndk" >&2; exit 2; }
command -v gradle >/dev/null || { echo "Install Gradle 8.11+ and put gradle on PATH." >&2; exit 2; }
abis="${ANDROID_ABIS:-arm64-v8a x86_64}"
mkdir -p "$root/android/app/src/main/jniLibs"
"$root/tools/build-ruby-android.sh"
for abi in $abis; do
  case "$abi" in
    arm64-v8a) target=aarch64-linux-android ;;
    x86_64) target=x86_64-linux-android ;;
    *) echo "Unsupported ABI: $abi" >&2; exit 2 ;;
  esac
  rustup target add "$target"
  # rb-sys must see the target-built CRuby configuration. This links the
  # interpreter statically into the NativeActivity library for this ABI.
  source "$root/android/ruby/$abi/rbconfig.env"
  export RUBY_STATIC=true RUBY_VERSION=3.4.7
  cargo ndk -t "$abi" -o "$root/android/app/src/main/jniLibs" build --manifest-path "$root/Cargo.toml" --release --no-default-features --features android --lib
done
cd "$root/android"
gradle --no-daemon assembleDebug
echo "APK: $root/android/app/build/outputs/apk/debug/app-debug.apk"
