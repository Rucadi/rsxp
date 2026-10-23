// SPDX-License-Identifier: GPL-2.0-or-later
//! Build-time core registry.
//!
//! A frontend enables only the cores it ships. Disabled cores are not linked,
//! which keeps a GBA-only build free of MRI, egui and RGSS dependencies. The
//! generic routing logic lives in [`rsxp_frontend::CoreCatalog`].

use rsxp_frontend::CoreCatalog;

/// Builds the catalog for every core enabled by Cargo features.
pub fn catalog() -> CoreCatalog {
    let mut catalog = CoreCatalog::new();
    #[cfg(feature = "rgss")]
    catalog.register(rsxp_core_rgss::registration());
    #[cfg(feature = "gba")]
    catalog.register(rsxp_core_gba::registration());
    catalog
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_cores_are_registered() {
        let catalog = catalog();
        #[cfg(feature = "gba")]
        assert!(catalog.get("gba").is_some());
        #[cfg(feature = "rgss")]
        assert!(catalog.get("rgss").is_some());
    }

    #[test]
    fn gba_content_routes_by_extension() {
        let catalog = catalog();
        let path = std::env::temp_dir().join(format!(
            "rsxp-core-registry-{}-{}.gba",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, vec![0u8; 0x200]).unwrap();
        let content = rsxp_core::Content::file(&path);
        #[cfg(feature = "gba")]
        assert_eq!(catalog.route(&content).unwrap().info.id, "gba");
        let _ = std::fs::remove_file(path);
    }
}
