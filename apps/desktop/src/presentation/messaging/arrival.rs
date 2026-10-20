//! Presentation-only arrival admission and one finite post-layout follow deadline.
use std::time::{Duration, Instant};
use talk_core::ui::presentation::{DeliveryState, MessageView};

pub(super) const ENTRY: Duration = Duration::from_millis(140);
const FOLLOW: Duration = Duration::from_millis(75);
const MAX_ENTRIES: usize = 100; // The snapshot's existing message-page bound.

struct Batch {
    ids: Vec<u128>,
    // None while the admitted bottom-follow waits for its layout deadline.
    // Measuring an offscreen row must not consume its visible entry clock.
    started: Option<Instant>,
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
        if self
            .entry
            .as_ref()
            .is_some_and(|batch| batch.started.is_none())
        {
            self.entry = None;
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
            let waiting = at_bottom || self.follow.is_some();
            if let Some(batch) = self.entry.as_mut().filter(|batch| match batch.started {
                None => waiting && self.follow.is_some(),
                Some(started) => !waiting && now.saturating_duration_since(started) < ENTRY,
            }) {
                // A burst shares the original deadline; it cannot keep motion alive.
                for id in ids {
                    if batch.ids.len() < MAX_ENTRIES && !batch.ids.contains(&id) {
                        batch.ids.push(id);
                    }
                }
            } else {
                self.entry = Some(Batch {
                    ids,
                    started: (!waiting).then_some(now),
                });
            }
        }
        if at_bottom && self.follow.is_none() {
            self.follow = Some(None);
        }
    }

    pub fn progress(&self, id: u128, now: Instant) -> Option<f32> {
        self.entry
            .as_ref()
            .filter(|batch| batch.ids.contains(&id))
            .and_then(|batch| {
                let Some(started) = batch.started else {
                    return self.follow.is_some().then_some(0.);
                };
                let elapsed = now.saturating_duration_since(started);
                (elapsed < ENTRY).then(|| elapsed.as_secs_f32() / ENTRY.as_secs_f32())
            })
    }

    /// The timeline owns one finite frame driver, including one final paint.
    /// Lazy row measurement cannot start or restart this clock.
    pub fn animate_next_frame(&mut self, now: Instant) -> bool {
        let Some(started) = self.entry.as_ref().and_then(|batch| batch.started) else {
            return false;
        };
        if now.saturating_duration_since(started) >= ENTRY {
            self.entry = None;
        }
        true
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
            if let Some(batch) = self.entry.as_mut()
                && batch.started.is_none()
            {
                batch.started = Some(now);
            }
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
        assert!(state.progress(10, now).is_none());
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
        assert_eq!(state.progress(11, now), Some(0.));
        assert_eq!(state.progress(12, now), Some(0.));
        assert!(!state.after_layout(now, false));
        assert!(state.after_layout(now + FOLLOW, false));
        let settled = now + FOLLOW + ENTRY;
        state.observe(
            &[row(11)],
            &[row(5), row(10), row(11)],
            true,
            false,
            false,
            settled,
        );
        assert!(state.progress(5, settled).is_none());
        assert!(state.progress(10, settled).is_none());
        assert!(state.progress(11, settled).is_none());
        state.observe(&[row(11)], &[], false, false, false, settled);
        state.observe(&[], &[row(11), row(13)], true, true, false, settled);
        assert!(state.progress(13, settled).is_none());
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
        assert!(state.progress(11, now).is_none());
        state.observe(&[row(11)], &[row(11), row(12)], true, true, false, now);
        assert!(!state.after_layout(now, false));
        assert!(state.after_layout(now + FOLLOW, false));
        let visible = now + FOLLOW;
        assert_eq!(
            state.progress(12, visible + Duration::from_millis(70)),
            Some(0.5)
        );
        // Re-querying a remounted row cannot restart or reverse its clock.
        assert_eq!(
            state.progress(12, visible + Duration::from_millis(70)),
            Some(0.5)
        );
        assert!(state.progress(12, visible + ENTRY).is_none());
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
        assert!(state.progress(13, now + ENTRY + FOLLOW).is_none());
        state.observe(&[row(13)], &[row(14)], true, false, false, now + ENTRY);
        assert!(!state.following());
    }

    #[test]
    fn reduced_motion_follows_immediately_after_layout_and_hidden_or_retired_cancels() {
        let now = Instant::now();
        let mut state = baseline(now);
        state.observe(&[row(10)], &[row(11)], true, true, true, now);
        assert!(state.progress(11, now).is_none());
        assert!(state.after_layout(now, true));
        state.observe(&[row(11)], &[row(12)], true, true, false, now);
        state.visibility(false, false);
        assert!(!state.following());
        assert!(state.progress(12, now).is_none());
        state.visibility(true, false);
        state.observe(&[row(12)], &[row(13)], true, true, false, now);
        state.reset();
        assert!(!state.following());
        assert!(state.progress(13, now).is_none());
    }

    #[test]
    fn visible_entry_starts_at_follow_commit_and_has_one_finite_frame_driver() {
        let now = Instant::now();
        let mut state = baseline(now);
        state.observe(&[row(10)], &[row(10), row(11)], true, true, false, now);
        // A slow first layout must not consume the visible 140 ms fade.
        let layout = now + Duration::from_millis(200);
        assert_eq!(state.progress(11, layout), Some(0.));
        assert!(!state.animate_next_frame(layout));
        assert!(!state.after_layout(layout, false));
        let visible = layout + FOLLOW;
        assert!(state.after_layout(visible, false));
        assert_eq!(state.progress(11, visible), Some(0.));
        assert_eq!(
            state.progress(11, visible + Duration::from_millis(70)),
            Some(0.5)
        );
        assert!(state.animate_next_frame(visible + Duration::from_millis(70)));
        assert!(state.animate_next_frame(visible + ENTRY));
        assert!(state.progress(11, visible + ENTRY).is_none());
        assert!(!state.animate_next_frame(visible + ENTRY));
    }
}
