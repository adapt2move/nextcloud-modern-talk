//! Bounded mono PCM accumulator for a deliberately opened microphone stream.
//! The audio callback uses a nonblocking lock; contention is a recording failure,
//! never silent data loss. This module performs no I/O or device discovery.
use cpal::{FromSample, Sample};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU16, AtomicU32, Ordering},
};

pub(super) const RATE: u32 = 16_000;
pub(super) const MAX_SECONDS: usize = 120;
const MAX_SAMPLES: usize = RATE as usize * MAX_SECONDS;

pub(super) struct Recording {
    samples: Mutex<Vec<i16>>,
    failed: AtomicBool,
    full: AtomicBool,
    level: AtomicU16,
    count: AtomicU32,
}
#[derive(Default)]
pub(super) struct Resampler {
    phase: u32,
    sum: f32,
    count: u32,
}
impl Recording {
    pub(super) fn new() -> Self {
        Self {
            samples: Mutex::new(Vec::with_capacity(MAX_SAMPLES)),
            failed: AtomicBool::new(false),
            full: AtomicBool::new(false),
            level: AtomicU16::new(0),
            count: AtomicU32::new(0),
        }
    }
    pub(super) fn push<T: Sample>(
        &self,
        frames: &[T],
        channels: usize,
        rate: u32,
        resampler: &mut Resampler,
    ) where
        f32: FromSample<T>,
    {
        if channels == 0 || rate < RATE || frames.len() > 65_536 || frames.len() % channels != 0 {
            self.failed.store(true, Ordering::Release);
            return;
        }
        let Ok(mut samples) = self.samples.try_lock() else {
            self.failed.store(true, Ordering::Release);
            return;
        };
        let mut energy = 0.0f64;
        let mut count = 0u32;
        for frame in frames.chunks_exact(channels) {
            let value = frame
                .iter()
                .map(|s| {
                    let s = f32::from_sample(*s);
                    if s.is_finite() {
                        s.clamp(-1.0, 1.0)
                    } else {
                        0.0
                    }
                })
                .sum::<f32>()
                / channels as f32;
            energy += f64::from(value) * f64::from(value);
            count += 1;
            resampler.sum += value;
            resampler.count += 1;
            resampler.phase += RATE;
            if resampler.phase >= rate {
                resampler.phase -= rate;
                if samples.len() == MAX_SAMPLES {
                    self.full.store(true, Ordering::Release);
                    break;
                }
                samples.push(
                    (resampler.sum / resampler.count as f32 * f32::from(i16::MAX)).round() as i16,
                );
                resampler.sum = 0.0;
                resampler.count = 0;
            }
        }
        let rms = if count == 0 {
            0.0
        } else {
            (energy / f64::from(count)).sqrt()
        };
        let level = if rms <= 0.001 {
            0
        } else {
            ((20.0 * rms.log10() + 60.0) / 60.0 * 1000.0).clamp(0.0, 1000.0) as u16
        };
        self.level.store(level, Ordering::Relaxed);
        self.count.store(samples.len() as u32, Ordering::Relaxed);
    }
    pub(super) fn failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }
    pub(super) fn full(&self) -> bool {
        self.full.load(Ordering::Acquire)
    }
    pub(super) fn level(&self) -> u16 {
        self.level.load(Ordering::Relaxed)
    }
    pub(super) fn snapshot(&self) -> (u64, u16) {
        (
            u64::from(self.count.load(Ordering::Relaxed)) * 1000 / u64::from(RATE),
            self.level(),
        )
    }
    /// Call only after stream disposal, so no callback can retain or add samples.
    pub(super) fn wave(&self) -> Option<Vec<u8>> {
        if self.failed() {
            return None;
        }
        let samples = self.samples.lock().ok()?;
        if samples.is_empty() {
            return None;
        }
        let data_len = u32::try_from(samples.len().checked_mul(2)?).ok()?;
        let mut out = Vec::with_capacity(44 + data_len as usize);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_len).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&RATE.to_le_bytes());
        out.extend_from_slice(&(RATE * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        for sample in samples.iter() {
            out.extend_from_slice(&sample.to_le_bytes());
        }
        Some(out)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wave_is_bounded_mono_pcm_and_length_matches() {
        let recording = Recording::new();
        let mut resampler = Resampler::default();
        recording.push(&vec![0.5; 960], 2, 48_000, &mut resampler);
        let wave = recording.wave().unwrap();
        assert_eq!(&wave[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(wave[40..44].try_into().unwrap()), 320);
        assert_eq!(wave.len(), 364);
        assert_eq!(recording.snapshot().0, 10);
    }
    #[test]
    fn malformed_audio_fails_without_publishing_partial_wave() {
        let recording = Recording::new();
        recording.push(&[0.2], 2, 48_000, &mut Resampler::default());
        assert!(recording.failed());
        assert!(recording.wave().is_none());
    }
}
