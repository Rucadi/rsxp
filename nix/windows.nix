
{ pkgs }:

let
  root = ../.;
  crossPkgs = pkgs.pkgsCross.mingwW64;

  ruby = pkgs.callPackage ./ruby-mingw64.nix {};
  gmp = crossPkgs.gmp;
  hostRuby = pkgs.buildPackages.ruby_4_0;

  target = "x86_64-pc-windows-gnu";

  rubyInclude = "${ruby}/include/ruby-4.0.0";
  rubyArchInclude = "${rubyInclude}/x64-mingw32";

  crossCC = crossPkgs.stdenv.cc;
  mingwRuntime = crossPkgs.windows.mingw_w64;

in
crossPkgs.rustPlatform.buildRustPackage {
  pname = "rsxp";
  version = "0.1.0";

  src = root;

  cargoLock.lockFile = root + "/Cargo.lock";

  CARGO_BUILD_TARGET = target;

  cargoBuildFlags = [
    "-p"
    "rsxp-desktop"
    "--bin"
    "rsxp"
    "--target"
    target
  ];

  cargoInstallFlags = [
    "-p"
    "rsxp-desktop"
    "--bin"
    "rsxp"
    "--target"
    target
  ];

  buildInputs = [
    crossPkgs.windows.pthreads
  ];

  nativeBuildInputs = with pkgs.buildPackages; [
    pkg-config
    rustPlatform.bindgenHook
    hostRuby
  ];

  preBuild = ''
    # Host Ruby executes rb-sys build scripts.
    export RUBY="${hostRuby}/bin/ruby"

    # Target Ruby configuration.
    export RBCONFIG_CROSS_COMPILING=yes
    export RBCONFIG_target_os=mingw
    export RBCONFIG_arch=x64-mingw32

    # Ruby version configuration.
    export RBCONFIG_RUBY_PROGRAM_VERSION="4.0.0"
    export RBCONFIG_MAJOR="4"
    export RBCONFIG_MINOR="0"
    export RBCONFIG_TEENY="0"
    export RBCONFIG_PATCHLEVEL="0"
    export RBCONFIG_ruby_version="4.0.0"

    # Target Ruby headers.
    export RBCONFIG_rubyhdrdir="${rubyInclude}"
    export RBCONFIG_rubyarchhdrdir="${rubyArchInclude}"

    # Target Ruby libraries.
    export RBCONFIG_libdir="${ruby}/lib"
    export RBCONFIG_archlibdir="${ruby}/lib"

    export RBCONFIG_LIBRUBYARG_SHARED="-L${ruby}/lib -lx64-msvcrt-ruby400"

    # C header search paths.
    export C_INCLUDE_PATH="${rubyInclude}:${rubyArchInclude}''${C_INCLUDE_PATH:+:$C_INCLUDE_PATH}"

    # Library search paths.
    export LIBRARY_PATH="${ruby}/lib''${LIBRARY_PATH:+:$LIBRARY_PATH}"

    # Locate the MinGW cross compiler.
    cross_gcc="${crossCC}/bin/${crossCC.targetPrefix}gcc"

    if [ ! -x "$cross_gcc" ]; then
      echo "MinGW cross compiler not found: $cross_gcc" >&2
      exit 1
    fi

    # Extract the compiler's target system include paths. Do not pass GCC's
    # own headers to Clang: its intrinsic headers are not compatible with
    # Clang's builtins. Clang supplies its own intrinsic headers.
    mingw_includes="$(
      "$cross_gcc" -E -x c - -v </dev/null 2>&1 |
      sed -n '/#include <...> search starts here:/,/End of search list./p' |
      sed '1d;$d;s/^[[:space:]]*//' |
      grep -v '/lib/gcc/'
    )"

    # Construct Clang arguments using the same Windows headers
    # as the MinGW GCC cross compiler.
    bindgen_args="--target=x86_64-w64-windows-gnu -nostdlibinc"

    bindgen_args="$bindgen_args -I${rubyInclude}"
    bindgen_args="$bindgen_args -I${rubyArchInclude}"

    while IFS= read -r include_dir; do
      if [ -n "$include_dir" ] && [ -d "$include_dir" ]; then
        bindgen_args="$bindgen_args -isystem $include_dir"
      fi
    done <<< "$mingw_includes"

    # Replace host-oriented bindgenHook arguments.
    export BINDGEN_EXTRA_CLANG_ARGS="$bindgen_args"

    echo "BINDGEN_EXTRA_CLANG_ARGS=$BINDGEN_EXTRA_CLANG_ARGS"

    # Rust linker search path.
    export RUSTFLAGS="-L native=${ruby}/lib ''${RUSTFLAGS:-}"
  '';

  doCheck = false;
  dontPatchShebangs = true;

  postInstall = ''
    mkdir -p "$out/bin"

    # Windows executable.
    if [ ! -f "$out/bin/rsxp.exe" ]; then
      cp "target/${target}/release/rsxp.exe" "$out/bin/"
    fi

    # Ruby runtime DLL.
    for dll in ${ruby}/bin/*ruby*.dll; do
      if [ -f "$dll" ]; then
        cp "$dll" "$out/bin/"
      fi
    done

    # Ruby's MSYS2 DLL imports this MinGW GMP runtime.
    cp ${gmp}/bin/libgmp-10.dll "$out/bin/"

    # Bundle the GNU runtime DLLs required by the MinGW-linked executable.
    # These are available in the compiler's target runtime; Windows system
    # DLLs remain provided by Windows/Wine.
    for dll in \
      ${mingwRuntime}/bin/libgcc_s_seh-1.dll \
      ${mingwRuntime}/bin/libwinpthread-1.dll \
      ${mingwRuntime}/bin/libstdc++-6.dll \
      ${crossPkgs.windows.pthreads}/bin/libwinpthread-1.dll; do
      if [ -f "$dll" ]; then
        cp "$dll" "$out/bin/"
      fi
    done

    # Ruby's extension modules and standard library are shipped next to the
    # executable. Keep any DLLs from Ruby in the same loader-search directory.
    find ${ruby}/bin ${ruby}/lib -type f -iname '*.dll' -exec cp -n {} "$out/bin/" \; 2>/dev/null || true

    # Ruby standard library, next to the executable and its runtime DLLs.
    # bundled_runtime discovers this as bin/ruby/lib/ruby at startup.
    mkdir -p "$out/bin/ruby/lib"
    cp -r ${ruby}/lib/ruby "$out/bin/ruby/lib/"
  '';

  meta = with pkgs.lib; {
    description = "RSXP Windows desktop application";
    platforms = [ "x86_64-windows" ];
  };
}
