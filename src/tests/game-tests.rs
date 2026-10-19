// SPDX-License-Identifier: GPL-2.0-or-later
//! Manual integration tests that need the local game trees (`ericlostie/`,
//! `pokemonz/`), which are not part of this repository and therefore never run
//! in the default `cargo test`/CI suite. Run them explicitly:
//!
//! ```text
//! cargo test -p rsxp-desktop --features game-tests --test game-tests
//! ```
//!
//! The `pokemonz` cases accept `RSXP_POKEMONZ_ROOT`/`RSXP_POKEMONZ_SAVE`; the
//! world-camera case also needs a wgpu adapter and is `#[ignore]`d (add
//! `-- --ignored`).
//!
//! Coverage: the Pokémon Z starter event writes and reads its binary save data;
//! the Pokédex/movedex/itemdex/encounter and debug game-API surface reads legacy
//! PBS data; continuing loads a temporary save copy (with
//! `RUBY_FIBER_MACHINE_STACK_SIZE` raised for the debug dispatcher); and the
//! retained Essentials map frames match legacy frames. The `ericlostie/` case
//! exercises the real Essentials `TilemapRenderer`; its GPU world-camera
//! comparison is `#[ignore]`d.
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Self {
        static ID: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "rsxp-game-test-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rsxp"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn pokemonz_starter_event_reads_binary_data_and_stores_fennekin() {
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../src/testdata/pokemonz_starter.rb"
    );
    let result = run(&["run", "--script", script, "--headless"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output = String::from_utf8_lossy(&result.stdout);
    for marker in [
        "STARTER_OK:FENNEKIN:5:",
        "STARTER_OK:CHESPIN:5:",
        "STARTER_OK:FROAKIE:5:",
        "STARTER_EVENT_OK:FENNEKIN:5",
        "POKEMONZ_PANORAMIC_HOOKS_OK",
        "POKEMONZ_PROXY_PLANES_OK",
        "POKEMONZ_TITLE_DISPOSE_OK",
    ] {
        assert!(output.contains(marker), "missing {marker}: {output}");
    }
}

#[test]
fn pokemonz_pokedex_movedex_itemdex_and_encounters_use_legacy_pbs_data() {
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../src/testdata/pokemonz_game_api.rb"
    );
    let result = run(&["run", "--script", script, "--headless"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output = String::from_utf8_lossy(&result.stdout);
    for marker in [
        "POKEDEX_OK:",
        "MOVEDEX_OK:",
        "ITEMDEX_OK:",
        "MASCHINE_OK:",
        "INVENTORY_OK:",
        "ENCOUNTERS_OK:",
        "WILD_ENCOUNTER_OK:",
        "DEBUG_MODE_OK:",
        "POKEMONZ_GAME_API_OK",
    ] {
        assert!(output.contains(marker), "missing {marker}: {output}");
    }
}

#[test]
fn pokemonz_continue_loads_a_temporary_save_copy() {
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../src/testdata/pokemonz_save_load.rb"
    );
    // The debug native dispatcher needs more machine stack than Ruby's
    // default Fiber stack; the game itself runs on the main Ruby stack.
    let result = Command::new(env!("CARGO_BIN_EXE_rsxp"))
        .args(["run", "--script", script, "--headless"])
        .env("RUBY_FIBER_MACHINE_STACK_SIZE", "8388608")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("SAVE_LOAD_OK:"));
}

/// The real Essentials `TilemapRenderer` and its scripts only exist in the
/// local game tree (`ericlostie/`).
#[test]
fn retained_map_matches_legacy_frames() {
    retained_frames_match("rgss_retained_map.rb");
}

fn retained_frames_match(fixture: &str) {
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../src/testdata")
        .join(fixture);
    let mut results = Vec::new();
    for (state, map, scroll, world, chunks) in [
        ("0", "0", "0", "0", "0"),
        ("1", "0", "0", "0", "0"),
        ("0", "1", "0", "0", "0"),
        ("1", "1", "0", "0", "0"),
        ("1", "1", "1", "0", "0"),
        ("1", "1", "1", "1", "0"),
        ("1", "1", "1", "0", "1"),
        ("1", "1", "1", "1", "1"),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_rsxp"))
            .args([
                "run",
                "--script",
                script.to_str().unwrap(),
                "--headless",
                "--renderer",
                "software",
            ])
            .env("RSXP_NATIVE_STATE", state)
            .env("RSXP_RETAINED_MAP", map)
            .env("RSXP_NATIVE_SCROLL", scroll)
            .env("RSXP_WORLD_CAMERA", world)
            .env("RSXP_MAP_CHUNKS", chunks)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{fixture}/{state}/{map}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        results.push(result.stdout);
    }
    for result in &results[1..] {
        assert_eq!(&results[0], result, "{fixture}");
    }
}

#[test]
#[ignore = "requires a wgpu adapter"]
fn world_camera_gpu_frames_match_software_during_scrolling() {
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../src/testdata/rgss_retained_map.rb"
    );
    let dir = TempDir::new();
    for frame in [3, 4, 10, 14] {
        let mut images = Vec::new();
        for (renderer, camera) in [("software", "0"), ("wgpu", "1")] {
            let path = dir.0.join(format!("{frame}-{renderer}.ppm"));
            let output = Command::new(env!("CARGO_BIN_EXE_rsxp"))
                .args([
                    "run",
                    "--script",
                    script,
                    "--headless",
                    "--renderer",
                    renderer,
                    "--output",
                    path.to_str().unwrap(),
                ])
                .env("RSXP_NATIVE_STATE", "1")
                .env("RSXP_RETAINED_MAP", "1")
                .env("RSXP_NATIVE_SCROLL", "1")
                .env("RSXP_WORLD_CAMERA", camera)
                .env("RSXP_MAP_CHUNKS", camera)
                .env("RSXP_TEST_CAPTURE_FRAME", frame.to_string())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "frame {frame}/{renderer}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            images.push(fs::read(path).unwrap());
        }
        assert_eq!(
            images[0], images[1],
            "GPU camera frame {frame} differs from software"
        );
    }
}
