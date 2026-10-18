// SPDX-License-Identifier: GPL-2.0-or-later
//! Optional desktop-side Ruby sampling profiler, disabled by default.
//!
//! [`Options`] mirrors the `--ruby-profile*` CLI flags (output path, `cpu`/`wall`
//! mode, sampling interval, start frame and frame count); [`Options::validate`]
//! enforces those constraints. When no output path is set, [`install`] is a no-op:
//! the bundled `sampler.rb` and its `stackprof` dependency are only loaded on demand,
//! so profiling costs nothing when disabled.
//!
//! [`install`] evaluates `sampler.rb` (which defines `RSXP::RubyProfiler`), configures
//! it, and starts sampling immediately when `start == 0`. Otherwise a thread-local
//! `Recording` tracks frame numbers: the runtime calls [`frame`] once per update to
//! open and close the requested recording window, and [`finish`] flushes it at
//! shutdown. If the requested start frame is never reached, a warning goes to stderr.
//! [`module`] resolves the `RSXP::RubyProfiler` module defined by `sampler.rb`.
use magnus::{prelude::*, Error, RModule, Ruby, Value};
use std::{cell::RefCell, path::PathBuf};

pub struct Options {
    pub output: Option<PathBuf>,
    pub mode: String,
    pub interval: u64,
    pub start: u64,
    pub frames: Option<u64>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            output: None,
            mode: "cpu".into(),
            interval: 1000,
            start: 0,
            frames: None,
        }
    }
}
impl Options {
    pub fn validate(&self) -> Result<(), String> {
        if !["cpu", "wall"].contains(&self.mode.as_str()) {
            return Err("--ruby-profile-mode requires cpu or wall".into());
        }
        if !(100..=1_000_000).contains(&self.interval) {
            return Err("--ruby-profile-interval must be 100..1000000 microseconds".into());
        }
        if self.frames == Some(0)
            || self
                .frames
                .is_some_and(|frames| self.start.checked_add(frames).is_none())
        {
            return Err(
                "--ruby-profile-frames must be positive and fit the recording window".into(),
            );
        }
        if self.output.is_none()
            && (self.start != 0
                || self.frames.is_some()
                || self.mode != "cpu"
                || self.interval != 1000)
        {
            return Err("Ruby profiling options require --ruby-profile FILE".into());
        }
        Ok(())
    }
}
#[derive(Clone, Copy)]
enum Action {
    Start,
    Finish,
}
struct Recording {
    frame: u64,
    start: u64,
    end: Option<u64>,
    active: bool,
    finished: bool,
}
thread_local! { static RECORDING: RefCell<Option<Recording>> = const { RefCell::new(None) }; }

fn module(ruby: &Ruby) -> Result<RModule, Error> {
    ruby.eval("RSXP::RubyProfiler")
}

pub fn install(ruby: &Ruby, options: &Options) -> Result<(), Error> {
    let Some(path) = &options.output else {
        return Ok(());
    };
    let profiler: RModule = ruby.eval(include_str!("sampler.rb"))?;
    profiler.funcall::<_, _, Value>(
        "configure",
        (
            path.to_string_lossy().as_ref(),
            options.mode.as_str(),
            options.interval,
        ),
    )?;
    if options.start == 0 {
        profiler.funcall::<_, _, Value>("start", (0,))?;
    }
    RECORDING.with(|slot| {
        *slot.borrow_mut() = Some(Recording {
            frame: 0,
            start: options.start,
            end: options.frames.map(|n| options.start + n),
            active: options.start == 0,
            finished: false,
        })
    });
    Ok(())
}
pub fn frame(ruby: &Ruby) -> Result<(), Error> {
    let action = RECORDING.with(|slot| {
        let mut slot = slot.borrow_mut();
        let state = slot.as_mut()?;
        state.frame += 1;
        if state.finished {
            return None;
        }
        if !state.active && state.frame >= state.start {
            state.active = true;
            return Some((Action::Start, state.frame));
        }
        if state.active && state.end.is_some_and(|end| state.frame >= end) {
            state.active = false;
            state.finished = true;
            return Some((Action::Finish, state.frame));
        }
        None
    });
    if let Some((action, frame)) = action {
        module(ruby)?.funcall::<_, _, Value>(
            match action {
                Action::Start => "start",
                Action::Finish => "finish",
            },
            (frame,),
        )?;
    }
    Ok(())
}
pub fn finish(ruby: &Ruby) -> Result<(), Error> {
    let state = RECORDING.with(|slot| slot.borrow_mut().take());
    if let Some(state) = state {
        if state.active {
            module(ruby)?.funcall::<_, _, Value>("finish", (state.frame,))?;
        } else if !state.finished {
            eprintln!(
                "Ruby profiling window was not reached ({} updates; requested start {}).",
                state.frame, state.start
            );
        }
    }
    Ok(())
}
