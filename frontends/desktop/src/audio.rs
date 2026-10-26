// SPDX-License-Identifier: GPL-2.0-or-later
//! Pure-Rust desktop audio backend built on Kira over CPAL.
//!
//! [`DesktopAudio`] is the concrete [`fanplay_rgss::host::AudioBackend`] the desktop
//! host installs to service the engine's four RGSS audio channels
//! ([`AudioChannel::Bgm`], `Bgs`, `Me`, `Se`), which are indexed here as
//! `[Bgm, Bgs, Me, Se]` for per-channel master volumes.
//!
//! Design and lifecycle:
//! - [`DesktopAudio::new`] opens a default Kira/CPAL [`AudioManager`]; if no
//!   device is available it prints a warning and degrades to no sound instead
//!   of failing, so the game still runs. [`DesktopAudio::silent`] skips the
//!   manager outright and is used for headless runs. Both are non-fatal.
//! - BGM and BGS are decoded as `StreamingSoundData` and looped
//!   (`loop_region(..)`) so long music tracks do not have to fit in memory;
//!   ME and SE use `StaticSoundData`, cached by source path in `static_sounds`
//!   so repeated effects are decoded once. Each active sound is tracked as a
//!   [`Voice`] wrapping either handle in a [`VoiceHandle`].
//! - Volume is applied in decibels. A play call's volume is combined with the
//!   channel master via [`DesktopAudio::effective_volume`]; zero gain maps to
//!   [`Decibels::SILENCE`]. Pitch is expressed as RGSS percent and clamped to
//!   a `0.01..=10.0` playback rate.
//! - Playing on BGM/BGS/ME first stops the previous voice on that channel;
//!   SE voices accumulate. Finished SE handles are reaped each `play` call so
//!   a long session does not leak. `fade` stops with a tween (removing the
//!   voice only when it is not fading); `position` reports the playback
//!   position for the current non-fading BGM voice and `0.0` otherwise.
use kira::{
    sound::{
        static_sound::{StaticSoundData, StaticSoundHandle},
        streaming::{StreamingSoundData, StreamingSoundHandle},
        FromFileError, PlaybackState,
    },
    AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Tween,
};
use fanplay_rgss::host::{AudioBackend, AudioChannel};
use std::{collections::HashMap, io::Cursor, time::Duration};

enum VoiceHandle {
    Static(StaticSoundHandle),
    Streaming(StreamingSoundHandle<FromFileError>),
}

impl VoiceHandle {
    fn position(&self) -> f64 {
        match self {
            Self::Static(handle) => handle.position(),
            Self::Streaming(handle) => handle.position(),
        }
    }

    fn state(&self) -> PlaybackState {
        match self {
            Self::Static(handle) => handle.state(),
            Self::Streaming(handle) => handle.state(),
        }
    }

    fn set_volume(&mut self, volume: Decibels) {
        match self {
            Self::Static(handle) => handle.set_volume(volume, Tween::default()),
            Self::Streaming(handle) => handle.set_volume(volume, Tween::default()),
        }
    }

    fn stop(&mut self, tween: Tween) {
        match self {
            Self::Static(handle) => handle.stop(tween),
            Self::Streaming(handle) => handle.stop(tween),
        }
    }
}

struct Voice {
    channel: AudioChannel,
    volume: f64,
    stopping: bool,
    handle: VoiceHandle,
}

pub struct DesktopAudio {
    manager: Option<AudioManager<DefaultBackend>>,
    static_sounds: HashMap<String, StaticSoundData>,
    voices: Vec<Voice>,
    volumes: [i32; 4],
}

impl DesktopAudio {
    pub fn new() -> Self {
        Self::with_manager(true)
    }

    pub fn silent() -> Self {
        Self::with_manager(false)
    }

    fn with_manager(enabled: bool) -> Self {
        let manager = if enabled {
            match AudioManager::<DefaultBackend>::new(AudioManagerSettings::default()) {
                Ok(manager) => Some(manager),
                Err(error) => {
                    eprintln!("audio unavailable ({error}); continuing without sound");
                    None
                }
            }
        } else {
            None
        };
        Self {
            manager,
            static_sounds: HashMap::new(),
            voices: Vec::new(),
            volumes: [100; 4],
        }
    }

    fn channel_index(channel: AudioChannel) -> usize {
        match channel {
            AudioChannel::Bgm => 0,
            AudioChannel::Bgs => 1,
            AudioChannel::Me => 2,
            AudioChannel::Se => 3,
        }
    }

    fn effective_volume(&self, channel: AudioChannel, play_volume: i32) -> Decibels {
        let gain = f64::from(play_volume.clamp(0, 100))
            * f64::from(self.volumes[Self::channel_index(channel)])
            / 10_000.0;
        if gain <= 0.0 {
            Decibels::SILENCE
        } else {
            Decibels::from((20.0 * gain.log10()) as f32)
        }
    }

    fn static_sound(&mut self, path: &str, bytes: &[u8]) -> Result<StaticSoundData, String> {
        if let Some(sound) = self.static_sounds.get(path) {
            return Ok(sound.clone());
        }
        let sound = StaticSoundData::from_cursor(Cursor::new(bytes.to_vec()))
            .map_err(|error| format!("could not decode {path}: {error:?}"))?;
        self.static_sounds.insert(path.to_owned(), sound.clone());
        Ok(sound)
    }

    fn stop_channel(&mut self, channel: AudioChannel, tween: Tween, fading: bool) {
        for voice in self
            .voices
            .iter_mut()
            .filter(|voice| voice.channel == channel)
        {
            voice.handle.stop(tween);
            voice.stopping = fading;
        }
        if !fading {
            self.voices.retain(|voice| voice.channel != channel);
        }
    }

    fn play_voice(
        &mut self,
        channel: AudioChannel,
        path: &str,
        bytes: &[u8],
        volume: i32,
        pitch: f64,
        position: f64,
    ) {
        if self.manager.is_none() {
            return;
        }
        let volume_db = self.effective_volume(channel, volume);
        let rate = (pitch / 100.0).clamp(0.01, 10.0);
        let position = position.max(0.0);
        let handle = if matches!(channel, AudioChannel::Bgm | AudioChannel::Bgs) {
            let mut sound = match StreamingSoundData::from_cursor(Cursor::new(bytes.to_vec())) {
                Ok(sound) => sound,
                Err(error) => {
                    eprintln!("could not decode music {path}: {error:?}");
                    return;
                }
            }
            .volume(volume_db)
            .playback_rate(rate);
            // Avoid seeking to zero explicitly. Some Vorbis streams begin at
            // a non-zero timestamp, and Symphonia can reject that seek even
            // though normal playback from the stream start is valid.
            if position > 0.0 {
                sound = sound.start_position(position);
            }
            sound = sound.loop_region(..);
            match self.manager.as_mut().unwrap().play(sound) {
                Ok(handle) => VoiceHandle::Streaming(handle),
                Err(error) => {
                    eprintln!("could not play music {path}: {error:?}");
                    return;
                }
            }
        } else {
            let sound = match self.static_sound(path, bytes) {
                Ok(sound) => sound,
                Err(error) => {
                    eprintln!("{error}");
                    return;
                }
            }
            .volume(volume_db)
            .playback_rate(rate)
            .start_position(position);
            match self.manager.as_mut().unwrap().play(sound) {
                Ok(handle) => VoiceHandle::Static(handle),
                Err(error) => {
                    eprintln!("could not play sound {path}: {error:?}");
                    return;
                }
            }
        };
        self.voices.push(Voice {
            channel,
            volume: f64::from(volume.clamp(0, 100)) / 100.0,
            stopping: false,
            handle,
        });
    }
}

impl Default for DesktopAudio {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioBackend for DesktopAudio {
    fn play(
        &mut self,
        channel: AudioChannel,
        path: &str,
        bytes: &[u8],
        volume: i32,
        pitch: f64,
        position: f64,
    ) {
        if channel != AudioChannel::Se {
            self.stop_channel(channel, Tween::default(), false);
        }
        // Drop completed SE handles as a long-running game plays sound effects.
        self.voices
            .retain(|voice| voice.handle.state() != PlaybackState::Stopped);
        self.play_voice(channel, path, bytes, volume, pitch, position);
    }

    fn stop(&mut self, channel: AudioChannel) {
        self.stop_channel(channel, Tween::default(), false);
    }

    fn fade(&mut self, channel: AudioChannel, ms: i32) {
        let tween = Tween {
            duration: Duration::from_millis(ms.max(0) as u64),
            ..Tween::default()
        };
        self.stop_channel(channel, tween, ms > 0);
    }

    fn position(&self, channel: AudioChannel) -> f64 {
        if channel != AudioChannel::Bgm {
            return 0.0;
        }
        self.voices
            .iter()
            .find(|voice| voice.channel == channel && !voice.stopping)
            .map(|voice| voice.handle.position())
            .unwrap_or(0.0)
    }

    fn set_volume(&mut self, channel: AudioChannel, volume: i32) {
        self.volumes[Self::channel_index(channel)] = volume.clamp(0, 100);
        let master = f64::from(self.volumes[Self::channel_index(channel)]) / 100.0;
        let voices = &mut self.voices;
        for voice in voices.iter_mut().filter(|voice| voice.channel == channel) {
            if voice.stopping {
                continue;
            }
            let gain = voice.volume * master;
            let volume_db = if gain <= 0.0 {
                Decibels::SILENCE
            } else {
                Decibels::from((20.0 * gain.log10()) as f32)
            };
            voice.handle.set_volume(volume_db);
        }
    }

    fn volume(&self, channel: AudioChannel) -> i32 {
        self.volumes[Self::channel_index(channel)]
    }
}
