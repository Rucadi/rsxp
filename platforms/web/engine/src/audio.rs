// SPDX-License-Identifier: GPL-2.0-or-later
//! Browser audio playback using the browser's native media decoder.
//!
//! HTMLMediaElement keeps compressed music compressed and streams it through
//! the browser's codec stack. This is a better fit for the web build than
//! decoding entire BGM files to PCM in WASM.
//!
//! `BrowserAudio` implements `fanplay_rgss::host::AudioBackend`. Each play creates an
//! `HtmlAudioElement` routed through a Web Audio `GainNode` (when an
//! `AudioContext` is available) so per-channel master volume and fades can be
//! applied without touching the element's own volume. BGM/BGS loop and are
//! exclusive per channel; SE voices are one-shot and stack. Because browsers
//! block autoplay until a user gesture, `pointerdown`/`touchstart`/`keydown`
//! listeners resume the context and retry pending playback. A 25 ms interval
//! advances fades even while the game frame pump is idle, and `Drop` removes the
//! listeners, clears the timer and revokes the object URLs.
use js_sys::{Array, Date, Uint8Array};
use fanplay_rgss::host::{AudioBackend, AudioChannel};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AudioContext, Blob, BlobPropertyBag, Event, GainNode, HtmlAudioElement,
    MediaElementAudioSourceNode, Url, Window,
};

struct Fade {
    from: f64,
    started_at_ms: f64,
    duration_ms: f64,
}

struct Voice {
    id: u64,
    channel: AudioChannel,
    element: HtmlAudioElement,
    _source: Option<MediaElementAudioSourceNode>,
    gain: Option<GainNode>,
    volume: f64,
    fade: Option<Fade>,
}

impl Voice {
    fn stop(&self) {
        self.element.pause().ok();
        self.element.set_current_time(0.0);
        self.disconnect();
    }

    fn disconnect(&self) {
        if let Some(source) = &self._source {
            source.disconnect().ok();
        }
        if let Some(gain) = &self.gain {
            gain.disconnect().ok();
        }
    }
}

struct State {
    enabled: bool,
    context: Option<AudioContext>,
    voices: Vec<Voice>,
    urls: HashMap<String, String>,
    volumes: [f64; 4],
    next_id: u64,
}

impl State {
    fn channel_index(channel: AudioChannel) -> usize {
        match channel {
            AudioChannel::Bgm => 0,
            AudioChannel::Bgs => 1,
            AudioChannel::Me => 2,
            AudioChannel::Se => 3,
        }
    }

    fn stop_channel(&mut self, channel: AudioChannel) {
        for voice in self.voices.iter().filter(|voice| voice.channel == channel) {
            voice.stop();
        }
        self.voices.retain(|voice| voice.channel != channel);
    }

    fn stop_all(&mut self) {
        for voice in &self.voices {
            voice.stop();
        }
        self.voices.clear();
    }

    fn try_play(&self, element: &HtmlAudioElement) {
        // play() may reject until the page has received a user gesture. Consume
        // the promise rejection; the registered gesture handlers retry it.
        if let Ok(promise) = element.play() {
            Self::consume_promise(promise);
        }
    }

    fn consume_promise(promise: js_sys::Promise) {
        wasm_bindgen_futures::spawn_local(async move {
            let _ = JsFuture::from(promise).await;
        });
    }

    fn gesture(&self) {
        if !self.enabled {
            return;
        }
        if let Some(context) = &self.context {
            if let Ok(promise) = context.resume() {
                Self::consume_promise(promise);
            }
        }
        for voice in &self.voices {
            if voice.element.paused() {
                self.try_play(&voice.element);
            }
        }
    }

    fn set_voice_volume(&self, voice: &Voice) {
        let channel_volume = self.volumes[Self::channel_index(voice.channel)];
        let volume = (voice.volume * channel_volume).clamp(0.0, 1.0);
        if let Some(gain) = &voice.gain {
            gain.gain().set_value(volume as f32);
        } else {
            voice.element.set_volume(volume);
        }
    }
}

/// Audio backend for the browser frontend. It only uses browser APIs through
/// Rust bindings; no JavaScript audio implementation or C++ code is involved.
pub struct BrowserAudio {
    state: Rc<RefCell<State>>,
    window: Option<Window>,
    gesture_handlers: Vec<(&'static str, Closure<dyn FnMut(Event)>)>,
    fade_timer: Option<i32>,
    fade_callback: Option<Closure<dyn FnMut()>>,
}

impl BrowserAudio {
    pub fn new() -> Self {
        let window = web_sys::window();
        let state = Rc::new(RefCell::new(State {
            enabled: window.is_some(),
            context: window.as_ref().and_then(|_| AudioContext::new().ok()),
            voices: Vec::new(),
            urls: HashMap::new(),
            volumes: [1.0; 4],
            next_id: 0,
        }));
        // Browsers block media playback before user activation. Install the
        // listeners from Rust so a pending first sound can start on the first
        // click, tap, or key press without changing the JS page glue.
        let mut gesture_handlers = Vec::new();
        if let Some(window) = &window {
            for event_name in ["pointerdown", "touchstart", "keydown"] {
                let state = Rc::clone(&state);
                let callback = Closure::wrap(Box::new(move |_event: Event| {
                    state.borrow().gesture();
                }) as Box<dyn FnMut(Event)>);
                if window
                    .add_event_listener_with_callback(event_name, callback.as_ref().unchecked_ref())
                    .is_ok()
                {
                    gesture_handlers.push((event_name, callback));
                }
            }
        }

        // Fades progress independently of the game frame pump, including when
        // the Ruby game is temporarily not issuing native calls.
        let mut fade_timer = None;
        let mut fade_callback = None;
        if let Some(window) = &window {
            let weak: Weak<RefCell<State>> = Rc::downgrade(&state);
            let callback = Closure::wrap(Box::new(move || {
                let Some(state) = weak.upgrade() else {
                    return;
                };
                let now = Date::now();
                let mut state = state.borrow_mut();
                let mut finished = Vec::new();
                for voice in &mut state.voices {
                    let Some(fade) = &voice.fade else {
                        continue;
                    };
                    let progress = ((now - fade.started_at_ms) / fade.duration_ms).clamp(0.0, 1.0);
                    let volume = (fade.from * (1.0 - progress)).clamp(0.0, 1.0);
                    if let Some(gain) = &voice.gain {
                        gain.gain().set_value(volume as f32);
                    } else {
                        voice.element.set_volume(volume);
                    }
                    if progress >= 1.0 {
                        voice.stop();
                        finished.push(voice.id);
                    }
                }
                for voice in &state.voices {
                    if voice.element.ended() {
                        voice.disconnect();
                    }
                }
                state
                    .voices
                    .retain(|voice| !voice.element.ended() && !finished.contains(&voice.id));
            }) as Box<dyn FnMut()>);
            if let Ok(id) = window.set_interval_with_callback_and_timeout_and_arguments_0(
                callback.as_ref().unchecked_ref(),
                25,
            ) {
                fade_timer = Some(id);
                fade_callback = Some(callback);
            }
        }

        Self {
            state,
            window,
            gesture_handlers,
            fade_timer,
            fade_callback,
        }
    }

    fn url_for(&mut self, path: &str, bytes: &[u8]) -> Option<String> {
        let mut state = self.state.borrow_mut();
        if let Some(url) = state.urls.get(path) {
            return Some(url.clone());
        }
        let parts = Array::new();
        parts.push(&Uint8Array::from(bytes).into());
        let options = BlobPropertyBag::new();
        let mime = match path
            .rsplit('.')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "mp3" => "audio/mpeg",
            "ogg" | "oga" => "audio/ogg",
            "wav" => "audio/wav",
            "m4a" => "audio/mp4",
            "flac" => "audio/flac",
            _ => "application/octet-stream",
        };
        options.set_type(mime);
        let blob = Blob::new_with_u8_array_sequence_and_options(&parts, &options).ok()?;
        let url = Url::create_object_url_with_blob(&blob).ok()?;
        state.urls.insert(path.to_owned(), url.clone());
        Some(url)
    }

    fn start_voice(
        &mut self,
        channel: AudioChannel,
        path: &str,
        bytes: &[u8],
        volume: i32,
        pitch: f64,
        position: f64,
    ) {
        if !self.state.borrow().enabled {
            return;
        }
        let Some(url) = self.url_for(path, bytes) else {
            eprintln!("audio: could not create media URL for {path}");
            return;
        };
        let element = match HtmlAudioElement::new_with_src(&url) {
            Ok(element) => element,
            Err(error) => {
                eprintln!("audio: could not create media element for {path}: {error:?}");
                return;
            }
        };
        element.set_preload("auto");
        element.set_loop(matches!(channel, AudioChannel::Bgm | AudioChannel::Bgs));
        element.set_playback_rate((pitch / 100.0).clamp(0.01, 10.0));
        if position.is_finite() && position > 0.0 {
            element.set_current_time(position);
        }

        let context = self.state.borrow().context.clone();
        let (source, gain) = if let Some(context) = context {
            let graph = (|| {
                let source = context.create_media_element_source(&element).ok()?;
                let gain = context.create_gain().ok()?;
                source.connect_with_audio_node(&gain).ok()?;
                gain.connect_with_audio_node(&context.destination()).ok()?;
                Some((source, gain))
            })();
            match graph {
                Some((source, gain)) => (Some(source), Some(gain)),
                None => {
                    eprintln!("audio: could not connect media graph for {path}");
                    (None, None)
                }
            }
        } else {
            (None, None)
        };

        let mut state = self.state.borrow_mut();
        if channel != AudioChannel::Se {
            state.stop_channel(channel);
        }
        let id = state.next_id;
        state.next_id = state.next_id.wrapping_add(1);
        let voice = Voice {
            id,
            channel,
            element,
            _source: source,
            gain,
            volume: f64::from(volume.clamp(0, 100)) / 100.0,
            fade: None,
        };
        state.set_voice_volume(&voice);
        state.try_play(&voice.element);
        state.voices.push(voice);
    }
}

impl Default for BrowserAudio {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioBackend for BrowserAudio {
    fn play(
        &mut self,
        channel: AudioChannel,
        path: &str,
        bytes: &[u8],
        volume: i32,
        pitch: f64,
        position: f64,
    ) {
        self.start_voice(channel, path, bytes, volume, pitch, position);
    }

    fn stop(&mut self, channel: AudioChannel) {
        self.state.borrow_mut().stop_channel(channel);
    }

    fn fade(&mut self, channel: AudioChannel, ms: i32) {
        let mut state = self.state.borrow_mut();
        let ms = ms.max(0) as f64;
        if ms == 0.0 {
            state.stop_channel(channel);
            return;
        }
        let now = Date::now();
        for voice in state
            .voices
            .iter_mut()
            .filter(|voice| voice.channel == channel)
        {
            let from = voice
                .gain
                .as_ref()
                .map(|gain| f64::from(gain.gain().value()))
                .unwrap_or_else(|| voice.element.volume());
            if let Some(gain) = &voice.gain {
                gain.gain().set_value(from as f32);
            } else {
                voice.element.set_volume(from);
            }
            voice.fade = Some(Fade {
                from,
                started_at_ms: now,
                duration_ms: ms,
            });
        }
    }

    fn position(&self, channel: AudioChannel) -> f64 {
        if channel != AudioChannel::Bgm {
            return 0.0;
        }
        self.state
            .borrow()
            .voices
            .iter()
            .find(|voice| voice.channel == channel)
            .map(|voice| voice.element.current_time())
            .unwrap_or(0.0)
    }

    fn set_volume(&mut self, channel: AudioChannel, volume: i32) {
        let mut state = self.state.borrow_mut();
        let index = State::channel_index(channel);
        state.volumes[index] = f64::from(volume.clamp(0, 100)) / 100.0;
        let master = state.volumes[index];
        for voice in state
            .voices
            .iter_mut()
            .filter(|voice| voice.channel == channel && voice.fade.is_none())
        {
            let value = (voice.volume * master).clamp(0.0, 1.0);
            if let Some(gain) = &voice.gain {
                gain.gain().set_value(value as f32);
            } else {
                voice.element.set_volume(value);
            }
        }
    }

    fn volume(&self, channel: AudioChannel) -> i32 {
        (self.state.borrow().volumes[State::channel_index(channel)] * 100.0).round() as i32
    }
}

impl Drop for BrowserAudio {
    fn drop(&mut self) {
        if let Some(window) = &self.window {
            for (event_name, callback) in &self.gesture_handlers {
                let _ = window.remove_event_listener_with_callback(
                    event_name,
                    callback.as_ref().unchecked_ref(),
                );
            }
            if let Some(timer) = self.fade_timer {
                window.clear_interval_with_handle(timer);
            }
        }
        // Keep the callback alive until its interval is cancelled above.
        let _ = self.fade_callback.take();

        let mut state = self.state.borrow_mut();
        state.stop_all();
        for url in state.urls.values() {
            let _ = Url::revoke_object_url(url);
        }
        state.urls.clear();
        if let Some(context) = state.context.take() {
            if let Ok(promise) = context.close() {
                State::consume_promise(promise);
            }
        }
    }
}
