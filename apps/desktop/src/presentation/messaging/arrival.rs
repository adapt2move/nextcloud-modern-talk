//! Presentation-only arrival admission and one finite post-layout follow deadline.
use std::time::{Duration, Instant};
use talk_core::ui::presentation::{DeliveryState, MessageView};

pub(super) const ENTRY: Duration = Duration::from_millis(140);
const FOLLOW: Duration = Duration::from_millis(75);
const MAX_ENTRIES: usize = 100; // The snapshot's existing message-page bound.

struct Batch {
    ids: Vec<u128>,
    admitted: Instant,
}

#[derive(Default)]
pub(super) struct Arrival {
    high_water: u128,
    ready: bool,
    visible: bool,
    entry: Option<Batch>,
    // Outer Some: admitted; inner None: awaiting its first completed layout.
    follow: Option<Option<Instant>>,
    new_rows: u64,
    follows: u64,
    cancelled_follows: u64,
    last_follow_ms: Option<u128>,
}

impl Arrival {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn visibility(&mut self, visible: bool, reduced: bool) {
        self.visible = visible;
        if !visible {
            self.cancel();
        } else if reduced {
            self.entry = None;
        }
    }

    pub fn cancel(&mut self) {
        self.entry = None;
        self.manual_scroll();
    }

    pub fn manual_scroll(&mut self) {
        if self.follow.is_some() {
            self.cancelled_follows = self.cancelled_follows.saturating_add(1);
        }
        self.follow = None;
    }

    pub fn observe(
        &mut self,
        previous: &[MessageView],
        rows: &[MessageView],
        ready: bool,
        at_bottom: bool,
        reduced: bool,
        now: Instant,
    ) {
        let eligible = self.ready && ready && self.visible;
        self.ready = ready;
        let old_high_water = self.high_water;
        // The production projection uses increasing server sequences as local IDs.
        // Pending fixture rows are not server identities. A same-ID reconciliation
        // is already observed and never starts a second entry animation.
        self.high_water = rows
            .iter()
            .filter(|row| row.delivery != DeliveryState::Pending)
            .map(|row| row.local_id)
            .max()
            .unwrap_or(0)
            .max(self.high_water);
        if !eligible {
            self.cancel();
            return;
        }
        let ids: Vec<_> = rows
            .iter()
            .filter(|row| {
                row.local_id > old_high_water
                    && row.delivery != DeliveryState::Pending
                    && !row.metadata.system
                    && !previous.iter().any(|old| old.local_id == row.local_id)
            })
            .take(MAX_ENTRIES)
            .map(|row| row.local_id)
            .collect();
        if ids.is_empty() {
            return;
        }
        self.new_rows = self.new_rows.saturating_add(ids.len() as u64);
        if !reduced {
            if let Some(batch) = self
                .entry
                .as_mut()
                .filter(|batch| now.saturating_duration_since(batch.admitted) < ENTRY)
            {
                // A burst shares the original deadline; it cannot keep motion alive.
                for id in ids {
                    if batch.ids.len() < MAX_ENTRIES && !batch.ids.contains(&id) {
                        batch.ids.push(id);
                    }
                }
            } else {
                self.entry = Some(Batch { ids, admitted: now });
            }
        }
        if at_bottom && self.follow.is_none() {
            self.follow = Some(None);
        }
    }

    pub fn admitted(&self, id: u128, now: Instant) -> Option<Instant> {
        self.entry
            .as_ref()
            .filter(|batch| {
                now.saturating_duration_since(batch.admitted) < ENTRY && batch.ids.contains(&id)
            })
            .map(|batch| batch.admitted)
    }

    pub fn following(&self) -> bool {
        self.follow.is_some()
    }

    pub fn diagnostic(&self) -> (u64, u64, u64, Option<u128>) {
        (
            self.new_rows,
            self.follows,
            self.cancelled_follows,
            self.last_follow_ms,
        )
    }

    /// Called only after the actual timeline has laid out. One frame driver, no
    /// delayed tasks or per-message timers. The deadline never slides on a burst.
    pub fn after_layout(&mut self, now: Instant, reduced: bool) -> bool {
        let Some(deadline) = &mut self.follow else {
            return false;
        };
        if reduced || now >= *deadline.get_or_insert(now + FOLLOW) {
            self.last_follow_ms = Some(if reduced {
                0
            } else {
                now.saturating_duration_since(deadline.expect("initialized") - FOLLOW)
                    .as_millis()
            });
            self.follows = self.follows.saturating_add(1);
            self.follow = None;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::ui::{Page, Text};

    fn row(id: u128) -> MessageView {
        MessageView {
            local_id: id,
            author: Text::new("Fixture").unwrap(),
            initials: Text::new("F").unwrap(),
            time_label: Text::new("").unwrap(),
            body: Text::new("Fictional message").unwrap(),
            own: false,
            delivery: DeliveryState::Received,
            attachments: Page::empty(),
            reply_count: 0,
            metadata: Default::default(),
        }
    }
    fn baseline(now: Instant) -> Arrival {
        let mut state = Arrival::default();
        state.visibility(true, false);
        state.observe(&[], &[row(10)], true, true, false, now);
        assert!(state.admitted(10, now).is_none());
        assert!(!state.following());
        state
    }

    #[test]
    fn new_incoming_and_own_rows_enter_but_edits_history_and_replay_do_not() {
        let now = Instant::now();
        let mut state = baseline(now);
        let mut own = row(12);
        own.own = true;
        state.observe(&[row(10)], &[row(10), row(11), own], true, true, false, now);
        assert_eq!(state.admitted(11, now), Some(now));
        assert_eq!(state.admitted(12, now), Some(now));
        let settled = now + ENTRY;
        state.observe(
            &[row(11)],
            &[row(5), row(10), row(11)],
            true,
            false,
            false,
            settled,
        );
        assert!(state.admitted(5, settled).is_none());
        assert!(state.admitted(10, settled).is_none());
        assert!(state.admitted(11, settled).is_none());
        state.observe(&[row(11)], &[], false, false, false, settled);
        state.observe(&[], &[row(11), row(13)], true, true, false, settled);
        assert!(state.admitted(13, settled).is_none());
        assert!(!state.following());
    }

    #[test]
    fn same_id_pending_confirmation_and_virtual_remount_cannot_restart_entry() {
        let now = Instant::now();
        let mut state = baseline(now);
        let mut pending = row(11);
        pending.delivery = DeliveryState::Pending;
        state.observe(
            &[row(10)],
            &[row(10), pending.clone()],
            true,
            true,
            false,
            now,
        );
        state.observe(
            &[row(10), pending],
            &[row(10), row(11)],
            true,
            true,
            false,
            now,
        );
        assert!(state.admitted(11, now).is_none());
        state.observe(&[row(11)], &[row(11), row(12)], true, true, false, now);
        assert_eq!(
            state.admitted(12, now + Duration::from_millis(90)),
            Some(now)
        );
        assert!(state.admitted(12, now + ENTRY).is_none());
    }

    #[test]
    fn follow_starts_after_layout_is_coalesced_and_manual_scroll_cancels() {
        let now = Instant::now();
        let mut state = baseline(now);
        state.observe(&[row(10)], &[row(11)], true, true, false, now);
        let layout = now + Duration::from_millis(20);
        assert!(!state.after_layout(layout, false));
        state.observe(
            &[row(11)],
            &[row(12)],
            true,
            false,
            false,
            layout + Duration::from_millis(50),
        );
        assert!(!state.after_layout(layout + FOLLOW - Duration::from_millis(1), false));
        assert!(state.after_layout(layout + FOLLOW, false));
        assert!(!state.following());
        state.observe(&[row(12)], &[row(13)], true, true, false, now + ENTRY);
        state.manual_scroll();
        assert!(!state.after_layout(now + ENTRY + FOLLOW, false));
        state.observe(&[row(13)], &[row(14)], true, false, false, now + ENTRY);
        assert!(!state.following());
    }

    #[test]
    fn reduced_motion_follows_immediately_after_layout_and_hidden_or_retired_cancels() {
        let now = Instant::now();
        let mut state = baseline(now);
        state.observe(&[row(10)], &[row(11)], true, true, true, now);
        assert!(state.admitted(11, now).is_none());
        assert!(state.after_layout(now, true));
        state.observe(&[row(11)], &[row(12)], true, true, false, now);
        state.visibility(false, false);
        assert!(!state.following());
        assert!(state.admitted(12, now).is_none());
        state.visibility(true, false);
        state.observe(&[row(12)], &[row(13)], true, true, false, now);
        state.reset();
        assert!(!state.following());
        assert!(state.admitted(13, now).is_none());
    }
}
