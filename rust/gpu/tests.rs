// SPDX-License-Identifier: GPL-2.0-or-later
//! GPU regression tests: compositor/software parity, retained-resource and
//! binding upkeep, batching correctness, and native tile invariants.
//!
//! Most tests construct a real `GpuRenderer`, so they are `#[ignore = "requires
//! a wgpu adapter"]` and run with
//! `cargo test -p rsxp --lib gpu::tests -- --ignored --nocapture`. The pure-CPU
//! `batches_preserve_overlaps_and_viewport_dependencies` exercises `draw_batches`
//! directly. `compare` asserts per-channel ARGB equality within a tolerance.
//!
//! Coverage:
//! - `native_tile_atlas_and_layers_match_individual_sprites`: the native tile
//!   path matches per-sprite draws across animation phases and confirms quads
//!   outnumber batches (adjacent priority layers batch).
//! - `retained_batches_handle_pixel_animation_and_changed_overlap`: batch
//!   schedules track pixel animation and changed overlap, and fully clipped
//!   sprites add no command or upload.
//! - `retained_bindings_follow_atlas_growth_and_frame_resize`: retained bind
//!   groups stay valid after atlas growth and frame resize.
//! - `batched_draws_match_serial_with_overlaps_and_viewports`: grouped output
//!   matches serial and actually reduces dispatches.
//! - `compositor_matches_software_and_tracks_dirty_sources`: blend modes,
//!   mirror/invert, color/tone, pattern, bush and rotation plus plane and
//!   viewport effects match the software renderer; unchanged bitmaps are not
//!   re-uploaded and unchanged frames are reused.
//! - `culled_sprites_and_planes_do_not_split_native_tile_draws`: culled or
//!   transparent non-tile draws leave a native tile run intact.
use super::batching::draw_batches;
use super::*;
use crate::graphics::{Bitmap, Color, Rect, SpriteFx, SpriteTransform, Tone};
#[test]
#[ignore = "requires a wgpu adapter"]
fn native_tile_atlas_and_layers_match_individual_sprites() {
    let mut gpu = GpuRenderer::new(128, 96).expect("wgpu test adapter");
    let mut tileset = Bitmap::new(96, 64).unwrap();
    for y in 0..64 {
        for x in 0..96 {
            tileset.set_pixel(
                x,
                y,
                Color::rgba(
                    (x * 2) as u8,
                    (y * 3) as u8,
                    120,
                    if x % 3 == 0 { 100 } else { 255 },
                ),
            );
        }
    }
    let clip = Rect::new(2, 1, 124, 92);
    for phase in 0..3 {
        tileset.set_pixel(32, 0, Color::rgba(20, 50 + phase * 30, 220, 180));
        let mut images = Vec::new();
        for native in [false, true] {
            gpu.begin(128, 96).unwrap();
            gpu.clear(true, 0);
            let tile = |gpu: &mut GpuRenderer, z, src, tr, fx: &SpriteFx| {
                if native {
                    gpu.tile(z, 1, &tileset, src, clip, tr, (0, 0), fx, 0, true)
                        .unwrap();
                } else {
                    gpu.sprite(1, &tileset, src, clip, tr, (0, 0), fx, 0, true)
                        .unwrap();
                }
            };
            // Ground layers overlap; adjacent priority layers can batch,
            // but a character draw between them must remain a barrier.
            for layer in 0..2 {
                for row in 0..3 {
                    for col in 0..4 {
                        let src = Rect::new(if layer == 0 { 0 } else { 32 }, 0, 32, 32);
                        tile(
                            &mut gpu,
                            0,
                            src,
                            SpriteTransform {
                                x: col as f64 * 32.0 - phase as f64,
                                y: row as f64 * 32.0,
                                ..Default::default()
                            },
                            &SpriteFx {
                                opacity: if layer == 0 { 255 } else { 150 },
                                ..Default::default()
                            },
                        );
                    }
                }
            }
            tile(
                &mut gpu,
                33,
                Rect::new(-3, 20, 32, 32),
                SpriteTransform {
                    x: 20.0,
                    y: 10.0,
                    mirror: true,
                    ..Default::default()
                },
                &SpriteFx {
                    mirror: true,
                    ..Default::default()
                },
            );
            gpu.sprite(
                2,
                &tileset,
                Rect::new(64, 0, 24, 24),
                clip,
                SpriteTransform {
                    x: 25.0,
                    y: 20.0,
                    ..Default::default()
                },
                (0, 0),
                &SpriteFx::default(),
                0,
                true,
            )
            .unwrap();
            for z in [65, 97] {
                tile(
                    &mut gpu,
                    z,
                    Rect::new(32, phase as i32, 32, 32),
                    SpriteTransform {
                        x: 40.0,
                        y: (z - 45) as f64,
                        zoom_x: 1.25,
                        zoom_y: 1.5,
                        ..Default::default()
                    },
                    &SpriteFx {
                        color: Color::rgba(80, 20, 100, 50),
                        tone: Tone {
                            red: 10,
                            green: -8,
                            blue: 5,
                            gray: 20,
                        },
                        ..Default::default()
                    },
                );
            }
            gpu.viewport(Color::rgba(30, 50, 100, 25), Tone::default());
            gpu.submit().unwrap();
            images.push(gpu.readback().unwrap());
        }
        compare(&images[0], &images[1], 0);
    }
    assert!(gpu.native_tile_quads > gpu.native_tile_batches);
    assert!(
        gpu.native_tile_layers > gpu.native_tile_batches,
        "adjacent priority layers must batch"
    );
}

#[test]
#[ignore = "requires a wgpu adapter"]
fn retained_batches_handle_pixel_animation_and_changed_overlap() {
    let mut gpu = GpuRenderer::new(32, 24).expect("wgpu test adapter");
    gpu.batching = true;
    let mut source = Bitmap::new(8, 8).unwrap();
    let clip = Rect::new(0, 0, 32, 24);
    for (phase, x) in [8, 8, 4].into_iter().enumerate() {
        source.clear(Color::rgba(80 + phase as u8 * 50, 120, 60, 180));
        let mut cpu = Bitmap::new(32, 24).unwrap();
        cpu.clear(Color::rgb(0, 0, 0));
        gpu.begin(32, 24).unwrap();
        for x in [0, x] {
            cpu.draw_sprite(
                Rect::new(x, 0, 8, 8),
                &source,
                source.rect(),
                clip,
                &SpriteFx::default(),
            );
            gpu.sprite(
                1,
                &source,
                source.rect(),
                clip,
                SpriteTransform {
                    x: x as f64,
                    ..Default::default()
                },
                (0, 0),
                &SpriteFx::default(),
                0,
                false,
            )
            .unwrap();
        }
        gpu.submit().unwrap();
        compare(&cpu, &gpu.readback().unwrap(), 0);
        assert_eq!(gpu.batches.len(), if phase == 2 { 3 } else { 2 });
    }
    let uploaded = gpu.uploaded_bytes;
    gpu.begin(32, 24).unwrap();
    gpu.sprite(
        2,
        &source,
        source.rect(),
        clip,
        SpriteTransform {
            x: 40.0,
            ..Default::default()
        },
        (0, 0),
        &SpriteFx::default(),
        0,
        false,
    )
    .unwrap();
    assert_eq!(
        gpu.commands.len(),
        1,
        "fully clipped sprite must add no command"
    );
    assert_eq!(
        gpu.uploaded_bytes, uploaded,
        "fully clipped sprite must not upload"
    );
}

#[test]
#[ignore = "requires a wgpu adapter"]
fn retained_bindings_follow_atlas_growth_and_frame_resize() {
    let mut gpu = GpuRenderer::new(32, 24).expect("wgpu test adapter");
    for (size, extent) in [(8, (32, 24)), (256, (32, 24)), (256, (40, 30))] {
        let mut bitmap = Bitmap::new(size, size).unwrap();
        bitmap.clear(Color::rgb(size as u8, 120, 80));
        gpu.begin(extent.0, extent.1).unwrap();
        gpu.sprite(
            size as u64,
            &bitmap,
            bitmap.rect(),
            Rect::new(0, 0, extent.0, extent.1),
            SpriteTransform::default(),
            (0, 0),
            &SpriteFx::default(),
            0,
            false,
        )
        .unwrap();
        gpu.submit().unwrap();
        assert_eq!(
            gpu.readback().unwrap().get_pixel(0, 0),
            bitmap.get_pixel(0, 0)
        );
    }
}

#[test]
fn batches_preserve_overlaps_and_viewport_dependencies() {
    let draw = |operation, target, bounds| {
        let mut p = Params { a: [[0; 4]; 16] };
        p.a[0] = [operation, target, 0, 0];
        p.a[1] = bounds;
        p
    };
    let commands = [
        draw(0, 0, [0, 0, 64, 48]),
        draw(1, 0, [0, 0, 8, 8]),
        draw(1, 0, [8, 0, 16, 8]),
        draw(1, 0, [4, 0, 12, 8]),       // overlaps both previous draws
        draw(1, 64 * 48, [4, 0, 12, 8]), // independent scratch buffer
        draw(2, 0, [0, 0, 64, 48]),      // reads scratch: must be a barrier
        draw(1, 64 * 48, [16, 0, 24, 8]),
    ];
    let lengths: Vec<_> = draw_batches(&commands, true)
        .iter()
        .map(|b| b.len)
        .collect();
    assert_eq!(lengths, [1, 2, 2, 1, 1]);
    assert_eq!(draw_batches(&commands, false).len(), commands.len());
}

#[test]
#[ignore = "requires a wgpu adapter"]
fn batched_draws_match_serial_with_overlaps_and_viewports() {
    let mut gpu = GpuRenderer::new(64, 48).expect("wgpu test adapter");
    let mut source = Bitmap::new(8, 8).unwrap();
    source.clear(Color::rgba(180, 90, 40, 130));
    let mut images = Vec::new();
    let mut counts = Vec::new();
    for batching in [false, true] {
        gpu.batching = batching;
        gpu.previous_commands.clear();
        let before = gpu.dispatches;
        gpu.begin(64, 48).unwrap();
        gpu.clear(true, 0);
        for row in 0..6 {
            for column in 0..8 {
                // The second layer overlaps the first, testing barriers
                // between batches as well as disjoint tiles within one.
                for offset in [0.0, 3.0] {
                    gpu.sprite(
                        1,
                        &source,
                        source.rect(),
                        Rect::new(0, 0, 64, 48),
                        SpriteTransform {
                            x: column as f64 * 8.0 + offset,
                            y: row as f64 * 8.0,
                            ..Default::default()
                        },
                        (0, 0),
                        &SpriteFx::default(),
                        0,
                        true,
                    )
                    .unwrap();
                }
            }
        }
        gpu.viewport(Color::rgba(10, 20, 30, 40), Tone::default());
        gpu.clear(true, 0);
        gpu.viewport(Color::default(), Tone::default());
        gpu.submit().unwrap();
        images.push(gpu.readback().unwrap());
        counts.push(gpu.dispatches - before);
    }
    compare(&images[0], &images[1], 0);
    assert!(
        counts[1] < counts[0],
        "batching must reduce actual dispatches"
    );
}

fn compare(cpu: &Bitmap, gpu: &Bitmap, tolerance: i32) {
    for (index, (&a, &b)) in cpu.pixels().iter().zip(gpu.pixels()).enumerate() {
        for shift in [0, 8, 16, 24] {
            assert!(
                (((a >> shift) & 255) as i32 - ((b >> shift) & 255) as i32).abs() <= tolerance,
                "pixel {index}: CPU {a:08x}, GPU {b:08x}"
            );
        }
    }
}
#[test]
#[ignore = "requires a wgpu adapter; run cargo test -p rsxp --lib gpu::tests -- --ignored --nocapture"]
fn compositor_matches_software_and_tracks_dirty_sources() {
    let mut gpu = GpuRenderer::new(64, 48).expect("wgpu test adapter");
    eprintln!("GPU test adapter: {}", gpu.adapter_name);
    let mut source = Bitmap::new(23, 17).unwrap();
    for y in 0..17 {
        for x in 0..23 {
            source.set_pixel(
                x,
                y,
                Color::rgba(
                    (x * 9) as u8,
                    (y * 13) as u8,
                    97,
                    if x % 3 == 0 { 100 } else { 255 },
                ),
            );
        }
    }
    let mut pattern = Bitmap::new(8, 8).unwrap();
    pattern.clear(Color::rgba(170, 25, 90, 160));
    let clip = Rect::new(3, 2, 55, 42);
    for mode in 0..6 {
        let fx = SpriteFx {
            opacity: 137,
            blend_type: if mode == 1 {
                1
            } else if mode == 2 {
                2
            } else {
                0
            },
            mirror: mode == 1,
            color: if mode == 3 {
                Color::rgba(40, 170, 90, 65)
            } else {
                Color::default()
            },
            tone: if mode == 3 {
                Tone {
                    red: 17,
                    green: -12,
                    blue: 4,
                    gray: 40,
                }
            } else {
                Tone::default()
            },
            pattern: if mode == 4 { Some(&pattern) } else { None },
            bush_y: (mode == 5).then_some(8),
            bush_opacity: 80,
            ..Default::default()
        };
        let mut cpu = Bitmap::new(64, 48).unwrap();
        cpu.clear(Color::rgb(0, 0, 0));
        gpu.begin(64, 48).unwrap();
        let tr = SpriteTransform {
            x: 8.0,
            y: 7.0,
            zoom_x: 1.3,
            zoom_y: 1.6,
            mirror: fx.mirror,
            ..Default::default()
        };
        let dst = Rect::new(8, 7, (23.0 * 1.3) as u32, (17.0 * 1.6) as u32);
        for _ in 0..3 {
            cpu.draw_sprite(dst, &source, source.rect(), clip, &fx);
            gpu.sprite(1, &source, source.rect(), clip, tr, (0, 0), &fx, 2, false)
                .unwrap();
        }
        gpu.submit().unwrap();
        let result = gpu.readback().unwrap();
        compare(&cpu, &result, if mode >= 3 { 2 } else { 0 });
    }
    let mut cpu = Bitmap::new(64, 48).unwrap();
    cpu.clear(Color::rgb(0, 0, 0));
    gpu.begin(64, 48).unwrap();
    let tr = SpriteTransform {
        x: 30.0,
        y: 21.0,
        ox: 8.0,
        oy: 7.0,
        angle: 27.0,
        zoom_x: 1.2,
        zoom_y: 1.4,
        ..Default::default()
    };
    cpu.draw_sprite_rotated(
        &source,
        source.rect(),
        clip,
        &tr,
        (0, 0),
        &SpriteFx::default(),
    );
    gpu.sprite(
        1,
        &source,
        source.rect(),
        clip,
        tr,
        (0, 0),
        &SpriteFx::default(),
        0,
        false,
    )
    .unwrap();
    gpu.submit().unwrap();
    compare(&cpu, &gpu.readback().unwrap(), 0);
    let uploaded = gpu.uploaded_bytes;
    gpu.begin(64, 48).unwrap();
    gpu.sprite(
        1,
        &source,
        source.rect(),
        clip,
        tr,
        (0, 0),
        &SpriteFx::default(),
        0,
        false,
    )
    .unwrap();
    gpu.submit().unwrap();
    let _ = gpu.readback().unwrap();
    assert_eq!(
        gpu.uploaded_bytes, uploaded,
        "unchanged bitmaps must not be uploaded again"
    );
    assert!(
        gpu.reused_frames > 0,
        "unchanged frames should reuse GPU pixels"
    );
    source.set_pixel(0, 0, Color::rgb(255, 0, 0));
    gpu.begin(64, 48).unwrap();
    gpu.sprite(
        1,
        &source,
        source.rect(),
        clip,
        tr,
        (0, 0),
        &SpriteFx::default(),
        0,
        false,
    )
    .unwrap();
    gpu.submit().unwrap();
    let _ = gpu.readback().unwrap();
    assert!(gpu.uploaded_bytes > uploaded);

    // Cropped animation cells, tiled planes, and viewport effects share
    // the same scene semantics as the software compositor.
    let mut cpu = Bitmap::new(64, 48).unwrap();
    cpu.clear(Color::rgb(0, 0, 0));
    let mut layer = Bitmap::new(64, 48).unwrap();
    gpu.begin(64, 48).unwrap();
    gpu.clear(true, 0);
    let fx = SpriteFx {
        opacity: 170,
        ..Default::default()
    };
    layer.draw_plane(clip, (3, 2), &source, 2.0, 1.0, 5, -7, &fx);
    gpu.plane(1, &source, clip, (3, 2), (2.0, 1.0), (5, -7), &fx, true)
        .unwrap();
    let area = Rect::new(4, 3, 6, 5);
    let fx = SpriteFx {
        invert: true,
        ..Default::default()
    };
    let tr = SpriteTransform {
        x: 10.0,
        y: 8.0,
        ..Default::default()
    };
    layer.draw_sprite(Rect::new(10, 8, 6, 5), &source, area, clip, &fx);
    gpu.sprite(1, &source, area, clip, tr, (0, 0), &fx, 0, true)
        .unwrap();
    let color = Color::rgba(40, 100, 70, 30);
    let tone = Tone {
        red: 12,
        green: -8,
        blue: 4,
        gray: 75,
    };
    layer.apply_color_tone(color, tone);
    cpu.stretch_blt(cpu.rect(), &layer, layer.rect(), 255);
    cpu.brightness(193);
    gpu.viewport(color, tone);
    gpu.brightness(193);
    gpu.submit().unwrap();
    compare(&cpu, &gpu.readback().unwrap(), 2);
}

#[test]
#[ignore = "requires a wgpu adapter"]
fn culled_sprites_and_planes_do_not_split_native_tile_draws() {
    let mut gpu = GpuRenderer::new(32, 24).expect("wgpu test adapter");
    let mut bitmap = Bitmap::new(8, 8).unwrap();
    bitmap.clear(Color::rgba(60, 120, 180, 180));
    let clip = Rect::new(0, 0, 32, 24);
    let mut reference = None;
    for mode in 0..8 {
        let before = gpu.native_tile_batches;
        gpu.begin(32, 24).unwrap();
        gpu.tile(
            0,
            1,
            &bitmap,
            bitmap.rect(),
            clip,
            SpriteTransform::default(),
            (0, 0),
            &SpriteFx::default(),
            0,
            false,
        )
        .unwrap();
        match mode {
            0 => {}
            1..=5 => {
                let area = if mode == 1 {
                    Rect::new(0, 0, 0, 8)
                } else {
                    bitmap.rect()
                };
                let tr = SpriteTransform {
                    x: if mode == 3 || mode == 5 { 200.0 } else { 0.0 },
                    angle: if mode == 5 { 30.0 } else { 0.0 },
                    zoom_x: if mode == 4 { 0.0 } else { 1.0 },
                    ..Default::default()
                };
                let fx = SpriteFx {
                    opacity: if mode == 2 { 0 } else { 255 },
                    ..Default::default()
                };
                gpu.sprite(2, &bitmap, area, clip, tr, (0, 0), &fx, 0, false)
                    .unwrap();
            }
            _ => {
                let clip = if mode == 7 {
                    Rect::new(100, 100, 4, 4)
                } else {
                    clip
                };
                let fx = SpriteFx {
                    opacity: if mode == 6 { 0 } else { 255 },
                    ..Default::default()
                };
                gpu.plane(2, &bitmap, clip, (0, 0), (1.0, 1.0), (0, 0), &fx, false)
                    .unwrap();
            }
        }
        gpu.tile(
            1,
            1,
            &bitmap,
            bitmap.rect(),
            clip,
            SpriteTransform {
                x: 4.0,
                y: 4.0,
                ..Default::default()
            },
            (0, 0),
            &SpriteFx::default(),
            0,
            false,
        )
        .unwrap();
        gpu.submit().unwrap();
        assert_eq!(
            gpu.native_tile_batches - before,
            1,
            "culled draw mode {mode} split the tile run"
        );
        let pixels = gpu.readback().unwrap();
        if let Some(reference) = &reference {
            compare(reference, &pixels, 0);
        } else {
            reference = Some(pixels);
        }
    }
}
