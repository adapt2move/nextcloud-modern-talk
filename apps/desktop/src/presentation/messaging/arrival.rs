//! Presentation-only arrival admission and one finite post-layout follow deadline.
use std::time::{Duration, Instant};
use talk_core::ui::presentation::{DeliveryState, MessageView};

pub(super) const ENTRY: Duration = Duration::from_millis(200);
const FOLLOW: Duration = Duration::from_millis(75);
const MAX_ENTRIES: usize = 100; // The snapshot's existing message-page bound.
// A large append settles together instead of moving many rows at once.
const BURST: usize = 3;

struct Batch {
    ids: Vec<u128>,
    // None while the admitted bottom-follow waits for its layout deadline.
    // Measuring an offscreen row must not consume its visible entry clock.
    started: Option<Instant>,
}

#[derive(Default)]
pub(super) struct Arrival {
    high_water: u128,
    // At most one page of recent own operation identities. Retaining these
    // across temporary projection gaps prevents a canonical reply from replaying
    // its already observed optimistic entry. Reset retires the conversation.
    operations: Vec<u128>,
    ready: bool,
    visible: bool,
    entry: Option<Batch>,
    // Outer Some: admitted; inner None: awaiting its first completed layout.
    follow: Option<Option<Instant>>,
    // A requested scroll takes effect during the next list layout. Entry time
    // must not advance while the list is still painting the previous viewport.
    awaiting_scroll_layout: bool,
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
        if self.following() {
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
        self.awaiting_scroll_layout = false;
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
        // Only canonical server sequences can advance this watermark. Local
        // pending/uncertain IDs may exceed every server ID in the conversation.
        self.high_water = rows
            .iter()
            .filter(|row| {
                matches!(
                    row.delivery,
                    DeliveryState::Received | DeliveryState::ServerAccepted
                )
            })
            .map(|row| row.local_id)
            .max()
            .unwrap_or(0)
            .max(self.high_water);
        if !eligible {
            self.remember_operations(rows);
            self.cancel();
            return;
        }
        // A server reply can replace the local row ID while its entry is still
        // visible. Keep the original clock and follow deadline for that operation.
        if let Some(batch) = &mut self.entry {
            for id in &mut batch.ids {
                if let Some(operation) = previous
                    .iter()
                    .find(|old| old.local_id == *id && old.own)
                    .and_then(|old| old.metadata.local_operation)
                    && let Some(canonical) = rows
                        .iter()
                        .find(|row| row.own && row.metadata.local_operation == Some(operation))
                {
                    *id = canonical.local_id;
                }
            }
        }
        let appended: Vec<_> = rows
            .iter()
            .filter(|row| {
                let correlated = row.own
                    && row.metadata.local_operation.is_some_and(|operation| {
                        self.operations.contains(&operation)
                            || previous.iter().any(|old| {
                                old.own && old.metadata.local_operation == Some(operation)
                            })
                    });
                let canonical = matches!(
                    row.delivery,
                    DeliveryState::Received | DeliveryState::ServerAccepted
                ) && row.local_id > old_high_water;
                let local = row.own
                    && row.metadata.local_operation.is_some()
                    && matches!(
                        row.delivery,
                        DeliveryState::Pending | DeliveryState::Uncertain
                    );
                (canonical || local)
                    && !correlated
                    && !previous.iter().any(|old| old.local_id == row.local_id)
            })
            .take(MAX_ENTRIES)
            .collect();
        self.remember_operations(rows);
        if appended.is_empty() {
            return;
        }
        self.new_rows = self.new_rows.saturating_add(appended.len() as u64);
        let own_send = appended
            .iter()
            .any(|row| row.own && row.metadata.local_operation.is_some());
        // Baselines are excluded by ready/high-water above, not by an arbitrary
        // time window that would also suppress a real send just after opening.
        // System and bulk rows still follow; one's own accepted send keeps its
        // feedback even when it shares a response with several other messages.
        let ids: Vec<_> = appended
            .iter()
            .filter(|row| !row.metadata.system && (appended.len() <= BURST || row.own))
            .map(|row| row.local_id)
            .collect();
        if !reduced && !ids.is_empty() {
            let following = self.following();
            let waiting = at_bottom || own_send || following;
            if let Some(batch) = self.entry.as_mut().filter(|batch| match batch.started {
                None => waiting && following,
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
        if (at_bottom || own_send) && !self.following() {
            self.follow = Some(None);
        }
    }

    fn remember_operations(&mut self, rows: &[MessageView]) {
        for operation in rows
            .iter()
            .filter(|row| row.own)
            .filter_map(|row| row.metadata.local_operation)
            .take(MAX_ENTRIES)
        {
            if !self.operations.contains(&operation) {
                if self.operations.len() == MAX_ENTRIES {
                    self.operations.remove(0);
                }
                self.operations.push(operation);
            }
        }
    }

    pub fn progress(&self, id: u128, now: Instant) -> Option<f32> {
        self.entry
            .as_ref()
            .filter(|batch| batch.ids.contains(&id))
            .and_then(|batch| {
                let Some(started) = batch.started else {
                    return self.following().then_some(0.);
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
        self.follow.is_some() || self.awaiting_scroll_layout
    }

    /// Confirm the viewport after the requested scroll, not the layout that
    /// requested it. A user/focus override cancels motion rather than animating
    /// an offscreen bubble or fighting the new reading position.
    pub fn after_scroll_layout(&mut self, now: Instant, at_bottom: bool) {
        if !self.awaiting_scroll_layout {
            return;
        }
        if !at_bottom {
            self.manual_scroll();
            return;
        }
        self.awaiting_scroll_layout = false;
        if let Some(batch) = self.entry.as_mut()
            && batch.started.is_none()
        {
            batch.started = Some(now);
        }
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
            self.awaiting_scroll_layout = true;
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

    fn local(id: u128, operation: u128, delivery: DeliveryState) -> MessageView {
        let mut row = row(id);
        row.own = true;
        row.delivery = delivery;
        row.metadata.local_operation = Some(operation);
        row
    }

    #[test]
    fn own_send_from_older_history_scrolls_before_starting_visible_entry() {
        let now = Instant::now();
        let mut state = baseline(now);
        let pending = local(u128::MAX - 1, 801, DeliveryState::Pending);
        state.observe(
            &[row(10)],
            &[row(10), pending.clone()],
            true,
            false,
            false,
            now,
        );
        assert!(
            state.following(),
            "an own send overrides the old reading position"
        );
        assert_eq!(state.progress(pending.local_id, now), Some(0.));
        assert!(!state.after_layout(now, false));
        assert!(state.after_layout(now + FOLLOW, false));

        let scrolled = now + FOLLOW + Duration::from_millis(350);
        assert_eq!(state.progress(pending.local_id, scrolled), Some(0.));
        assert!(!state.animate_next_frame(scrolled));
        assert!(
            state.following(),
            "requesting the scroll is not its completion"
        );
        state.after_scroll_layout(scrolled, true);
        assert!(!state.following());
        assert_eq!(state.progress(pending.local_id, scrolled), Some(0.));
        assert_eq!(
            state.progress(pending.local_id, scrolled + ENTRY / 2),
            Some(0.5)
        );
    }

    #[test]
    fn overridden_scroll_cancels_waiting_entry_and_remote_own_messages_do_not_force_follow() {
        let now = Instant::now();
        let mut state = baseline(now);
        let pending = local(u128::MAX - 2, 802, DeliveryState::Pending);
        state.observe(
            &[row(10)],
            std::slice::from_ref(&pending),
            true,
            false,
            false,
            now,
        );
        assert!(!state.after_layout(now, false));
        assert!(state.after_layout(now + FOLLOW, false));
        state.after_scroll_layout(now + FOLLOW, false);
        assert!(!state.following());
        assert!(state.progress(pending.local_id, now + FOLLOW).is_none());

        let mut remote_own = row(11);
        remote_own.own = true;
        remote_own.delivery = DeliveryState::ServerAccepted;
        state.observe(&[pending], &[remote_own], true, false, false, now + FOLLOW);
        assert!(
            !state.following(),
            "another client's send preserves the reading position"
        );
    }

    #[test]
    fn optimistic_own_entry_follows_and_canonical_operation_keeps_its_original_clock() {
        let now = Instant::now();
        let mut state = baseline(now);
        let pending = local(u128::MAX - 1, 501, DeliveryState::Pending);
        state.observe(
            &[row(10)],
            &[row(10), pending.clone()],
            true,
            true,
            false,
            now,
        );
        assert_eq!(state.progress(pending.local_id, now), Some(0.));
        assert!(state.following());
        assert_eq!(state.high_water, 10);
        assert!(!state.after_layout(now, false));
        assert!(state.after_layout(now + FOLLOW, false));
        state.after_scroll_layout(now + FOLLOW, true);
        let middle = now + FOLLOW + ENTRY / 2;
        let canonical = local(11, 501, DeliveryState::ServerAccepted);
        state.observe(
            &[row(10), pending.clone()],
            &[row(10), canonical.clone()],
            true,
            true,
            false,
            middle,
        );
        assert_eq!(state.progress(11, middle), Some(0.5));
        assert!(state.progress(pending.local_id, middle).is_none());
        assert!(
            !state.following(),
            "correlation does not schedule another follow"
        );
        assert_eq!(state.diagnostic().0, 1);
        // An equal body is irrelevant when operation identity differs.
        let other = local(u128::MAX - 2, 502, DeliveryState::Pending);
        state.observe(
            &[canonical],
            &[row(11), other.clone()],
            true,
            false,
            false,
            middle,
        );
        assert!(state.progress(other.local_id, middle).is_some());
        assert_eq!(state.diagnostic().0, 2);
    }

    #[test]
    fn uncertain_local_ids_cannot_advance_the_server_watermark_or_block_incoming_rows() {
        let now = Instant::now();
        let mut state = baseline(now);
        let uncertain = local(u128::MAX - 3, 503, DeliveryState::Uncertain);
        state.observe(
            &[row(10)],
            &[row(10), uncertain.clone()],
            true,
            true,
            false,
            now,
        );
        assert_eq!(state.progress(uncertain.local_id, now), Some(0.));
        assert_eq!(state.high_water, 10);
        state.observe(
            &[row(10), uncertain.clone()],
            &[row(10), row(11), uncertain],
            true,
            true,
            false,
            now,
        );
        assert_eq!(state.progress(11, now), Some(0.));
        assert_eq!(state.high_water, 11);
        assert_eq!(state.diagnostic().0, 2);
        let failed = local(u128::MAX - 4, 504, DeliveryState::Failed);
        state.observe(&[row(11)], &[row(11), failed], true, false, false, now);
        assert_eq!(state.high_water, 11);
        assert_eq!(state.diagnostic().0, 2);
    }

    #[test]
    fn operation_retention_is_bounded_and_prevents_replay_after_a_projection_gap() {
        let now = Instant::now();
        let mut state = baseline(now);
        let pending = local(u128::MAX - 5, 505, DeliveryState::Pending);
        state.observe(
            &[row(10)],
            std::slice::from_ref(&pending),
            true,
            false,
            false,
            now,
        );
        assert!(!state.after_layout(now, false));
        assert!(state.after_layout(now + FOLLOW, false));
        state.after_scroll_layout(now + FOLLOW, true);
        state.observe(&[pending], &[row(10)], true, false, false, now + FOLLOW);
        state.animate_next_frame(now + FOLLOW + ENTRY);
        state.observe(
            &[row(10)],
            &[local(11, 505, DeliveryState::ServerAccepted)],
            true,
            true,
            false,
            now + ENTRY,
        );
        assert!(state.progress(11, now + ENTRY).is_none());
        assert!(!state.following());
        assert_eq!(state.diagnostic().0, 1);
        for operation in 600..710 {
            state.observe(
                &[row(11)],
                &[local(
                    u128::MAX - operation,
                    operation,
                    DeliveryState::Pending,
                )],
                true,
                false,
                true,
                now,
            );
        }
        assert_eq!(state.operations.len(), MAX_ENTRIES);
        assert!(!state.operations.contains(&505));
        assert!(state.operations.contains(&709));
        state.reset();
        assert!(state.operations.is_empty());
    }

    #[test]
    fn bursts_and_system_rows_follow_without_entry_but_own_sends_still_enter() {
        let now = Instant::now();
        let mut state = baseline(now);
        let burst: Vec<_> = (11..=14).map(row).collect();
        state.observe(&[row(10)], &burst, true, true, false, now);
        assert!(
            burst
                .iter()
                .all(|r| state.progress(r.local_id, now).is_none())
        );
        assert!(state.following());
        assert!(!state.after_layout(now, false));
        assert!(state.after_layout(now + FOLLOW, false));
        state.after_scroll_layout(now + FOLLOW, true);
        let mut system = row(15);
        system.metadata.system = true;
        state.observe(
            &[row(14)],
            &[row(14), system],
            true,
            true,
            false,
            now + FOLLOW,
        );
        assert!(state.progress(15, now + FOLLOW).is_none());
        assert!(state.following());
        assert!(!state.animate_next_frame(now + FOLLOW));
        assert!(!state.after_layout(now + FOLLOW, false));
        assert!(state.after_layout(now + FOLLOW * 2, false));
        state.after_scroll_layout(now + FOLLOW * 2, true);
        let mut own = row(16);
        own.own = true;
        let mut batch = vec![own];
        batch.extend((17..=20).map(row));
        let sent = now + FOLLOW * 2;
        state.observe(&[row(15)], &batch, true, true, false, sent);
        assert_eq!(state.progress(16, sent), Some(0.));
        assert!((17..=20).all(|id| state.progress(id, sent).is_none()));
        assert!(state.following());
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
        state.after_scroll_layout(now + FOLLOW, true);
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
        state.after_scroll_layout(now + FOLLOW, true);
        let visible = now + FOLLOW;
        assert_eq!(state.progress(12, visible + ENTRY / 2), Some(0.5));
        // Re-querying a remounted row cannot restart or reverse its clock.
        assert_eq!(state.progress(12, visible + ENTRY / 2), Some(0.5));
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
        state.after_scroll_layout(layout + FOLLOW, true);
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
        state.after_scroll_layout(now, true);
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
        // A slow first layout must not consume the visible entry.
        let layout = now + Duration::from_millis(200);
        assert_eq!(state.progress(11, layout), Some(0.));
        assert!(!state.animate_next_frame(layout));
        assert!(!state.after_layout(layout, false));
        let visible = layout + FOLLOW;
        assert!(state.after_layout(visible, false));
        state.after_scroll_layout(visible, true);
        assert_eq!(state.progress(11, visible), Some(0.));
        assert_eq!(state.progress(11, visible + ENTRY / 2), Some(0.5));
        assert!(state.animate_next_frame(visible + ENTRY / 2));
        assert!(state.animate_next_frame(visible + ENTRY));
        assert!(state.progress(11, visible + ENTRY).is_none());
        assert!(!state.animate_next_frame(visible + ENTRY));
    }
}
