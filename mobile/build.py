#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""One entry point for shared assets, native core, and platform app builds."""
import argparse
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import sys
import zipfile

ROOT = Path(__file__).resolve().parent.parent
MOBILE = ROOT / "mobile"
BUILD = MOBILE / "build"

def run(*command, env=None, cwd=ROOT):
    print("==>", *map(str, command), flush=True)
    subprocess.run(list(map(str,command)), cwd=cwd, env=env, check=True)

def patch_elf(tool, *arguments):
    subprocess.run([tool,*map(str,arguments)], check=True)

RUBY_ANDROID_VERSION='0.1.5'
RUBY_ANDROID_SHA256='81a8beb55a352eb7cb726e4e71bf6be40fe7e0aeed7819233a6c8ce70bdd40d4'

def android_ruby():
    """Provision the Termux-patched MRI build for Android arm64.

    The release contains a shared libruby, its headers and the standard
    library, but no static archive. The Rust core links the shared library and
    the APK stages the runtime library and its small dependency set next to the
    JNI library.
    """
    prefix=BUILD/f'ruby-android-{RUBY_ANDROID_VERSION}'
    archive=BUILD/f'ruby-android-{RUBY_ANDROID_VERSION}.zip'
    ready=prefix/'.rsxp-ready'
    if not ready.is_file():
        BUILD.mkdir(parents=True,exist_ok=True)
        if not archive.is_file():
            url=f'https://github.com/jekyllex/ruby-android/releases/download/v{RUBY_ANDROID_VERSION}/ruby-aarch64.zip'
            run('curl','-fL',url,'-o',archive)
        if hashlib.sha256(archive.read_bytes()).hexdigest()!=RUBY_ANDROID_SHA256:
            raise SystemExit('Android Ruby checksum mismatch')
        temporary=BUILD/f'ruby-android-{RUBY_ANDROID_VERSION}.tmp'
        if temporary.exists(): shutil.rmtree(temporary)
        temporary.mkdir()
        with zipfile.ZipFile(archive) as bundle:
            for member in bundle.infolist():
                path=(temporary/member.filename).resolve()
                if not path.is_relative_to(temporary.resolve()): raise SystemExit('Invalid Android Ruby archive entry')
            bundle.extractall(temporary)
        # zipfile cannot restore the Termux bootstrap's symlinks. They matter
        # for the Ruby standard library and shared-library lookup.
        links=temporary/'SYMLINKS.txt'
        if links.is_file():
            for line in links.read_text().splitlines():
                if '←' not in line: continue
                target,link=line.split('←',1)
                link_path=(temporary/link.removeprefix('./')).resolve()
                if not link_path.is_relative_to(temporary.resolve()): raise SystemExit('Invalid Android Ruby symlink')
                link_path.parent.mkdir(parents=True,exist_ok=True)
                if link_path.is_symlink() or link_path.exists(): link_path.unlink()
                link_path.symlink_to(target)
        # Drop dangling links from tools that are irrelevant to the embedded
        # runtime (for example lib/terminfo -> ../share/terminfo).
        for link in [path for path in temporary.rglob('*') if path.is_symlink()]:
            if not link.exists(): link.unlink()
        if prefix.exists(): shutil.rmtree(prefix)
        temporary.rename(prefix)
        ready.touch()
    # Android's APK packager only ships names ending in .so, and the loader
    # resolves the SONAME. Normalise the versioned Termux libraries once, and
    # remove the libz version suffix from extensions that expect libz.so.1.
    patchelf=shutil.which('patchelf')
    if not patchelf: raise SystemExit('Android builds need patchelf (provided by shell-droid.nix)')
    for link in [path for path in prefix.rglob('*') if path.is_symlink()]:
        if not link.exists(): link.unlink()
    library=prefix/'lib'
    ruby_versioned=library/'libruby.so.3.3.4'
    if ruby_versioned.is_file():
        ruby_link=library/'libruby.so'
        if ruby_link.is_symlink(): ruby_link.unlink()
        shutil.copy2(ruby_versioned,ruby_link)
        patch_elf(patchelf,'--set-soname','libruby.so',ruby_link)
        patch_elf(patchelf,'--replace-needed','libz.so.1','libz.so',ruby_link)
    zlib=library/'libz.so.1.3.2'
    if zlib.is_file():
        zlib_link=library/'libz.so'
        if zlib_link.is_symlink(): zlib_link.unlink()
        shutil.copy2(zlib,zlib_link)
        patch_elf(patchelf,'--set-soname','libz.so',zlib_link)
    # OpenSSL extension dependencies use versioned names that Android's APK
    # packager will not accept. Stage unversioned copies and rewrite the links.
    crypto=library/'libcrypto.so.3'
    if crypto.is_file():
        crypto_link=library/'libcrypto.so'
        if crypto_link.is_symlink(): crypto_link.unlink()
        shutil.copy2(crypto,crypto_link)
        patch_elf(patchelf,'--set-soname','libcrypto.so',crypto_link)
    ssl=library/'libssl.so.3'
    if ssl.is_file():
        ssl_link=library/'libssl.so'
        if ssl_link.is_symlink(): ssl_link.unlink()
        shutil.copy2(ssl,ssl_link)
        patch_elf(patchelf,'--set-soname','libssl.so',ssl_link)
        patch_elf(patchelf,'--replace-needed','libcrypto.so.3','libcrypto.so',ssl_link)
    for extension in (library/'ruby/3.3.0').rglob('*.so'):
        if extension.is_symlink(): continue
        try:
            data = extension.read_bytes()
            replace = []
            if b'libruby.so.3.3' in data:
                replace += ['--replace-needed','libruby.so.3.3','libruby.so']
            if b'libz.so.1' in data:
                replace += ['--replace-needed','libz.so.1','libz.so']
            if b'libcrypto.so.3' in data:
                replace += ['--replace-needed','libcrypto.so.3','libcrypto.so']
            if b'libssl.so.3' in data:
                replace += ['--replace-needed','libssl.so.3','libssl.so']
            if replace:
                patch_elf(patchelf,*replace,extension)
        except (OSError,subprocess.CalledProcessError):
            pass
    if not (library/'libruby.so').is_file():
        raise SystemExit('Android Ruby archive has no libruby shared library')
    return prefix

def native_ruby(target=None):
    import tarfile
    if target and 'android' in target:
        supplied=os.environ.get('RSXP_RUBY_PREFIX')
        if supplied: return Path(supplied)
        return android_ruby()
    version='4.0.701'
    expected='e364fce6fcf6649d9f42d9094d960fcf6c8c7a4a2431b45f87aa2fa08fa2a9ae'
    archive=BUILD/f'CRuby_prebuilt-{version}.tar.gz'
    prefix=BUILD/'cruby-native/CRuby'
    BUILD.mkdir(parents=True,exist_ok=True)
    if not archive.is_file():
        run('curl','-fL',f'https://github.com/xord/cruby/releases/download/v{version}/{archive.name}','-o',archive)
    if hashlib.sha256(archive.read_bytes()).hexdigest()!=expected: raise SystemExit('Native Ruby checksum mismatch')
    if not (prefix/'include/ruby.h').is_file():
        destination=prefix.parent; destination.mkdir(parents=True,exist_ok=True)
        with tarfile.open(archive) as tar:
            for member in tar.getmembers():
                path=(destination/member.name).resolve()
                if not path.is_relative_to(destination.resolve()) or member.issym() or member.islnk(): raise SystemExit('Invalid native Ruby archive entry')
            tar.extractall(destination)
    return prefix

def assets(runtime=None):
    runtime=runtime or native_ruby()
    destination=BUILD/'ui'
    if destination.exists(): shutil.rmtree(destination)
    shutil.copytree(MOBILE/'ui',destination)
    output=BUILD/'runtime'
    if output.exists(): shutil.rmtree(output)
    shutil.copytree(runtime/'lib',output/'lib')
    notices=output/'licenses'; notices.mkdir(parents=True,exist_ok=True)
    for source in (MOBILE/'ruby').iterdir():
        if source.name != 'bridge.c': shutil.copy2(source,notices/source.name)
    for directory in (destination,output):
        if any(directory.rglob('*.wasm')):
            raise SystemExit('Mobile apps require a native game runtime; refusing to package WebAssembly.')


def core(target=None, environment=None):
    environment = dict(environment or os.environ)
    command = ["cargo", "build", "--release", "--manifest-path", str(MOBILE / "core/Cargo.toml")]
    if target:
        command += ["--target",target]
        sysroot = Path(subprocess.check_output(["rustc","--print","sysroot"],text=True).strip())
        if not (sysroot / "lib/rustlib" / target / "lib").is_dir():
            # Keep mobile targets in a local rustup toolchain. Nix's desktop
            # rustc does not ship the iOS/Android standard libraries.
            rustup = shutil.which("rustup")
            if not rustup and shutil.which("nix-build"):
                package = subprocess.check_output(["nix-build", "<nixpkgs>", "-A", "rustup", "--no-out-link"],text=True).strip().splitlines()[-1]
                rustup = str(Path(package) / "bin/rustup")
            if not rustup:
                raise SystemExit("Install rustup or use nix-shell so the mobile target toolchain can be provisioned automatically.")
            environment["RUSTUP_HOME"] = str(BUILD / "rustup")
            for name in ["RUSTC","RUSTDOC","RUSTC_WRAPPER","RUSTC_WORKSPACE_WRAPPER"]:
                environment.pop(name,None)
            probe = subprocess.run([rustup,"run","stable","rustc","--print","target-libdir","--target",target],env=environment,text=True,capture_output=True)
            if probe.returncode or not Path(probe.stdout.strip()).is_dir():
                run(rustup,"toolchain","install","stable","--profile","minimal","--target",target,env=environment)
            # Nix cargo/rustc wrappers can select a different sysroot even when
            # invoked through rustup. Use the selected compiler binaries directly.
            def tool(name):
                return subprocess.check_output([rustup,"which","--toolchain","stable",name],env=environment,text=True).strip()
            environment["RUSTC"] = tool("rustc")
            environment["RUSTDOC"] = tool("rustdoc")
            command[0] = tool("cargo")
            if sys.platform=="darwin":
                environment["CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER"]="/usr/bin/cc"
                environment["CARGO_TARGET_X86_64_APPLE_DARWIN_LINKER"]="/usr/bin/cc"
        # Build scripts and proc macros link for the host. Prefer the system
        # linker over Nix's clang wrapper, which can pick up a different
        # libLLVM through wrapper environment variables.
        for name in ["RUSTFLAGS","CARGO_ENCODED_RUSTFLAGS","NIX_LDFLAGS","NIX_CFLAGS_COMPILE","NIX_CFLAGS_LINK","LD","CC","CXX","AR","AS","NM","RANLIB","STRIP","DYLD_LIBRARY_PATH"]:
            environment.pop(name,None)
        if sys.platform=="darwin":
            environment["CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER"]="/usr/bin/cc"
            environment["CARGO_TARGET_X86_64_APPLE_DARWIN_LINKER"]="/usr/bin/cc"
    environment["RSXP_RUBY_PREFIX"] = str(native_ruby(target))
    run(*command,env=environment)
    return MOBILE / "core/target" / (target or "") / "release"

def ios(device=False, launch=False, debug=False):
    configuration = "Debug" if debug else "Release"
    environment = dict(os.environ)
    developer = Path(environment.get("DEVELOPER_DIR", "/Applications/Xcode.app/Contents/Developer"))
    if not (developer / "usr/bin/xcodebuild").is_file():
        developer = Path("/Applications/Xcode.app/Contents/Developer")
    environment["DEVELOPER_DIR"] = str(developer)
    xcodebuild = str(developer / "usr/bin/xcodebuild")
    xcrun = "/usr/bin/xcrun"
    try:
        run(xcodebuild,"-checkFirstLaunchStatus",env=environment)
    except (OSError,subprocess.CalledProcessError):
        raise SystemExit("Xcode is not ready. Open Xcode, accept its license, and complete first-launch setup before building iOS.")
    sdk = "iphoneos" if device else "iphonesimulator"
    target = "aarch64-apple-ios" if device else "aarch64-apple-ios-sim"
    try:
        environment["SDKROOT"] = subprocess.check_output([xcrun,"--sdk",sdk,"--show-sdk-path"],env=environment,text=True).strip()
    except subprocess.CalledProcessError:
        raise SystemExit("Xcode is not ready. Open Xcode, accept its license, and complete first-launch setup before building iOS.")
    # Avoid Nix's macOS compiler wrapper when linking iOS libraries.
    clang = str(Path(environment["DEVELOPER_DIR"]) / "Toolchains/XcodeDefault.xctoolchain/usr/bin/clang")
    environment["PATH"] = str(Path(clang).parent) + os.pathsep + "/usr/bin" + os.pathsep + environment["PATH"]
    for name in ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "NIX_LDFLAGS", "NIX_CFLAGS_COMPILE", "NIX_CFLAGS_LINK", "LD", "CC", "CXX", "AR", "AS", "NM", "RANLIB", "STRIP"]:
        environment.pop(name,None)
    environment["CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER"] = clang
    environment["CC_" + target.replace("-","_")] = clang
    environment["CARGO_TARGET_" + target.upper().replace("-","_") + "_LINKER"] = clang
    environment["IPHONEOS_DEPLOYMENT_TARGET"] = "16.0"
    assets()
    library = core(target,environment)
    destination=BUILD / sdk; destination.mkdir(parents=True,exist_ok=True)
    shutil.copy2(library / "librsxp_mobile.a",destination)
    command = [xcodebuild,"-project",str(MOBILE / "ios/RSXP.xcodeproj"),"-scheme","RSXP","-configuration",configuration,"-sdk",sdk,"-derivedDataPath",str(BUILD / "ios"),"ARCHS=arm64","ONLY_ACTIVE_ARCH=YES","build"]
    if not device: command += ["-destination","generic/platform=iOS Simulator","CODE_SIGNING_ALLOWED=NO"]
    run(*command,env=environment)
    if launch:
        if device: raise SystemExit("Open the Xcode project and select your signed device to install.")
        import json
        devices=json.loads(subprocess.check_output([xcrun,"simctl","list","devices","available","--json"],env=environment,text=True))
        available=[d for group in devices["devices"].values() for d in group if d.get("isAvailable") and "iPhone" in d["name"]]
        if not available:
            runtimes=json.loads(subprocess.check_output([xcrun,"simctl","list","runtimes","--json"],env=environment,text=True))["runtimes"]
            runtimes=[r for r in runtimes if r.get("isAvailable") and r["name"].startswith("iOS")]
            if not runtimes:
                run(xcodebuild,"-downloadPlatform","iOS","-architectureVariant","arm64",env=environment)
                runtimes=json.loads(subprocess.check_output([xcrun,"simctl","list","runtimes","--json"],env=environment,text=True))["runtimes"]
                runtimes=[r for r in runtimes if r.get("isAvailable") and r["name"].startswith("iOS")]
            if not runtimes: raise SystemExit("No available iOS runtime after download; check Xcode Settings → Platforms.")
            runtime=max(runtimes,key=lambda r:tuple(int(n) for n in r["version"].split(".")))
            types=json.loads(subprocess.check_output([xcrun,"simctl","list","devicetypes","--json"],env=environment,text=True))["devicetypes"]
            phones=[d for d in types if d.get("productFamily")=="iPhone" or d["name"].startswith("iPhone")]
            phone=next((d for d in phones if d["name"]=="iPhone 16 Pro"),phones[-1])
            udid=subprocess.check_output([xcrun,"simctl","create","RSXP iPhone",phone["identifier"],runtime["identifier"]],env=environment,text=True).strip()
            available=[{"udid":udid,"state":"Shutdown","name":"RSXP iPhone"}]
        device=next((d for d in available if d["state"]=="Booted"),available[0])
        if device["state"]!="Booted": run(xcrun,"simctl","boot",device["udid"],env=environment)
        developer=Path(environment["DEVELOPER_DIR"])
        choices=[developer/"Applications/Simulator.app",developer.parent/"Applications/Simulator.app",developer.parent/"Applications/DeviceHub.app"]
        simulator=next((p for p in choices if p.is_dir()),None)
        if not simulator: raise SystemExit("The installed Xcode has no Simulator or Device Hub app.")
        run("/usr/bin/open",str(simulator))
        run(xcrun,"simctl","bootstatus",device["udid"],"-b",env=environment)
        run(xcrun,"simctl","install",device["udid"],BUILD / f"ios/Build/Products/{configuration}-iphonesimulator/RSXP.app",env=environment)
        run(xcrun,"simctl","launch",device["udid"],"org.rsxp.mobile",env=environment)

def android():
    sdk=Path(os.environ.get("ANDROID_HOME",str(Path.home()/"Library/Android/sdk")))
    ndk=os.environ.get("ANDROID_NDK_HOME")
    if not ndk:
        versions=sorted((sdk/"ndk").glob("*"))
        if not versions: raise SystemExit("Install Android Studio's SDK, NDK, and CMake 3.22.1, then set ANDROID_HOME.")
        ndk=str(versions[-1])
    toolchains=list((Path(ndk)/"toolchains/llvm/prebuilt").glob("*"))
    if not toolchains: raise SystemExit("No NDK LLVM toolchain found")
    environment=dict(os.environ); environment["ANDROID_HOME"]=str(sdk)
    target="aarch64-linux-android"
    compiler=toolchains[0]/"bin/aarch64-linux-android26-clang"
    environment["CC_aarch64_linux_android"]=str(compiler)
    environment["CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER"]=str(compiler)
    runtime=native_ruby(target)
    assets(runtime)
    # RGSS games do not need RubyGems, IRB, RDoc or Bundler in the APK. Drop
    # those trees to keep the packaged runtime small; the core stdlib and
    # native extensions remain available.
    for relative in ["ruby/gems","ruby/3.3.0/rdoc","ruby/3.3.0/irb",
                     "ruby/3.3.0/bundler","ruby/3.3.0/rubygems","ruby/3.3.0/reline"]:
        shutil.rmtree(BUILD/"runtime/lib"/relative,ignore_errors=True)
    # Shared libraries at the top of lib/ are linked from the APK's native
    # library directory instead; only Ruby's stdlib and extension modules need
    # to be extracted from assets.
    for shared in (BUILD/"runtime/lib").glob("*.so*"):
        shared.unlink()
    android_assets=BUILD/"android/assets"
    if android_assets.exists(): shutil.rmtree(android_assets)
    android_assets.mkdir(parents=True)
    shutil.copytree(BUILD/"ui",android_assets/"ui")
    shutil.copytree(BUILD/"runtime",android_assets/"runtime")
    library=core(target,environment)
    # Rust cdylibs have no SONAME. Without one the JNI link records this absolute
    # build-host path as DT_NEEDED, which Android cannot resolve.
    patchelf=shutil.which('patchelf')
    if not patchelf: raise SystemExit('Android builds need patchelf (provided by shell-droid.nix)')
    patch_elf(patchelf,'--set-soname','librsxp_mobile.so',library/"librsxp_mobile.so")
    destination=BUILD/"android/arm64-v8a"; destination.mkdir(parents=True,exist_ok=True)
    shutil.copy2(library/"librsxp_mobile.so",destination)
    # Shared Ruby and its non-system dependencies must live in the APK's
    # native-library directory. Ruby extensions themselves are extracted from
    # assets at first launch (the manifest targets SDK 28 so the platform
    # permits loading them from app storage).
    jni=BUILD/"android/jniLibs/arm64-v8a"
    if jni.exists(): shutil.rmtree(jni)
    jni.mkdir(parents=True)
    shutil.copy2(library/"librsxp_mobile.so",jni/"librsxp_mobile.so")
    for name in ["libruby.so","libz.so","libandroid-support.so","libandroid-execinfo.so",
                 "libgmp.so","libffi.so","libyaml-0.so","libssl.so","libcrypto.so"]:
        source=runtime/"lib"/name
        if not source.is_file(): raise SystemExit(f"Android Ruby is missing lib/{name}")
        shutil.copy2(source,jni/name)
    gradle=shutil.which("gradle")
    if not gradle: raise SystemExit("Install Gradle 8.9+, or open mobile/android in Android Studio and choose Build APK.")
    run(gradle,"assembleDebug",cwd=MOBILE/"android",env=environment)

if __name__ == "__main__":
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("platform",choices=["ios","android","host","assets"])
    parser.add_argument("--run",action="store_true",help="Install and launch iOS Simulator")
    parser.add_argument("--device",action="store_true",help="Build iOS device core; requires an Xcode signing team")
    parser.add_argument("--debug",action="store_true",help="Build the iOS app in Debug (enables the --game/--demo debug launch arguments)")
    args=parser.parse_args()
    if args.platform=="ios": ios(args.device,args.run,args.debug)
    elif args.platform=="android": android()
    elif args.platform=="assets": assets()
    else: assets(); core()
