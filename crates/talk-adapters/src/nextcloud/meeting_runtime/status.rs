//! Current media state, never a replay of commands or a publication grant.
use std::time::{Duration, Instant};
use talk_core::calls::MediaWishes;

const SFU_REFRESH: Duration = Duration::from_secs(5);
const RETRY_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Default)]
pub(super) struct StatusSnapshot {
    accepted: Option<(MediaWishes, Instant)>,
    attempted: Option<Instant>,
}

impl StatusSnapshot {
    /// Janus fans publisher data out only to subscribers already connected when
    /// it arrives. Its local channel acceptance does not acknowledge delivery to
    /// a later subscriber, and Talk's protocol has no publisher-facing readiness
    /// event. Refresh the latest pair on an SFU publisher; direct peers need only
    /// changes. The caller enables refresh only while SFU policy is current.
    /// The existing call loop owns this clock: no task, queue, catch-up burst or
    /// state survives the peer. Permission and publication remain with the
    /// enclosing call owner.
    pub(super) fn attempt(&mut self, now: Instant, wishes: MediaWishes, refresh: bool) -> bool {
        if self
            .attempted
            .is_some_and(|last| now.saturating_duration_since(last) < RETRY_INTERVAL)
        {
            return false;
        }
        let due = match self.accepted {
            None => true,
            Some((previous, at)) => {
                previous != wishes || (refresh && now.saturating_duration_since(at) >= SFU_REFRESH)
            }
        };
        if due {
            self.attempted = Some(now);
            // A partial pair may change the remote state even if the next
            // wishes revert to the previous complete pair.
            self.accepted = None;
        }
        due
    }

    /// Record only a complete pair accepted by the local channel. A partial or
    /// refused send retries the then-current pair at a bounded rate.
    pub(super) fn accepted(&mut self, now: Instant, wishes: MediaWishes) {
        self.accepted = Some((wishes, now));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wishes(microphone: bool, camera: bool) -> MediaWishes {
        MediaWishes {
            microphone,
            camera,
            ..Default::default()
        }
    }

    #[test]
    fn direct_status_changes_without_periodic_announcements() {
        let start = Instant::now();
        let mut status = StatusSnapshot::default();
        let on = wishes(true, true);
        assert!(status.attempt(start, on, false));
        status.accepted(start, on);
        assert!(!status.attempt(start + Duration::from_secs(86_400), on, false));
        assert!(status.attempt(
            start + Duration::from_secs(86_400),
            wishes(false, true),
            false
        ));
    }

    #[test]
    fn sfu_refreshes_current_state_without_a_catch_up_burst() {
        let start = Instant::now();
        let mut status = StatusSnapshot::default();
        let off = wishes(false, false);
        assert!(status.attempt(start, off, true));
        status.accepted(start, off);
        assert!(!status.attempt(start + Duration::from_secs(4), off, true));
        assert!(status.attempt(start + SFU_REFRESH, off, true));
        status.accepted(start + SFU_REFRESH, off);
        let resumed = start + Duration::from_secs(600);
        let on = wishes(true, true);
        assert!(status.attempt(resumed, on, true));
        status.accepted(resumed, on);
        assert!(!status.attempt(resumed, on, true));
        assert!(!status.attempt(resumed + Duration::from_secs(4), on, true));
        // Pending policy prevents refresh, while a current-state change can
        // still report publication stopping.
        assert!(!status.attempt(resumed + SFU_REFRESH, on, false));
        assert!(status.attempt(resumed + SFU_REFRESH, off, false));
    }

    #[test]
    fn refused_or_partial_pair_retries_only_the_latest_state_at_a_finite_rate() {
        for refresh in [false, true] {
            let start = Instant::now();
            let mut status = StatusSnapshot::default();
            let on = wishes(true, true);
            assert!(status.attempt(start, on, refresh));
            // A refused initial pair retries only the latest state.
            let off = wishes(false, false);
            assert!(!status.attempt(start + Duration::from_millis(20), off, refresh));
            let retried = start + RETRY_INTERVAL;
            assert!(status.attempt(retried, off, refresh));
            status.accepted(retried, off);
            assert!(!status.attempt(start + Duration::from_secs(1), off, refresh));
            let changed = start + Duration::from_secs(2);
            assert!(status.attempt(changed, on, refresh));
            // Only audioOn was accepted. Reverting to the previous complete
            // pair must send both fields, including for a Direct peer.
            assert!(!status.attempt(changed + Duration::from_millis(20), off, refresh));
            assert!(status.attempt(changed + RETRY_INTERVAL, off, refresh));
            status.accepted(changed + RETRY_INTERVAL, off);
            assert!(!status.attempt(changed + Duration::from_secs(1), off, refresh));
        }
    }
}
