// SPDX-License-Identifier: GPL-2.0-or-later
//! Audio output for both sample-based emulators and asset-driven engines.
//!
//! GBA/GB/DS cores submit interleaved `i16` PCM. RGSS submits decoded asset
//! streams with playback controls. Both use the same sink so the frontend owns
//! the device, mixer, volume and pause behavior.
//!
//! [`PcmRing`] provides a preallocated, bounded single-producer/single-consumer
//! buffer for the PCM path. Producers never allocate or lock, and a full ring
//! drops the incoming block and records an overrun instead of blocking the
//! emulation thread.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

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

/// Output format negotiated for a session before the core starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub channels: u16,
}

impl AudioConfig {
    pub const fn new(sample_rate: u32, channels: u16) -> Self {
        Self {
            sample_rate,
            channels,
        }
    }

    pub const fn silence() -> Self {
        Self::new(0, 0)
    }

    pub fn is_active(self) -> bool {
        self.sample_rate > 0 && self.channels > 0
    }
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

/// Queue-depth and overload feedback for the PCM path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PcmStatus {
    pub capacity_frames: usize,
    pub queued_frames: usize,
    pub underruns: u64,
    pub overruns: u64,
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

    /// Format the device is actually consuming, if it is known. Cores use this
    /// to configure resampling before the first frame instead of mutating
    /// process environment variables.
    fn output_config(&self) -> Option<AudioConfig> {
        None
    }

    /// Drops queued PCM and resets discontinuity counters. Called after a
    /// reset, a state restore or a long pause.
    fn reset_pcm(&mut self) {}

    /// Current queue depth and overload counters. Cheap enough for a cold-path
    /// status display.
    fn pcm_status(&self) -> PcmStatus {
        PcmStatus::default()
    }
}

/// Silent sink for headless runs.
#[derive(Default)]
pub struct NullAudioSink;

impl AudioSink for NullAudioSink {}

struct RingShared {
    capacity: usize,
    buffer: Box<[UnsafeCell<i16>]>,
    /// Next sample the consumer will read. Only the consumer advances it.
    head: AtomicUsize,
    /// Next sample the producer will write. Only the producer advances it.
    tail: AtomicUsize,
    underruns: AtomicU64,
    overruns: AtomicU64,
    overrun_pending: AtomicBool,
}

// SAFETY: the ring is single-producer/single-consumer. The producer only writes
// slots in `[tail, head + capacity - 1)` and the consumer only reads slots in
// `[head, tail)`, so a slot is never accessed by both sides at once. The atomic
// indices establish the required happens-before relation.
unsafe impl Sync for RingShared {}
unsafe impl Send for RingShared {}

/// Builds a bounded SPSC PCM ring. `capacity_samples` is rounded up; one slot
/// is reserved to distinguish full from empty.
pub struct PcmRing;

impl PcmRing {
    pub fn new(capacity_samples: usize) -> (PcmRingProducer, PcmRingConsumer) {
        let capacity = capacity_samples.max(1) + 1;
        let shared = Arc::new(RingShared {
            capacity,
            buffer: (0..capacity).map(|_| UnsafeCell::new(0)).collect(),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            underruns: AtomicU64::new(0),
            overruns: AtomicU64::new(0),
            overrun_pending: AtomicBool::new(false),
        });
        (
            PcmRingProducer(Arc::clone(&shared)),
            PcmRingConsumer(shared),
        )
    }
}

#[derive(Clone)]
pub struct PcmRingProducer(Arc<RingShared>);

impl PcmRingProducer {
    /// Pushes samples, returning how many were accepted.
    ///
    /// When the ring is full the incoming block is dropped and the overrun
    /// counter is incremented; the caller is never blocked.
    pub fn push(&self, samples: &[i16]) -> usize {
        let ring = &self.0;
        let head = ring.head.load(Ordering::Acquire);
        let tail = ring.tail.load(Ordering::Relaxed);
        let used = tail.wrapping_sub(head);
        let free = (ring.capacity - 1).saturating_sub(used);
        if free == 0 || samples.is_empty() {
            if !samples.is_empty() {
                ring.overruns.fetch_add(1, Ordering::Relaxed);
                ring.overrun_pending.store(true, Ordering::Release);
            }
            return 0;
        }
        let count = samples.len().min(free);
        if count < samples.len() {
            ring.overruns.fetch_add(1, Ordering::Relaxed);
            ring.overrun_pending.store(true, Ordering::Release);
        }
        for (offset, sample) in samples[..count].iter().enumerate() {
            // SAFETY: the slot is in the producer-owned region and no consumer
            // can observe it until `tail` is published below.
            unsafe {
                *ring.buffer[(tail.wrapping_add(offset)) % ring.capacity].get() = *sample;
            }
        }
        ring.tail.store(tail.wrapping_add(count), Ordering::Release);
        count
    }

    pub fn capacity_samples(&self) -> usize {
        self.0.capacity - 1
    }
}

pub struct PcmRingConsumer(Arc<RingShared>);

impl PcmRingConsumer {
    /// Pops up to `out.len()` samples.
    pub fn pop(&self, out: &mut [i16]) -> usize {
        let ring = &self.0;
        let head = ring.head.load(Ordering::Relaxed);
        let tail = ring.tail.load(Ordering::Acquire);
        let available = tail.wrapping_sub(head);
        if available == 0 {
            return 0;
        }
        let count = out.len().min(available);
        for (offset, sample) in out[..count].iter_mut().enumerate() {
            // SAFETY: the slot is in the consumer-owned region and the producer
            // cannot reuse it until `head` is published below.
            *sample = unsafe { *ring.buffer[(head.wrapping_add(offset)) % ring.capacity].get() };
        }
        ring.head.store(head.wrapping_add(count), Ordering::Release);
        count
    }

    /// Fills `out` from the ring, padding an underrun with silence.
    pub fn pop_fill(&self, out: &mut [i16]) -> usize {
        let count = self.pop(out);
        if count < out.len() {
            out[count..].fill(0);
            self.0.underruns.fetch_add(1, Ordering::Relaxed);
        }
        // Discard stale audio after this callback has consumed its current
        // block. Otherwise a transient overrun can become a persistent delay.
        if self.0.overrun_pending.swap(false, Ordering::Acquire) {
            let tail = self.0.tail.load(Ordering::Acquire);
            self.0.head.store(tail, Ordering::Release);
        }
        count
    }

    /// Discards queued samples. Called after a reset or state load.
    pub fn reset(&self) {
        let ring = &self.0;
        let tail = ring.tail.load(Ordering::Acquire);
        ring.head.store(tail, Ordering::Release);
        ring.overrun_pending.store(false, Ordering::Release);
    }

    pub fn available_samples(&self) -> usize {
        let ring = &self.0;
        ring.tail
            .load(Ordering::Acquire)
            .wrapping_sub(ring.head.load(Ordering::Relaxed))
    }

    pub fn status(&self, channels: u16) -> PcmStatus {
        let channels = usize::from(channels.max(1));
        PcmStatus {
            capacity_frames: (self.0.capacity - 1) / channels,
            queued_frames: self.available_samples() / channels,
            underruns: self.0.underruns.load(Ordering::Relaxed),
            overruns: self.0.overruns.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_wraps_and_reports_overruns() {
        let (producer, consumer) = PcmRing::new(8);
        assert_eq!(producer.capacity_samples(), 8);
        assert_eq!(producer.push(&[1, 2, 3, 4, 5, 6, 7, 8, 9]), 8);
        assert_eq!(producer.push(&[10]), 0);
        assert_eq!(consumer.status(1).overruns, 2);
        let mut out = [0; 4];
        assert_eq!(consumer.pop(&mut out), 4);
        assert_eq!(out, [1, 2, 3, 4]);
        assert_eq!(producer.push(&[10, 11]), 2);
        let mut out = [0; 8];
        assert_eq!(consumer.pop(&mut out), 6);
        assert_eq!(&out[..6], &[5, 6, 7, 8, 10, 11]);
        consumer.reset();
        assert_eq!(consumer.available_samples(), 0);
    }

    #[test]
    fn underrun_pads_with_silence() {
        let (producer, consumer) = PcmRing::new(4);
        producer.push(&[7, 8]);
        let mut out = [1; 4];
        assert_eq!(consumer.pop_fill(&mut out), 2);
        assert_eq!(out, [7, 8, 0, 0]);
        assert_eq!(consumer.status(1).underruns, 1);
    }

    #[test]
    fn overrun_discards_stale_audio_after_the_current_callback() {
        let (producer, consumer) = PcmRing::new(4);
        producer.push(&[1, 2, 3, 4]);
        assert_eq!(producer.push(&[5]), 0);

        let mut out = [0; 2];
        assert_eq!(consumer.pop_fill(&mut out), 2);
        assert_eq!(out, [1, 2]);
        assert_eq!(consumer.available_samples(), 0);
        assert_eq!(consumer.status(1).overruns, 1);
    }
}
