// SPDX-License-Identifier: GPL-2.0-or-later
//! Provision the native MRI runtime, link it into the mobile app, and inline
//! the touch UI so the same WebView content works on every cargo-mobile2 target.
use sha2::{Digest as _, Sha256};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

const RUBY_VERSION: &str = "4.0.701";
const RUBY_SHA256: &str = "e364fce6fcf6649d9f42d9094d960fcf6c8c7a4a2431b45f87aa2fa08fa2a9ae";

fn run(command: &mut Command) {
    println!("cargo:warning=running {command:?}");
    let status = command.status().unwrap_or_else(|error| {
        panic!("failed to run {command:?}: {error}");
    });
    assert!(status.success(), "{command:?} failed with {status}");
}

fn native_ruby(manifest: &Path, target: &str) -> PathBuf {
    if target.contains("android") {
        let supplied = env::var_os("RSXP_RUBY_PREFIX").unwrap_or_else(|| {
            panic!(
                "Android needs a native MRI distribution. Set RSXP_RUBY_PREFIX to a prefix \
                 containing include/ruby.h and lib/libruby-static.a; WASM/WASI is not a fallback."
            )
        });
        return PathBuf::from(supplied);
    }

    // The downloadable CRuby archive contains Apple frameworks and an
    // Android static library, not a Linux host library. Desktop previews use
    // the native Ruby supplied by the development environment instead.
    if target.contains("linux") {
        let output = Command::new("ruby")
            .args([
                "-rrbconfig",
                "-e",
                "print [RbConfig::CONFIG['prefix'], RbConfig::CONFIG['rubyhdrdir'], RbConfig::CONFIG['rubyarchhdrdir'], RbConfig::CONFIG['libdir'], RbConfig::CONFIG['LIBRUBYARG']].join(\"\\n\")",
            ])
            .output()
            .unwrap_or_else(|error| panic!("failed to query host Ruby; enter the Nix development shell: {error}"));
        assert!(output.status.success(), "failed to query host Ruby configuration");
        let values = String::from_utf8(output.stdout).expect("Ruby config was not UTF-8");
        let mut values = values.lines();
        let prefix = PathBuf::from(values.next().expect("missing Ruby prefix"));
        let header = PathBuf::from(values.next().expect("missing Ruby headers"));
        let arch_header = PathBuf::from(values.next().expect("missing Ruby arch headers"));
        let libdir = PathBuf::from(values.next().expect("missing Ruby library directory"));
        let library = values.next().expect("missing Ruby linker argument");
        fs::create_dir_all(manifest.join("build")).expect("failed to create build directory");
        fs::write(
            manifest.join("build/host-ruby-config"),
            format!("{}\n{}\n{}\n{}\n{}", prefix.display(), header.display(), arch_header.display(), libdir.display(), library),
        ).expect("failed to cache host Ruby configuration");
        return prefix;
    }

    let build = manifest.join("build/cruby-native");
    let prefix = build.join("CRuby");
    if prefix.join("include/ruby.h").is_file() {
        return prefix;
    }

    fs::create_dir_all(&build).unwrap();
    let archive = build.join(format!("CRuby_prebuilt-{RUBY_VERSION}.tar.gz"));
    if !archive.is_file() {
        run(Command::new("curl").args([
            "-fL",
            &format!(
                "https://github.com/xord/cruby/releases/download/v{RUBY_VERSION}/{}",
                archive.file_name().unwrap().to_string_lossy()
            ),
            "-o",
            archive.to_str().unwrap(),
        ]));
    }

    let bytes = fs::read(&archive).unwrap();
    let digest = format!("{:x}", Sha256::digest(&bytes));
    assert_eq!(digest, RUBY_SHA256, "native Ruby checksum mismatch");

    run(Command::new("tar").args([
        "-xzf",
        archive.to_str().unwrap(),
        "-C",
        build.to_str().unwrap(),
    ]));
    assert!(
        prefix.join("include/ruby.h").is_file(),
        "native Ruby archive did not contain include/ruby.h"
    );
    prefix
}

fn link_ruby(manifest: &Path, prefix: &Path, target: &str) {
    if target.contains("linux") {
        let config = fs::read_to_string(manifest.join("build/host-ruby-config"))
            .expect("host Ruby configuration was not captured");
        let mut values = config.lines();
        let _prefix = values.next();
        let header = values.next().expect("missing Ruby headers");
        let arch_header = values.next().expect("missing Ruby arch headers");
        let libdir = values.next().expect("missing Ruby library directory");
        let library = values.next().expect("missing Ruby linker argument");
        println!("cargo:rustc-link-search=native={libdir}");
        for flag in library.split_whitespace() {
            if let Some(name) = flag.strip_prefix("-l") {
                println!("cargo:rustc-link-lib=dylib={name}");
            }
        }
        println!("cargo:rustc-link-lib=z");
        let mut bridge = cc::Build::new();
        bridge.file(manifest.join("../ruby_rgss_core/mobile_bridge.c"));
        bridge.include(header).include(arch_header);
        let platform = Path::new(arch_header).file_name().unwrap().to_string_lossy();
        bridge.define("RSXP_RUBY_PLATFORM", Some(format!("\"{platform}\"").as_str()));
        bridge.define("RSXP_STOCK_RUBY", None);
        bridge.cargo_metadata(false);
        bridge
            .flag_if_supported("-Wno-unused-parameter")
            .flag_if_supported("-O2")
            .compile("rsxp_mri_bridge");
        println!("cargo:rustc-link-search=native={}", env::var("OUT_DIR").unwrap());
        println!("cargo:rustc-link-lib=static:+whole-archive=rsxp_mri_bridge");
        println!("cargo:rustc-env=RSXP_NATIVE_RUBY_DEFAULT={}", prefix.display());
        println!("cargo:rustc-env=RSXP_RUBY_VERSION={RUBY_VERSION}");
        return;
    }
    let apple = target.contains("apple");
    let (directory, platform) = if target.contains("ios-sim") {
        ("ios-arm64_x86_64-simulator", "iphonesimulator-arm64")
    } else if target.contains("ios") {
        ("ios-arm64", "iphoneos-arm64")
    } else if apple {
        ("macos-arm64_x86_64", "macosx-arm64")
    } else {
        ("", "android-arm64")
    };
    let library = if apple {
        prefix.join("CRuby.xcframework").join(directory)
    } else {
        prefix.join("lib")
    };
    let archive = library.join("libruby-static.a");
    let bytes = fs::read(&archive).unwrap_or_else(|error| {
        panic!(
            "native Ruby library {} is missing: {error}",
            archive.display()
        )
    });
    let library = if apple
        && (bytes.starts_with(&[0xca, 0xfe, 0xba, 0xbe])
            || bytes.starts_with(&[0xca, 0xfe, 0xba, 0xbf]))
    {
        let out = PathBuf::from(env::var("OUT_DIR").unwrap());
        let architecture = if target.starts_with("aarch64") {
            "arm64"
        } else {
            "x86_64"
        };
        run(Command::new("/usr/bin/lipo").args([
            archive.to_str().unwrap(),
            "-thin",
            architecture,
            "-output",
            out.join("libruby-static.a").to_str().unwrap(),
        ]));
        out
    } else {
        library
    };
    println!("cargo:rustc-link-search=native={}", library.display());
    println!("cargo:rustc-link-lib=static=ruby-static");
    println!("cargo:rustc-link-lib=z");
    if target.ends_with("apple-darwin") {
        println!("cargo:rustc-link-lib=ffi");
    }
    if apple {
        println!("cargo:rustc-link-lib=framework=Security");
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
    }

    let mut bridge = cc::Build::new();
    bridge
        .file(manifest.join("../ruby_rgss_core/mobile_bridge.c"))
        .include(prefix.join("include"))
        .define(
            "RSXP_RUBY_PLATFORM",
            Some(format!("\"{platform}\"").as_str()),
        );
    if !apple {
        bridge.define("RSXP_STOCK_RUBY", None);
    }
    bridge
        .cargo_metadata(false)
        .flag_if_supported("-Wno-unused-parameter")
        .flag_if_supported("-O2")
        .compile("rsxp_mri_bridge");
    println!("cargo:rustc-link-search=native={}", env::var("OUT_DIR").unwrap());
    println!("cargo:rustc-link-lib=static:+whole-archive=rsxp_mri_bridge");
    println!(
        "cargo:rustc-env=RSXP_NATIVE_RUBY_DEFAULT={}",
        prefix.display()
    );
    println!("cargo:rustc-env=RSXP_RUBY_VERSION={RUBY_VERSION}");
}

fn copy_tree(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        let target = destination.join(&name);
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if entry.file_type()?.is_file() {
            // The interpreter itself is linked into the app; only its script
            // and extension tree needs to ship as an asset.
            if name != "libruby-static.a" {
                fs::copy(entry.path(), target)?;
            }
        }
    }
    Ok(())
}

/// Keep the Ruby library in cargo-mobile2's asset directory. iOS bundles the
/// folder as a resource; Android packages it and `platform.rs` extracts it.
fn stage_runtime(manifest: &Path, prefix: &Path, target: &str) {
    let runtime = manifest.join("assets/runtime");
    let marker = runtime.join(".staged");
    let wanted = format!("{}:{target}", prefix.display());
    if fs::read_to_string(&marker).is_ok_and(|value| value == wanted)
        && runtime.join("lib").is_dir()
    {
        return;
    }
    let _ = fs::remove_dir_all(&runtime);
    fs::create_dir_all(&runtime).unwrap();
    copy_tree(&prefix.join("lib"), &runtime.join("lib"))
        .unwrap_or_else(|error| panic!("failed to stage the Ruby runtime: {error}"));
    let notices = runtime.join("licenses");
    fs::create_dir_all(&notices).unwrap();
    if let Ok(entries) = fs::read_dir(manifest.join("ruby")) {
        for entry in entries {
            let entry = entry.unwrap();
            if entry.file_name() != "bridge.c" && entry.file_type().unwrap().is_file() {
                fs::copy(entry.path(), notices.join(entry.file_name())).unwrap();
            }
        }
    }
    fs::write(marker, wanted).unwrap();
}

/// Inline the shared UI into a single document so no platform asset loader is
/// needed for HTML, CSS, or JavaScript.
fn build_ui(manifest: &Path) {
    let ui = manifest.join("ui");
    let html = fs::read_to_string(ui.join("index.html")).unwrap();
    let style = fs::read_to_string(ui.join("style.css")).unwrap();
    let input = fs::read_to_string(ui.join("input.js")).unwrap();
    let app = fs::read_to_string(ui.join("app.js")).unwrap();
    let input = input.replace(
        "export function createTouchInput",
        "function createTouchInput",
    );
    let app = app.replacen("import { createTouchInput } from './input.js';\n", "", 1);
    let html = html
        .replace(
            "<link rel=\"stylesheet\" href=\"style.css\">",
            &format!("<style>\n{style}\n</style>"),
        )
        .replace(
            "<script type=\"module\" src=\"app.js\"></script>",
            &format!("<script type=\"module\">\n{input}\n{app}\n</script>"),
        );
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("ui.html");
    fs::write(out, html).unwrap();
}

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let target = env::var("TARGET").unwrap();
    for file in [
        "build.rs",
        "../ruby_rgss_core/mobile_bridge.c",
        "ui/index.html",
        "ui/style.css",
        "ui/input.js",
        "ui/app.js",
    ] {
        println!("cargo:rerun-if-changed={file}");
    }
    println!("cargo:rerun-if-env-changed=RSXP_RUBY_PREFIX");

    let prefix = native_ruby(&manifest, &target);
    link_ruby(&manifest, &prefix, &target);
    stage_runtime(&manifest, &prefix, &target);
    build_ui(&manifest);
}
