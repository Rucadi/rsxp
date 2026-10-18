// SPDX-License-Identifier: GPL-2.0-or-later
//! Build orchestration lives here, not in the shipped player.
use serde_json::{json, Value};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

const MODULES: &[&str] = &[
    "app.js",
    "ruby-host.js",
    "native-bridge.js",
    "game-pump.js",
    "gamepad.js",
    "local-game.js",
    "player-launcher.js",
    "game-import.js",
    "vendor/zip.js",
    "vendor/webtorrent.js",
    "pkg/rsxp_web.js",
    "ruby/browser.umd.js",
];
fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for part in bytes.chunks(3) {
        let n = (part[0] as u32) << 16
            | (part.get(1).copied().unwrap_or(0) as u32) << 8
            | part.get(2).copied().unwrap_or(0) as u32;
        out.push(ABC[(n >> 18) as usize] as char);
        out.push(ABC[((n >> 12) & 63) as usize] as char);
        out.push(if part.len() > 1 {
            ABC[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if part.len() > 2 {
            ABC[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}
fn execute(command: &mut Command) -> io::Result<()> {
    println!("==> {command:?}");
    let status = command.status()?;
    if !status.success() {
        return Err(io::Error::other(format!("Command failed: {status}")));
    }
    Ok(())
}
fn copy(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to.parent().unwrap())?;
    if to.exists() && fs::canonicalize(from)? == fs::canonicalize(to)? {
        return Ok(());
    }
    fs::copy(from, to)?;
    Ok(())
}
fn runtime(web: &Path, provided: Option<PathBuf>) -> io::Result<()> {
    let target = web.join("dist/ruby");
    let paths = if let Some(dir) = provided {
        (dir.join("browser.umd.js"), dir.join("ruby+stdlib.wasm"))
    } else if target.join("browser.umd.js").is_file() && target.join("ruby+stdlib.wasm").is_file() {
        return Ok(());
    } else if web
        .join("node_modules/@ruby/4.0-wasm-wasi/dist/ruby+stdlib.wasm")
        .is_file()
    {
        (
            web.join("node_modules/@ruby/wasm-wasi/dist/browser.umd.js"),
            web.join("node_modules/@ruby/4.0-wasm-wasi/dist/ruby+stdlib.wasm"),
        )
    } else {
        return Err(io::Error::other("Ruby WASM runtime missing. Supply --ruby-runtime DIR containing browser.umd.js and ruby+stdlib.wasm (Ruby 4.0 / @ruby packages 2.10.1), or provision it once with: npm --prefix web ci. Node/npm are not used by this build."));
    };
    copy(&paths.0, &target.join("browser.umd.js"))?;
    copy(&paths.1, &target.join("ruby+stdlib.wasm"))?;
    let magic = fs::read(target.join("ruby+stdlib.wasm"))?;
    if !magic.starts_with(b"\0asm") {
        return Err(io::Error::other("Ruby runtime is not a WASM binary"));
    }
    Ok(())
}
fn render(template: &str, payload: &Value) -> io::Result<String> {
    if template.matches("__RSXP_PAYLOAD__").count() != 1 {
        return Err(io::Error::other(
            "Player template must have exactly one payload placeholder",
        ));
    }
    let serialized = serde_json::to_string(payload)?.replace('<', "\\u003c");
    Ok(template.replacen("__RSXP_PAYLOAD__", &serialized, 1))
}
fn package(root: &Path, dist: &Path) -> io::Result<()> {
    let www = root.join("web/browser");
    let mut modules = serde_json::Map::new();
    for name in MODULES {
        let path = if name.starts_with("pkg/") || name.starts_with("ruby/") {
            dist.join(name)
        } else {
            www.join(name)
        };
        modules.insert(name.to_string(), Value::String(fs::read_to_string(path)?));
    }
    let payload = json!({
        "modules": modules,
        "licenses": {
            "zip.js": fs::read_to_string(www.join("vendor/zip.LICENSE"))?,
            "webtorrent": fs::read_to_string(www.join("vendor/webtorrent.LICENSE"))?,
        },
        "engine": base64(&fs::read(dist.join("pkg/rsxp_web_bg.wasm"))?),
        "ruby": base64(&fs::read(dist.join("ruby/ruby+stdlib.wasm"))?),
        "rgss": fs::read_to_string(root.join("rust/rgss.rb"))?,
        "manifest": serde_json::from_str::<Value>(&fs::read_to_string(www.join("assets.json"))?)?,
    });
    let html = render(&fs::read_to_string(www.join("player.html"))?, &payload)?;
    fs::write(dist.join("player.html"), &html)?;
    println!(
        "Packaged {} ({:.1} MiB)",
        dist.join("player.html").display(),
        html.len() as f64 / 1048576.
    );
    Ok(())
}
fn run() -> io::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let web = root.join("web");
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_default();
    if command != "web" {
        println!("cargo xtask web [--ruby-runtime DIR]\nBuild WASM, package dist/player.html and build web/target/release/rsxp-web.\nRequires wasm32-unknown-unknown, wasm-bindgen 0.2.121, and wasm-ld.");
        return if command.is_empty() || command == "--help" {
            Ok(())
        } else {
            Err(io::Error::other("Unknown xtask"))
        };
    }
    let mut ruby = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--ruby-runtime" => {
                ruby = Some(PathBuf::from(args.next().ok_or_else(|| {
                    io::Error::other("Missing --ruby-runtime directory")
                })?))
            }
            "--help" => {
                println!("cargo xtask web [--ruby-runtime DIR]");
                return Ok(());
            }
            _ => return Err(io::Error::other(format!("Unknown option: {arg}"))),
        }
    }
    runtime(&web, ruby)?;
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    execute(
        Command::new(&cargo)
            .current_dir(&root)
            .args([
                "build",
                "--release",
                "--manifest-path",
                "web/engine/Cargo.toml",
                "--target",
                "wasm32-unknown-unknown",
            ])
            .env("CARGO_TARGET_DIR", web.join("engine/target"))
            .env(
                "CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_LINKER",
                env::var_os("WASM_LD").unwrap_or_else(|| "wasm-ld".into()),
            ),
    )?;
    let dist = web.join("dist");
    let bindgen = env::var_os("WASM_BINDGEN").unwrap_or_else(|| "wasm-bindgen".into());
    execute(
        Command::new(bindgen)
            .arg("--target")
            .arg("web")
            .arg("--out-dir")
            .arg(dist.join("pkg"))
            .arg(web.join("engine/target/wasm32-unknown-unknown/release/rsxp_web.wasm")),
    )?;
    package(root, &dist)?;
    execute(
        Command::new(cargo)
            .current_dir(root)
            .args(["build", "--release", "--manifest-path", "web/Cargo.toml"])
            .env("CARGO_TARGET_DIR", web.join("target")),
    )?;
    println!("Built {}", web.join("target/release/rsxp-web").display());
    Ok(())
}
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask: {error}");
            ExitCode::FAILURE
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn safe_payload() {
        let payload = json!({"modules":{"app.js":"</script><script>bad</script>"}});
        let rendered = render("<script>__RSXP_PAYLOAD__</script>", &payload).unwrap();
        assert_eq!(rendered.matches("</script>").count(), 1);
        let json = &rendered[8..rendered.len() - 9];
        assert_eq!(serde_json::from_str::<Value>(json).unwrap(), payload);
    }
    #[test]
    fn template_requires_one_placeholder() {
        assert!(render("none", &json!({})).is_err());
        assert!(render("__RSXP_PAYLOAD____RSXP_PAYLOAD__", &json!({})).is_err());
    }
    #[test]
    fn binary_encoding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"h"), "aA==");
        assert_eq!(base64(b"he"), "aGU=");
        assert_eq!(base64(b"hello"), "aGVsbG8=");
    }
}
