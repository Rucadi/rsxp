#!/usr/bin/env bash
# Cross-build CRuby as a static library for the selected Android ABI.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
sdk="${ANDROID_NDK_HOME:?Enter shell-droid.nix first}"
version="${RUBY_ANDROID_VERSION:-3.4.7}"
build_stamp="$version-android-stdlib-monitor-json-encoding-v1"
work="$root/android/ruby-build"
mkdir -p "$work/src" "$work/out"
if [[ ! -f "$work/src/ruby-$version.tar.gz" ]]; then
  curl -fL "https://cache.ruby-lang.org/pub/ruby/3.4/ruby-$version.tar.gz" -o "$work/src/ruby-$version.tar.gz"
fi
if [[ ! -d "$work/src/ruby-$version" ]]; then tar -C "$work/src" -xzf "$work/src/ruby-$version.tar.gz"; fi
src="$work/src/ruby-$version"
host="$(uname -m)-linux-gnu"
for abi in ${ANDROID_ABIS:-arm64-v8a x86_64}; do
  case "$abi" in
    arm64-v8a) triple=aarch64-linux-android; rust_target=aarch64-linux-android ;;
    x86_64) triple=x86_64-linux-android; rust_target=x86_64-linux-android ;;
    *) echo "Unsupported ABI $abi" >&2; exit 2 ;;
  esac
  build="$work/build-$abi"; prefix="$work/out/$abi"
  tool="$sdk/toolchains/llvm/prebuilt/linux-x86_64/bin"
  if [[ ! -f "$prefix/lib/libruby-static.a" || ! -f "$prefix/.built-version" || "$(<"$prefix/.built-version")" != "$build_stamp" ]]; then
    rm -rf "$build" "$prefix"
    mkdir -p "$build" "$prefix"
  export CC="$tool/${triple}23-clang" AR="$tool/llvm-ar" RANLIB="$tool/llvm-ranlib" STRIP="$tool/llvm-strip"
  export CFLAGS="-O2 -fPIC" CPPFLAGS="-DNO_LOCALE_CHARMAP=1" LDFLAGS="-fPIC"
  (cd "$build" && "$src/configure" --build="$host" --host="$triple" --prefix="$prefix" \
    --with-baseruby="$(command -v ruby)" --disable-shared --enable-static --with-static-linked-ext \
    --disable-install-doc --disable-yjit --disable-install-capi --without-openssl --without-readline \
    --without-gdbm --without-tk --without-dbm --with-ext=zlib,json,json/parser,json/generator,monitor)
  make -C "$build" -j"$(nproc)" install
  echo "$build_stamp" > "$prefix/.built-version"
  fi
  # CRuby's static extension build leaves extension object archives beside
  # libruby-static.a. The normal Ruby executable links those objects itself,
  # but our Magnus host links only libruby-static.a. Fold configured extension
  # objects into that archive so require can resolve built-ins such as
  # monitor.so and json/ext/generator.so in the embedded runtime.
  archive_symbols="$("$tool/llvm-nm" -A -g "$prefix/lib/libruby-static.a")"
  if ! grep -q ' T Init_monitor$' <<<"$archive_symbols"; then
    static_exts="$build/.android-static-exts"
    rm -rf "$static_exts"
    mkdir -p "$static_exts"
    index=0
    while IFS= read -r archive; do
      index=$((index + 1))
      extract="$static_exts/extract-$index"
      mkdir -p "$extract"
      (cd "$extract" && "$tool/llvm-ar" x "$archive")
      for object in "$extract"/*.o; do
        [[ -f "$object" ]] || continue
        mv "$object" "$static_exts/${index}-$(basename "$object")"
      done
      rmdir "$extract"
    done < <(find "$build/ext" -type f -name '*.a' -print | sort)
    if compgen -G "$static_exts/*.o" >/dev/null; then
      "$tool/llvm-ar" q "$prefix/lib/libruby-static.a" "$static_exts"/*.o
      "$tool/llvm-ranlib" "$prefix/lib/libruby-static.a"
    fi
  fi
  # The static library contains dmyext.o, a no-op Init_ext intended for
  # dynamically loaded extensions. Embedded Ruby needs the generated Init_ext
  # table so ruby_init_ext registers the configured built-ins at startup.
  archive_symbols="$("$tool/llvm-nm" -A "$prefix/lib/libruby-static.a")"
  if grep -q ':dmyext.o:.*Init_ext' <<<"$archive_symbols"; then
    "$tool/llvm-ar" d "$prefix/lib/libruby-static.a" dmyext.o
    "$tool/llvm-ar" q "$prefix/lib/libruby-static.a" "$build/ext/extinit.o"
    "$tool/llvm-ranlib" "$prefix/lib/libruby-static.a"
  fi
  # ruby_options() normally calls the generated encoding initializer after
  # ruby_init(). Magnus embeds Ruby without ruby_options(), so replace the
  # dynamic-build no-op Init_enc with the statically generated encoding table.
  archive_symbols="$("$tool/llvm-nm" -A "$prefix/lib/libruby-static.a")"
  if grep -q ':dmyenc.o:.*Init_enc' <<<"$archive_symbols"; then
    "$tool/llvm-ar" d "$prefix/lib/libruby-static.a" dmyenc.o
    "$tool/llvm-ar" q "$prefix/lib/libruby-static.a" "$build/enc/encinit.o"
    "$tool/llvm-ranlib" "$prefix/lib/libruby-static.a"
  fi
  mkdir -p "$root/android/ruby/$abi"
  rm -rf "$root/android/ruby/$abi/stdlib"
  cp -a "$prefix/include/." "$root/android/ruby/$abi/"
  cp -a "$prefix/lib/." "$root/android/ruby/$abi/"
  mkdir -p "$root/android/ruby/$abi/stdlib"
  cp -a "$prefix/lib/ruby/3.4.0/." "$root/android/ruby/$abi/stdlib/"
  cp -a "$src/ext/monitor/lib/." "$root/android/ruby/$abi/stdlib/"
  # Ruby's default gems are installed separately from the core standard
  # library. The Android embed only adds the staged stdlib directories to
  # RUBYLIB, so stage their pure-Ruby libraries there too (notably monitor,
  # which is required by parts of the RGSS runtime).
  for gem_lib in "$prefix"/lib/ruby/gems/3.4.0/gems/*/lib; do
    [[ -d "$gem_lib" ]] || continue
    cp -a "$gem_lib/." "$root/android/ruby/$abi/stdlib/"
  done
  rbconfig="$(find "$prefix/lib/ruby/3.4.0" -name rbconfig.rb -print -quit)"
  if [[ -z "$rbconfig" ]]; then echo "Target Ruby RbConfig was not installed for $abi" >&2; exit 1; fi
  ruby --disable-gems -e 'require "shellwords"; module RbConfig; remove_const(:CONFIG) if const_defined?(:CONFIG, false); remove_const(:MAKEFILE_CONFIG) if const_defined?(:MAKEFILE_CONFIG, false); end; load ARGV.fetch(0); RbConfig::CONFIG.each { |k,v| puts "export RBCONFIG_#{k}=#{Shellwords.escape(v.to_s)}" }' "$rbconfig" > "$root/android/ruby/$abi/rbconfig.env"
  cat >> "$root/android/ruby/$abi/rbconfig.env" <<EOF
export RBCONFIG_CROSS_COMPILING=yes
export RBCONFIG_target_os=android
export RBCONFIG_LIBRUBYARG_STATIC=-lruby-static\ -Wl,--no-as-needed\ -lz\ -Wl,--as-needed\ -ldl\ -lm
export RBCONFIG_libdir=$root/android/ruby/$abi
export RBCONFIG_includedir=$root/android/ruby/$abi/include/ruby-3.4.0
export RBCONFIG_archincludedir=$root/android/ruby/$abi/include/ruby-3.4.0/$triple
EOF
  echo "CRuby $version built for $abi at android/ruby/$abi"
done
