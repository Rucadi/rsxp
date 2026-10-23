// SPDX-License-Identifier: GPL-2.0-or-later
//! Core-agnostic frontend building blocks.
//!
//! This crate owns the parts of a frontend that must not know whether it is
//! running RGSS, GBA, GB or DS: content routing, the core catalog and the
//! dedicated core thread. Window/audio/egui implementations live in a concrete
//! frontend binary and enter through [`rsxp_core::CoreContext`].

use rsxp_core::{
    Content, ContentProbe, Core, CoreContext, CoreInfo, CoreRegistration, CoreRegistry, CoreResult,
    ExitStatus,
};
use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

const HEADER_BYTES: usize = 64;

/// Routing failure for content that no registered core claims.
#[derive(Debug)]
pub enum RouteError {
    NoCore { path: String },
    Io(String),
}

impl fmt::Display for RouteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoCore { path } => write!(f, "no installed core supports {path}"),
            Self::Io(message) => f.write_str(message),
        }
    }
}

impl Error for RouteError {}

/// Registry plus content probing. The desktop, web and mobile frontends share
/// the selection logic so a new core only has to be registered once.
#[derive(Default)]
pub struct CoreCatalog {
    registry: CoreRegistry,
}

impl CoreCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, registration: CoreRegistration) {
        self.registry.register(registration);
    }

    pub fn get(&self, id: &str) -> Option<&CoreRegistration> {
        self.registry.get(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &CoreRegistration> {
        self.registry.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.registry.is_empty()
    }

    /// Chooses the highest-priority core for `content`.
    pub fn route(&self, content: &Content) -> Result<CoreRegistration, RouteError> {
        let mut header = Vec::new();
        let is_dir = content.kind == rsxp_core::ContentKind::Directory;
        if !is_dir {
            read_header(&content.path, &mut header).map_err(|error| {
                RouteError::Io(format!(
                    "could not inspect {}: {error}",
                    content.path.display()
                ))
            })?;
        }
        let probe = ContentProbe::new(&content.path, is_dir, &header);
        self.registry
            .select(&probe)
            .copied()
            .ok_or_else(|| RouteError::NoCore {
                path: content.path.display().to_string(),
            })
    }

    /// Convenience: route then construct the core.
    pub fn create(&self, content: &Content) -> Result<Box<dyn Core>, RouteError> {
        let registration = self.route(content)?;
        Ok((registration.create)())
    }
}

fn read_header(path: &Path, out: &mut Vec<u8>) -> std::io::Result<()> {
    let mut file = File::open(path)?;
    out.resize(HEADER_BYTES, 0);
    let read = file.read(out)?;
    out.truncate(read);
    Ok(())
}

/// A running core thread and the control plane used to talk to it.
pub struct CoreHandle {
    info: CoreInfo,
    control: Arc<rsxp_core::CoreControl>,
    finished: Arc<AtomicBool>,
    join: Option<JoinHandle<CoreResult<ExitStatus>>>,
}

impl CoreHandle {
    pub fn info(&self) -> &CoreInfo {
        &self.info
    }

    pub fn control(&self) -> &Arc<rsxp_core::CoreControl> {
        &self.control
    }

    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    /// Requests a clean stop. The core observes this at its next frame or
    /// scheduler boundary.
    pub fn stop(&self) {
        self.control.stop();
    }

    /// Waits for the core thread and returns its result.
    pub fn join(mut self) -> std::thread::Result<CoreResult<ExitStatus>> {
        self.join.take().expect("core handle joined twice").join()
    }
}

impl Drop for CoreHandle {
    fn drop(&mut self) {
        self.control.stop();
    }
}

/// Runs a core on a dedicated thread.
///
/// The factory is invoked on the new thread, so MRI and other thread-affine
/// state are created where they will run and never cross a thread boundary.
/// The thread name includes the core id to make profiler traces readable. The
/// join handle is owned by [`CoreHandle`]; dropping the handle requests a stop
/// but does not block the UI thread waiting for the game to unwind.
pub fn spawn_core(
    registration: CoreRegistration,
    content: Content,
    context: CoreContext,
) -> std::io::Result<CoreHandle> {
    let info = registration.info;
    let control = Arc::clone(&context.control);
    let finished = Arc::new(AtomicBool::new(false));
    let thread_finished = Arc::clone(&finished);
    let join = std::thread::Builder::new()
        .name(format!("rsxp-{}-core", info.id))
        .spawn(move || {
            let result = (|| {
                let mut core = (registration.create)();
                core.load(content, context)?;
                core.run()
            })();
            thread_finished.store(true, Ordering::Release);
            result
        })?;
    Ok(CoreHandle {
        info,
        control,
        finished,
        join: Some(join),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsxp_core::audio::NullAudioSink;
    use rsxp_core::events::NullEventSink;
    use rsxp_core::video::NullVideoSink;
    use rsxp_core::{
        CoreCapabilities, CoreControl, CoreError, OptionValue, Platform, PresentationSupport,
        WindowCommand,
    };

    struct TestPlatform;
    impl Platform for TestPlatform {
        fn read_file(&self, _path: &str) -> Option<Vec<u8>> {
            None
        }
        fn save_dir(&self) -> std::path::PathBuf {
            std::env::temp_dir()
        }
    }

    struct TestCore(CoreInfo);
    impl Core for TestCore {
        fn info(&self) -> &CoreInfo {
            &self.0
        }
        fn load(&mut self, _content: Content, _context: CoreContext) -> CoreResult<()> {
            Ok(())
        }
        fn run(&mut self) -> CoreResult<ExitStatus> {
            Ok(ExitStatus::success())
        }
    }

    const INFO: CoreInfo = CoreInfo {
        id: "test",
        name: "Test",
        version: "1",
        extensions: &["test"],
        capabilities: CoreCapabilities::empty(),
        presentation: PresentationSupport::Cpu,
    };

    fn create() -> Box<dyn Core> {
        Box::new(TestCore(INFO))
    }
    fn detect(probe: &ContentProbe<'_>) -> u32 {
        if probe.has_extension("test") {
            10
        } else {
            0
        }
    }

    fn registration() -> CoreRegistration {
        CoreRegistration {
            info: INFO,
            create,
            detect,
        }
    }

    fn context() -> CoreContext {
        CoreContext::new(
            Arc::new(TestPlatform),
            Box::new(NullVideoSink::default()),
            Box::new(NullAudioSink),
            Box::new(NullEventSink),
            Arc::new(CoreControl::new()),
        )
    }

    #[test]
    fn catalog_routes_by_extension() {
        let mut catalog = CoreCatalog::new();
        catalog.register(registration());
        let path = std::env::temp_dir().join(format!("rsxp-route-{}.test", std::process::id()));
        std::fs::write(&path, b"test").unwrap();
        let content = Content::file(&path);
        let routed = catalog.route(&content).unwrap();
        assert_eq!(routed.info.id, "test");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn unknown_content_is_rejected() {
        let catalog = CoreCatalog::new();
        let path = std::env::temp_dir().join(format!(
            "rsxp-unknown-{}-{}.zzz",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, b"unknown").unwrap();
        let content = Content::file(&path);
        assert!(matches!(
            catalog.route(&content),
            Err(RouteError::NoCore { .. })
        ));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn core_runs_on_its_own_thread() {
        let mut catalog = CoreCatalog::new();
        catalog.register(registration());
        let content = Content::file("x.test");
        let handle = spawn_core(registration(), content, context()).unwrap();
        assert!(handle.join().unwrap().unwrap().is_success());
    }

    #[test]
    fn commands_and_events_are_core_agnostic() {
        let control = CoreControl::new();
        assert!(control.send(rsxp_core::CoreCommand::SetOption {
            key: "renderer".into(),
            value: OptionValue::String("software".into()),
        }));
        let mut commands = Vec::new();
        control.take_commands(&mut commands);
        assert_eq!(commands.len(), 1);
        let _ = CoreError::unsupported("test");
        let _ = WindowCommand::SetTitle("x".into());
    }
}
