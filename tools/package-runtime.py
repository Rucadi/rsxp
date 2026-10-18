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


def ruby_runtime(destination):
    config = json.loads(output("ruby", "-rjson", "-rrbconfig", "-rrubygems", "-e", """
      puts JSON.generate({lib: RbConfig::CONFIG.fetch('rubylibprefix'),
        bin: RbConfig::CONFIG.fetch('bindir'), load: $LOAD_PATH,
        gems: Gem.path, prefix: RbConfig::CONFIG.fetch('prefix')})
    """))
    source = Path(config["lib"])
    library = destination / "ruby/lib"
    shutil.copytree(source, library, symlinks=False)
    loads = []
    for index, entry in enumerate(config["load"]):
        path = Path(entry)
        if not path.is_dir():
            continue
        if path.is_relative_to(source):
            target = library / path.relative_to(source)
        else:
            target = destination / f"ruby/extra/{index}"
            shutil.copytree(path, target, symlinks=False)
        loads.append(target.relative_to(destination).as_posix())
    gems = []
    for index, entry in enumerate(config["gems"]):
        path = Path(entry)
        if not path.is_dir():
            continue
        if path.is_relative_to(source):
            target = library / path.relative_to(source)
        else:
            target = destination / f"ruby/gems/{index}"
            shutil.copytree(path, target, symlinks=False)
        gems.append(target.relative_to(destination).as_posix())
    if not gems:
        gems = ["ruby/gems"]
    (destination / "ruby-runtime.json").write_text(json.dumps({
        "load_paths": loads, "gem_paths": gems, "gem_home": gems[0],
    }, indent=2))
    return config


def mac_dependencies(executable, ruby, frameworks):
    frameworks.mkdir(parents=True)
    copied = {}
    queue = [(executable, executable)]
    queue.extend((path, path) for path in executable.parent.rglob("*")
                 if path.suffix in (".bundle", ".dylib", ".so"))
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
            relative = os.path.relpath(bundled, target.parent)
            run("install_name_tool", "-change", dependency, "@loader_path/" + relative, str(target))
        if target.suffix == ".dylib":
            run("install_name_tool", "-id", "@rpath/" + target.name, str(target))
    # Fail packaging if anything still requires a build-machine library.
    for path in processed:
        for line in output("otool", "-L", str(path)).splitlines()[1:]:
            dep = line.strip().split(" (compatibility")[0]
            if dep.startswith("/") and not dep.startswith(("/System/", "/usr/lib/")):
                raise RuntimeError(f"Non-relocatable dependency {dep} in {path}")
        run("codesign", "--force", "--sign", "-", str(path))


def windows_dependencies(executable, ruby):
    destination = executable.parent
    search = [Path(ruby["bin"])] + [Path(p) for p in os.environ["PATH"].split(os.pathsep)]
    system = Path(os.environ["SystemRoot"]).resolve()
    queue = [executable]
    queue.extend(p for p in destination.rglob("*") if p.suffix.lower() in (".dll", ".so"))
    # Ruby may load these dynamically, so include its runtime DLLs up front.
    for path in Path(ruby["bin"]).glob("*.dll"):
        copy(path, destination / path.name)
        queue.append(destination / path.name)
    done = set()
    while queue:
        path = queue.pop()
        if path in done:
            continue
        done.add(path)
        for name in re.findall(r"DLL Name:\s*(\S+)", output("objdump", "-p", str(path))):
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
        binaries = app / "Contents/MacOS"
    elif sys.platform == "win32":
        binaries = destination
    else:
        raise RuntimeError("Use the Nix AppImage target for Linux")
    binaries.mkdir(parents=True)
    executable = binaries / args.binary.name
    copy(args.binary, executable)
    for name in ("COPYING", "README.md"):
        copy(root / name, binaries / name)
    config = ruby_runtime(binaries)
    if sys.platform == "darwin":
        with (app / "Contents/Info.plist").open("wb") as handle:
            plistlib.dump({"CFBundleExecutable": executable.name, "CFBundleName": "RSXP",
                "CFBundleIdentifier": "org.rsxp.engine", "CFBundlePackageType": "APPL",
                "CFBundleShortVersionString": "0.1.0", "CFBundleVersion": "1",
                "LSMinimumSystemVersion": "14.0", "NSHighResolutionCapable": True}, handle)
        mac_dependencies(executable, config, app / "Contents/Frameworks")
        run("codesign", "--force", "--sign", "-", str(app))
        run("codesign", "--verify", "--deep", "--strict", str(app))
    else:
        windows_dependencies(executable, config)
    print(f"Staged standalone desktop at {destination}")


if __name__ == "__main__":
    main()
