// SPDX-License-Identifier: GPL-2.0-or-later
use std::{env, path::PathBuf};
fn main() {
    println!("cargo:rerun-if-changed=../ruby/bridge.c");
    println!("cargo:rerun-if-env-changed=RSXP_RUBY_PREFIX");
    let target = env::var("TARGET").unwrap();
    let apple = target.contains("apple");
    let prefix = env::var_os("RSXP_RUBY_PREFIX")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
                .join("../build/cruby-native/CRuby")
        });
    let include = prefix.join("include");
    let versioned_include = std::fs::read_dir(&include).ok().and_then(|entries| {
        entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("ruby-"))
                    && path.join("ruby.h").is_file()
            })
    });
    assert!(
        include.join("ruby.h").is_file() || versioned_include.is_some(),
        "Native Ruby is missing; run python3 mobile/build.py first. Android requires RSXP_RUBY_PREFIX pointing to a native MRI build."
    );
    let (directory, platform) = if target.contains("ios-sim") {
        ("ios-arm64_x86_64-simulator", "iphonesimulator-arm64")
    } else if target.contains("ios") {
        ("ios-arm64", "iphoneos-arm64")
    } else if target.contains("apple") {
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
    let android = target.contains("android");
    if android {
        println!("cargo:rustc-link-search=native={}", library.display());
        println!("cargo:rustc-link-lib=dylib=ruby");
        println!("cargo:rustc-link-lib=log");
        println!("cargo:rustc-link-lib=dl");
        println!("cargo:rustc-link-lib=m");
    } else {
        let bytes = std::fs::read(&archive).expect("Native Ruby library is missing");
        let magic = &bytes[..4];
        let library =
            if apple && (magic == [0xca, 0xfe, 0xba, 0xbe] || magic == [0xca, 0xfe, 0xba, 0xbf]) {
                let out = PathBuf::from(env::var("OUT_DIR").unwrap());
                let architecture = if target.starts_with("aarch64") {
                    "arm64"
                } else {
                    "x86_64"
                };
                assert!(std::process::Command::new("/usr/bin/lipo")
                    .arg(&archive)
                    .args(["-thin", architecture, "-output"])
                    .arg(out.join("libruby-static.a"))
                    .status()
                    .unwrap()
                    .success());
                out
            } else {
                library
            };
        println!("cargo:rustc-link-search=native={}", library.display());
        println!("cargo:rustc-link-lib=static=ruby-static");
        println!("cargo:rustc-link-lib=z");
    }
    if target.ends_with("apple-darwin") {
        println!("cargo:rustc-link-lib=ffi");
    }
    if apple {
        println!("cargo:rustc-link-lib=framework=Security");
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
    }
    let mut c = cc::Build::new();
    c.file("../ruby/bridge.c").define(
        "RSXP_RUBY_PLATFORM",
        Some(format!("\"{platform}\"").as_str()),
    );
    // Termux's Android build installs a versioned include tree:
    // include/ruby-3.3.0 and include/ruby-3.3.0/aarch64-linux-android.
    // Apple distributions keep a flat include directory.
    c.include(prefix.join("include"));
    if android {
        if let Ok(entries) = std::fs::read_dir(prefix.join("include")) {
            if let Some(versioned) = entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .find(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("ruby-"))
                })
            {
                if let Ok(architectures) = std::fs::read_dir(&versioned) {
                    if let Some(arch) = architectures
                        .filter_map(Result::ok)
                        .map(|entry| entry.path())
                        .find(|path| path.is_dir())
                    {
                        c.include(arch);
                    }
                }
                c.include(versioned);
            }
        }
    }
    if !apple {
        c.define("RSXP_STOCK_RUBY", None);
    }
    c.flag_if_supported("-Wno-unused-parameter")
        .compile("rsxp_mri_bridge");
    println!(
        "cargo:rustc-env=RSXP_NATIVE_RUBY_DEFAULT={}",
        prefix.display()
    );
}
