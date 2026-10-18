// SPDX-License-Identifier: GPL-2.0-or-later
//! Runtime integration checks with lightweight test hosts.
use super::*;
use std::time::Duration;
#[cfg(feature = "gpu")]
#[test]
fn retained_scene_tracks_order_membership_and_disposal() {
    struct TestHost;
    impl Host for TestHost {
        fn read_file(&self, _: &str) -> Option<Vec<u8>> {
            None
        }
        fn image(&self, _: &[u8]) -> Result<Bitmap, String> {
            Err("unused".into())
        }
        fn text(
            &self,
            _: Option<&[u8]>,
            _: i32,
            _: bool,
            _: bool,
            _: &str,
            _: Color,
            _: bool,
        ) -> Result<(u32, u32, Option<Bitmap>), String> {
            Err("unused".into())
        }
    }
    let mut runtime = Runtime::new(RuntimeOptions {
        width: 64,
        height: 48,
        version: 1,
        headless: true,
        frame_limit: None,
        rate: 60,
        fast_forward: true,
        archives: vec![],
        scripted: vec![],
        host: Box::new(TestHost),
    })
    .unwrap();
    runtime.scene_cache = true;
    let new_id = |runtime: &mut Runtime, op: &[u8]| -> u64 {
        String::from_utf8(runtime.dispatch(op).unwrap())
            .unwrap()
            .parse()
            .unwrap()
    };
    let vp = new_id(&mut runtime, b"viewport_new\n");
    let a = new_id(&mut runtime, b"sprite_new\n");
    let b = new_id(&mut runtime, b"sprite_new\n");
    let mut state = SpriteState {
        visible: true,
        viewport: vp,
        ..Default::default()
    };
    runtime.sync_sprite(a, state.clone()).unwrap();
    state.y = 1.0;
    runtime.sync_sprite(b, state.clone()).unwrap();
    runtime.prepare_scene();
    assert_eq!(
        runtime.scene_plan,
        [
            SceneDraw::BeginViewport(vp),
            SceneDraw::Sprite(a),
            SceneDraw::Sprite(b),
            SceneDraw::EndViewport(vp)
        ]
    );
    state.x = 5.0;
    state.opacity = 100;
    runtime.sync_sprite(b, state.clone()).unwrap();
    assert!(
        !runtime.scene_dirty,
        "geometry/effects do not change ordering"
    );
    // y must not change draw order: RGSS sorts by z only, and equal z keeps
    // creation order so full-screen overlays (y = 0) stay on top of windows.
    state.y = -1.0;
    runtime.sync_sprite(b, state.clone()).unwrap();
    assert!(!runtime.scene_dirty, "y does not change ordering");
    runtime.prepare_scene();
    assert_eq!(
        runtime.scene_plan[1..3],
        [SceneDraw::Sprite(a), SceneDraw::Sprite(b)]
    );
    state.viewport = 0;
    state.z = -1;
    runtime.sync_sprite(b, state.clone()).unwrap();
    runtime.prepare_scene();
    assert_eq!(runtime.scene_plan[0], SceneDraw::Sprite(b));
    state.visible = false;
    runtime.sync_sprite(b, state).unwrap();
    runtime.prepare_scene();
    assert!(!runtime.scene_plan.contains(&SceneDraw::Sprite(b)));
    runtime
        .dispatch(format!("viewport_dispose\t{vp}\n").as_bytes())
        .unwrap();
    runtime.prepare_scene();
    assert_eq!(runtime.scene_plan, [SceneDraw::Sprite(a)]);
    runtime
        .dispatch(format!("sprite_dispose\t{a}\n").as_bytes())
        .unwrap();
    runtime.prepare_scene();
    assert!(runtime.scene_plan.is_empty());
}

#[test]
fn pacing_includes_ruby_work_between_updates() {
    use std::{cell::Cell, rc::Rc};
    struct ClockHost(Rc<Cell<f64>>);
    impl Host for ClockHost {
        fn now(&self) -> f64 {
            self.0.get()
        }
        fn sleep(&self, duration: Duration) {
            self.0.set(self.0.get() + duration.as_secs_f64());
        }
        fn read_file(&self, _: &str) -> Option<Vec<u8>> {
            None
        }
        fn image(&self, _: &[u8]) -> Result<Bitmap, String> {
            Err("unused".into())
        }
        fn text(
            &self,
            _: Option<&[u8]>,
            _: i32,
            _: bool,
            _: bool,
            _: &str,
            _: Color,
            _: bool,
        ) -> Result<(u32, u32, Option<Bitmap>), String> {
            Err("unused".into())
        }
    }
    let clock = Rc::new(Cell::new(0.0));
    let mut runtime = Runtime::new(RuntimeOptions {
        width: 1,
        height: 1,
        version: 1,
        headless: false,
        frame_limit: None,
        rate: 60,
        fast_forward: false,
        archives: vec![],
        scripted: vec![],
        host: Box::new(ClockHost(clock.clone())),
    })
    .unwrap();
    runtime.update_frame().unwrap();
    clock.set(clock.get() + 0.010); // Ruby spends 10 ms computing the next frame.
    runtime.update_frame().unwrap();
    assert!((clock.get() - 2.0 / 60.0).abs() < 1e-8);
    clock.set(clock.get() + 0.050); // An overloaded frame adds no sleep.
    let overloaded = clock.get();
    runtime.update_frame().unwrap();
    assert_eq!(clock.get(), overloaded);
}
#[test]
fn parses_scripted_input() {
    let keys = parse_scripted("10:40:3,20:81").unwrap();
    assert_eq!(keys.len(), 2);
    assert_eq!((keys[0].start, keys[0].key, keys[0].hold), (10, 40, 3));
    assert_eq!((keys[1].start, keys[1].key, keys[1].hold), (20, 81, 1));
}
#[test]
fn rejects_bad_scripted_input() {
    assert!(parse_scripted("x:1").is_err());
    assert!(parse_scripted("1:2:0").is_err());
    assert!(parse_scripted("1").is_err());
    assert!(parse_scripted("1:2:3:4").is_err());
}

/// Checks the engine decoder composes GIF frames for the RGSS bitmap animation API.
#[test]
fn animated_gif_decodes_and_advances_frames() {
    // Two 1x1 frames with different colors; generated in memory so the test
    // does not depend on assets from a bundled game.
    let mut bytes = Vec::new();
    {
        let mut encoder = gif::Encoder::new(&mut bytes, 1, 1, &[0xff, 0, 0, 0, 0xff, 0]).unwrap();
        encoder.set_repeat(gif::Repeat::Infinite).unwrap();
        for color in [0, 1] {
            let mut frame = gif::Frame::default();
            frame.width = 1;
            frame.height = 1;
            frame.delay = 10;
            frame.buffer = vec![color].into();
            encoder.write_frame(&frame).unwrap();
        }
    }
    assert!(crate::media::gif::is_gif(&bytes), "fixture should be a GIF");
    let mut bitmap = crate::media::gif::decode(&bytes).expect("gif should decode");
    assert!(bitmap.animated(), "fixture should be animated");
    let frames = bitmap.frame_count();
    assert!(frames > 1, "expected multiple frames, got {frames}");
    assert_eq!(bitmap.current_frame(), 0);
    assert!(bitmap.playing());
    assert!(bitmap.frame_rate() > 0.0);
    let first = bitmap.pixels().to_vec();
    bitmap.advance_animation(0.5);
    assert!(bitmap.current_frame() > 0, "advancing should move frames");
    assert_ne!(bitmap.pixels(), first.as_slice());
    bitmap.goto_and_stop(frames - 1);
    assert_eq!(bitmap.current_frame(), frames - 1);
    assert!(!bitmap.playing());
    let stopped = bitmap.current_frame();
    bitmap.advance_animation(0.5);
    assert_eq!(bitmap.current_frame(), stopped, "stop holds the frame");
    bitmap.goto_and_play(0);
    assert!(bitmap.playing());
    assert_eq!(bitmap.current_frame(), 0);
}

#[test]
fn movie_ycbcr_conversion_uses_the_visible_picture() {
    use theora_rs::{ImgPlane, Info, PixelFmt};
    let mut info = Info::new();
    info.pic_width = 2;
    info.pic_height = 1;
    info.pixel_fmt = PixelFmt::Pf444;
    let plane = |value: u8| ImgPlane {
        width: 2,
        height: 1,
        stride: 2,
        data: vec![value; 2],
        data_offset: 0,
    };
    // Luma near white with neutral chroma should convert to a light pixel.
    let frame = [plane(235), plane(128), plane(128)];
    let bitmap = crate::media::movie::ycbcr_to_bitmap(&frame, &info).expect("conversion");
    assert_eq!((bitmap.width(), bitmap.height()), (2, 1));
    let pixel = bitmap.get_pixel(1, 0).unwrap();
    assert!(pixel.red > 200 && pixel.green > 200 && pixel.blue > 200);
    assert_eq!(pixel.alpha, 255);
    // A movie that is not an Ogg/Theora stream is rejected without panicking.
    assert!(crate::media::movie::Movie::open(vec![0, 1, 2, 3]).is_err());
}

/// `Graphics.snap_to_bitmap` must composite the *current* scene, matching
/// mkxp-z's `Graphics::snapToBitmap`. This is what lets the "Map Zoom" plugin
/// clear its overlay bitmap and capture the clean map instead of feeding the
/// zoomed overlay back into itself (which blacks the screen).
#[test]
fn graphics_snapshot_recomposites_the_current_scene() {
    struct SnapHost;
    impl Host for SnapHost {
        fn read_file(&self, _: &str) -> Option<Vec<u8>> {
            None
        }
        fn image(&self, _: &[u8]) -> Result<Bitmap, String> {
            Err("unused".into())
        }
        fn text(
            &self,
            _: Option<&[u8]>,
            _: i32,
            _: bool,
            _: bool,
            _: &str,
            _: Color,
            _: bool,
        ) -> Result<(u32, u32, Option<Bitmap>), String> {
            Err("unused".into())
        }
    }
    let mut runtime = Runtime::new(RuntimeOptions {
        width: 4,
        height: 4,
        version: 3,
        headless: true,
        frame_limit: None,
        rate: 60,
        fast_forward: true,
        archives: vec![],
        scripted: vec![],
        host: Box::new(SnapHost),
    })
    .unwrap();
    let id = |runtime: &mut Runtime, op: &str| -> u64 {
        String::from_utf8(runtime.dispatch(op.as_bytes()).unwrap())
            .unwrap()
            .parse()
            .unwrap()
    };
    let base = id(&mut runtime, "bitmap_new\t4\t4\n");
    let overlay = id(&mut runtime, "bitmap_new\t4\t4\n");
    runtime
        .dispatch(format!("bitmap_fill\t{base}\t0\t0\t4\t4\t255\t0\t0\t255\n").as_bytes())
        .unwrap();
    runtime
        .dispatch(format!("bitmap_fill\t{overlay}\t0\t0\t4\t4\t0\t0\t255\t255\n").as_bytes())
        .unwrap();
    let base_sprite = id(&mut runtime, "sprite_new\n");
    let overlay_sprite = id(&mut runtime, "sprite_new\n");
    let state = |bitmap: u64, z: i32| SpriteState {
        clip: Rect::new(0, 0, 4, 4),
        bitmap,
        src: Rect::new(0, 0, 4, 4),
        zoom_x: 1.0,
        zoom_y: 1.0,
        opacity: 255,
        visible: true,
        z,
        ..Default::default()
    };
    runtime.sync_sprite(base_sprite, state(base, 0)).unwrap();
    runtime.sync_sprite(overlay_sprite, state(overlay, 10)).unwrap();
    let pixel = |runtime: &mut Runtime, snapshot: u64| -> (i64, i64, i64) {
        let bytes = runtime
            .dispatch(format!("bitmap_get\t{snapshot}\t1\t1\n").as_bytes())
            .unwrap();
        let values: Vec<i64> = String::from_utf8(bytes)
            .unwrap()
            .split('\t')
            .map(|part| part.parse().unwrap())
            .collect();
        (values[0], values[1], values[2])
    };
    // The opaque overlay (blue) is on top.
    let first = id(&mut runtime, "graphics_snap\n");
    assert_eq!(pixel(&mut runtime, first), (0, 0, 255));
    // Clearing the overlay bitmap must be reflected by the next snapshot.
    runtime
        .dispatch(format!("bitmap_clear\t{overlay}\n").as_bytes())
        .unwrap();
    let second = id(&mut runtime, "graphics_snap\n");
    assert_eq!(pixel(&mut runtime, second), (255, 0, 0));
}
