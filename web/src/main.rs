// SPDX-License-Identifier: GPL-2.0-or-later
//! `rsxp-web`: host binary for the standalone browser player.
//!
//! It embeds `web/dist/player.html` at compile time (`include_str!`) and offers
//! three subcommands:
//!
//! - `serve [GAME_FOLDER] [--bind 127.0.0.1] [--port 8000]` — serves the player
//!   plus an optional game folder over loopback HTTP. It serves `/` and
//!   `/player.html` (the player with an injected `globalThis.RSXP_GAME` config
//!   script), `/game/files.json` (the sorted asset list) and `/game/<asset>`
//!   (raw bytes, matched case-insensitively against that list). Responses carry
//!   COOP/COEP headers so the page is cross-origin isolated.
//! - `export [player.html]` — writes the embedded player HTML unchanged.
//! - `embed GAME_FOLDER [player.html]` — writes the player with every game file
//!   base64-encoded into `<script type="application/octet-stream">` blocks
//!   (chunked to 768 KiB to bound DOM string size) and recorded in
//!   `globalThis.RSXP_GAME`.
//!
//! Asset paths are validated by `decode_path`, which rejects traversal (`..`),
//! hidden/dot segments, backslashes and NUL bytes. `--help` prints the usage
//! summary.
use std::{
    env, fs,
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
};
const PLAYER: &str = include_str!("../dist/player.html");
fn json(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '<' => out.push_str("\\u003c"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
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
fn files(
    root: &Path,
    dir: &Path,
    output: Option<&Path>,
    found: &mut Vec<String>,
) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with('.') || entry.file_type()?.is_symlink() {
            continue;
        }
        if output == Some(path.as_path()) {
            continue;
        }
        if entry.file_type()?.is_dir() {
            files(root, &path, output, found)?;
        } else if entry.file_type()?.is_file() {
            found.push(
                path.strip_prefix(root)
                    .unwrap()
                    .to_str()
                    .ok_or_else(|| io::Error::other("Asset path is not UTF-8"))?
                    .replace('\\', "/"),
            );
        }
    }
    Ok(())
}
fn game(root: &Path, output: Option<&Path>, embedded: bool) -> io::Result<(String, Vec<String>)> {
    let mut paths = vec![];
    files(root, root, output, &mut paths)?;
    paths.sort();
    let mut seen = std::collections::HashSet::new();
    for path in &paths {
        if !seen.insert(path.to_lowercase()) {
            return Err(io::Error::other(format!("Ambiguous asset name: {path}")));
        }
    }
    let mut entries = Vec::new();
    let mut blocks = String::new();
    for (asset, path) in paths.iter().enumerate() {
        let data = if embedded {
            let bytes = fs::read(root.join(path))?;
            let mut ids = Vec::new();
            // Keep asset bytes out of executable JavaScript and bound each DOM
            // string. Large games otherwise exceed the browser script parser.
            for (chunk, bytes) in bytes.chunks(768 * 1024).enumerate() {
                let id = format!("rsxp-asset-{asset}-{chunk}");
                blocks.push_str(&format!(
                    "<script id=\"{id}\" type=\"application/octet-stream\">{}</script>\n",
                    base64(bytes)
                ));
                ids.push(json(&id));
            }
            format!("{{\"chunks\":[{}]}}", ids.join(","))
        } else {
            "null".into()
        };
        entries.push(format!("[{},{}]", json(path), data));
    }
    Ok((
        format!(
            "<script>globalThis.RSXP_GAME={{\"folder\":{},\"files\":[{}]}};</script>\n{blocks}",
            json(&root.file_name().unwrap_or_default().to_string_lossy()),
            entries.join(",")
        ),
        paths,
    ))
}

fn html(config: &str) -> String {
    PLAYER.replacen(
        "<script type=\"module\">",
        &format!("{config}<script type=\"module\">"),
        1,
    )
}
fn decode_path(raw: &str) -> Option<String> {
    let raw = raw.split('?').next()?;
    let mut bytes = Vec::new();
    let mut iter = raw.bytes();
    while let Some(b) = iter.next() {
        if b == b'%' {
            let a = (iter.next()? as char).to_digit(16)?;
            let b = (iter.next()? as char).to_digit(16)?;
            bytes.push((a * 16 + b) as u8);
        } else {
            bytes.push(b);
        }
    }
    let path = String::from_utf8(bytes).ok()?;
    if path.contains('\\')
        || path.contains('\0')
        || path.split('/').any(|p| p == ".." || p.starts_with('.'))
    {
        return None;
    }
    Some(path)
}
fn respond(
    stream: &mut TcpStream,
    status: &str,
    mime: &str,
    data: &[u8],
    head: bool,
) -> io::Result<()> {
    write!(stream,"HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\nCross-Origin-Opener-Policy: same-origin\r\nCross-Origin-Embedder-Policy: require-corp\r\nCache-Control: no-cache\r\n\r\n",data.len())?;
    if !head {
        stream.write_all(data)?;
    }
    Ok(())
}
fn request(
    mut stream: TcpStream,
    page: &str,
    root: Option<&Path>,
    paths: &[String],
) -> io::Result<()> {
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
    let mut buffer = [0; 8192];
    let mut used = 0;
    while used < buffer.len() {
        let n = stream.read(&mut buffer[used..])?;
        if n == 0 {
            break;
        }
        used += n;
        if buffer[..used].windows(4).any(|s| s == b"\r\n\r\n") {
            break;
        }
    }
    let text = String::from_utf8_lossy(&buffer[..used]);
    let mut line = text.lines().next().unwrap_or("").split_whitespace();
    let method = line.next().unwrap_or("");
    let target = line.next().unwrap_or("");
    let head = method == "HEAD";
    if method != "GET" && !head {
        return respond(
            &mut stream,
            "405 Method Not Allowed",
            "text/plain",
            b"GET or HEAD required",
            false,
        );
    }
    if let Some(path) = decode_path(target) {
        if path == "/" || path == "/player.html" {
            return respond(
                &mut stream,
                "200 OK",
                "text/html; charset=utf-8",
                page.as_bytes(),
                head,
            );
        }
        if let Some(root) = root {
            if path == "/game/files.json" {
                let list = format!(
                    "[{}]",
                    paths.iter().map(|s| json(s)).collect::<Vec<_>>().join(",")
                );
                return respond(
                    &mut stream,
                    "200 OK",
                    "application/json",
                    list.as_bytes(),
                    head,
                );
            }
            if let Some(asset) = path
                .strip_prefix("/game/")
                .and_then(|p| paths.iter().find(|s| s.eq_ignore_ascii_case(p)))
            {
                let file = root.join(asset).canonicalize()?;
                if file.starts_with(root) {
                    let data = fs::read(file)?;
                    return respond(
                        &mut stream,
                        "200 OK",
                        "application/octet-stream",
                        &data,
                        head,
                    );
                }
            }
        }
    }
    respond(
        &mut stream,
        "404 Not Found",
        "text/plain",
        b"Not found",
        head,
    )
}
fn run() -> io::Result<()> {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_default();
    let mut positional = vec![];
    let mut bind = "127.0.0.1".to_string();
    let mut port = "8000".to_string();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--bind" => {
                bind = args
                    .next()
                    .ok_or_else(|| io::Error::other("Missing --bind value"))?
            }
            "--port" => {
                port = args
                    .next()
                    .ok_or_else(|| io::Error::other("Missing --port value"))?
            }
            _ => positional.push(arg),
        }
    }
    match command.as_str() {
        "export" if positional.len()<=1 => {let output=positional.first().map(String::as_str).unwrap_or("player.html");fs::write(output,PLAYER)?;println!("Exported {output}");}
        "embed" if (1..=2).contains(&positional.len()) => {
            let root=fs::canonicalize(&positional[0])?;let output=PathBuf::from(positional.get(1).map(String::as_str).unwrap_or("player.html"));
            let absolute=if output.is_absolute(){output.clone()}else{env::current_dir()?.join(&output)};
            let parent=absolute.parent().unwrap().canonicalize()?;let absolute=parent.join(absolute.file_name().unwrap());
            let (config,_)=game(&root,Some(&absolute),true)?;fs::write(&output,html(&config))?;println!("Exported {} with embedded game",output.display());
        }
        "serve" if positional.len()<=1 => {
            let root=positional.first().map(fs::canonicalize).transpose()?;
            let (page,paths)=if let Some(root)=&root {let(config,paths)=game(root,None,false)?;(html(&config),paths)}else{(PLAYER.to_string(),vec![])};
            let listener=TcpListener::bind(format!("{bind}:{port}"))?;println!("rsxp player: http://{}/",listener.local_addr()?);
            // Each connection owns only an Arc clone of the bundled player.
            let page=std::sync::Arc::new(page);let paths=std::sync::Arc::new(paths);
            for stream in listener.incoming() {let stream=stream?;let page=page.clone();let paths=paths.clone();let root=root.clone();std::thread::spawn(move||{if let Err(error)=request(stream,&page,root.as_deref(),&paths){eprintln!("HTTP: {error}");}});}
        }
        "--help"|"-h"|"help"|"" => println!("rsxp-web serve [GAME_FOLDER] [--port 8000] [--bind 127.0.0.1]\nrsxp-web export [player.html]\nrsxp-web embed GAME_FOLDER [player.html]"),
        _ => return Err(io::Error::other("Invalid command. Use rsxp-web --help")),
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("rsxp-web: {error}");
        std::process::exit(1);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encoding() {
        assert_eq!(base64(b"hello"), "aGVsbG8=");
        assert_eq!(json("</script>\n"), "\"\\u003c/script>\\u000a\"");
    }
    #[test]
    fn traversal() {
        for path in [
            "/game/../secret",
            "/game/%2e%2e/secret",
            "/game/a%5cb",
            "/game/%00",
            "/game/.hidden",
        ] {
            assert!(decode_path(path).is_none());
        }
        assert_eq!(decode_path("/game/a%20b?x=1").unwrap(), "/game/a b");
    }
    #[test]
    fn bundled_player() {
        assert!(PLAYER.contains("RSXP_GAME"));
        assert!(html("MARKER").contains("MARKER<script type=\"module\">"));
    }
}
