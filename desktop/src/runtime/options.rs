// SPDX-License-Identifier: GPL-2.0-or-later
//! Desktop `run` subcommand arguments, validation and path resolution.
//!
//! [`RunOptions::parse`] consumes the argument list into the renderer selection,
//! exactly one of `--script` or `--game`, `--headless`, `--frames`, `--output`, an
//! optional `--rgss` version override, and the `--ruby-profile*` sampler settings
//! (validated by [`rsxp_core::ruby::profiling::Options::validate`]). The renderer defaults to
//! `RSXP_RENDERER` or `auto` and must be `auto`, `software` or `wgpu`.
//! `--trace-scripts` sets `RSXP_TRACE=1`, and the legacy `--profile NAME` flag is
//! accepted and ignored. Unknown arguments are an error.
//!
//! Relative `--output` and `--ruby-profile` paths are resolved against the current
//! (launch) directory before `GameConfig::prepare` switches it to the game directory;
//! the profiler file's parent directory is created, and it may not point at an existing
//! directory.
use super::set_startup_env;
use std::{error::Error, fs, path::PathBuf};

pub(super) struct RunOptions {
    pub renderer: String,
    pub script: Option<PathBuf>,
    pub game: Option<PathBuf>,
    pub headless: bool,
    pub frames: Option<u64>,
    pub output: Option<PathBuf>,
    pub version: Option<u8>,
    pub ruby_profile: rsxp_core::ruby::profiling::Options,
}

impl RunOptions {
    pub fn parse(args: &[String]) -> Result<Self, Box<dyn Error>> {
        let mut renderer = std::env::var("RSXP_RENDERER").unwrap_or_else(|_| "auto".into());
        let mut script = None;
        let mut game = None;
        let mut headless = false;
        let mut frames = None;
        let mut output = None;
        let mut version = None;
        let mut ruby_profile = rsxp_core::ruby::profiling::Options::default();
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--ruby-profile" => {
                    ruby_profile.output = Some(PathBuf::from(
                        args.next().ok_or("--ruby-profile requires FILE")?,
                    ))
                }
                "--ruby-profile-mode" => {
                    ruby_profile.mode = args
                        .next()
                        .ok_or("--ruby-profile-mode requires cpu or wall")?
                        .clone()
                }
                "--ruby-profile-interval" => {
                    ruby_profile.interval = args
                        .next()
                        .ok_or("--ruby-profile-interval requires microseconds")?
                        .parse()?
                }
                "--ruby-profile-start-frame" => {
                    ruby_profile.start = args
                        .next()
                        .ok_or("--ruby-profile-start-frame requires N")?
                        .parse()?
                }
                "--ruby-profile-frames" => {
                    ruby_profile.frames = Some(
                        args.next()
                            .ok_or("--ruby-profile-frames requires N")?
                            .parse()?,
                    )
                }
                "--renderer" => {
                    renderer = args
                        .next()
                        .ok_or("--renderer requires auto, software or wgpu")?
                        .clone();
                }
                "--script" => {
                    script = Some(PathBuf::from(
                        args.next().ok_or("--script requires a path")?,
                    ))
                }
                "--game" => {
                    game = Some(PathBuf::from(
                        args.next().ok_or("--game requires a directory")?,
                    ))
                }
                "--headless" => headless = true,
                "--trace-scripts" => set_startup_env("RSXP_TRACE", "1"),
                "--profile" => {
                    // Accepted for compatibility; the plugin repairs it once gated
                    // are now applied automatically.
                    let _ = args.next().ok_or("--profile requires a name")?;
                }
                "--frames" => {
                    let n: u64 = args
                        .next()
                        .ok_or("--frames requires a positive integer")?
                        .parse()?;
                    if n == 0 {
                        return Err("--frames must be positive".into());
                    }
                    frames = Some(n);
                }
                "--output" => {
                    output = Some(PathBuf::from(
                        args.next().ok_or("--output requires a path")?,
                    ))
                }
                "--rgss" => {
                    let n: u8 = args.next().ok_or("--rgss requires 1, 2 or 3")?.parse()?;
                    if !(1..=3).contains(&n) {
                        return Err("--rgss requires 1, 2 or 3".into());
                    }
                    version = Some(n);
                }
                other => return Err(format!("unknown runtime argument: {other}").into()),
            }
        }
        if !["auto", "software", "wgpu"].contains(&renderer.as_str()) {
            return Err("--renderer requires auto, software or wgpu".into());
        }
        if script.is_some() == game.is_some() {
            return Err("specify exactly one of --script FILE or --game DIRECTORY".into());
        }
        ruby_profile.validate()?;
        if let Some(path) = &mut ruby_profile.output {
            if !path.is_absolute() {
                *path = std::env::current_dir()?.join(&path);
            }
            if path.is_dir() {
                return Err("--ruby-profile requires a file path, not a directory".into());
            }
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
        }
        // Resolve output before switching to the game's working directory.
        let output = output.map(|p| {
            if p.is_absolute() {
                p
            } else {
                std::env::current_dir().unwrap().join(p)
            }
        });
        Ok(Self {
            renderer,
            script,
            game,
            headless,
            frames,
            output,
            version,
            ruby_profile,
        })
    }
}
