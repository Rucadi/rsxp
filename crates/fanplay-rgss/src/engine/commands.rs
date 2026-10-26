// SPDX-License-Identifier: GPL-2.0-or-later
//! Serialized RGSS command protocol: decode requests and update the runtime.
//!
//! A request is a header line followed by an optional raw payload:
//! `op\targ\targ...\n<payload>`. `op` selects the branch; positional args are
//! parsed with the `number`/`rect`/`color` helpers (rect = x,y,w,h; color =
//! r,g,b,a). Text results are returned as raw UTF-8 bytes. An error string of
//! `__FANPLAY_STOP__` tells the bridge to unwind and stop the game cleanly.
//!
//! `sync_batch` is a micro-batch: its payload holds 0x1e-separated records that
//! must be `sprite_sync`, `plane_sync` or `viewport_sync`, dispatched in order
//! so one bridge call replaces many per-object queries. Object syncs are
//! index-based and therefore positional — e.g. `sprite_sync` reads bitmap/src,
//! transform, clip, z/blend/color/tone, pattern fields and viewport from fixed
//! argument slots, with trailing bush_depth/bush_opacity optional.
//!
//! Some operations are feature-gated (`game-api`) or platform-gated (writing
//! bitmap files), and repo/host catalogs (runtime patches, settings, RPG
//! scripts) are served straight from embedded bytes.
use super::{Runtime, RuntimePlane, RuntimeSprite, RuntimeViewport, SpriteState};
use crate::graphics::{Bitmap, Color, Rect, Tone};
use crate::host::AudioChannel;

fn number<T: std::str::FromStr>(args: &[&str], index: usize) -> Result<T, String> {
    args.get(index)
        .ok_or_else(|| "missing native argument".to_string())?
        .parse()
        .map_err(|_| "invalid native argument".into())
}
fn rect(args: &[&str], index: usize) -> Result<Rect, String> {
    Ok(Rect::new(
        number(args, index)?,
        number(args, index + 1)?,
        number(args, index + 2)?,
        number(args, index + 3)?,
    ))
}
fn color(args: &[&str], index: usize) -> Result<Color, String> {
    Ok(Color::rgba(
        number(args, index)?,
        number(args, index + 1)?,
        number(args, index + 2)?,
        number(args, index + 3)?,
    ))
}
fn text(value: impl ToString) -> Vec<u8> {
    value.to_string().into_bytes()
}
fn audio_channel(args: &[&str], index: usize) -> Result<AudioChannel, String> {
    match args.get(index).copied() {
        Some("bgm") => Ok(AudioChannel::Bgm),
        Some("bgs") => Ok(AudioChannel::Bgs),
        Some("me") => Ok(AudioChannel::Me),
        Some("se") => Ok(AudioChannel::Se),
        _ => Err("invalid audio channel".into()),
    }
}

impl Runtime {
    pub(super) fn dispatch(&mut self, request: &[u8]) -> Result<Vec<u8>, String> {
        let split = request
            .iter()
            .position(|&b| b == b'\n')
            .unwrap_or(request.len());
        let header =
            std::str::from_utf8(&request[..split]).map_err(|_| "invalid native command")?;
        let payload = request.get(split + 1..).unwrap_or_default();
        let args: Vec<_> = header.split('\t').collect();
        match args[0] {
            "api_supported" => {
                return Ok(text(if cfg!(feature = "game-api") { "1" } else { "0" }));
            }
            #[cfg(feature = "game-api")]
            "api_game_bridge" => {
                return Ok(crate::ruby::GAME_BRIDGE_SOURCE.as_bytes().to_vec());
            }
            #[cfg(feature = "game-api")]
            "api_poll" => return Ok(crate::api_bridge::poll()),
            #[cfg(feature = "game-api")]
            "api_respond" => {
                let id = number::<u64>(&args, 1)?;
                let payload = std::str::from_utf8(payload).map_err(|e| e.to_string())?;
                crate::api_bridge::respond(id, payload)?;
            }
            "sync_batch" => {
                for command in payload
                    .split(|byte| *byte == 0x1e)
                    .filter(|part| !part.is_empty())
                {
                    let operation = command
                        .split(|byte| *byte == b'\t')
                        .next()
                        .unwrap_or_default();
                    if !matches!(operation, b"sprite_sync" | b"plane_sync" | b"viewport_sync") {
                        return Err(
                            "sync_batch only accepts sprite/plane/viewport snapshots".into()
                        );
                    }
                    self.dispatch(command)?;
                }
            }
            "version" => return Ok(text(self.version)),
            "system_power" => {
                let (seconds, percent, discharging) = self.host.power_state();
                return Ok(text(format!(
                    "{seconds}\t{percent}\t{}",
                    u8::from(discharging)
                )));
            }
            "window_title" => {
                self.host
                    .set_title(std::str::from_utf8(payload).map_err(|e| e.to_string())?);
            }
            "window_fullscreen" => return Ok(text(u8::from(self.fullscreen))),
            "window_fullscreen_set" => {
                let enabled = number::<u8>(&args, 1)? != 0;
                self.host.set_fullscreen(enabled)?;
                self.fullscreen = enabled;
            }
            "window_scale" => return Ok(text(self.scale)),
            "window_scale_set" => {
                let scale: f64 = number(&args, 1)?;
                if !scale.is_finite() || !(0.1..=8.0).contains(&scale) {
                    return Err("window scale must be 0.1..8".into());
                }
                self.host.set_scale(scale);
                self.scale = scale;
            }
            "window_center" => self.host.center(),
            "window_cursor" => self.host.show_cursor(number::<u8>(&args, 1)? != 0),
            // Runtime patches are a host-drivable catalog. `states` is what the
            // Ruby shim polls, `source` hands a patch its program on first use,
            // `list` feeds the browser panel, and `set` lets a host without a
            // shared process (browser, tests) toggle one by id.
            "runtime_patch_states" => return Ok(text(crate::runtime_patch::state_report())),
            "runtime_patch_list" => return Ok(text(crate::runtime_patch::list_report())),
            "runtime_patch_source" => {
                let id = args
                    .get(1)
                    .copied()
                    .ok_or("runtime_patch_source requires an id")?;
                return match crate::runtime_patch::find(id) {
                    Some(patch) => Ok(patch.source.as_bytes().to_vec()),
                    None => Err(format!("unknown runtime patch: {id}")),
                };
            }
            "runtime_patch_set" => {
                let id = args
                    .get(1)
                    .copied()
                    .ok_or("runtime_patch_set requires an id")?;
                let enabled = number::<u8>(&args, 2)? != 0;
                if !crate::runtime_patch::set_enabled(id, enabled) {
                    return Err(format!("unknown runtime patch: {id}"));
                }
            }
            // Rhai extensions inject only reversible runtime patches. The
            // source is carried as the raw command payload so arbitrary Ruby
            // whitespace and newlines never enter the tab-separated bridge.
            "runtime_patch_register" => {
                let name = args
                    .get(1)
                    .copied()
                    .ok_or("runtime_patch_register requires a name")?;
                let source = std::str::from_utf8(payload)
                    .map_err(|_| "runtime patch source must be UTF-8")?;
                if !crate::runtime_patch::register_user_patch(name, source.to_owned()) {
                    return Err("invalid runtime patch name".into());
                }
                if !crate::runtime_patch::set_enabled(&format!("user/{name}"), true) {
                    return Err("runtime patch could not be enabled".into());
                }
                return Ok(text(format!("user/{name}")));
            }
            "runtime_patch_unregister" => {
                let id = args
                    .get(1)
                    .copied()
                    .ok_or("runtime_patch_unregister requires an id")?;
                if !crate::runtime_patch::remove_user_patch(id) {
                    return Err(format!("unknown user runtime patch: {id}"));
                }
            }
            // Live frame pacing for hosts without a shared process (browser,
            // tests). The desktop workspace sets these in-process instead.
            "runtime_settings" => {
                return Ok(text(format!(
                    "{}\t{}",
                    self.rate,
                    u8::from(crate::runtime_settings::fast_forward())
                )));
            }
            "runtime_settings_set" => {
                if let Some(Some(rate)) = args.get(1).map(|value| value.parse::<u32>().ok()) {
                    crate::runtime_settings::set_rate_override(rate);
                }
                if let Some(value) = args.get(2) {
                    crate::runtime_settings::set_fast_forward(*value != "0");
                }
            }
            "runtime_settings_reset" => crate::runtime_settings::request_reset(),
            "runtime_patch_value_set" => {
                let id = args
                    .get(1)
                    .copied()
                    .ok_or("runtime_patch_value_set requires an id")?;
                let value: f64 = number(&args, 2)?;
                if !value.is_finite() || !crate::runtime_patch::set_value(id, value) {
                    return Err(format!("runtime patch has no settable value: {id}"));
                }
            }
            "rpg" => {
                return Ok(match self.version {
                    1 => include_bytes!("../ruby/rgss/rpg1.rb").to_vec(),
                    2 => include_bytes!("../ruby/rgss/rpg2.rb").to_vec(),
                    _ => include_bytes!("../ruby/rgss/rpg3.rb").to_vec(),
                })
            }
            "read" => {
                return self
                    .asset(std::str::from_utf8(payload).map_err(|_| "asset path must be UTF-8")?)
            }
            "exists" => {
                let path = std::str::from_utf8(payload).map_err(|_| "asset path must be UTF-8")?;
                return Ok(text(u8::from(self.asset(path).is_ok())));
            }
            "graphics_size" => {
                return Ok(text(format!(
                    "{}\t{}",
                    self.target.width(),
                    self.target.height()
                )))
            }
            "graphics_update" => self.update_frame()?,
            "graphics_freeze" => {
                self.frozen = Some(self.capture()?.clone());
                self.transitioning = None;
            }
            "graphics_transition" => {
                let frames: u32 = number(&args, 1)?;
                if frames == 0 {
                    self.frozen = None;
                    self.transitioning = None;
                } else {
                    self.transitioning = Some((0, frames));
                }
            }
            "graphics_brightness" => return Ok(text(self.brightness)),
            "graphics_brightness_set" => self.brightness = number(&args, 1)?,
            "graphics_frame_reset" => self.next_frame_deadline = None,
            "graphics_frame" => return Ok(text(self.frame)),
            "graphics_frame_set" => self.frame = number(&args, 1)?,
            "graphics_rate" => return Ok(text(self.rate)),
            "graphics_rate_set" => {
                let rate = number(&args, 1)?;
                if !(1..=240).contains(&rate) {
                    return Err("frame rate must be 1..240".into());
                }
                self.rate = rate;
                self.next_frame_deadline = None;
            }
            "graphics_resize" => {
                let (w, h) = (number(&args, 1)?, number(&args, 2)?);
                let target = Bitmap::new(w, h).map_err(|e| e.to_string())?;
                self.host.resize(w, h)?;
                self.target = target;
                self.gpu_frame = false;
            }
            // Resizes only the logical render target, leaving the native window
            // alone. The presentation layer scales the target to fit, so a
            // larger target reveals more of the scene (map panoramic mode).
            "graphics_resolution_set" => {
                let (w, h) = (number(&args, 1)?, number(&args, 2)?);
                if !(1..=4096).contains(&w) || !(1..=4096).contains(&h) {
                    return Err("render resolution must be 1..4096".into());
                }
                if (self.target.width(), self.target.height()) != (w, h) {
                    self.target = Bitmap::new(w, h).map_err(|e| e.to_string())?;
                    self.gpu_frame = false;
                }
            }
            "graphics_snap" => {
                let id = self.id();
                let snapshot = self.snapshot()?;
                self.bitmaps.insert(id, snapshot);
                return Ok(text(id));
            }
            "input_update" | "input_poll" => {
                // A workspace "reset" unwinds the running game here; the host
                // then restarts the process from the beginning.
                if crate::runtime_settings::reset_requested() {
                    return Err("__FANPLAY_STOP__".into());
                }
                self.text_buffer.push_str(&self.pending.text);
                self.pending.text.clear();
                self.input.copy_state_from(&self.pending);
                let (repeat_start, repeat_delay) =
                    if self.version == 1 { (15, 4) } else { (23, 6) };
                self.input.advance(repeat_start, repeat_delay);
                self.pending.begin_frame();
                if args[0] == "input_poll" {
                    return Ok(self.input.snapshot());
                }
            }
            "input_repeat" => return Ok(text(u8::from(self.input.repeated(number(&args, 1)?)))),
            "input_gets" => {
                let text = std::mem::take(&mut self.text_buffer);
                return Ok(text.into_bytes());
            }
            "input_text_mode" => return Ok(text(u8::from(self.text_input))),
            "input_text_mode_set" => {
                let enabled = number::<u8>(&args, 1)? != 0;
                self.host.text_input(enabled);
                self.text_input = enabled;
            }
            "input_press" => return Ok(text(u8::from(self.input.pressed(number(&args, 1)?)))),
            "input_trigger" => return Ok(text(u8::from(self.input.triggered(number(&args, 1)?)))),
            "input_release" => return Ok(text(u8::from(self.input.released(number(&args, 1)?)))),
            "input_mouse" => {
                return Ok(text(format!(
                    "{}\t{}\t{}\t{}",
                    self.input.mouse_x,
                    self.input.mouse_y,
                    u8::from(self.input.mouse_inside),
                    self.input.scroll_v
                )))
            }
            "audio_play" => {
                let channel = audio_channel(&args, 1)?;
                let volume: i32 = number(&args, 2)?;
                let pitch: f64 = number(&args, 3)?;
                let position: f64 = number(&args, 4)?;
                let path = std::str::from_utf8(payload).map_err(|_| "audio path must be UTF-8")?;
                if self.host.audio().is_some() {
                    match self.audio_bytes(path) {
                        Ok(bytes) => {
                            if let Some(audio) = self.host.audio() {
                                audio.play(channel, path, &bytes, volume, pitch, position);
                            }
                        }
                        Err(error) => eprintln!("audio: {error}"),
                    }
                }
            }
            "audio_stop" => {
                let channel = audio_channel(&args, 1)?;
                if let Some(audio) = self.host.audio() {
                    audio.stop(channel);
                }
            }
            "audio_fade" => {
                let channel = audio_channel(&args, 1)?;
                let ms: i32 = number(&args, 2)?;
                if let Some(audio) = self.host.audio() {
                    audio.fade(channel, ms);
                }
            }
            "audio_pos" => {
                let channel = audio_channel(&args, 1)?;
                let position = self
                    .host
                    .audio()
                    .map(|audio| audio.position(channel))
                    .unwrap_or(0.0);
                return Ok(text(position));
            }
            "audio_volume" => {
                let channel = audio_channel(&args, 1)?;
                let volume = self
                    .host
                    .audio()
                    .map(|audio| audio.volume(channel))
                    .unwrap_or(100);
                return Ok(text(volume));
            }
            "audio_set_volume" => {
                let channel = audio_channel(&args, 1)?;
                let volume: i32 = number(&args, 2)?;
                if let Some(audio) = self.host.audio() {
                    audio.set_volume(channel, volume);
                }
            }
            "bitmap_new" => {
                let bitmap =
                    Bitmap::new(number(&args, 1)?, number(&args, 2)?).map_err(|e| e.to_string())?;
                let id = self.id();
                self.bitmaps.insert(id, bitmap);
                return Ok(text(id));
            }
            "bitmap_load" => {
                let name = std::str::from_utf8(payload)
                    .map_err(|e| e.to_string())?
                    .replace('\\', "/");
                let bytes = self.image_bytes(&name)?;
                let bitmap = if crate::media::gif::is_gif(&bytes) {
                    crate::media::gif::decode(&bytes)?
                } else {
                    self.host.image(&bytes)?
                };
                let id = self.id();
                self.bitmaps.insert(id, bitmap);
                return Ok(text(id));
            }
            "text_size" | "bitmap_text" => {
                let split = payload
                    .iter()
                    .position(|&b| b == 0)
                    .ok_or("missing font/text separator")?;
                let font = self
                    .font_bytes(std::str::from_utf8(&payload[..split]).map_err(|e| e.to_string())?);
                let string =
                    std::str::from_utf8(&payload[split + 1..]).map_err(|e| e.to_string())?;
                let render = args[0] == "bitmap_text";
                let start = if render { 7 } else { 1 };
                let foreground = if render {
                    color(&args, 10)?
                } else {
                    Color::rgb(255, 255, 255)
                };
                let (w, h, pixels) = self.host.text(
                    font.as_ref().map(|bytes| bytes.as_slice()),
                    number(&args, start)?,
                    number::<u8>(&args, start + 1)? != 0,
                    number::<u8>(&args, start + 2)? != 0,
                    string,
                    foreground,
                    render,
                )?;
                if !render {
                    return Ok(text(format!("{w}\t{h}")));
                }
                if let Some(pixels) = pixels {
                    let mut area = rect(&args, 2)?;
                    let clip = area;
                    let align: i32 = number(&args, 6)?;
                    let scaled = w.min(area.width);
                    area.x += (match align {
                        1 => (area.width - scaled) / 2,
                        2 => area.width - scaled,
                        _ => 0,
                    }) as i32;
                    area.y += (area.height.saturating_sub(h) / 2) as i32;
                    area.width = scaled;
                    area.height = h;
                    let outline = number::<u8>(&args, 14)? != 0;
                    let shadow = number::<u8>(&args, 15)? != 0;
                    if outline || shadow {
                        let (_, _, edge) = self.host.text(
                            font.as_ref().map(|bytes| bytes.as_slice()),
                            number(&args, 7)?,
                            number::<u8>(&args, 8)? != 0,
                            number::<u8>(&args, 9)? != 0,
                            string,
                            color(&args, 16)?,
                            true,
                        )?;
                        if let Some(edge) = edge {
                            let offsets = if outline {
                                vec![
                                    (-1, -1),
                                    (0, -1),
                                    (1, -1),
                                    (-1, 0),
                                    (1, 0),
                                    (-1, 1),
                                    (0, 1),
                                    (1, 1),
                                ]
                            } else {
                                vec![(1, 1)]
                            };
                            for (x, y) in offsets {
                                let shifted = Rect::new(
                                    area.x.saturating_add(x),
                                    area.y.saturating_add(y),
                                    area.width,
                                    area.height,
                                );
                                self.bitmap_mut(number(&args, 1)?)?.stretch_blt_clipped(
                                    shifted,
                                    &edge,
                                    edge.rect(),
                                    255,
                                    clip,
                                );
                            }
                        }
                    }
                    self.bitmap_mut(number(&args, 1)?)?.stretch_blt_clipped(
                        area,
                        &pixels,
                        pixels.rect(),
                        255,
                        clip,
                    );
                }
            }
            "bitmap_clone" => {
                let bitmap = self.bitmap(number(&args, 1)?)?.clone();
                let id = self.id();
                self.bitmaps.insert(id, bitmap);
                return Ok(text(id));
            }
            "bitmap_dispose" => {
                self.bitmaps.remove(&number(&args, 1)?);
            }
            "bitmap_size" => {
                let b = self.bitmap(number(&args, 1)?)?;
                return Ok(text(format!("{}\t{}", b.width(), b.height())));
            }
            "bitmap_animated" => {
                return Ok(text(u8::from(self.bitmap(number(&args, 1)?)?.animated())));
            }
            "bitmap_frame_count" => {
                return Ok(text(self.bitmap(number(&args, 1)?)?.frame_count()));
            }
            "bitmap_current_frame" => {
                return Ok(text(self.bitmap(number(&args, 1)?)?.current_frame()));
            }
            "bitmap_frame_rate" => {
                return Ok(text(self.bitmap(number(&args, 1)?)?.frame_rate()));
            }
            "bitmap_frame_rate_set" => {
                self.bitmap_mut(number(&args, 1)?)?
                    .set_frame_rate(number(&args, 2)?);
            }
            "bitmap_playing" => {
                return Ok(text(u8::from(self.bitmap(number(&args, 1)?)?.playing())));
            }
            "bitmap_playing_set" => {
                self.bitmap_mut(number(&args, 1)?)?
                    .set_playing(number::<u8>(&args, 2)? != 0);
            }
            "bitmap_goto_and_play" => {
                self.bitmap_mut(number(&args, 1)?)?
                    .goto_and_play(number(&args, 2)?);
            }
            "bitmap_goto_and_stop" => {
                self.bitmap_mut(number(&args, 1)?)?
                    .goto_and_stop(number(&args, 2)?);
            }
            "bitmap_play" => self.bitmap_mut(number(&args, 1)?)?.set_playing(true),
            "bitmap_stop" => self.bitmap_mut(number(&args, 1)?)?.set_playing(false),
            "movie_open" => {
                let name = std::str::from_utf8(payload)
                    .map_err(|e| e.to_string())?
                    .replace('\\', "/");
                let bytes = self.movie_bytes(&name)?;
                let movie = crate::media::movie::Movie::open(bytes)?;
                let id = self.id();
                let (width, height) = (movie.width(), movie.height());
                self.movies.insert(id, movie);
                return Ok(text(format!("{id}\t{width}\t{height}")));
            }
            "movie_frame" => {
                let id = number::<u64>(&args, 1)?;
                let bitmap_id = number::<u64>(&args, 2)?;
                // Headless runs do not sleep between frames, so wall-clock time
                // can advance much slower than the frame counter. Drive movie
                // timing from the frame clock there to keep playback (and the
                // frame budget in automated tests) deterministic.
                let now = if self.headless {
                    self.updates as f64 / f64::from(self.rate.max(1))
                } else {
                    self.host.now()
                };
                let start = *self.movie_start.entry(id).or_insert(now);
                let elapsed = (now - start).max(0.0);
                let frame = match self.movies.get_mut(&id) {
                    Some(movie) => movie.frame_at(elapsed)?.cloned(),
                    None => return Err("movie is not open".into()),
                };
                let finished = self.movies.get(&id).is_some_and(|movie| movie.finished());
                if let Some(frame) = frame {
                    self.bitmaps.insert(bitmap_id, frame);
                    return Ok(text(if finished { "0" } else { "1" }));
                }
                return Ok(text("0"));
            }
            "movie_close" => {
                let id = number::<u64>(&args, 1)?;
                self.movies.remove(&id);
                self.movie_start.remove(&id);
            }
            "bitmap_fill" => {
                let area = rect(&args, 2)?;
                let color = color(&args, 6)?;
                self.bitmap_mut(number(&args, 1)?)?.fill_rect(area, color);
            }
            "bitmap_clear" => self.bitmap_mut(number(&args, 1)?)?.clear(Color::default()),
            "bitmap_hue" => {
                let hue: f32 = number(&args, 2)?;
                if !hue.is_finite() {
                    return Err("hue must be finite".into());
                }
                self.bitmap_mut(number(&args, 1)?)?.hue_change(hue);
            }
            "bitmap_raw_data" => {
                return Ok(self.bitmap(number(&args, 1)?)?.to_rgba());
            }
            "bitmap_set_raw_data" => {
                if !self.bitmap_mut(number(&args, 1)?)?.replace_rgba(payload) {
                    return Err("raw_data size does not match the bitmap".into());
                }
            }
            "bitmap_kgl_invert" => self.bitmap_mut(number(&args, 1)?)?.kgl_invert(),
            "bitmap_kgl_compress_alpha" => {
                self.bitmap_mut(number(&args, 1)?)?.kgl_compress_alpha();
            }
            "bitmap_kgl_subtract" => {
                let src = self.bitmap(number(&args, 4)?)?.clone();
                self.bitmap_mut(number(&args, 1)?)?.kgl_subtract_rect(
                    number(&args, 2)?,
                    number(&args, 3)?,
                    &src,
                    rect(&args, 5)?,
                    number(&args, 9)?,
                );
            }
            "bitmap_kgl_shadow_h" => {
                let status = self.bitmap_mut(number(&args, 1)?)?.kgl_shadow_shader_h(
                    number(&args, 2)?,
                    number(&args, 3)?,
                    number(&args, 4)?,
                    number::<u8>(&args, 5)? != 0,
                );
                return Ok(text(status));
            }
            "bitmap_kgl_shadow_v" => {
                let status = self.bitmap_mut(number(&args, 1)?)?.kgl_shadow_shader_v(
                    number(&args, 2)?,
                    number(&args, 3)?,
                    number(&args, 4)?,
                    number::<u8>(&args, 5)? != 0,
                    number::<u8>(&args, 6)? != 0,
                );
                return Ok(text(status));
            }
            "bitmap_blur" => self.bitmap_mut(number(&args, 1)?)?.blur(),
            "bitmap_radial_blur" => {
                self.bitmap_mut(number(&args, 1)?)?
                    .radial_blur(number(&args, 2)?, number(&args, 3)?);
            }
            "bitmap_gradient_fill" => {
                let area = rect(&args, 2)?;
                let color1 = color(&args, 6)?;
                let color2 = color(&args, 10)?;
                let vertical = number::<u8>(&args, 14)? != 0;
                self.bitmap_mut(number(&args, 1)?)?
                    .gradient_fill_rect(area, color1, color2, vertical);
            }
            "bitmap_snap" => {
                let mut copy = self.bitmap(number(&args, 1)?)?.clone();
                let position: i32 = number(&args, 2)?;
                if position >= 0 {
                    copy.goto_and_stop(position as u32);
                }
                copy.set_playing(false);
                let id = self.id();
                self.bitmaps.insert(id, copy.static_copy());
                return Ok(text(id));
            }
            "bitmap_to_file" => {
                let path = std::str::from_utf8(payload).map_err(|e| e.to_string())?;
                let (width, height, rgba) = {
                    let bitmap = self.bitmap(number(&args, 1)?)?;
                    (bitmap.width(), bitmap.height(), bitmap.to_rgba())
                };
                #[cfg(all(feature = "game-api", not(target_arch = "wasm32")))]
                {
                    image::save_buffer(path, &rgba, width, height, image::ExtendedColorType::Rgba8)
                        .map_err(|error| error.to_string())?;
                }
                #[cfg(not(all(feature = "game-api", not(target_arch = "wasm32"))))]
                {
                    let _ = (path, width, height, rgba);
                    return Err("writing bitmap files is not supported on this platform".into());
                }
            }
            "bitmap_set" => {
                let color = color(&args, 4)?;
                self.bitmap_mut(number(&args, 1)?)?.set_pixel(
                    number(&args, 2)?,
                    number(&args, 3)?,
                    color,
                );
            }
            "bitmap_get" => {
                let c = self
                    .bitmap(number(&args, 1)?)?
                    .get_pixel(number(&args, 2)?, number(&args, 3)?)
                    .unwrap_or_default();
                return Ok(text(format!(
                    "{}\t{}\t{}\t{}",
                    c.red, c.green, c.blue, c.alpha
                )));
            }
            "bitmap_blt" => {
                let src = self.bitmap(number(&args, 6)?)?.clone();
                self.bitmap_mut(number(&args, 1)?)?.stretch_blt(
                    rect(&args, 2)?,
                    &src,
                    rect(&args, 7)?,
                    number(&args, 11)?,
                );
            }
            "sprite_new" => {
                self.invalidate_scene();
                let id = self.id();
                self.sprites.insert(
                    id,
                    RuntimeSprite {
                        world: None,
                        #[cfg(feature = "gpu")]
                        native_tile: false,
                        order: id,
                        state: SpriteState {
                            clip: self.target.rect(),
                            zoom_x: 1.0,
                            zoom_y: 1.0,
                            opacity: 255,
                            visible: true,
                            pattern_tile: true,
                            pattern_opacity: 255,
                            pattern_zoom_x: 1.0,
                            pattern_zoom_y: 1.0,
                            bush_opacity: 128,
                            ..Default::default()
                        },
                    },
                );
                return Ok(text(id));
            }
            "sprite_tile" => {
                #[cfg(feature = "gpu")]
                {
                    self.sprites
                        .get_mut(&number(&args, 1)?)
                        .ok_or("disposed tile sprite")?
                        .native_tile = true;
                }
            }
            "sprite_dispose" => {
                self.invalidate_scene();
                self.sprites.remove(&number(&args, 1)?);
            }
            "plane_new" => {
                self.invalidate_scene();
                let id = self.id();
                self.planes.insert(
                    id,
                    RuntimePlane {
                        bitmap: 0,
                        viewport: 0,
                        clip: self.target.rect(),
                        ox: 0,
                        oy: 0,
                        zoom_x: 1.0,
                        zoom_y: 1.0,
                        opacity: 255,
                        blend_type: 0,
                        color: Color::default(),
                        tone: Tone::default(),
                        visible: true,
                        z: 0,
                        order: id,
                    },
                );
                return Ok(text(id));
            }
            "plane_dispose" => {
                self.invalidate_scene();
                self.planes.remove(&number(&args, 1)?);
            }
            "plane_sync" => {
                let id: u64 = number(&args, 1)?;
                let visible = number::<u8>(&args, 8)? != 0;
                let z = number(&args, 9)?;
                let viewport = number(&args, 23)?;
                let previous = self.planes.get(&id).ok_or("disposed plane")?;
                if previous.visible != visible
                    || (visible && (previous.z != z || previous.viewport != viewport))
                {
                    self.invalidate_scene();
                }
                let plane = self.planes.get_mut(&id).ok_or("disposed plane")?;
                plane.bitmap = number(&args, 2)?;
                plane.ox = number(&args, 3)?;
                plane.oy = number(&args, 4)?;
                plane.zoom_x = number(&args, 5)?;
                plane.zoom_y = number(&args, 6)?;
                plane.opacity = number(&args, 7)?;
                plane.visible = visible;
                plane.z = z;
                plane.blend_type = number(&args, 10)?;
                plane.color = color(&args, 11)?;
                plane.tone = Tone {
                    red: number(&args, 15)?,
                    green: number(&args, 16)?,
                    blue: number(&args, 17)?,
                    gray: number(&args, 18)?,
                };
                plane.clip = rect(&args, 19)?;
                plane.viewport = viewport;
            }
            "viewport_new" => {
                self.invalidate_scene();
                let id = self.id();
                self.viewports.insert(
                    id,
                    RuntimeViewport {
                        rect: self.target.rect(),
                        z: 0,
                        ox: 0,
                        oy: 0,
                        visible: true,
                        color: Color::default(),
                        tone: Tone::default(),
                        order: id,
                        buffer: None,
                    },
                );
                return Ok(text(id));
            }
            "viewport_dispose" => {
                self.invalidate_scene();
                self.viewports.remove(&number(&args, 1)?);
            }
            "viewport_sync" => {
                let id: u64 = number(&args, 1)?;
                let z = number(&args, 9)?;
                // As in the engine's SceneElement setters, geometry and effects
                // do not reinsert an element into its ordered scene. Child
                // synchronization handles viewport visibility changes.
                if self.viewports.get(&id).ok_or("disposed viewport")?.z != z {
                    self.invalidate_scene();
                }
                let viewport = self.viewports.get_mut(&id).ok_or("disposed viewport")?;
                viewport.rect = rect(&args, 2)?;
                viewport.ox = number(&args, 6)?;
                viewport.oy = number(&args, 7)?;
                viewport.visible = number::<u8>(&args, 8)? != 0;
                viewport.z = z;
                viewport.color = color(&args, 10)?;
                viewport.tone = Tone {
                    red: number(&args, 14)?,
                    green: number(&args, 15)?,
                    blue: number(&args, 16)?,
                    gray: number(&args, 17)?,
                };
            }
            "sprite_sync" => {
                let id: u64 = number(&args, 1)?;
                let state = SpriteState {
                    bitmap: number(&args, 2)?,
                    src: rect(&args, 3)?,
                    x: number(&args, 7)?,
                    y: number(&args, 8)?,
                    ox: number(&args, 9)?,
                    oy: number(&args, 10)?,
                    zoom_x: number(&args, 11)?,
                    zoom_y: number(&args, 12)?,
                    angle: number(&args, 13)?,
                    mirror: number::<u8>(&args, 14)? != 0,
                    opacity: number(&args, 15)?,
                    visible: number::<u8>(&args, 16)? != 0,
                    clip: rect(&args, 17)?,
                    z: number(&args, 21)?,
                    blend_type: number(&args, 22)?,
                    color: color(&args, 23)?,
                    tone: Tone {
                        red: number(&args, 27)?,
                        green: number(&args, 28)?,
                        blue: number(&args, 29)?,
                        gray: number(&args, 30)?,
                    },
                    invert: number::<u8>(&args, 31)? != 0,
                    pattern: number(&args, 32)?,
                    pattern_blend: number(&args, 33)?,
                    pattern_tile: number::<u8>(&args, 34)? != 0,
                    pattern_opacity: number(&args, 35)?,
                    pattern_scroll_x: number(&args, 36)?,
                    pattern_scroll_y: number(&args, 37)?,
                    pattern_zoom_x: number(&args, 38)?,
                    pattern_zoom_y: number(&args, 39)?,
                    viewport: number(&args, 40)?,
                    bush_depth: args
                        .get(41)
                        .map(|_| number::<i32>(&args, 41))
                        .transpose()?
                        .unwrap_or(0)
                        .max(0),
                    bush_opacity: args
                        .get(42)
                        .map(|_| number(&args, 42))
                        .transpose()?
                        .unwrap_or(128),
                };
                self.sync_sprite(id, state)?;
            }
            op => return Err(format!("unsupported native operation: {op}")),
        }
        Ok(Vec::new())
    }
}
