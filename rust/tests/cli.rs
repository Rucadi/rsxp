// SPDX-License-Identifier: GPL-2.0-or-later
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
            "rsxp-test-{}-{}",
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
fn headless_framebuffer_is_deterministic() {
    let dir = TempDir::new();
    let mut outputs = Vec::new();
    for name in ["a.ppm", "b.ppm"] {
        let path = dir.0.join(name);
        let result = run(&[
            "--headless",
            "--frames",
            "2",
            "--output",
            path.to_str().unwrap(),
        ]);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        outputs.push(fs::read(path).unwrap());
    }
    assert_eq!(outputs[0], outputs[1]);
    let header = b"P6\n640 480\n255\n";
    assert!(outputs[0].starts_with(header));
    assert_eq!(outputs[0].len(), header.len() + 640 * 480 * 3);
    assert!(outputs[0][header.len()..]
        .chunks_exact(3)
        .any(|p| p == [64, 208, 160]));
}

#[test]
fn archive_commands_match_known_encrypted_fixture() {
    let dir = TempDir::new();
    let path = dir.0.join("Game.rgssad");
    // v1 archive: one entry named "a", containing "XYZ". These bytes are
    // fixed independently of the parser and its test fixture generator.
    fs::write(
        &path,
        [
            82, 71, 83, 83, 65, 68, 0, 1, 255, 202, 173, 222, 148, 181, 218, 67, 159, 165, 163, 128,
        ],
    )
    .unwrap();
    let path = path.to_str().unwrap();
    let list = run(&["archive-list", path]);
    assert!(list.status.success());
    assert_eq!(list.stdout, b"3\ta\n");
    let cat = run(&["archive-cat", path, "a"]);
    assert!(cat.status.success());
    assert_eq!(cat.stdout, b"XYZ");
    assert!(cat.stderr.is_empty());
    assert!(!run(&["archive-cat", path, "missing"]).status.success());
}

#[test]
fn invalid_arguments_fail_without_starting_a_window() {
    for args in [
        vec!["--frames", "0"],
        vec!["--frames"],
        vec!["--output"],
        vec!["--unknown"],
        vec!["archive-list"],
        vec!["run", "--ruby-profile"],
        vec!["run", "--ruby-profile-mode", "invalid"],
        vec![
            "run",
            "--ruby-profile",
            "unused.dump",
            "--ruby-profile-frames",
            "0",
        ],
        // Frame pacing moved to the workspace toolbar; the flags are gone.
        vec!["run", "--fps", "60"],
        vec!["run", "--fast-forward"],
        vec!["run", "--uncapped"],
    ] {
        let result = run(&args);
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).starts_with("rsxp: "));
    }
    assert!(run(&["--help"]).status.success());
    assert!(run(&["--version"]).status.success());
}

#[test]
fn runtime_patch_toggles_map_panoramic_mode_on_the_map_only() {
    let dir = TempDir::new();
    let script = dir.0.join("runtime_patch.rb");
    fs::write(
        &script,
        r#"
class Scene_Map
  def disposeSpritesets; end
  def createSpritesets; end
end
listing = RSXP.call('runtime_patch_list')
raise 'map_panoramic missing from the catalog' unless listing.include?("map_panoramic")
raise 'panoramic mode should start disabled' if RuntimePatches.active?('map_panoramic')
w0, h0 = Graphics.width, Graphics.height
$scene = Object.new
RSXP.call('runtime_patch_set', 'map_panoramic', 1)
RuntimePatches.sync
raise 'patch did not report active' unless RuntimePatches.active?('map_panoramic')
raise "resized while off the map: #{Graphics.width}x#{Graphics.height}" unless Graphics.width == w0 && Graphics.height == h0
$scene = Scene_Map.new
Graphics.update
raise "frame did not grow on the map: #{Graphics.width}x#{Graphics.height}" unless Graphics.width == (w0 * 1.5).round && Graphics.height == (h0 * 1.5).round
$scene = Object.new
Graphics.update
raise "frame did not revert off the map: #{Graphics.width}x#{Graphics.height}" unless Graphics.width == w0 && Graphics.height == h0
$scene = Scene_Map.new
Graphics.update
raise "frame did not re-apply on the map: #{Graphics.width}x#{Graphics.height}" unless Graphics.width == (w0 * 1.5).round
RSXP.call('runtime_patch_set', 'map_panoramic', 0)
RuntimePatches.sync
raise 'patch did not revert' if RuntimePatches.active?('map_panoramic')
raise "frame did not shrink back: #{Graphics.width}x#{Graphics.height}" unless Graphics.width == w0 && Graphics.height == h0
puts 'RUNTIME_PATCH_OK'
"#,
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_rsxp"))
        .args(["run", "--script", script.to_str().unwrap(), "--headless"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("RUNTIME_PATCH_OK"));
}

#[test]
fn cable_club_rng_runtime_patch_repairs_missing_rng_and_instance_srand() {
    let dir = TempDir::new();
    let script = dir.0.join("cable_club_rng_patch.rb");
    fs::write(
        &script,
        r#"
class Battle_CableClub
  def battleRNG; @battleRNG; end
end
raise 'Cable Club RNG patch missing from catalog' unless RSXP.call('runtime_patch_list').include?('cable_club_rng')
RSXP.call('runtime_patch_set', 'cable_club_rng', 1)
RuntimePatches.sync
first = Battle_CableClub.new
second = Battle_CableClub.new
raise 'patch did not initialize a missing RNG' unless first.battleRNG.is_a?(Random)
seed = first.battleRNG.srand
first.battleRNG.srand(seed)
second.battleRNG.srand(seed)
raise 'Cable Club RNGs did not synchronize' unless first.battleRNG.rand(1_000_000) == second.battleRNG.rand(1_000_000)
RSXP.call('runtime_patch_set', 'cable_club_rng', 0)
RuntimePatches.sync
raise 'patch did not disable' if RuntimePatches.active?('cable_club_rng')
puts 'CABLE_CLUB_RNG_PATCH_OK'
"#,
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_rsxp"))
        .args(["run", "--script", script.to_str().unwrap(), "--headless"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("CABLE_CLUB_RNG_PATCH_OK"));
}

#[test]
fn runtime_patch_can_be_preenabled_for_headless_runs() {
    let dir = TempDir::new();
    let script = dir.0.join("runtime_patch_env.rb");
    fs::write(
        &script,
        r#"
raise 'RSXP_PATCH did not enable the patch' unless RSXP.call('runtime_patch_states').include?("map_panoramic=1")
RuntimePatches.sync
raise 'pre-enabled patch was not applied' unless RuntimePatches.active?('map_panoramic')
puts 'RUNTIME_PATCH_ENV_OK'
"#,
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_rsxp"))
        .args(["run", "--script", script.to_str().unwrap(), "--headless"])
        .env("RSXP_PATCH", "map_panoramic")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("RUNTIME_PATCH_ENV_OK"));
}

#[test]
fn runtime_patch_zoom_slider_resizes_the_frame_live() {
    let dir = TempDir::new();
    let script = dir.0.join("runtime_patch_zoom.rb");
    fs::write(
        &script,
        r#"
class Scene_Map
  def disposeSpritesets; end
  def createSpritesets; end
end
$scene = Scene_Map.new
listing = RSXP.call('runtime_patch_list')
raise 'zoom parameter missing from the catalog' unless listing.include?("map_panoramic\t") && listing.include?("3")
w0, h0 = Graphics.width, Graphics.height
RSXP.call('runtime_patch_set', 'map_panoramic', 1)
RSXP.call('runtime_patch_value_set', 'map_panoramic', 2.0)
RuntimePatches.sync
raise "zoom 2.0 not applied: #{Graphics.width}x#{Graphics.height}" unless Graphics.width == (w0 * 2.0).round && Graphics.height == (h0 * 2.0).round
RSXP.call('runtime_patch_value_set', 'map_panoramic', 1.2)
RuntimePatches.sync
raise "zoom 1.2 not applied: #{Graphics.width}x#{Graphics.height}" unless Graphics.width == (w0 * 1.2).round && Graphics.height == (h0 * 1.2).round
RSXP.call('runtime_patch_set', 'map_panoramic', 0)
RuntimePatches.sync
raise 'frame not restored' unless Graphics.width == w0 && Graphics.height == h0
puts 'RUNTIME_PATCH_ZOOM_OK'
"#,
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_rsxp"))
        .args(["run", "--script", script.to_str().unwrap(), "--headless"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("RUNTIME_PATCH_ZOOM_OK"));
}

#[test]
fn runtime_patch_panoramic_hooks_centre_the_enlarged_map() {
    for fixture in [
        "runtime_patch_panoramic.rb",
        "runtime_patch_legacy_camera.rb",
    ] {
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("rust/testdata")
            .join(fixture);
        let result = Command::new(env!("CARGO_BIN_EXE_rsxp"))
            .args(["run", "--script", script.to_str().unwrap(), "--headless"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(String::from_utf8_lossy(&result.stdout).contains("PANORAMIC_HOOKS_OK"));
    }
}

#[test]
#[ignore = "requires a wgpu adapter"]
fn native_tiles_match_sprite_path_with_animation_priorities_and_plugins() {
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/rust/testdata/rgss_native_tiles.rb"
    );
    let mut outputs = Vec::new();
    for mode in ["0", "1"] {
        let output = Command::new(env!("CARGO_BIN_EXE_rsxp"))
            .args([
                "run",
                "--script",
                script,
                "--headless",
                "--renderer",
                "wgpu",
            ])
            .env("RSXP_NATIVE_TILEMAP", mode)
            .env("RSXP_PROFILE", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let diagnostics = String::from_utf8_lossy(&output.stderr);
        let gpu = diagnostics
            .lines()
            .find(|line| line.starts_with("GPU:"))
            .unwrap();
        let batches: u64 = gpu
            .split(", ")
            .last()
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            batches > 0,
            mode == "1",
            "native tile adapter must actually run"
        );
        outputs.push(output.stdout);
    }
    assert_eq!(outputs[0], outputs[1]);
}

#[test]
#[ignore = "requires a wgpu adapter"]
fn retained_scene_matches_rebuilt_scene_across_resource_changes() {
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/rust/testdata/rgss_scene_cache.rb"
    );
    let mut outputs = Vec::new();
    for mode in ["0", "1"] {
        let output = Command::new(env!("CARGO_BIN_EXE_rsxp"))
            .args([
                "run",
                "--script",
                script,
                "--headless",
                "--renderer",
                "wgpu",
            ])
            .env("RSXP_SCENE_CACHE", mode)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        outputs.push(output.stdout);
    }
    assert_eq!(outputs[0], outputs[1]);
}

#[test]
fn typed_sprite_sync_matches_ruby_with_effects_and_getter_overrides() {
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/rust/testdata/rgss_sprite_sync.rb"
    );
    let dir = TempDir::new();
    let mut results = Vec::new();
    for mode in ["native", "ruby"] {
        let image = dir.0.join(format!("{mode}.ppm"));
        let result = Command::new(env!("CARGO_BIN_EXE_rsxp"))
            .args([
                "run",
                "--script",
                script,
                "--headless",
                "--renderer",
                "software",
                "--output",
                image.to_str().unwrap(),
            ])
            .env("RSXP_SPRITE_SYNC", mode)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        results.push((result.stdout, fs::read(image).unwrap()));
    }
    assert_eq!(results[0], results[1]);
}

#[test]
fn sprite_cache_preserves_lifecycle_and_pixels() {
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/rust/testdata/rgss_sync_lifecycle.rb"
    );
    let mut results = Vec::new();
    for (binding, cache) in [("native", "1"), ("ruby", "1"), ("ruby", "0")] {
        let result = Command::new(env!("CARGO_BIN_EXE_rsxp"))
            .args([
                "run",
                "--script",
                script,
                "--headless",
                "--renderer",
                "software",
            ])
            .env("RSXP_SPRITE_SYNC", binding)
            .env("RSXP_SPRITE_CACHE", cache)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{binding}/{cache}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        results.push(result.stdout);
    }
    assert!(results.windows(2).all(|pair| pair[0] == pair[1]));
}

#[test]
#[ignore = "requires StackProf; run inside nix-shell with --ignored"]
fn ruby_sampler_records_delayed_windows_and_flushes_on_exception() {
    let dir = TempDir::new();
    let script = dir.0.join("profile.rb");
    fs::write(&script,"def profile_work; finish=Process.clock_gettime(Process::CLOCK_MONOTONIC)+0.02; while Process.clock_gettime(Process::CLOCK_MONOTONIC)<finish; 200.times { |n| n*n }; end; end; 8.times { profile_work; Graphics.update; Graphics.frame_count=0 }; raise 'profile exception marker'\n").unwrap();
    for bounded in [true, false] {
        let output = dir.0.join(if bounded {
            "bounded.dump"
        } else {
            "exception.dump"
        });
        let mut command = Command::new(env!("CARGO_BIN_EXE_rsxp"));
        command.args([
            "run",
            "--script",
            script.to_str().unwrap(),
            "--headless",
            "--ruby-profile",
            output.to_str().unwrap(),
            "--ruby-profile-start-frame",
            "2",
        ]);
        if bounded {
            command.args(["--ruby-profile-frames", "3"]);
        }
        let result = command.output().unwrap();
        assert_eq!(result.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&result.stderr).contains("profile exception marker"));
        assert!(fs::read(&output).unwrap().starts_with(&[4, 8]));
        let json = fs::read_to_string(output.with_extension("dump.json")).unwrap();
        assert!(json.contains("\"start_update\": 2"));
        assert!(json.contains(if bounded {
            "\"end_update\": 5"
        } else {
            "\"end_update\": 8"
        }));
        assert!(json.contains("profile_work"));
        assert!(fs::read_to_string(output.with_extension("dump.txt"))
            .unwrap()
            .contains("Source locations:"));
    }
}

#[test]
fn ruby_rgss_values_gc_and_rendering_all_versions() {
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/rust/testdata/rgss_smoke.rb");
    let dir = TempDir::new();
    for version in ["1", "2", "3"] {
        let output = dir.0.join(format!("ruby-{version}.ppm"));
        let result = run(&[
            "run",
            "--script",
            script,
            "--rgss",
            version,
            "--headless",
            "--output",
            output.to_str().unwrap(),
        ]);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(result.stdout, format!("RGSS_OK:{version}\n").as_bytes());
        let bytes = fs::read(output).unwrap();
        let header = b"P6\n64 48\n255\n";
        assert!(bytes.starts_with(header));
        let pixel = header.len() + (5 * 64 + 4) * 3;
        assert_eq!(&bytes[pixel..pixel + 3], &[255, 0, 0]);
    }
}

#[test]
fn scripted_headless_input_triggers_ruby_keys() {
    let dir = TempDir::new();
    let script = dir.0.join("input.rb");
    fs::write(
        &script,
        "loop { Graphics.update; Input.update; break if Input.trigger?(Input::C) }; raise 'raw trigger' unless Input.triggerex?(:RETURN); 2.times { Graphics.update; Input.update; raise 'held snapshot' unless Input.press?(:C) && Input.pressex?(:RETURN) && !Input.trigger?(:C) }; Graphics.update; Input.update; raise 'released snapshot' unless !Input.press?(:C) && Input.release?(:C) && Input.releaseex?(:RETURN); Input.update; raise 'stale release' if Input.release?(:C); puts 'C_OK'\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rsxp"))
        .args([
            "run",
            "--script",
            script.to_str().unwrap(),
            "--rgss",
            "1",
            "--headless",
            "--frames",
            "30",
        ])
        .env("RSXP_INPUT", "10:40:3")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"C_OK\n");
}

#[test]
fn controller_actions_coexist_with_keyboard_input() {
    let dir = TempDir::new();
    let script = dir.0.join("controller.rb");
    fs::write(&script, "loop { Graphics.update; Input.update; break if Input.trigger?(:C) }; raise 'connection' unless Input.connected? && Input::Controller.count == 1; 3.times { Graphics.update; Input.update }; raise 'keyboard release canceled controller' unless Input.press?(:C) && !Input.pressex?(:RETURN); 3.times { Graphics.update; Input.update }; raise 'controller release' unless Input.release?(:C) && !Input.press?(:C); raise 'disconnect' unless Input.count == 0; puts 'CONTROLLER_OK'\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rsxp"))
        .args([
            "run",
            "--script",
            script.to_str().unwrap(),
            "--headless",
            "--frames",
            "30",
        ])
        .env("RSXP_INPUT", "10:606:6,10:40:3,10:620:6")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"CONTROLLER_OK\n");
}

#[test]
fn ruby_images_fonts_and_viewports() {
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/rust/testdata/rgss_media_smoke.rb"
    );
    let result = run(&["run", "--script", script, "--headless"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.stdout, b"MEDIA_OK\n");
}

#[test]
fn media_demo_reports_animated_gif_metadata() {
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/rust/testdata/media_demo.rb"
    );
    let result = Command::new(env!("CARGO_BIN_EXE_rsxp"))
        .args(["run", "--script", script, "--headless", "--frames", "5"])
        .env("RSXP_DEMO_FRAMES", "1")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output = String::from_utf8_lossy(&result.stdout);
    for marker in ["gif=", "frames=", "GIF_DONE:"] {
        assert!(output.contains(marker), "missing {marker}: {output}");
    }
}

#[test]
fn kgl2_compatibility_functions_run_headless() {
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/rust/testdata/kgl2_smoke.rb"
    );
    let result = run(&["run", "--script", script, "--headless"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        String::from_utf8_lossy(&result.stdout).contains("KGL2_OK"),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
}

#[test]
fn mkxp_compatibility_methods_run_headless() {
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/rust/testdata/mkxp_compat.rb"
    );
    let result = run(&["run", "--script", script, "--headless"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        String::from_utf8_lossy(&result.stdout).contains("MKXP_COMPAT_OK"),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
}

#[test]
fn legacy_rgss_win32_tilemap_and_bush_rendering() {
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/rust/testdata/rgss_legacy_compat.rb"
    );
    for cache in ["0", "1"] {
        let result = Command::new(env!("CARGO_BIN_EXE_rsxp"))
            .args(["run", "--script", script, "--headless"])
            .env("RSXP_SPRITE_CACHE", cache)
            .env("RSXP_INPUT", "0:4:2")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(String::from_utf8_lossy(&result.stdout).contains("LEGACY_COMPAT_OK"));
    }
}

#[test]
#[ignore = "requires local Pokemon Z assets in pokemonz/ or RSXP_POKEMONZ_ROOT"]
fn pokemonz_starter_event_reads_binary_data_and_stores_fennekin() {
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/rust/testdata/pokemonz_starter.rb"
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
#[ignore = "requires local Pokemon Z assets in pokemonz/ or RSXP_POKEMONZ_ROOT"]
fn pokemonz_pokedex_movedex_itemdex_and_encounters_use_legacy_pbs_data() {
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/rust/testdata/pokemonz_game_api.rb"
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
#[ignore = "requires local Pokemon Z assets and Game.rxdata (or RSXP_POKEMONZ_SAVE)"]
fn pokemonz_continue_loads_a_temporary_save_copy() {
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/rust/testdata/pokemonz_save_load.rb"
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

#[test]
fn ruby_exceptions_exit_status_and_frame_limit() {
    let dir = TempDir::new();
    let script = dir.0.join("script.rb");
    let path = script.to_str().unwrap();
    fs::write(&script, "raise 'ruby failure marker'\n").unwrap();
    let result = run(&["run", "--script", path, "--headless"]);
    assert_eq!(result.status.code(), Some(1));
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(
        error.contains("ruby failure marker") && error.contains("script.rb:1"),
        "{error}"
    );
    fs::write(&script, "exit 7\n").unwrap();
    assert_eq!(
        run(&["run", "--script", path, "--headless"]).status.code(),
        Some(7)
    );
    fs::write(&script, "loop { Graphics.update }\n").unwrap();
    assert!(
        run(&["run", "--script", path, "--headless", "--frames", "2",])
            .status
            .success()
    );
    fs::write(
        &script,
        "begin; Bitmap.new(0,0); rescue RuntimeError; puts 'RECOVERED'; end\n",
    )
    .unwrap();
    let result = run(&["run", "--script", path, "--headless"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.stdout, b"RECOVERED\n");
}

fn make_bundle(dir: &TempDir, version: u8) -> Vec<u8> {
    let generator = dir.0.join("generate.rb");
    let source = format!(
        r#"
require 'zlib'
sections=[
  [0,'Bootstrap', '$boot_value=41'],
  [1,'Game Main', 'raise "section order" unless $boot_value==41; raise "source slot" unless $RGSS_SCRIPTS[0][3]=="$boot_value=41"; raise "RGSS mode" unless RGSS_VERSION=={version}; Graphics.update; puts "BUNDLE:{version}"']
]
File.binwrite('bundle.bin',Marshal.dump(sections.map {{ |id,name,source| [id,name,Zlib::Deflate.deflate(source)] }}))
"#
    );
    fs::write(&generator, source).unwrap();
    let result = run(&["run", "--script", generator.to_str().unwrap(), "--headless"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    fs::read(dir.0.join("bundle.bin")).unwrap()
}

fn pack_archive(name: &[u8], data: &[u8], version: u8) -> Vec<u8> {
    let mut archive = b"RGSSAD\0".to_vec();
    archive.push(version);
    let advance = |key: &mut u32| {
        let old = *key;
        *key = key.wrapping_mul(7).wrapping_add(3);
        old
    };
    let mut data_key;
    if version == 1 {
        let mut key = 0xDEADCAFEu32;
        archive.extend_from_slice(&((name.len() as u32) ^ advance(&mut key)).to_le_bytes());
        for &b in name {
            archive.push(b ^ advance(&mut key) as u8);
        }
        archive.extend_from_slice(&((data.len() as u32) ^ advance(&mut key)).to_le_bytes());
        data_key = key;
    } else {
        let key = 123u32 * 9 + 3;
        data_key = 0x12345678;
        archive.extend_from_slice(&123u32.to_le_bytes());
        for n in [
            12 + 16 + name.len() as u32 + 4,
            data.len() as u32,
            data_key,
            name.len() as u32,
        ] {
            archive.extend_from_slice(&(n ^ key).to_le_bytes());
        }
        archive.extend(
            name.iter()
                .enumerate()
                .map(|(i, b)| b ^ key.to_le_bytes()[i % 4]),
        );
        archive.extend_from_slice(&key.to_le_bytes());
    }
    for chunk in data.chunks(4) {
        let key = advance(&mut data_key).to_le_bytes();
        archive.extend(chunk.iter().zip(key).map(|(b, k)| b ^ k));
    }
    archive
}

#[test]
fn compressed_script_bundles_and_encrypted_games() {
    for (version, extension, archive_name) in [
        (1, "rxdata", "Game.rgssad"),
        (2, "rvdata", "Game.rgss2a"),
        (3, "rvdata2", "Game.rgss3a"),
    ] {
        let dir = TempDir::new();
        let bytes = make_bundle(&dir, version);
        let name = format!("Data/Custom.{extension}");
        fs::write(
            dir.0.join("Game.ini"),
            format!("[Game]\nScripts=DATA\\Custom.{extension}\n"),
        )
        .unwrap();
        fs::create_dir(dir.0.join("dAtA")).unwrap();
        let script_file = dir.0.join(format!("dAtA/custom.{extension}"));
        fs::write(&script_file, &bytes).unwrap();
        let args = ["run", "--game", dir.0.to_str().unwrap(), "--headless"];
        let result = run(&args);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(result.stdout, format!("BUNDLE:{version}\n").as_bytes());
        fs::remove_file(script_file).unwrap();
        fs::write(
            dir.0.join(archive_name),
            pack_archive(name.as_bytes(), &bytes, if version == 3 { 3 } else { 1 }),
        )
        .unwrap();
        let result = run(&args);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(result.stdout, format!("BUNDLE:{version}\n").as_bytes());
    }
}

#[test]
fn isolated_runner_streams_frames_input_and_inspector_replies() {
    isolated_runner_transport("software");
}

#[cfg(feature = "gpu")]
#[test]
#[ignore = "requires a wgpu adapter"]
fn isolated_gpu_runner_streams_commands_without_readbacks() {
    isolated_runner_transport("wgpu");
}

fn isolated_runner_transport(renderer: &str) {
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        time::{Duration, Instant},
    };
    fn read(stream: &mut TcpStream) -> serde_json::Value {
        let mut length = [0; 4];
        stream.read_exact(&mut length).unwrap();
        let length = u32::from_le_bytes(length) as usize;
        assert!(length < 16 * 1024 * 1024);
        let mut bytes = vec![0; length];
        stream.read_exact(&mut bytes).unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }
    fn write(stream: &mut TcpStream, value: serde_json::Value) {
        let bytes = serde_json::to_vec(&value).unwrap();
        stream
            .write_all(&(bytes.len() as u32).to_le_bytes())
            .unwrap();
        stream.write_all(&bytes).unwrap();
    }
    let dir = TempDir::new();
    let script = dir.0.join("session.rb");
    fs::write(&script,"Graphics.update; Input.update; raise 'missing remote key' unless Input.triggerex?(:RETURN); 3.times { Graphics.update; Input.update }; puts 'SESSION_OK'\n").unwrap();
    // Reopening uses a fresh runner, so Ruby's VM lifecycle is never reused.
    for _ in 0..2 {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_rsxp"))
            .args([
                "run",
                "--script",
                script.to_str().unwrap(),
                "--renderer",
                renderer,
            ])
            .env(
                "RSXP_SESSION_ADDR",
                listener.local_addr().unwrap().to_string(),
            )
            .env("RSXP_SESSION_TOKEN", "session-test-token")
            .env("RSXP_API", "0")
            .env("RSXP_PROFILE", if renderer == "wgpu" { "1" } else { "0" })
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let start = Instant::now();
        let mut stream = loop {
            match listener.accept() {
                Ok((s, _)) => break s,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        start.elapsed() < Duration::from_secs(15),
                        "runner failed to connect"
                    );
                    assert!(
                        child.try_wait().unwrap().is_none(),
                        "runner exited before handshake"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) => panic!("{e}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        assert_eq!(read(&mut stream)["token"], "session-test-token");
        let mut frames = 0;
        let mut polls = 0;
        let mut replied = false;
        while frames < 4 {
            let header = read(&mut stream);
            if header["poll"] == true {
                polls += 1;
                let paused = polls <= 2;
                write(
                    &mut stream,
                    serde_json::json!({"paused":paused,"keys":[40],"downs":if polls==3{vec![40]}else{vec![]},"rate":120,"api":if polls==3{"91\tgame_status\n"}else{""}}),
                );
            } else {
                frames += 1;
                assert_eq!(header["frames"], frames);
                assert_eq!(header["width"], 640);
                assert_eq!(header["height"], 480);
                if let Some(len) = header["gpu_bytes"].as_u64() {
                    assert!(
                        len < 4096,
                        "blank GPU frame should contain commands, not pixels"
                    );
                    let mut bytes = vec![0; len as usize];
                    stream.read_exact(&mut bytes).unwrap();
                    #[cfg(feature = "gpu")]
                    assert_eq!(
                        rsxp::gpu::GpuFrame::decode(&bytes).unwrap().size(),
                        (640, 480)
                    );
                } else {
                    assert_eq!(renderer, "software");
                    let mut pixels = vec![0; 640 * 480 * 4];
                    stream.read_exact(&mut pixels).unwrap();
                }
                if let Some(replies) = header["replies"].as_array() {
                    replied |= replies.iter().any(|r| {
                        r[0] == 91 && r[1].as_str().is_some_and(|s| s.contains("resolution"))
                    });
                }
            }
        }
        let result = child.wait_with_output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(String::from_utf8_lossy(&result.stdout).contains("SESSION_OK"));
        if renderer == "wgpu" {
            assert!(String::from_utf8_lossy(&result.stderr).contains("0 explicit readbacks"));
        }
        assert!(polls >= 6);
        assert!(
            replied,
            "inspector response did not cross the runner boundary"
        );
    }
}

#[test]
fn retained_native_state_and_map_match_legacy_frames() {
    for fixture in ["rgss_retained_state.rb", "rgss_retained_map.rb"] {
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("rust/testdata")
            .join(fixture);
        let mut results = Vec::new();
        for (state, map, scroll, world, chunks) in [
            ("0","0","0","0","0"), ("1","0","0","0","0"),
            ("0","1","0","0","0"), ("1","1","0","0","0"),
            ("1","1","1","0","0"), ("1","1","1","1","0"),
            ("1","1","1","0","1"), ("1","1","1","1","1")
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
}

#[test]
#[ignore = "requires a wgpu adapter"]
fn world_camera_gpu_frames_match_software_during_scrolling() {
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/rust/testdata/rgss_retained_map.rb"
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

#[test]
fn retained_legacy_tilemap_matches_rebuilt_pixels_and_coordinates() {
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/rust/testdata/rgss_legacy_retained.rb"
    );
    let mut results = Vec::new();
    for (retained, batch, compact) in [
        ("0", "0", "0"),
        ("1", "0", "0"),
        ("1", "1", "0"),
        ("1", "1", "1"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rsxp"))
            .args(["run", "--script", script, "--headless"])
            .env("RSXP_RETAINED_TILEMAP", retained)
            .env("RSXP_NATIVE_TILEMAP_BATCH", batch)
            .env("RSXP_TEST_COMPACT", compact)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        results.push(
            stdout
                .lines()
                .find(|line| line.starts_with("LEGACY_RETAINED:"))
                .unwrap()
                .to_owned(),
        );
    }
    assert_eq!(results[0], results[1]);
    assert_eq!(results[0], results[2]);
    assert_eq!(results[0], results[3]);
}

#[test]
fn case_insensitive_directory_index_observes_file_changes_and_cwd() {
    let dir = TempDir::new();
    let script = dir.0.join("case-cache.rb");
    fs::write(
        &script,
        r#"
require 'tmpdir'
Dir.mktmpdir do |root|
  Dir.mkdir(File.join(root,'Assets'))
  Dir.chdir(root) do
    raise unless !FileTest.exist?('assets/NEW.txt')
    File.write('Assets/New.TXT','first')
    raise unless FileTest.exist?('assets/NEW.txt')
    raise unless File.open('assets/new.txt', &:read)=='first'
    File.rename('Assets/New.TXT','Assets/Other.TXT')
    raise if FileTest.exist?('assets/NEW.txt')
    raise unless FileTest.exist?('ASSETS/OTHER.txt')
    File.delete('Assets/Other.TXT')
    raise if FileTest.exist?('assets/other.txt')
  end
  Dir.mktmpdir do |other|
    Dir.chdir(other) { raise if FileTest.exist?('assets/new.txt') }
  end
end
puts 'CASE_INDEX_OK'
"#,
    )
    .unwrap();
    let output = run(&["run", "--script", script.to_str().unwrap(), "--headless"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("CASE_INDEX_OK"));
}

#[test]
#[ignore = "requires a wgpu adapter"]
fn retained_legacy_tilemap_gpu_matches_rebuilt_software() {
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/rust/testdata/rgss_legacy_retained.rb"
    );
    let dir = TempDir::new();
    for (capture, frame) in [
        ("RSXP_TEST_CAPTURE_FRAME", 4),
        ("RSXP_TEST_CAPTURE_FRAME", 11),
        ("RSXP_TEST_CAPTURE_FRAME", 15),
        ("RSXP_TEST_CAPTURE_FRAME", 25),
        ("RSXP_TEST_CAPTURE_REVERSAL", 2),
        ("RSXP_TEST_CAPTURE_ANIMATION", 15),
    ] {
        let mut images = Vec::new();
        for (renderer, retained) in [("software", "0"), ("wgpu", "1")] {
            let path = dir.0.join(format!("legacy-{frame}-{renderer}.ppm"));
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
                .env("RSXP_RETAINED_TILEMAP", retained)
                .env(capture, frame.to_string())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            images.push(fs::read(path).unwrap());
        }
        assert_eq!(images[0], images[1], "legacy GPU frame {frame}");
    }
}

#[test]
fn legacy_kernel_helpers_remain_callable_after_runtime_definition_or_privatization() {
    let result = run(&[
        "run", "--script",
        concat!(env!("CARGO_MANIFEST_DIR"), "/rust/testdata/rgss_kernel_helpers.rb"),
        "--headless",
    ]);
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "KERNEL_GAME_HELPERS_OK");
}
