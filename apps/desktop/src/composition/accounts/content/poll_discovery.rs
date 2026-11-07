//! Bounded discovery of poll cards in the current room's loaded history.
//! A missing poll receives at most three automatic reads before its card offers
//! a deliberate retry. A cached observation never grants network authority.
use super::*;
use talk_core::ui::presentation::MessageView;

const LIMIT: usize = 20;
const ATTEMPTS: u8 = 3;

struct Attempt {
    id: u64,
    count: u8,
    next: Instant,
}

#[derive(Default)]
pub(super) struct Discovery {
    attempts: Vec<Attempt>,
}

impl Discovery {
    pub fn clear(&mut self) {
        self.attempts.clear();
    }

    pub fn retain(&mut self, candidates: &[u64]) {
        self.attempts
            .retain(|attempt| candidates.contains(&attempt.id));
    }

    pub fn next(&self, candidates: &[u64], loaded: &[view::PollView], now: Instant) -> Option<u64> {
        candidates.iter().copied().find(|id| {
            !loaded.iter().any(|poll| poll.id == *id)
                && self
                    .attempts
                    .iter()
                    .find(|attempt| attempt.id == *id)
                    .is_none_or(|attempt| attempt.count < ATTEMPTS && attempt.next <= now)
        })
    }

    pub fn started(&mut self, id: u64, now: Instant) {
        if let Some(attempt) = self.attempts.iter_mut().find(|attempt| attempt.id == id) {
            attempt.count = attempt.count.saturating_add(1).min(ATTEMPTS);
            attempt.next = now + Duration::from_secs(1 << attempt.count);
        } else if self.attempts.len() < LIMIT {
            self.attempts.push(Attempt {
                id,
                count: 1,
                next: now + Duration::from_secs(2),
            });
        }
    }
}

pub(super) fn candidates(snapshot: &PresentationSnapshot) -> Vec<u64> {
    let mut ids = Vec::with_capacity(LIMIT);
    let now = history::now_unix();
    let threads = snapshot
        .message_panel
        .as_ref()
        .filter(|panel| Some(panel.context) == snapshot.selected)
        .into_iter()
        .flat_map(|panel| panel.thread_messages.as_slice().iter().rev());
    for message in threads.chain(snapshot.messages.as_slice().iter().rev()) {
        if message.metadata.deleted
            || message
                .metadata
                .expires_at
                .is_some_and(|at| at != 0 && at <= now)
        {
            continue;
        }
        for id in poll_ids(message) {
            if id != 0 && !ids.contains(&id) {
                ids.push(id);
                if ids.len() == LIMIT {
                    return ids;
                }
            }
        }
    }
    ids
}

fn poll_ids(message: &MessageView) -> impl Iterator<Item = u64> + '_ {
    message
        .metadata
        .rich
        .as_slice()
        .iter()
        .filter(|object| matches!(object.kind.as_str(), "poll" | "talk-poll"))
        .filter_map(|object| object.id.as_ref()?.as_str().parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_back_off_and_stop_without_starving_other_polls() {
        let mut discovery = Discovery::default();
        let now = Instant::now();
        let ids = [10, 20];
        assert_eq!(discovery.next(&ids, &[], now), Some(10));
        discovery.started(10, now);
        assert_eq!(discovery.next(&ids, &[], now), Some(20));
        discovery.started(10, now + Duration::from_secs(2));
        discovery.started(10, now + Duration::from_secs(6));
        assert_eq!(
            discovery.next(&[10], &[], now + Duration::from_secs(60)),
            None
        );
        discovery.clear();
        assert_eq!(discovery.next(&[10], &[], now), Some(10));
    }

    #[test]
    fn removed_candidates_release_their_attempt_budget() {
        let mut discovery = Discovery::default();
        let now = Instant::now();
        for id in 1..=LIMIT as u64 {
            discovery.started(id, now);
        }
        discovery.retain(&[1]);
        discovery.started(100, now);
        assert_eq!(discovery.next(&[100], &[], now), None);
        assert_eq!(
            discovery.next(&[100], &[], now + Duration::from_secs(2)),
            Some(100)
        );
    }

    #[test]
    fn discovery_uses_current_non_deleted_message_objects() {
        let mut snapshot = super::super::super::super::fixtures::snapshot(Some("chat-polls"));
        assert_eq!(candidates(&snapshot), vec![9001]);
        let mut row = snapshot.messages.as_slice()[0].clone();
        row.metadata.deleted = true;
        snapshot.messages = page([row]);
        assert!(candidates(&snapshot).is_empty());
    }
}
