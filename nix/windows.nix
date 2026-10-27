
{ pkgs }:

let
  root = ../.;
  crossPkgs = pkgs.pkgsCross.mingwW64;


  target = "x86_64-pc-windows-gnu";


  crossCC = crossPkgs.stdenv.cc;
  mingwRuntime = crossPkgs.windows.mingw_w64;

  # webview2-com-sys links the MinGW build to the WebView2 loader DLL. The
  # crate ships the matching architecture-specific loader beside its import
  # libraries, but Cargo only consumes the latter during linking.
  webview2ComSys = pkgs.fetchurl {
    url = "https://crates.io/api/v1/crates/webview2-com-sys/0.39.1/download";
    hash = "sha256-s6BxMndRF9YGWFPZ0ReBV7jJDiKN5HEp1rzix+3r7fs=";
  };

in
crossPkgs.rustPlatform.buildRustPackage {
  pname = "fanplay";
  version = "0.1.0";

  src = root;

  cargoLock = {
    lockFile = root + "/Cargo.lock";
    outputHashes = {
      "arm-decoder-0.0.0" = "sha256-HG8Gh5H7ez1ad6VtWNCKcQ6ywA1yguwrMeFL5ZbKyIE=";
      "dust-core-0.0.0" = "sha256-BT/AofCPtDt22l0G/yW2bvDG+QqP2Po/8Q17Lp3lwIs=";
      "arm7tdmi-0.1.0" = "sha256-wRsghc7uRlv+fUh7hpibQ7ajw+3kasEqdHTV+1N2QnI=";
      "rustboyadvance-core-0.1.0" = "sha256-wRsghc7uRlv+fUh7hpibQ7ajw+3kasEqdHTV+1N2QnI=";
      "rustboyadvance-utils-0.1.0" = "sha256-wRsghc7uRlv+fUh7hpibQ7ajw+3kasEqdHTV+1N2QnI=";
    };
  };

  CARGO_BUILD_TARGET = target;

  cargoBuildFlags = [
    "-p"
    "fanplay-desktop"
    "--bin"
    "fanplay"
    "--target"
    target
  ];

  cargoInstallFlags = [
    "-p"
    "fanplay-desktop"
    "--bin"
    "fanplay"
    "--target"
    target
  ];

  # Dust uses nightly-only language features and is in the desktop defaults.
  RUSTC_BOOTSTRAP = "1";

  buildInputs = [
    crossPkgs.windows.pthreads
  ];

  nativeBuildInputs = with pkgs.buildPackages; [
    pkg-config
    rustPlatform.bindgenHook
  ];

  preBuild = ''
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


    while IFS= read -r include_dir; do
      if [ -n "$include_dir" ] && [ -d "$include_dir" ]; then
        bindgen_args="$bindgen_args -isystem $include_dir"
      fi
    done <<< "$mingw_includes"

    # Replace host-oriented bindgenHook arguments.
    export BINDGEN_EXTRA_CLANG_ARGS="$bindgen_args"

    echo "BINDGEN_EXTRA_CLANG_ARGS=$BINDGEN_EXTRA_CLANG_ARGS"

  '';

  doCheck = false;
  dontPatchShebangs = true;

  postInstall = ''
    mkdir -p "$out/bin"

    # Windows executable.
    if [ ! -f "$out/bin/fanplay.exe" ]; then
      cp "target/${target}/release/fanplay.exe" "$out/bin/"
    fi

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

    # webview2-com-sys uses the dynamic WebView2 loader for MinGW. Ship the
    # x64 loader next to fanplay.exe so Windows can resolve it from the portable
    # distribution directory.
    webview2_src="$TMPDIR/webview2-com-sys"
    mkdir -p "$webview2_src"
    tar -xzf ${webview2ComSys} -C "$webview2_src" --strip-components=1
    cp "$webview2_src/x64/WebView2Loader.dll" "$out/bin/"

  '';

  meta = with pkgs.lib; {
    description = "FANPLAY Windows desktop application";
    platforms = [ "x86_64-windows" ];
  };
}
