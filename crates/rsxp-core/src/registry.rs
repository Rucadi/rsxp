// SPDX-License-Identifier: GPL-2.0-or-later
//! Core metadata, capabilities and content routing.

use crate::{ContentProbe, Core};
use std::collections::BTreeMap;

/// Optional features advertised by a core.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CoreCapabilities(u32);

impl CoreCapabilities {
    pub const SAVE_STATES: Self = Self(1 << 0);
    pub const SAVE_RAM: Self = Self(1 << 1);
    pub const CHEATS: Self = Self(1 << 2);
    pub const REWIND: Self = Self(1 << 3);
    pub const RUMBLE: Self = Self(1 << 4);
    pub const MULTI_SCREEN: Self = Self(1 << 5);
    pub const ACCELERATED_PRESENTATION: Self = Self(1 << 6);
    pub const SCREENSHOTS: Self = Self(1 << 7);
    pub const RESET: Self = Self(1 << 8);

    pub const fn empty() -> Self {
        Self(0)
    }
    pub const fn all() -> Self {
        Self(u32::MAX)
    }
    pub const fn bits(self) -> u32 {
        self.0
    }
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl std::ops::BitOr for CoreCapabilities {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl std::ops::BitOrAssign for CoreCapabilities {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// How a core delivers completed frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresentationSupport {
    /// The core always submits CPU pixels through [`crate::VideoSink::present`].
    Cpu,
    /// The core may submit an encoded packet through
    /// [`crate::VideoSink::present_packet`]. `cpu_fallback` says whether the
    /// core can also produce a CPU frame for frontends that do not understand
    /// `kind`.
    Accelerated {
        kind: &'static str,
        cpu_fallback: bool,
    },
}

/// Static description of one core.
#[derive(Clone, Copy, Debug)]
pub struct CoreInfo {
    /// Stable routing id, for example `rgss`, `gba`, `gb` or `nds`.
    pub id: &'static str,
    pub name: &'static str,
    pub version: &'static str,
    /// Lower-case file extensions without a leading dot. May be empty for
    /// cores that route directories through a probe.
    pub extensions: &'static [&'static str],
    pub capabilities: CoreCapabilities,
    pub presentation: PresentationSupport,
}

/// Builds a fresh core instance for one session.
pub type CoreFactory = fn() -> Box<dyn Core>;

/// Returns a priority for `probe`; `0` means the core does not claim it.
///
/// Higher values win when several cores match. File-extension matches are
/// intentionally low priority so a magic/header match can override them.
pub type ContentMatcher = fn(&ContentProbe<'_>) -> u32;

/// One entry in the frontend's core catalog.
#[derive(Clone, Copy)]
pub struct CoreRegistration {
    pub info: CoreInfo,
    pub create: CoreFactory,
    pub detect: ContentMatcher,
}

/// Ordered catalog of available cores.
#[derive(Default)]
pub struct CoreRegistry {
    entries: Vec<CoreRegistration>,
    by_id: BTreeMap<&'static str, usize>,
}

impl CoreRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a core. Replaces an existing registration with the same id.
    pub fn register(&mut self, registration: CoreRegistration) {
        if let Some(&index) = self.by_id.get(registration.info.id) {
            self.entries[index] = registration;
        } else {
            self.by_id.insert(registration.info.id, self.entries.len());
            self.entries.push(registration);
        }
    }

    pub fn get(&self, id: &str) -> Option<&CoreRegistration> {
        self.by_id
            .get(id)
            .and_then(|&index| self.entries.get(index))
    }

    pub fn iter(&self) -> impl Iterator<Item = &CoreRegistration> {
        self.entries.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Selects the highest-priority core that claims `probe`.
    ///
    /// Ties are broken by registration order so routing is deterministic.
    pub fn select(&self, probe: &ContentProbe<'_>) -> Option<&CoreRegistration> {
        self.entries
            .iter()
            .filter_map(|registration| {
                let priority = (registration.detect)(probe);
                (priority > 0).then_some((priority, registration))
            })
            .max_by_key(|(priority, _)| *priority)
            .map(|(_, registration)| registration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Content, CoreContext, CoreError, ExitStatus};
    use std::path::Path;

    struct Dummy(CoreInfo);
    impl Core for Dummy {
        fn info(&self) -> &CoreInfo {
            &self.0
        }
        fn load(&mut self, _: Content, _: CoreContext) -> crate::CoreResult<()> {
            Ok(())
        }
        fn run(&mut self) -> crate::CoreResult<ExitStatus> {
            Err(CoreError::Stopped)
        }
    }
    fn create() -> Box<dyn Core> {
        Box::new(Dummy(CoreInfo {
            id: "dummy",
            name: "Dummy",
            version: "1",
            extensions: &["dummy"],
            capabilities: CoreCapabilities::SAVE_RAM,
            presentation: PresentationSupport::Cpu,
        }))
    }
    fn detect(probe: &ContentProbe<'_>) -> u32 {
        u32::from(probe.extension() == Some("dummy")) * 10
    }

    #[test]
    fn registry_selects_by_priority() {
        let mut registry = CoreRegistry::new();
        registry.register(CoreRegistration {
            info: CoreInfo {
                id: "dummy",
                name: "Dummy",
                version: "1",
                extensions: &["dummy"],
                capabilities: CoreCapabilities::empty(),
                presentation: PresentationSupport::Cpu,
            },
            create,
            detect,
        });
        let path = Path::new("game.dummy");
        let probe = ContentProbe::new(path, false, &[]);
        assert_eq!(registry.select(&probe).unwrap().info.id, "dummy");
        assert!(registry.get("missing").is_none());
    }
}
