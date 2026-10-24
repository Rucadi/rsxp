// SPDX-License-Identifier: GPL-2.0-or-later
//! Core-agnostic frontend building blocks.
//!
//! This crate owns the parts of a frontend that must not know whether it is
//! running RGSS, GBA, GB or DS: content routing, the core catalog, the
//! dedicated core thread, content identity and save storage. Window/audio/egui
//! implementations live in a concrete frontend binary and enter through
//! [`rsxp_core::CoreContext`].

use rsxp_core::{
    Content, ContentProbe, Core, CoreContext, CoreEvent, CoreInfo, CoreRegistration, CoreRegistry,
    CoreResult, EventSink, ExitStatus,
};
use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

pub mod storage;
pub mod video;

pub use storage::{
    ContentId, FsStorage, MemoryStorage, SaveStore, Storage, StorageError, StorageSubmitter,
    StorageWorker, WorkerStatus,
};

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
        self.route_probe(&probe)
    }

    /// Chooses a core from a probe whose bytes were supplied by the caller.
    ///
    /// Browser and archive-backed hosts already own the content bytes and use
    /// this entry point to share routing policy without native filesystem I/O.
    pub fn route_probe(&self, probe: &ContentProbe<'_>) -> Result<CoreRegistration, RouteError> {
        self.registry
            .select(probe)
            .copied()
            .ok_or_else(|| RouteError::NoCore {
                path: probe.path.display().to_string(),
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

    /// True once the core thread has ended, including after a panic or a load
    /// failure. The flag is set by an unwind-safe guard, so it cannot remain
    /// false forever after the worker has terminated.
    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    /// Requests a clean stop. The core observes this at its next frame or
    /// scheduler boundary.
    pub fn stop(&self) {
        self.control.stop();
    }

    /// Requests a stop and waits for the worker to unwind. This is the explicit
    /// shutdown path for a host that owns the handle and can afford to wait.
    pub fn shutdown(mut self) -> std::thread::Result<CoreResult<ExitStatus>> {
        self.control.stop();
        self.join.take().expect("core handle joined twice").join()
    }

    /// Waits for the core thread and returns its result.
    ///
    /// `Ok(Err(_))` is a normal core failure; `Err(_)` is a panic in the core
    /// factory, load or run path.
    pub fn join(mut self) -> std::thread::Result<CoreResult<ExitStatus>> {
        self.join.take().expect("core handle joined twice").join()
    }
}

impl Drop for CoreHandle {
    fn drop(&mut self) {
        // Non-blocking ownership transfer: request a stop but never join from a
        // UI thread. Use `shutdown` when the caller must observe completion.
        self.control.stop();
    }
}

/// Sets `finished` on every exit path, including unwinding.
struct CompletionGuard(Arc<AtomicBool>);

impl Drop for CompletionGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

/// Wraps the frontend sink so the session driver can guarantee exactly one
/// terminal `Exited` event even when the core panics.
#[derive(Clone)]
struct SharedEvents {
    inner: Arc<Mutex<Box<dyn EventSink>>>,
    exit_emitted: Arc<AtomicBool>,
}

impl SharedEvents {
    fn new(inner: Box<dyn EventSink>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(inner)),
            exit_emitted: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl EventSink for SharedEvents {
    fn emit(&mut self, event: CoreEvent) {
        if matches!(event, CoreEvent::Exited(_)) {
            self.exit_emitted.store(true, Ordering::Release);
        }
        self.inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .emit(event);
    }
}

/// Emits one `Exited` event if the core did not. The status is written before
/// normal return and remains `1` when the closure unwinds.
struct TerminalGuard {
    events: Arc<Mutex<Box<dyn EventSink>>>,
    exit_emitted: Arc<AtomicBool>,
    status: Arc<AtomicI32>,
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.exit_emitted.swap(true, Ordering::AcqRel) {
            return;
        }
        let code = self.status.load(Ordering::Acquire);
        self.events
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .emit(CoreEvent::Exited(ExitStatus::new(code)));
    }
}

/// Runs a core on a dedicated thread.
///
/// The factory is invoked on the new thread, so MRI and other thread-affine
/// state are created where they will run and never cross a thread boundary.
/// The thread name includes the core id to make profiler traces readable. The
/// join handle is owned by [`CoreHandle`]; dropping the handle requests a stop
/// but does not block the UI thread waiting for the game to unwind.
///
/// Every terminated worker becomes observable through [`CoreHandle::is_finished`]
/// and exactly one terminal `CoreEvent::Exited` reaches the event sink.
pub fn spawn_core(
    registration: CoreRegistration,
    content: Content,
    context: CoreContext,
) -> std::io::Result<CoreHandle> {
    let info = registration.info;
    let control = Arc::clone(&context.control);
    let finished = Arc::new(AtomicBool::new(false));
    let thread_finished = Arc::clone(&finished);
    let events = SharedEvents::new(context.events);
    let terminal_events = Arc::clone(&events.inner);
    let terminal_flag = Arc::clone(&events.exit_emitted);
    let status = Arc::new(AtomicI32::new(1));
    let terminal_status = Arc::clone(&status);
    let context = CoreContext {
        platform: context.platform,
        video: context.video,
        audio: context.audio,
        events: Box::new(events),
        control: context.control,
        session: context.session,
    };
    let join = std::thread::Builder::new()
        .name(format!("rsxp-{}-core", info.id))
        .spawn(move || {
            let _finished = CompletionGuard(thread_finished);
            let _terminal = TerminalGuard {
                events: terminal_events,
                exit_emitted: terminal_flag,
                status: terminal_status,
            };
            let result = (|| {
                let mut core = (registration.create)();
                core.load(content, context)?;
                core.run()
            })();
            if let Ok(exit) = &result {
                status.store(exit.code, Ordering::Release);
            }
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
    use rsxp_core::registry::{AudioInfo, InputInfo};
    use rsxp_core::video::NullVideoSink;
    use rsxp_core::{
        CoreCapabilities, CoreControl, CoreError, OptionValue, PixelFormat, Platform,
        PresentationSupport, WindowCommand,
    };
    use std::time::{Duration, Instant};

    struct TestPlatform;
    impl Platform for TestPlatform {
        fn read_content(&self, path: &str) -> rsxp_core::PlatformResult<Vec<u8>> {
            Err(rsxp_core::PlatformError::not_found(path))
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
        state_revision: 1,
        extensions: &["test"],
        capabilities: CoreCapabilities::empty(),
        presentation: PresentationSupport::Cpu,
        pixel_formats: &[PixelFormat::Rgba8],
        audio: AudioInfo::SILENT,
        input: InputInfo::NONE,
        screens: &[],
        options: &[],
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

    fn wait_finished(handle: &CoreHandle) {
        let start = Instant::now();
        while !handle.is_finished() {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "worker did not finish"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[derive(Default)]
    struct RecordedEvents(Arc<Mutex<Vec<String>>>);
    impl EventSink for RecordedEvents {
        fn emit(&mut self, event: CoreEvent) {
            self.0.lock().unwrap().push(format!("{event:?}"));
        }
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
    fn catalog_routes_a_caller_supplied_probe_without_file_io() {
        let mut catalog = CoreCatalog::new();
        catalog.register(registration());
        let path = Path::new("browser-content.test");
        let probe = ContentProbe::new(path, false, b"already loaded");
        assert_eq!(catalog.route_probe(&probe).unwrap().info.id, "test");
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

    #[test]
    fn panics_are_observable_and_not_reported_as_success() {
        for phase in ["factory", "load", "run"] {
            let events = Arc::new(Mutex::new(Vec::new()));
            let sink = RecordedEvents(Arc::clone(&events));
            let mut context = CoreContext::new(
                Arc::new(TestPlatform),
                Box::new(NullVideoSink::default()),
                Box::new(NullAudioSink),
                Box::new(sink),
                Arc::new(CoreControl::new()),
            );
            context.session = rsxp_core::SessionConfig::default();
            let registration = match phase {
                "factory" => CoreRegistration {
                    info: INFO,
                    create: panic_factory,
                    detect,
                },
                "load" => CoreRegistration {
                    info: INFO,
                    create: panic_load,
                    detect,
                },
                _ => CoreRegistration {
                    info: INFO,
                    create: panic_run,
                    detect,
                },
            };
            let handle = spawn_core(registration, Content::file("x.test"), context).unwrap();
            wait_finished(&handle);
            assert!(handle.join().is_err(), "{phase} panic should propagate");
            let events = events.lock().unwrap();
            assert!(
                events.iter().any(|event| event.contains("Exited")),
                "{phase} should emit one terminal event: {events:?}"
            );
        }
    }

    struct PanicCore {
        phase: &'static str,
        info: CoreInfo,
    }
    fn panic_factory() -> Box<dyn Core> {
        panic!("factory panic")
    }
    fn panic_load() -> Box<dyn Core> {
        Box::new(PanicCore {
            phase: "load",
            info: INFO,
        })
    }
    fn panic_run() -> Box<dyn Core> {
        Box::new(PanicCore {
            phase: "run",
            info: INFO,
        })
    }
    impl Core for PanicCore {
        fn info(&self) -> &CoreInfo {
            &self.info
        }
        fn load(&mut self, _content: Content, _context: CoreContext) -> CoreResult<()> {
            if self.phase == "load" {
                panic!("load panic");
            }
            Ok(())
        }
        fn run(&mut self) -> CoreResult<ExitStatus> {
            panic!("run panic")
        }
    }

    #[test]
    fn load_failures_finish_and_emit_one_terminal_event() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let registration = CoreRegistration {
            info: INFO,
            create: || {
                Box::new(LoadFailure {
                    info: INFO,
                    calls: 0,
                })
            },
            detect,
        };
        let context = CoreContext::new(
            Arc::new(TestPlatform),
            Box::new(NullVideoSink::default()),
            Box::new(NullAudioSink),
            Box::new(RecordedEvents(Arc::clone(&events))),
            Arc::new(CoreControl::new()),
        );
        let handle = spawn_core(registration, Content::file("x.test"), context).unwrap();
        wait_finished(&handle);
        let result = handle.join().unwrap();
        assert!(matches!(result, Err(CoreError::Content(_))));
        let events = events.lock().unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| event.contains("Exited"))
                .count(),
            1
        );
    }

    struct LoadFailure {
        info: CoreInfo,
        calls: u8,
    }
    impl Core for LoadFailure {
        fn info(&self) -> &CoreInfo {
            &self.info
        }
        fn load(&mut self, _content: Content, _context: CoreContext) -> CoreResult<()> {
            self.calls += 1;
            Err(CoreError::content("bad content"))
        }
        fn run(&mut self) -> CoreResult<ExitStatus> {
            Ok(ExitStatus::success())
        }
    }

    #[test]
    fn stop_and_nonblocking_drop_are_explicit() {
        struct Blocking(CoreInfo);
        impl Core for Blocking {
            fn info(&self) -> &CoreInfo {
                &self.0
            }
            fn load(&mut self, _content: Content, _context: CoreContext) -> CoreResult<()> {
                Ok(())
            }
            fn run(&mut self) -> CoreResult<ExitStatus> {
                std::thread::sleep(Duration::from_secs(30));
                Ok(ExitStatus::success())
            }
        }
        fn blocking_create() -> Box<dyn Core> {
            Box::new(Blocking(INFO))
        }
        let registration = CoreRegistration {
            info: INFO,
            create: blocking_create,
            detect,
        };
        let handle = spawn_core(registration, Content::file("x.test"), context()).unwrap();
        let started = Instant::now();
        drop(handle);
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "dropping a handle must not wait for the core"
        );
    }

    #[test]
    fn stop_joins_a_cooperative_core() {
        struct Cooperative(CoreInfo);
        impl Core for Cooperative {
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
        fn cooperative_create() -> Box<dyn Core> {
            Box::new(Cooperative(INFO))
        }
        let registration = CoreRegistration {
            info: INFO,
            create: cooperative_create,
            detect,
        };
        let handle = spawn_core(registration, Content::file("x.test"), context()).unwrap();
        handle.stop();
        assert!(handle.shutdown().unwrap().unwrap().is_success());
    }
}
