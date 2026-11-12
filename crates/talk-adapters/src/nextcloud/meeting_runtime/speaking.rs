//! A short-lived indication derived from already received, decoded audio.
//! This is not speech recognition or an estimate of connection quality.
use std::time::{Duration, Instant};
use talk_core::calls::SPEAKING_LEVELS;

const HOLD: Duration = Duration::from_millis(600);
/// Mean absolute sample value at which each loudness step begins, for 16-bit
/// audio without gain control: quiet speech into a laptop microphone, an
/// ordinary voice, a raised one and a loud one. Room noise stays below the first.
const STEPS: [u64; SPEAKING_LEVELS as usize] = [250, 800, 2200, 5500];
/// A single frame this quiet may be a click; a voice lasts longer.
const QUIET_FRAMES: u8 = 3;

/// The loudness follows a rise at once and falls by a sixteenth per 10 ms
/// frame, so the indication does not flicker between the syllables of a word.
#[derive(Default)]
pub(super) struct Speaking {
    envelope: u64,
    quiet_run: u8,
    heard: Option<Instant>,
}

impl Speaking {
    pub(super) fn observe(&mut self, samples: &[i16], now: Instant) {
        if samples.len() != 480 {
            return;
        }
        let sum: u64 = samples
            .iter()
            .map(|sample| u64::from(sample.unsigned_abs()))
            .sum();
        let mean = sum / samples.len() as u64;
        self.envelope = mean.max(self.envelope - self.envelope / 16);
        if mean < STEPS[0] {
            self.quiet_run = 0;
            return;
        }
        self.quiet_run = self.quiet_run.saturating_add(1);
        if mean >= STEPS[1] || self.quiet_run >= QUIET_FRAMES || self.level(now) > 0 {
            self.heard = Some(now);
        }
    }
    /// 0 while nothing is heard, otherwise 1 to [`SPEAKING_LEVELS`].
    pub(super) fn level(&self, now: Instant) -> u8 {
        if !self
            .heard
            .is_some_and(|last| now.saturating_duration_since(last) < HOLD)
        {
            return 0;
        }
        // Held after the last sound: the lowest step until the hold ends.
        STEPS
            .iter()
            .filter(|step| self.envelope >= **step)
            .count()
            .max(1) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn speaking_requires_audio_and_expires_without_new_samples() {
        let now = Instant::now();
        let mut speaking = Speaking::default();
        speaking.observe(&[0; 480], now);
        assert_eq!(speaking.level(now), 0);
        speaking.observe(&[900; 479], now);
        assert_eq!(speaking.level(now), 0);
        speaking.observe(&[900; 480], now);
        assert!(speaking.level(now + Duration::from_millis(599)) > 0);
        assert_eq!(speaking.level(now + HOLD), 0);
    }

    #[test]
    fn the_level_rises_with_the_voice_and_falls_back_to_silence() {
        let start = Instant::now();
        let frame = |index: u64| start + Duration::from_millis(10 * index);
        let mut speaking = Speaking::default();
        // Room noise never shows.
        for index in 0..100 {
            speaking.observe(&[120; 480], frame(index));
        }
        assert_eq!(speaking.level(frame(100)), 0);
        // One quiet frame may be a click; a quiet voice shows after three.
        speaking.observe(&[300; 480], frame(100));
        assert_eq!(speaking.level(frame(100)), 0);
        speaking.observe(&[300; 480], frame(101));
        speaking.observe(&[300; 480], frame(102));
        assert_eq!(speaking.level(frame(102)), 1);
        // Louder speech raises the level at once, step by step.
        let mut index = 103;
        for (amplitude, level) in [(900, 2), (2500, 3), (6000, 4)] {
            speaking.observe(&[amplitude; 480], frame(index));
            assert_eq!(speaking.level(frame(index)), level);
            index += 1;
        }
        // A quieter voice lowers it again within a moment, not within a frame.
        speaking.observe(&[300; 480], frame(index));
        assert_eq!(speaking.level(frame(index)), 4);
        for _ in 0..60 {
            index += 1;
            speaking.observe(&[300; 480], frame(index));
        }
        assert_eq!(speaking.level(frame(index)), 1);
        // Silence ends the indication after the hold.
        for _ in 0..59 {
            index += 1;
            speaking.observe(&[0; 480], frame(index));
        }
        assert_eq!(speaking.level(frame(index)), 1);
        speaking.observe(&[0; 480], frame(index + 2));
        assert_eq!(speaking.level(frame(index + 2)), 0);
    }
}
