#!/usr/bin/env bash
# Build the Linux AppImage with a normal (non-Nix) toolchain.
#
# Run inside the image from packaging/appimage/Dockerfile with the repository at
# the working directory (the CI does exactly this), or directly on a Debian 11
# host that has Rust, Ruby 3.4, appimagetool and the development libraries
# listed in the Dockerfile installed.
#
# The build links against the host's glibc, so run it on the oldest supported
# distribution. Debian 11's glibc 2.31 covers Ubuntu 20.04+ and everything newer.
#
# Usage: tools/build-appimage.sh [output-directory]
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output="${1:-$root/artifacts}"
target="${CARGO_TARGET_DIR:-$root/target}"
appimagetool="${APPIMAGETOOL:-/opt/appimagetool/unpacked/AppRun}"

if [[ ! -x "$appimagetool" ]]; then
  echo "appimagetool not found at $appimagetool (set APPIMAGETOOL)" >&2
  exit 1
fi

mkdir -p "$output"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

echo "Building fanplay (release)..."
cargo build --locked --release --bin fanplay --manifest-path "$root/Cargo.toml"

echo "Staging AppDir with Ruby and native dependencies..."
python3 "$root/tools/package-runtime.py" "$target/release/fanplay" "$work/AppDir"

# Files the AppImage format expects at the AppDir root.
install -m 0755 "$root/packaging/appimage/AppRun" "$work/AppDir/AppRun"
install -m 0644 "$root/packaging/fanplay.desktop" "$work/AppDir/fanplay.desktop"
install -m 0644 "$root/packaging/fanplay.png" "$work/AppDir/fanplay.png"

echo "Building AppImage..."
out="$output/fanplay-linux-x86_64.AppImage"
rm -f "$out"
ARCH=x86_64 "$appimagetool" --no-appstream "$work/AppDir" "$out"
echo "Wrote $out"
