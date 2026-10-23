// SPDX-License-Identifier: GPL-2.0-or-later
//! Emulator core contract shared by RSXP cores and frontends.
//!
//! This crate deliberately has no dependency on RGSS, MRI, wgpu, egui or a
//! frontend. A core crate implements [`Core`]; a frontend builds a
//! [`CoreContext`] and calls [`Core::run`] once for the whole session.
//!
//! The hot path never crosses a `step`-style trait object: the core owns its
//! event loop, CPU state, decoded assets and caches. It reports completed
//! frames and audio through borrowed sink buffers, and reads input from
//! lock-free atomics. See `docs/multi-core-architecture.md` for the rules.
//!
//! # Example
//!
//! ```ignore
//! let content = Content::directory("/games/example");
//! let handle = rsxp_frontend::spawn_core(registration, content, context)?;
//! // The frontend thread owns presentation; the core thread owns emulation.
//! let status = handle.join()?;
//! ```

use std::error::Error;
use std::fmt;

pub mod audio;
pub mod content;
pub mod control;
pub mod events;
pub mod input;
pub mod platform;
pub mod registry;
pub mod video;

pub use audio::{AudioChannel, AudioSink, AudioStream, PcmBuffer};
pub use content::{Content, ContentKind, ContentProbe};
pub use control::{CoreCommand, CoreControl, OptionValue};
pub use events::{CoreEvent, CoreLoadInfo, EventSink, ExitStatus, LogLevel, WindowCommand};
pub use input::{
    Button, ButtonSet, InputEvent, InputSnapshot, InputState, AXIS_COUNT, BUTTON_COUNT,
};
pub use platform::Platform;
pub use registry::{
    CoreCapabilities, CoreInfo, CoreRegistration, CoreRegistry, PresentationSupport,
};
pub use video::{PixelFormat, VideoFrame, VideoSink};

/// Version of the source-level core contract. Cores and frontends built
/// against a different major version must not be mixed.
pub const CORE_API_VERSION: u32 = 1;

/// Errors reported by a core while loading content or running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    /// Content is missing, unreadable or not supported by this core.
    Content(String),
    /// The core reached an invalid internal or emulated state.
    Runtime(String),
    /// A frontend capability required by the core is unavailable.
    Unsupported(String),
    /// The core stopped because the frontend requested it.
    Stopped,
}

impl CoreError {
    pub fn content(message: impl Into<String>) -> Self {
        Self::Content(message.into())
    }
    pub fn runtime(message: impl Into<String>) -> Self {
        Self::Runtime(message.into())
    }
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::Unsupported(message.into())
    }
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Content(message) => write!(f, "core content error: {message}"),
            Self::Runtime(message) => write!(f, "core runtime error: {message}"),
            Self::Unsupported(message) => write!(f, "core does not support {message}"),
            Self::Stopped => f.write_str("core stopped"),
        }
    }
}

impl Error for CoreError {}

pub type CoreResult<T> = Result<T, CoreError>;

/// One emulator implementation.
///
/// `run` owns the emulation loop for the whole session. The frontend observes
/// the core exclusively through the context sinks and [`CoreControl`], so no
/// frontend code executes between emulated frames unless the core explicitly
/// calls a sink.
///
/// Cores are constructed **on the thread that will run them**. This matters for
/// MRI and for cores with thread-affine GPU/CPU state; the frontend never moves
/// a live core across threads.
pub trait Core: 'static {
    /// Static description used for routing, UI and save-state validation.
    fn info(&self) -> &CoreInfo;

    /// Resolve and load content, decode assets and prepare the first frame.
    ///
    /// This may block for as long as loading a game takes. It must publish
    /// [`CoreEvent::Loaded`](crate::CoreEvent::Loaded) once the core knows its
    /// initial video/audio format.
    fn load(&mut self, content: Content, context: CoreContext) -> CoreResult<()>;

    /// Run until the game exits or [`CoreControl::stop`] is observed.
    ///
    /// Returning `Ok` means a clean stop. Errors are reported to the user by
    /// the frontend; the core must not panic across this boundary.
    fn run(&mut self) -> CoreResult<ExitStatus>;
}

/// Services owned by the frontend and handed to a core at load time.
///
/// Sinks are stored by the core and called from the core thread. They are
/// deliberately not `Clone`: a core must not create a second presentation or
/// audio path behind the frontend's back.
pub struct CoreContext {
    pub platform: std::sync::Arc<dyn Platform>,
    pub video: Box<dyn VideoSink>,
    pub audio: Box<dyn AudioSink>,
    pub events: Box<dyn EventSink>,
    pub control: std::sync::Arc<CoreControl>,
}

impl CoreContext {
    pub fn new(
        platform: std::sync::Arc<dyn Platform>,
        video: Box<dyn VideoSink>,
        audio: Box<dyn AudioSink>,
        events: Box<dyn EventSink>,
        control: std::sync::Arc<CoreControl>,
    ) -> Self {
        Self {
            platform,
            video,
            audio,
            events,
            control,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct FakePlatform;
    impl Platform for FakePlatform {
        fn read_file(&self, _path: &str) -> Option<Vec<u8>> {
            None
        }
        fn save_dir(&self) -> std::path::PathBuf {
            std::path::PathBuf::new()
        }
    }

    struct FakeVideo(AtomicU32);
    impl VideoSink for FakeVideo {
        fn present(&mut self, _frame: VideoFrame<'_>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    struct FakeAudio;
    impl AudioSink for FakeAudio {}

    struct FakeEvents;
    impl EventSink for FakeEvents {
        fn emit(&mut self, _event: CoreEvent) {}
    }

    struct FakeCore {
        info: CoreInfo,
        frames: u32,
    }
    impl Core for FakeCore {
        fn info(&self) -> &CoreInfo {
            &self.info
        }
        fn load(&mut self, _content: Content, mut context: CoreContext) -> CoreResult<()> {
            let frame = VideoFrame::rgba8(1, 1, 4, &[0, 0, 0, 255]);
            context.video.present(frame);
            self.frames = 1;
            Ok(())
        }
        fn run(&mut self) -> CoreResult<ExitStatus> {
            Ok(ExitStatus::success())
        }
    }

    #[test]
    fn context_carries_frontend_services() {
        let info = CoreInfo {
            id: "fake",
            name: "Fake",
            version: "0",
            extensions: &[],
            capabilities: CoreCapabilities::empty(),
            presentation: PresentationSupport::Cpu,
        };
        let mut core = FakeCore { info, frames: 0 };
        let context = CoreContext::new(
            std::sync::Arc::new(FakePlatform),
            Box::new(FakeVideo(AtomicU32::new(0))),
            Box::new(FakeAudio),
            Box::new(FakeEvents),
            std::sync::Arc::new(CoreControl::new()),
        );
        core.load(Content::file("x"), context).unwrap();
        assert_eq!(core.frames, 1);
        assert!(core.run().unwrap().is_success());
    }
}
