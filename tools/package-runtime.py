#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Stage a relocatable desktop, including Ruby and native dependency closure."""
import argparse
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import sys


def output(*args):
    return subprocess.check_output(args, text=True).strip()


def run(*args):
    subprocess.run(args, check=True)


def copy(source, destination):
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)
    destination.chmod(destination.stat().st_mode | 0o200)


def relative(target, base):
    """Path to target as seen from base, in a form Ruby and Mach-O both accept."""
    return os.path.relpath(target, base).replace(os.sep, "/")


def ruby_runtime(binaries, root, manifest_dir=None):
    config = json.loads(output("ruby", "-rjson", "-rrbconfig", "-rrubygems", "-e", """
      puts JSON.generate({lib: RbConfig::CONFIG.fetch('rubylibprefix'),
        bin: RbConfig::CONFIG.fetch('bindir'), load: $LOAD_PATH,
        gems: Gem.path, prefix: RbConfig::CONFIG.fetch('prefix')})
    """))
    source = Path(config["lib"])
    library = root / "ruby/lib"
    shutil.copytree(source, library, symlinks=False)
    loads = []
    for index, entry in enumerate(config["load"]):
        path = Path(entry)
        if not path.is_dir():
            continue
        if path.is_relative_to(source):
            target = library / path.relative_to(source)
        else:
            target = root / f"ruby/extra/{index}"
            shutil.copytree(path, target, symlinks=False)
        loads.append(relative(target, binaries))
    gems = []
    for index, entry in enumerate(config["gems"]):
        path = Path(entry)
        if not path.is_dir():
            continue
        if path.is_relative_to(source):
            target = library / path.relative_to(source)
        else:
            target = root / f"ruby/gems/{index}"
            shutil.copytree(path, target, symlinks=False)
        gems.append(relative(target, binaries))
    if not gems:
        gems = [relative(root / "ruby/gems", binaries)]
    # Keep the manifest out of MacOS: codesign requires that code location to
    # contain only signed Mach-O files. On Linux the manifest ships beside the
    # executable (the loader looks for it there), so `manifest_dir` overrides
    # the default location while the relative paths stay anchored on `binaries`.
    manifest_dir = manifest_dir or root
    (manifest_dir / "ruby-runtime.json").write_text(json.dumps({
        "load_paths": loads, "gem_paths": gems, "gem_home": gems[0],
    }, indent=2))
    return config


def mac_dependencies(executable, ruby, contents):
    frameworks = contents / "Frameworks"
    frameworks.mkdir(parents=True, exist_ok=True)
    copied = {}
    queue = [(executable, executable)]
    queue.extend((path, path) for path in contents.rglob("*")
                 if path.is_file() and path.suffix in (".bundle", ".dylib", ".so"))
    processed = set()
    while queue:
        original, target = queue.pop()
        if target in processed:
            continue
        processed.add(target)
        # Read original load commands before rewriting any dependency paths.
        commands = output("otool", "-l", str(original))
        rpaths = re.findall(r"cmd LC_RPATH\n.*?\n\s*path (.*?) \(offset", commands)
        for line in output("otool", "-L", str(original)).splitlines()[1:]:
            dependency = line.strip().split(" (compatibility")[0]
            if dependency.startswith(("/System/", "/usr/lib/")):
                continue
            if dependency == str(original):
                continue
            def expand(path):
                return Path(path.replace("@loader_path", str(original.parent))
                            .replace("@executable_path", str(executable.parent)))
            if dependency.startswith("@rpath/"):
                candidates = [expand(p) / dependency[7:] for p in rpaths]
                candidates += [Path(ruby["prefix"]) / "lib" / Path(dependency).name]
            else:
                candidates = [expand(dependency)]
            found = next((p.resolve() for p in candidates if p.is_file()), None)
            if found is None:
                raise RuntimeError(f"Unresolved dependency {dependency} in {original}")
            name = found.name
            if name in copied and copied[name] != found:
                raise RuntimeError(f"Conflicting libraries named {name}")
            bundled = frameworks / name
            if name not in copied:
                copied[name] = found
                copy(found, bundled)
                queue.append((found, bundled))
            relative_target = os.path.relpath(bundled, target.parent)
            run("install_name_tool", "-change", dependency,
                "@loader_path/" + relative_target, str(target))
        if target.suffix == ".dylib":
            run("install_name_tool", "-id", "@rpath/" + target.name, str(target))
    # Fail packaging if anything still requires a build-machine library.
    for path in processed:
        for line in output("otool", "-L", str(path)).splitlines()[1:]:
            dep = line.strip().split(" (compatibility")[0]
            if dep.startswith("/") and not dep.startswith(("/System/", "/usr/lib/")):
                raise RuntimeError(f"Non-relocatable dependency {dep} in {path}")
    # Sign every nested Mach-O file. The main executable is left to the bundle
    # signature, which records the nested code in one pass.
    for path in processed:
        if path == executable:
            continue
        run("codesign", "--force", "--sign", "-", str(path))


# MinGW runtime DLLs the GNU linker pulls into `*-windows-gnu` binaries. They
# live next to the cross `gcc`, not in Ruby's bin, so bundle them explicitly.
MINGW_RUNTIME_DLLS = (
    "libgcc_s_seh-1.dll",
    "libstdc++-6.dll",
    "libwinpthread-1.dll",
    "libssp-0.dll",
    "libatomic-1.dll",
    "libgomp-1.dll",
)


def windows_dependencies(executable, ruby):
    destination = executable.parent
    # `x64-ucrt-ruby340.dll` embeds a side-by-side manifest that depends on the
    # `ruby_builtin_dlls` assembly, which Windows resolves from a sibling
    # directory of the same name holding the assembly manifest and its DLLs.
    # Copy that directory verbatim: without it the loader cannot generate an
    # activation context and every launch dies with STATUS_SXS_CANT_GEN_ACTCTX
    # (0xC0150002) before the program's entry point runs.
    builtin_dlls = Path(ruby["bin"]) / "ruby_builtin_dlls"
    if builtin_dlls.is_dir():
        shutil.copytree(builtin_dlls, destination / "ruby_builtin_dlls")
    # Prefer the MinGW PE-aware objdump over whatever else is on PATH.
    objdump = shutil.which("x86_64-w64-mingw32-objdump") or shutil.which("objdump")
    if objdump is None:
        raise RuntimeError("objdump is required to bundle Windows DLLs")
    compiler = shutil.which("x86_64-w64-mingw32-gcc") or shutil.which("gcc")
    toolchain = Path(compiler).parent if compiler else None
    search = [Path(ruby["bin"]), builtin_dlls]
    if toolchain is not None:
        search.append(toolchain)
    search += [Path(p) for p in os.environ["PATH"].split(os.pathsep)]
    system = Path(os.environ["SystemRoot"]).resolve()
    # Ruby loads its own runtime DLLs dynamically; the GNU toolchain supplies
    # the MinGW runtime. Copy both before scanning so their imports are covered.
    bundled = list(Path(ruby["bin"]).glob("*.dll"))
    if toolchain is not None:
        bundled += [toolchain / name for name in MINGW_RUNTIME_DLLS]
    for path in bundled:
        if path.is_file() and not (destination / path.name).exists():
            copy(path, destination / path.name)
    queue = [executable]
    queue.extend(p for p in destination.rglob("*") if p.suffix.lower() in (".dll", ".so"))
    done = set()
    while queue:
        path = queue.pop()
        if path in done:
            continue
        done.add(path)
        names = re.findall(r"DLL Name:\s*(\S+)", output(objdump, "-p", str(path)))
        if path == executable and not names:
            raise RuntimeError("objdump reported no imports for the executable")
        for name in names:
            if name.lower().startswith(("api-ms-", "ext-ms-")):
                continue
            existing = destination / name
            if existing.is_file():
                continue
            found = next((p / name for p in [path.parent] + search if (p / name).is_file()), None)
            if found is None:
                found = system / "System32" / name
            if not found.is_file():
                raise RuntimeError(f"Missing DLL {name} required by {path}")
            if found.resolve().is_relative_to(system):
                continue
            copy(found, existing)
            queue.append(existing)


# glibc libraries and the dynamic loader must come from the running system:
# bundling them would tie the AppImage to this build machine's libc and break on
# older hosts (which is exactly what the old Nix build did).
GLIBC_LIBRARIES = re.compile(
    r"^(ld-linux.*|libc|libm|libdl|libpthread|librt|libutil|libresolv|libnsl|libmvec)\.so")
# GPU and display-driver libraries are provided by the host's kernel driver and
# must not be shipped, otherwise the AppImage would override the local driver.
GPU_LIBRARIES = re.compile(
    r"^(libGL|libEGL|libGLX|libGLdispatch|libOpenGL|libdrm|libgbm|libvulkan"
    r"|libnvidia|libcuda|libvdpau)")


def linux_needed(path):
    """(soname, resolved path or None) for each shared library `path` needs."""
    # `ldd` exits non-zero when a dependency is missing; parse its output anyway
    # so the caller can report the unresolved name instead of failing opaquely.
    result = subprocess.run(("ldd", str(path)), text=True, capture_output=True,
                            env={**os.environ, "LC_ALL": "C"})
    needed = []
    for line in result.stdout.splitlines():
        line = line.strip()
        if "=>" in line:
            name, rest = (part.strip() for part in line.split("=>", 1))
            rest = rest.split(" (")[0]
            needed.append((name, None if rest.startswith("not found") else Path(rest)))
        elif line.startswith("/"):
            # The loader itself, printed as an absolute path without "=>".
            location = Path(line.split(" (")[0])
            needed.append((location.name, location))
    return needed


def linux_dependencies(executable, library_dir, extra_libraries=()):
    """Copy the relocatable shared-library closure next to `executable`.

    Every bundled library is patched to `$ORIGIN`, and the executable to
    `$ORIGIN/../lib`, so the AppImage resolves its own libraries regardless of
    where it is mounted and irrespective of the host's library versions.

    `extra_libraries` seeds the scan with files that are dlopen'd rather than
    linked (Ruby's native extensions), so their dependencies are bundled too.
    """
    library_dir.mkdir(parents=True, exist_ok=True)
    copied = {}
    processed = set()
    queue = [executable, *extra_libraries]
    while queue:
        original = queue.pop()
        if original in processed:
            continue
        processed.add(original)
        for name, resolved in linux_needed(original):
            if GLIBC_LIBRARIES.match(name) or GPU_LIBRARIES.match(name):
                continue
            if resolved is None:
                raise RuntimeError(f"Unresolved dependency {name} in {original}")
            if name in copied and copied[name] != resolved:
                raise RuntimeError(f"Conflicting libraries named {name}")
            if name not in copied:
                copied[name] = resolved
                copy(resolved, library_dir / name)
                queue.append(resolved)
    run("patchelf", "--set-rpath", "$ORIGIN/../lib", str(executable))
    for name in copied:
        run("patchelf", "--set-rpath", "$ORIGIN", str(library_dir / name))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    destination = args.destination.resolve()
    if destination.exists():
        raise RuntimeError(f"Refusing to overwrite {destination}")
    if sys.platform == "darwin":
        app = destination / "RSXP.app"
        contents = app / "Contents"
        binaries = contents / "MacOS"
        # Non-Mach-O content must live outside the code locations (MacOS,
        # Frameworks): codesign treats any directory with a dot in its name
        # there as a bundle and rejects it.
        runtime = contents / "Resources"
    elif sys.platform == "win32":
        binaries = destination
        runtime = destination
    else:
        # An AppDir for `appimagetool` (see tools/build-appimage.sh). Ruby's
        # standard library lives under usr/lib/rsxp while the executable and its
        # manifest sit in usr/bin, which is where the runtime looks for them.
        binaries = destination / "usr/bin"
        runtime = destination / "usr/lib/rsxp"
    binaries.mkdir(parents=True)
    executable = binaries / args.binary.name
    copy(args.binary, executable)
    for name in ("COPYING", "README.md"):
        copy(root / name, runtime / name)
    # macOS keeps the manifest in Resources (see `ruby_runtime`); every other
    # layout reads it from beside the executable.
    manifest_dir = binaries if not (sys.platform == "darwin" or sys.platform == "win32") else None
    config = ruby_runtime(binaries, runtime, manifest_dir=manifest_dir)
    if sys.platform == "darwin":
        with (contents / "Info.plist").open("wb") as handle:
            plistlib.dump({"CFBundleExecutable": executable.name, "CFBundleName": "RSXP",
                "CFBundleIdentifier": "org.rsxp.engine", "CFBundlePackageType": "APPL",
                "CFBundleShortVersionString": "0.1.0", "CFBundleVersion": "1",
                "LSMinimumSystemVersion": "14.0", "NSHighResolutionCapable": True}, handle)
        mac_dependencies(executable, config, contents)
        run("codesign", "--force", "--sign", "-", str(app))
        run("codesign", "--verify", "--deep", "--strict", str(app))
    elif sys.platform == "win32":
        windows_dependencies(executable, config)
    else:
        linux_dependencies(executable, destination / "usr/lib",
                           extra_libraries=sorted(runtime.rglob("*.so")))
    print(f"Staged standalone desktop at {destination}")


if __name__ == "__main__":
    main()
