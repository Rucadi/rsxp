// SPDX-License-Identifier: GPL-2.0-or-later
//! RGSS core adapter.
//!
//! This is the first implementation of [`rsxp_core::Core`]. It keeps the
//! existing RGSS engine and MRI bindings inside the core crate; the frontend
//! only sees video, audio, input, control and lifecycle events.
//!
//! The adapter currently assumes it owns the process working directory, because
//! MRI and the RGSS script loader use `Dir.pwd`. That is already true of the
//! isolated desktop runner. An in-process frontend must give the RGSS core an
//! exclusive worker process before running another core in the same process.

mod game;
mod host;

use rsxp::engine::{self, RuntimeOptions};
use rsxp::{api_bridge, runtime_patch, runtime_settings};
use rsxp_core::{
    Content, ContentProbe, Core, CoreCapabilities, CoreContext, CoreError, CoreInfo,
    CoreRegistration, CoreRegistry, CoreResult, ExitStatus, PixelFormat, PresentationSupport,
};
use ruby_rgss_core::RubyCore;
use std::path::Path;
use std::sync::Arc;

/// Stable core id used by content routing and save-state metadata.
pub const CORE_ID: &str = "rgss";

/// Capabilities advertised by the current adapter.
pub const INFO: CoreInfo = CoreInfo {
    id: CORE_ID,
    name: "RPG Maker RGSS",
    version: env!("CARGO_PKG_VERSION"),
    state_revision: 1,
    extensions: &[],
    capabilities: CoreCapabilities::ACCELERATED_PRESENTATION,
    presentation: PresentationSupport::Accelerated {
        kind: host::GPU_PACKET_KIND,
        cpu_fallback: true,
    },
    pixel_formats: &[PixelFormat::Bgra8, PixelFormat::Rgba8],
    audio: rsxp_core::AudioInfo::new(48_000, 2, false),
    input: rsxp_core::InputInfo {
        buttons: &[],
        pointer: true,
        pointer_button: true,
    },
    screens: &[],
    options: &[],
};

/// Registers the RGSS core with a frontend catalog.
pub fn register(registry: &mut CoreRegistry) {
    registry.register(registration());
}

/// Returns the registration entry for build-time core catalogs.
pub fn registration() -> CoreRegistration {
    CoreRegistration {
        info: INFO,
        create,
        detect,
    }
}

/// Creates an unloaded RGSS core.
pub fn create() -> Box<dyn Core> {
    Box::new(RgssCore {
        ruby: None,
        control: None,
    })
}

/// Content detection for directories and script files.
///
/// A directory must contain the usual RGSS entry points. A single file is
/// accepted when it is a script bundle or `Game.ini`.
pub fn detect(probe: &ContentProbe<'_>) -> u32 {
    if probe.is_dir {
        let root = probe.path;
        let has_ini = root.join("Game.ini").is_file();
        let has_scripts = [
            "Data/Scripts.rxdata",
            "Data/Scripts.rvdata",
            "Data/Scripts.rvdata2",
        ]
        .iter()
        .any(|path| root.join(path).is_file());
        let has_archive = ["Game.rgssad", "Game.rgss2a", "Game.rgss3a"]
            .iter()
            .any(|path| root.join(path).is_file());
        return if has_ini || has_scripts || has_archive {
            50
        } else {
            0
        };
    }
    if probe.has_file_name("Game.ini") {
        return 40;
    }
    match probe.extension() {
        Some(extension)
            if extension.eq_ignore_ascii_case("rxdata")
                || extension.eq_ignore_ascii_case("rvdata")
                || extension.eq_ignore_ascii_case("rvdata2") =>
        {
            30
        }
        _ => 0,
    }
}

struct RgssCore {
    ruby: Option<RubyCore>,
    control: Option<Arc<rsxp_core::CoreControl>>,
}

impl Core for RgssCore {
    fn info(&self) -> &CoreInfo {
        &INFO
    }

    fn load(&mut self, content: Content, context: CoreContext) -> CoreResult<()> {
        let config = game::prepare(&content, context.platform.as_ref())?;
        let control = Arc::clone(&context.control);
        let mut host = host::CoreHost::new(context, config.width, config.height)
            .map_err(CoreError::runtime)?;
        host.emit_loaded(config.width, config.height);

        let runtime = engine::Runtime::new(RuntimeOptions {
            width: config.width,
            height: config.height,
            version: config.version,
            headless: false,
            frame_limit: None,
            rate: config.rate,
            fast_forward: false,
            archives: config.archives,
            scripted: Vec::new(),
            host: Box::new(host),
        })
        .map_err(CoreError::runtime)?;
        engine::set_runtime(runtime);
        api_bridge::initialize();
        runtime_settings::reset();
        runtime_patch::reset();
        runtime_patch::load_user_patches(Path::new("."))
            .map_err(|error| CoreError::runtime(error.to_string()))?;
        if let Ok(value) = std::env::var("RSXP_PATCH") {
            for id in runtime_patch::enable_from(&value) {
                eprintln!("Runtime patch enabled: {id}");
            }
        }
        self.control = Some(control);
        self.ruby = Some(RubyCore::prepare().map_err(CoreError::runtime)?);
        Ok(())
    }

    fn run(&mut self) -> CoreResult<ExitStatus> {
        let mut ruby = self
            .ruby
            .take()
            .ok_or_else(|| CoreError::runtime("RGSS core was not loaded"))?;
        if let Some(control) = &self.control {
            runtime_settings::set_fast_forward(control.fast_forward());
            runtime_settings::set_rate_override(0);
        }
        let status = ruby.start();
        // Run Ruby finalizers while the native resource registry still exists.
        drop(ruby);
        let _ = engine::take_runtime();
        let code = status.map_err(CoreError::runtime)?;
        Ok(ExitStatus::new(i32::try_from(code).unwrap_or(1)))
    }
}
