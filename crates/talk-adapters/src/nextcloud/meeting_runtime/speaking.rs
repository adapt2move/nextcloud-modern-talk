//! A short-lived indication derived from already received, decoded audio.
//! This is not speech recognition or an estimate of connection quality.
use std::time::{Duration, Instant};

const HOLD: Duration = Duration::from_millis(600);
const MEAN_ABSOLUTE_THRESHOLD: u64 = 800;

#[derive(Default)]
pub(super) struct Speaking(Option<Instant>);

impl Speaking {
    pub(super) fn observe(&mut self, samples: &[i16], now: Instant) {
        if samples.len() != 480 {
            return;
        }
        let sum: u64 = samples
            .iter()
            .map(|sample| u64::from(sample.unsigned_abs()))
            .sum();
        if sum >= MEAN_ABSOLUTE_THRESHOLD * samples.len() as u64 {
            self.0 = Some(now);
        }
    }
    pub(super) fn active(&self, now: Instant) -> bool {
        self.0
            .is_some_and(|last| now.saturating_duration_since(last) < HOLD)
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
        assert!(!speaking.active(now));
        speaking.observe(&[900; 479], now);
        assert!(!speaking.active(now));
        speaking.observe(&[900; 480], now);
        assert!(speaking.active(now + Duration::from_millis(599)));
        assert!(!speaking.active(now + HOLD));
    }
}
