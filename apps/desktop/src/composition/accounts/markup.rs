//! One ephemeral, selected-room Markdown preparation owner. At most 256 sources
//! (1 MiB) and one 32-source background batch are retained. Results cannot cross
//! selection/account generations; nothing here is serialized or grants access.
use super::*;
use talk_core::ui::markup::Prepared;
const CAPACITY: usize = 256;
const BATCH: usize = 32;
#[derive(Default)]
pub(super) struct Owner {
    context: Option<ConversationRef>,
    ready: Vec<Prepared>,
    pending: Vec<Text<4096>>,
    flight: Vec<Text<4096>>,
}
pub(in crate::composition) struct Job {
    pub context: ConversationRef,
    sources: Vec<Text<4096>>,
}
pub(in crate::composition) fn prepare(job: Job) -> Vec<Prepared> {
    job.sources.into_iter().map(Prepared::new).collect()
}
impl Owner {
    pub(super) fn project(&mut self, snapshot: &mut PresentationSnapshot) {
        if self.context != snapshot.selected {
            *self = Self {
                context: snapshot.selected,
                ..Self::default()
            };
        }
        if self.context.is_none()
            || matches!(
                snapshot.content,
                ContentState::Denied | ContentState::Unconfigured
            )
        {
            self.ready.clear();
            self.pending.clear();
            self.flight.clear();
            return;
        }
        let now = history::now_unix();
        let eligible = |row: &talk_core::ui::presentation::MessageView| {
            !row.metadata.deleted && row.metadata.expires_at.is_none_or(|at| at == 0 || at > now)
        };
        let live = |source: &str| {
            snapshot
                .messages
                .as_slice()
                .iter()
                .chain(snapshot.message_panel.as_ref().into_iter().flat_map(|p| {
                    p.thread_messages
                        .as_slice()
                        .iter()
                        .chain(p.pinned_messages.as_slice())
                }))
                .any(|row| eligible(row) && row.body.as_str() == source)
        };
        self.ready.retain(|p| live(p.source()));
        self.pending.retain(|p| live(p.as_str()));
        self.flight.retain(|p| live(p.as_str()));
        let mut decorate = |row: &mut talk_core::ui::presentation::MessageView| {
            if !eligible(row) {
                row.metadata.prepared = None;
                return;
            }
            row.metadata.prepared = self
                .ready
                .iter()
                .find(|p| p.source() == row.body.as_str())
                .cloned();
            if row.metadata.prepared.is_none()
                && !row.body.as_str().is_empty()
                && self.ready.len() + self.pending.len() + self.flight.len() < CAPACITY
                && !self
                    .pending
                    .iter()
                    .chain(&self.flight)
                    .any(|s| s == &row.body)
            {
                self.pending.push(row.body.clone());
            }
        };
        snapshot.messages = page(snapshot.messages.as_slice().iter().cloned().map(|mut row| {
            decorate(&mut row);
            row
        }));
        if let Some(panel) = snapshot.message_panel.as_mut() {
            panel.thread_messages = page(panel.thread_messages.as_slice().iter().cloned().map(
                |mut row| {
                    decorate(&mut row);
                    row
                },
            ));
            panel.pinned_messages = page(panel.pinned_messages.as_slice().iter().cloned().map(
                |mut row| {
                    decorate(&mut row);
                    row
                },
            ));
        }
    }
    fn take(&mut self) -> Option<Job> {
        if !self.flight.is_empty() || self.pending.is_empty() {
            return None;
        }
        let context = self.context?;
        self.flight = self
            .pending
            .drain(..self.pending.len().min(BATCH))
            .collect();
        Some(Job {
            context,
            sources: self.flight.clone(),
        })
    }
    fn finish(&mut self, context: ConversationRef, prepared: Vec<Prepared>) -> bool {
        if self.context != Some(context) {
            return false;
        }
        let mut accepted = false;
        for value in prepared.into_iter().take(BATCH) {
            if !self.flight.iter().any(|s| s.as_str() == value.source()) {
                continue;
            }
            if !self.ready.iter().any(|p| p.source() == value.source()) {
                if self.ready.len() == CAPACITY {
                    self.ready.remove(0);
                }
                self.ready.push(value);
                accepted = true;
            }
        }
        self.flight.clear();
        accepted
    }
}
impl Accounts {
    pub(in crate::composition) fn take_markup(&mut self) -> Option<Job> {
        self.markup.take()
    }
    pub(in crate::composition) fn finish_markup(
        &mut self,
        context: ConversationRef,
        prepared: Vec<Prepared>,
    ) {
        if self.markup.finish(context, prepared) {
            self.touch();
        }
    }
    pub(in crate::composition) fn awaiting_cached_content(&self) -> bool {
        self.snapshot.messages.as_slice().is_empty()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn late_preparation_cannot_restore_another_room_or_an_edited_body() {
        let mut owner = Owner::default();
        let mut snapshot = crate::composition::fixtures::snapshot(Some("chat"));
        let original = snapshot.selected.unwrap();
        snapshot.messages = page(snapshot.messages.as_slice().iter().take(1).cloned());
        owner.project(&mut snapshot);
        let job = owner.take().unwrap();
        let source = snapshot.messages.as_slice()[0].body.clone();
        assert!(owner.finish(original, prepare(job)));
        owner.project(&mut snapshot);
        assert_eq!(
            snapshot.messages.as_slice()[0]
                .metadata
                .prepared
                .as_ref()
                .unwrap()
                .source(),
            source.as_str()
        );
        let mut row = snapshot.messages.as_slice()[0].clone();
        row.body = text("edited");
        snapshot.messages = page([row]);
        owner.project(&mut snapshot);
        assert!(snapshot.messages.as_slice()[0].metadata.prepared.is_none());
        assert!(
            owner.ready.is_empty(),
            "edited/deleted source is retired immediately"
        );
        let job = owner.take().unwrap();
        snapshot.selected = None;
        owner.project(&mut snapshot);
        assert!(!owner.finish(original, prepare(job)));
        assert!(owner.ready.is_empty());
    }
    #[test]
    fn deletion_and_expiry_retire_prepared_text_and_reject_late_worker_results() {
        for expired in [false, true] {
            let mut owner = Owner::default();
            let mut snapshot = crate::composition::fixtures::snapshot(Some("chat"));
            let context = snapshot.selected.unwrap();
            snapshot.messages = page(snapshot.messages.as_slice().iter().take(1).cloned());
            owner.project(&mut snapshot);
            let job = owner.take().unwrap();
            let mut row = snapshot.messages.as_slice()[0].clone();
            if expired {
                row.metadata.expires_at = Some(1);
            } else {
                row.metadata.deleted = true;
                row.body = text("");
            }
            snapshot.messages = page([row]);
            owner.project(&mut snapshot);
            assert!(!owner.finish(context, prepare(job)));
            assert!(owner.ready.is_empty());
            assert!(owner.pending.is_empty());
        }
    }
}
