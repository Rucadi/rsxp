// SPDX-License-Identifier: GPL-2.0-or-later
//! Unit tests for the software renderer: pixel parity, bitmap ownership and
//! revisions, clipping, alpha blending, GIF playback, KGL2 shaders and scene
//! ordering.
//!
//! The checks pin behaviour that the GPU compositor must match, for example that
//! the contiguous blit fast path produces the same pixels as the general scaled
//! path across clipping and opacity, and that a clone shares storage until it is
//! written.
use super::*;
#[test]
fn clones_share_until_written_and_revisions_invalidate() {
    let mut original = Bitmap::new(8, 8).unwrap();
    original.clear(Color::rgb(50, 60, 70));
    let mut copy = original.clone();
    assert_eq!(original.pixels().as_ptr(), copy.pixels().as_ptr());
    assert_eq!(original.revision(), copy.revision());
    copy.set_pixel(0, 0, Color::rgb(255, 0, 0));
    assert_ne!(original.revision(), copy.revision());
    assert_eq!(original.get_pixel(0, 0), Some(Color::rgb(50, 60, 70)));
}
#[test]
fn rgba_export_reuses_a_sized_output_buffer() {
    let mut bitmap = Bitmap::new(2, 1).unwrap();
    bitmap.set_pixel(0, 0, Color::rgba(10, 20, 30, 40));
    bitmap.set_pixel(1, 0, Color::rgba(50, 60, 70, 80));
    let mut rgba = Vec::with_capacity(8);
    let ptr = rgba.as_ptr();
    bitmap.write_rgba(&mut rgba);
    assert_eq!(rgba, [10, 20, 30, 40, 50, 60, 70, 80]);
    bitmap.set_pixel(1, 0, Color::rgb(90, 100, 110));
    bitmap.write_rgba(&mut rgba);
    assert_eq!(rgba, [10, 20, 30, 40, 90, 100, 110, 255]);
    assert_eq!(rgba.as_ptr(), ptr);
}
#[test]
fn clips_negative_rectangles_and_extreme_coordinates() {
    let mut bitmap = Bitmap::new(3, 3).unwrap();
    let red = Color::rgb(255, 0, 0);
    bitmap.fill_rect(Rect::new(-1, -1, 2, 2), red);
    assert_eq!(bitmap.get_pixel(0, 0), Some(red));
    assert_eq!(bitmap.get_pixel(1, 0), Some(Color::default()));
    bitmap.fill_rect(Rect::new(i32::MAX, i32::MAX, u32::MAX, u32::MAX), red);
    bitmap.fill_rect(Rect::new(i32::MIN, i32::MIN, 1, 1), red);
    assert_eq!(bitmap.get_pixel(-1, 0), None);
    assert!(Bitmap::new(u32::MAX, u32::MAX).is_err());
}
#[test]
fn scales_and_blends_with_transparent_destinations() {
    let mut src = Bitmap::new(1, 1).unwrap();
    src.clear(Color::rgba(255, 0, 0, 128));
    let mut dst = Bitmap::new(2, 2).unwrap();
    dst.stretch_blt(dst.rect(), &src, src.rect(), 255);
    assert_eq!(dst.get_pixel(1, 1), Some(Color::rgba(255, 0, 0, 128)));
    dst.clear(Color::rgb(0, 0, 255));
    dst.stretch_blt(Rect::new(-1, -1, 2, 2), &src, src.rect(), 255);
    assert_eq!(dst.get_pixel(0, 0), Some(Color::rgb(128, 0, 127)));
    assert_eq!(dst.get_pixel(1, 1), Some(Color::rgb(0, 0, 255)));
}
#[test]
fn hue_change_rotates_hue_and_keeps_alpha() {
    let mut bitmap = Bitmap::new(1, 1).unwrap();
    bitmap.clear(Color::rgba(255, 0, 0, 90));
    bitmap.hue_change(0.0);
    assert_eq!(bitmap.get_pixel(0, 0), Some(Color::rgba(255, 0, 0, 90)));
    bitmap.hue_change(120.0);
    assert_eq!(bitmap.get_pixel(0, 0), Some(Color::rgba(0, 255, 0, 90)));
    bitmap.hue_change(120.0);
    assert_eq!(bitmap.get_pixel(0, 0), Some(Color::rgba(0, 0, 255, 90)));
}
#[test]
fn fast_path_clamps_source_rows_for_non_square_bitmaps() {
    let mut src = Bitmap::new(4, 2).unwrap();
    src.fill_rect(src.rect(), Color::rgb(10, 200, 30));
    let mut target = Bitmap::new(4, 4).unwrap();
    target.clear(Color::rgb(0, 0, 0));
    let fx = SpriteFx::default();
    // The source area is taller than the bitmap: rows past the source
    // height must be skipped rather than reading out of bounds.
    target.draw_sprite(
        Rect::new(0, 0, 4, 4),
        &src,
        Rect::new(0, 0, 2, 4),
        target.rect(),
        &fx,
    );
    assert_eq!(target.get_pixel(0, 0), Some(Color::rgb(10, 200, 30)));
    assert_eq!(target.get_pixel(0, 1), Some(Color::rgb(10, 200, 30)));
    assert_eq!(target.get_pixel(0, 2), Some(Color::rgb(0, 0, 0)));
    assert_eq!(target.get_pixel(0, 3), Some(Color::rgb(0, 0, 0)));
}

#[test]
fn contiguous_blits_match_scaled_sampling_across_clipping_and_alpha() {
    let mut source = Bitmap::new(5, 4).unwrap();
    let mut doubled = Bitmap::new(10, 8).unwrap();
    let mut seed = 17u32;
    for y in 0..4 {
        for x in 0..5 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let [r, g, b, a] = seed.to_le_bytes();
            let color = Color::rgba(r, g, b, a);
            source.set_pixel(x, y, color);
            doubled.fill_rect(Rect::new(x * 2, y * 2, 2, 2), color);
        }
    }
    for opacity in [0, 1, 127, 254, 255] {
        for x in [-7, -2, 0, 4, 12] {
            for y in [-4, 0, 3, 10] {
                for area in [
                    Rect::new(0, 0, 5, 4),
                    Rect::new(-2, -1, 8, 7),
                    Rect::new(3, 2, 5, 4),
                ] {
                    let mut fast = Bitmap::new(11, 9).unwrap();
                    fast.clear(Color::rgba(32, 180, 70, 97));
                    let mut reference = fast.clone();
                    let dst = Rect::new(x, y, area.width, area.height);
                    let clip = Rect::new(1, -1, 8, 8);
                    fast.stretch_blt_clipped(dst, &source, area, opacity, clip);
                    // Doubled texels select the identical colors through the
                    // general scaled path instead of the contiguous fast path.
                    let scaled = Rect::new(area.x * 2, area.y * 2, area.width * 2, area.height * 2);
                    reference.stretch_blt_clipped(dst, &doubled, scaled, opacity, clip);
                    assert_eq!(
                        fast.pixels(),
                        reference.pixels(),
                        "opacity={opacity} dst={dst:?} area={area:?}"
                    );
                }
            }
        }
    }
}
#[test]
fn sprite_opacity_fast_path_matches_blend() {
    let mut src = Bitmap::new(2, 2).unwrap();
    src.clear(Color::rgba(255, 0, 0, 128));
    let mut target = Bitmap::new(2, 2).unwrap();
    target.clear(Color::rgb(0, 0, 255));
    let fx = SpriteFx {
        opacity: 128,
        ..SpriteFx::default()
    };
    target.draw_sprite(target.rect(), &src, src.rect(), target.rect(), &fx);
    assert_eq!(target.get_pixel(0, 0), Some(Color::rgba(64, 0, 191, 255)));
}
#[test]
fn sprite_color_and_blend_modes() {
    let mut src = Bitmap::new(1, 1).unwrap();
    src.clear(Color::rgb(100, 100, 100));
    let mut target = Bitmap::new(1, 1).unwrap();
    target.clear(Color::rgb(10, 20, 30));
    let fx = SpriteFx {
        color: Color::rgba(0, 0, 0, 255),
        ..SpriteFx::default()
    };
    target.draw_sprite(Rect::new(0, 0, 1, 1), &src, src.rect(), target.rect(), &fx);
    assert_eq!(target.get_pixel(0, 0), Some(Color::rgb(0, 0, 0)));
    target.clear(Color::rgb(10, 20, 30));
    let fx = SpriteFx {
        blend_type: 1,
        ..SpriteFx::default()
    };
    target.draw_sprite(Rect::new(0, 0, 1, 1), &src, src.rect(), target.rect(), &fx);
    assert_eq!(target.get_pixel(0, 0), Some(Color::rgb(110, 120, 130)));
}
#[test]
fn plane_tiles_and_scrolls() {
    let mut src = Bitmap::new(2, 2).unwrap();
    src.set_pixel(0, 0, Color::rgb(255, 0, 0));
    src.set_pixel(1, 0, Color::rgb(0, 255, 0));
    src.set_pixel(0, 1, Color::rgb(0, 0, 255));
    src.set_pixel(1, 1, Color::rgb(255, 255, 0));
    let mut target = Bitmap::new(4, 4).unwrap();
    let fx = SpriteFx::default();
    target.draw_plane(target.rect(), (0, 0), &src, 1.0, 1.0, 0, 0, &fx);
    assert_eq!(target.get_pixel(0, 0), Some(Color::rgb(255, 0, 0)));
    assert_eq!(target.get_pixel(2, 0), Some(Color::rgb(255, 0, 0)));
    assert_eq!(target.get_pixel(3, 3), Some(Color::rgb(255, 255, 0)));
    target.clear(Color::default());
    target.draw_plane(target.rect(), (0, 0), &src, 1.0, 1.0, 1, 0, &fx);
    assert_eq!(target.get_pixel(0, 0), Some(Color::rgb(0, 255, 0)));
    assert_eq!(target.get_pixel(1, 0), Some(Color::rgb(255, 0, 0)));
}
#[test]
fn sprite_z_order_and_visibility() {
    let mut scene = Scene::default();
    for (color, z) in [(Color::rgb(255, 0, 0), 10), (Color::rgb(0, 0, 255), 0)] {
        let mut bitmap = Bitmap::new(1, 1).unwrap();
        bitmap.clear(color);
        let mut sprite = Sprite::new(bitmap);
        sprite.z = z;
        scene.sprites.push(sprite);
    }
    let mut target = Bitmap::new(1, 1).unwrap();
    scene.render(&mut target, Color::default());
    assert_eq!(target.get_pixel(0, 0), Some(Color::rgb(255, 0, 0)));
    scene.sprites[0].visible = false;
    scene.render(&mut target, Color::default());
    assert_eq!(target.get_pixel(0, 0), Some(Color::rgb(0, 0, 255)));
}
#[test]
fn gif_animation_switches_frames_and_honours_playback() {
    let mut bitmap = Bitmap::new(1, 1).unwrap();
    let frames = vec![
        vec![Color::rgb(255, 0, 0).packed()],
        vec![Color::rgb(0, 255, 0).packed()],
        vec![Color::rgb(0, 0, 255).packed()],
    ];
    bitmap.set_gif_frames(frames, vec![0.1, 0.1, 0.1], 0, None);
    assert!(bitmap.animated());
    assert_eq!(bitmap.frame_count(), 3);
    assert_eq!(bitmap.current_frame(), 0);
    assert_eq!(bitmap.get_pixel(0, 0), Some(Color::rgb(255, 0, 0)));
    assert!(bitmap.playing());
    bitmap.advance_animation(0.1);
    assert_eq!(bitmap.current_frame(), 1);
    assert_eq!(bitmap.get_pixel(0, 0), Some(Color::rgb(0, 255, 0)));
    // A larger step crosses several frames and wraps past the last one.
    bitmap.advance_animation(0.25);
    assert_eq!(bitmap.current_frame(), 0);
    bitmap.goto_and_stop(1);
    assert!(!bitmap.playing());
    assert_eq!(bitmap.get_pixel(0, 0), Some(Color::rgb(0, 255, 0)));
    bitmap.advance_animation(1.0);
    assert_eq!(bitmap.current_frame(), 1, "a stopped animation stays put");
    bitmap.goto_and_play(2);
    assert!(bitmap.playing());
    assert_eq!(bitmap.current_frame(), 2);
    assert_eq!(bitmap.get_pixel(0, 0), Some(Color::rgb(0, 0, 255)));
    // A manual frame rate overrides the per-frame delays.
    bitmap.set_frame_rate(10.0);
    assert_eq!(bitmap.frame_rate(), 10.0);
    bitmap.goto_and_play(0);
    bitmap.advance_animation(0.11);
    assert_eq!(bitmap.current_frame(), 1);
}
#[test]
fn kgl_invert_and_compress_alpha_match_reference_values() {
    let mut bitmap = Bitmap::new(1, 1).unwrap();
    bitmap.set_pixel(0, 0, Color::rgba(0x03, 0x02, 0x01, 0xff));
    bitmap.kgl_invert();
    assert_eq!(bitmap.get_pixel(0, 0), Some(Color::rgba(0xfc, 0xfd, 0xfe, 0xff)));
    bitmap.set_pixel(0, 0, Color::rgba(0x80, 0x40, 0x20, 0x80));
    bitmap.kgl_compress_alpha();
    assert_eq!(bitmap.get_pixel(0, 0), Some(Color::rgba(0x40, 0x20, 0x10, 0x00)));
}
#[test]
fn kgl_light_shader_subtracts_and_force_opaque() {
    let subtract = |destination: Color, source: Color, opacity: u8| {
        let mut bitmap = Bitmap::new(1, 1).unwrap();
        bitmap.set_pixel(0, 0, destination);
        let mut src = Bitmap::new(1, 1).unwrap();
        src.set_pixel(0, 0, source);
        bitmap.kgl_subtract_rect(0, 0, &src, Rect::new(0, 0, 1, 1), opacity);
        bitmap.get_pixel(0, 0).unwrap()
    };
    // mkxp-z tests/kgl2/light.rb tests 1-3.
    assert_eq!(
        subtract(Color::rgb(0xff, 0xff, 0xff), Color::rgb(0xff, 0xff, 0xff), 255),
        Color::rgb(0x00, 0x00, 0x00)
    );
    assert_eq!(
        subtract(Color::rgb(0x03, 0x02, 0x01), Color::rgb(0x01, 0x02, 0x03), 255),
        Color::rgb(0x02, 0x00, 0x00)
    );
    assert_eq!(
        subtract(Color::rgba(0, 0, 0, 0), Color::rgba(0, 0, 0, 0), 255),
        Color::rgba(0, 0, 0, 255)
    );
}
#[test]
fn kgl_shadow_shaders_validate_and_cast() {
    let mut buffer = Bitmap::new(5, 5).unwrap();
    buffer.clear(Color::rgb(255, 255, 255));
    // y == h/2 and x == w/2 are rejected by the reference shaders.
    assert_eq!(buffer.kgl_shadow_shader_h(0, 4, 2, false), 111);
    assert_eq!(buffer.kgl_shadow_shader_v(0, 4, 2, false, false), 111);
    // A valid horizontal cast zeroes the wedge below the line and reports 1.
    buffer.clear(Color::rgb(255, 255, 255));
    assert_eq!(buffer.kgl_shadow_shader_h(4, 4, 4, false), 1);
    assert_eq!(buffer.get_pixel(4, 4), Some(Color::rgba(0, 0, 0, 0)));
    assert_eq!(buffer.get_pixel(0, 0), Some(Color::rgb(255, 255, 255)));
}
#[test]
fn gradient_blur_and_snapshot_effects() {
    // Horizontal gradient: red at the left, blue at the right.
    let mut gradient = Bitmap::new(3, 1).unwrap();
    gradient.gradient_fill_rect(
        Rect::new(0, 0, 3, 1),
        Color::rgb(255, 0, 0),
        Color::rgb(0, 0, 255),
        false,
    );
    assert_eq!(gradient.get_pixel(0, 0), Some(Color::rgb(255, 0, 0)));
    assert_eq!(gradient.get_pixel(2, 0), Some(Color::rgb(0, 0, 255)));
    let middle = gradient.get_pixel(1, 0).unwrap();
    assert!(middle.red > 100 && middle.blue > 100, "{middle:?}");

    // A single bright pixel spreads into its neighbours after a blur.
    let mut blurred = Bitmap::new(3, 3).unwrap();
    blurred.clear(Color::rgb(0, 0, 0));
    blurred.set_pixel(1, 1, Color::rgb(255, 255, 255));
    blurred.blur();
    let centre = blurred.get_pixel(1, 1).unwrap();
    assert!(centre.red > 0 && centre.red < 255, "{centre:?}");
    assert!(blurred.get_pixel(0, 0).unwrap().red > 0);

    // Radial blur of an angle of 0 is a no-op and never panics.
    let before = blurred.pixels().to_vec();
    blurred.radial_blur(0, 10);
    assert_eq!(blurred.pixels(), before.as_slice());
    blurred.radial_blur(30, 12);

    // A static snapshot of an animated bitmap drops the animation.
    let mut animated = Bitmap::new(1, 1).unwrap();
    animated.set_gif_frames(
        vec![
            vec![Color::rgb(10, 0, 0).packed()],
            vec![Color::rgb(20, 0, 0).packed()],
        ],
        vec![0.1, 0.1],
        0,
        None,
    );
    animated.goto_and_stop(1);
    let snapshot = animated.static_copy();
    assert!(!snapshot.animated());
    assert_eq!(snapshot.get_pixel(0, 0), Some(Color::rgb(20, 0, 0)));
}
