//! Short-lived, in-memory poll observations for recently visited rooms.
//! Cache entries carry no authority and are never written to durable storage.
use super::*;

const ROOMS: usize = 4;
const POLLS: usize = 20;
const LIFETIME: Duration = Duration::from_secs(5 * 60);

struct Room {
    context: ConversationRef,
    observed: Instant,
    polls: Vec<view::PollView>,
}

#[derive(Default)]
pub(super) struct Cache {
    rooms: Vec<Room>,
}

impl Cache {
    fn remember(&mut self, context: ConversationRef, polls: Vec<view::PollView>, now: Instant) {
        self.prune(now);
        self.forget(context);
        if polls.is_empty() {
            return;
        }
        if self.rooms.len() == ROOMS {
            self.rooms.remove(0);
        }
        self.rooms.push(Room {
            context,
            observed: now,
            polls: polls.into_iter().take(POLLS).collect(),
        });
    }

    fn restore(
        &mut self,
        context: ConversationRef,
        ids: &[u64],
        now: Instant,
    ) -> Vec<view::PollView> {
        self.prune(now);
        self.rooms
            .iter()
            .find(|room| room.context == context)
            .into_iter()
            .flat_map(|room| room.polls.iter())
            .filter(|poll| ids.contains(&poll.id))
            .cloned()
            .map(|mut poll| {
                // A saved observation does not authorize vote or closure.
                poll.vote.allowed = Decision::Unknown;
                poll.close.allowed = Decision::Unknown;
                poll
            })
            .collect()
    }

    pub(super) fn prune(&mut self, now: Instant) {
        self.rooms
            .retain(|room| now.saturating_duration_since(room.observed) < LIFETIME);
    }

    pub(super) fn retain_rooms(&mut self, rooms: &[ConversationView]) {
        self.rooms.retain(|cached| {
            rooms.iter().any(|room| {
                room.context == cached.context && room.draft_clipboard == Decision::Allowed
            })
        });
    }

    pub fn forget(&mut self, context: ConversationRef) {
        self.rooms.retain(|room| room.context != context);
    }

    pub fn retain_account(&mut self, account: Option<AccountScope>) {
        self.rooms
            .retain(|room| Some(room.context.account) == account);
    }
}

impl Accounts {
    fn poll_cache_allowed(&self, context: ConversationRef) -> bool {
        self.snapshot.active_account == Some(context.account)
            && self.snapshot.selected == Some(context)
            && matches!(
                self.snapshot.content,
                ContentState::Ready | ContentState::Offline
            )
            && self
                .snapshot
                .conversations
                .as_slice()
                .iter()
                .any(|room| room.context == context && room.draft_clipboard == Decision::Allowed)
    }

    pub(in crate::composition::accounts) fn remember_poll_room(&mut self) {
        let Some(context) = self.snapshot.selected else {
            return;
        };
        if !self.poll_cache_allowed(context) || self.snapshot.content_panel.context != Some(context)
        {
            self.content_owner.poll_cache.forget(context);
            return;
        }
        let ids = super::poll_discovery::candidates(&self.snapshot);
        let polls = self
            .snapshot
            .content_panel
            .polls
            .as_slice()
            .iter()
            .filter(|poll| ids.contains(&poll.id))
            .cloned()
            .collect();
        self.content_owner
            .poll_cache
            .remember(context, polls, Instant::now());
    }

    pub(in crate::composition::accounts) fn restore_poll_room(&mut self, context: ConversationRef) {
        if !self.poll_cache_allowed(context)
            || !self.snapshot.content_panel.polls.as_slice().is_empty()
        {
            return;
        }
        let ids = super::poll_discovery::candidates(&self.snapshot);
        let polls = self
            .content_owner
            .poll_cache
            .restore(context, &ids, Instant::now());
        if !polls.is_empty() {
            self.snapshot.content_panel.account = Some(context.account);
            self.snapshot.content_panel.context = Some(context);
            self.snapshot.content_panel.polls = page(polls);
            self.touch();
        }
    }

    pub(in crate::composition::accounts) fn forget_poll_room(&mut self, context: ConversationRef) {
        self.content_owner.poll_cache.forget(context);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_results_are_bounded_scoped_expiring_and_never_restore_authority() {
        let snapshot = super::super::super::super::fixtures::snapshot(Some("chat-polls"));
        let context = snapshot.selected.unwrap();
        let poll = snapshot.content_panel.polls.as_slice()[0].clone();
        let now = Instant::now();
        let mut cache = Cache::default();
        cache.remember(context, vec![poll.clone()], now);
        let restored = cache.restore(context, &[poll.id], now);
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].question, poll.question);
        assert_eq!(restored[0].vote.allowed, Decision::Unknown);
        assert_eq!(restored[0].close.allowed, Decision::Unknown);
        assert!(cache.restore(context, &[], now).is_empty());
        assert!(
            cache
                .restore(context, &[poll.id], now + LIFETIME)
                .is_empty()
        );
        for id in 100..105 {
            let room = super::super::super::super::fixtures::context(id);
            cache.remember(room, vec![poll.clone()], now);
        }
        assert_eq!(cache.rooms.len(), ROOMS);
        assert!(cache.restore(context, &[poll.id], now).is_empty());
        cache.remember(context, vec![poll], now);
        cache.retain_rooms(&[]);
        assert!(cache.rooms.is_empty());
        cache.remember(
            context,
            vec![snapshot.content_panel.polls.as_slice()[0].clone()],
            now,
        );
        cache.retain_account(None);
        assert!(cache.rooms.is_empty());
    }
}
