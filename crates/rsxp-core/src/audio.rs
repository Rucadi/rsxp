// SPDX-License-Identifier: GPL-2.0-or-later
//! Audio output for both sample-based emulators and asset-driven engines.
//!
//! GBA/GB/DS cores submit interleaved `i16` PCM. RGSS submits decoded asset
//! streams with playback controls. Both use the same sink so the frontend owns
//! the device, mixer, volume and pause behavior.

/// Logical destination for a decoded stream. Cores may use the named channels
/// or any other index; the frontend treats unknown channels as effect tracks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AudioChannel(pub u8);

impl AudioChannel {
    pub const MUSIC: Self = Self(0);
    pub const AMBIENT: Self = Self(1);
    pub const VOICE: Self = Self(2);
    pub const EFFECT: Self = Self(3);
    pub const MASTER: Self = Self(4);
}

/// An encoded/decoded sound buffer owned by the core for the duration of the
/// call. `path` is a stable identity for caching; `data` is the encoded asset
/// accepted by the frontend's decoder.
pub struct AudioStream<'a> {
    pub channel: AudioChannel,
    pub path: &'a str,
    pub data: &'a [u8],
    pub looping: bool,
    /// 0..=100, combined with the frontend's channel master.
    pub volume: i32,
    /// 100 is normal speed.
    pub pitch: f64,
    /// Start position in seconds.
    pub position: f64,
}

/// Interleaved signed 16-bit PCM from a sample-based core.
pub struct PcmBuffer<'a> {
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: &'a [i16],
}

impl PcmBuffer<'_> {
    pub fn frames(&self) -> usize {
        let channels = self.channels.max(1) as usize;
        self.samples.len() / channels
    }
}

/// Audio endpoint supplied by the frontend.
///
/// `position` and `volume` are queries, so the core can keep RGSS semantics
/// without an asynchronous reply. Calls occur at asset/command frequency, not
/// per sample.
pub trait AudioSink: Send {
    fn play_stream(&mut self, _stream: AudioStream<'_>) {}
    fn stop(&mut self, _channel: AudioChannel) {}
    fn fade(&mut self, _channel: AudioChannel, _milliseconds: i32) {}
    fn position(&self, _channel: AudioChannel) -> f64 {
        0.0
    }
    fn set_volume(&mut self, _channel: AudioChannel, _volume: i32) {}
    fn volume(&self, _channel: AudioChannel) -> i32 {
        100
    }

    /// Queues interleaved PCM. Implementations should use a bounded ring buffer
    /// and return promptly; they may drop the oldest samples under sustained
    /// overrun rather than blocking the emulation thread.
    fn queue_pcm(&mut self, _pcm: PcmBuffer<'_>) {}

    /// Pauses or resumes the whole output without destroying stream state.
    fn pause(&mut self, _paused: bool) {}
}

/// Silent sink for headless runs.
#[derive(Default)]
pub struct NullAudioSink;

impl AudioSink for NullAudioSink {}
