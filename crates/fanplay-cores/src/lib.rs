// SPDX-License-Identifier: GPL-2.0-or-later
//! Build-time core registry.
//!
//! A frontend enables only the cores it ships. Disabled cores are not linked,
//! which keeps a GBA-only build free of MRI, egui and RGSS dependencies. The
//! generic routing logic lives in [`fanplay_frontend::CoreCatalog`].

use fanplay_frontend::CoreCatalog;

/// Builds the catalog for every core enabled by Cargo features.
pub fn catalog() -> CoreCatalog {
    let mut catalog = CoreCatalog::new();
    #[cfg(feature = "gbc")]
    catalog.register(fanplay_core_gbc::registration());
    #[cfg(feature = "rgss")]
    catalog.register(fanplay_core_rgss::registration());
    #[cfg(feature = "gba")]
    catalog.register(fanplay_core_gba::registration());
    #[cfg(feature = "nds")]
    catalog.register(fanplay_core_nds::registration());
    catalog
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_cores_are_registered() {
        let catalog = catalog();
        #[cfg(feature = "gbc")]
        assert!(catalog.get("gbc").is_some());
        #[cfg(feature = "gba")]
        assert!(catalog.get("gba").is_some());
        #[cfg(feature = "nds")]
        assert!(catalog.get("nds").is_some());
        #[cfg(feature = "rgss")]
        assert!(catalog.get("rgss").is_some());
    }

    #[test]
    #[cfg(feature = "nds")]
    fn nds_content_routes_by_extension() {
        let path = std::env::temp_dir().join(format!("fanplay-registry-{}.NDS", std::process::id()));
        std::fs::write(&path, [0u8; 512]).unwrap();
        assert_eq!(
            catalog()
                .route(&fanplay_core::Content::file(&path))
                .unwrap()
                .info
                .id,
            "nds"
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    #[cfg(feature = "gba")]
    fn gba_content_routes_by_extension() {
        let catalog = catalog();
        let path = std::env::temp_dir().join(format!(
            "fanplay-core-registry-{}-{}.gba",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, vec![0u8; 0x200]).unwrap();
        let content = fanplay_core::Content::file(&path);
        #[cfg(feature = "gba")]
        assert_eq!(catalog.route(&content).unwrap().info.id, "gba");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    #[cfg(feature = "gbc")]
    fn game_boy_content_routes_by_extension() {
        let catalog = catalog();
        for extension in ["gb", "GBC"] {
            let path = std::env::temp_dir().join(format!(
                "fanplay-core-registry-{}-{}.{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                extension
            ));
            std::fs::write(&path, vec![0u8; 0x150]).unwrap();
            assert_eq!(
                catalog.route(&fanplay_core::Content::file(&path)).unwrap().info.id,
                "gbc"
            );
            let _ = std::fs::remove_file(path);
        }
    }
}
