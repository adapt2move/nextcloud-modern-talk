//! One account/room typing watch and one bounded, replaceable activity intent.
//! The signaling owner owns session identifiers and all network work.
use super::*;
use talk_core::ui::messaging::{MessageFeature, TypingPerson};

#[derive(Clone, Copy)]
pub(super) enum Work {
    Start,
    Stop,
    Activity(bool, u64),
    Poll,
}
pub(super) struct Typing {
    current: Option<ConversationRef>,
    target: Option<ConversationRef>,
    desired: bool,
    dirty: bool,
    revision: u64,
    failures: u8,
    started: Option<Instant>,
    due: Instant,
    seen: Vec<u128>,
    suppressed: Vec<Text<256>>,
    initialized: bool,
}
impl Default for Typing {
    fn default() -> Self {
        Self {
            current: None,
            target: None,
            desired: false,
            dirty: false,
            revision: 0,
            failures: 0,
            started: None,
            due: Instant::now(),
            seen: Vec::new(),
            suppressed: Vec::new(),
            initialized: false,
        }
    }
}
impl Accounts {
    pub(super) fn typing_activity(
        &mut self,
        context: ConversationRef,
        active: bool,
    ) -> Result<(), SubmitError> {
        if active
            && (self.snapshot.selected != Some(context)
                || !self.typing_supported(context)
                || self
                    .snapshot
                    .conversations
                    .as_slice()
                    .iter()
                    .find(|r| r.context == context)
                    .is_none_or(|r| r.send != Decision::Allowed))
        {
            return Err(SubmitError::StaleScope);
        }
        if active && self.typing.target != Some(context) {
            self.typing.target = Some(context);
            self.typing.seen.clear();
            self.typing.suppressed.clear();
            self.typing.initialized = false;
        }
        if self.typing.target == Some(context) || self.typing.current == Some(context) {
            self.typing.revision = self
                .typing
                .revision
                .checked_add(1)
                .ok_or(SubmitError::Busy)?;
            self.typing.desired = active;
            self.typing.dirty = true;
            self.typing.due = Instant::now();
        }
        Ok(())
    }
    fn typing_supported(&self, context: ConversationRef) -> bool {
        self.snapshot
            .message_panel
            .as_ref()
            .filter(|panel| panel.context == context)
            .and_then(|panel| {
                panel
                    .features
                    .as_slice()
                    .iter()
                    .find(|(feature, _)| *feature == MessageFeature::Typing)
            })
            .is_some_and(|(_, availability)| availability.enabled())
    }
    fn typing_target(&self) -> Option<ConversationRef> {
        let context = self.snapshot.selected?;
        (self.snapshot.active_account == Some(context.account)
            && self.snapshot.content == ContentState::Ready
            && self.typing_supported(context))
        .then_some(context)
    }
    pub(super) fn tick_typing(&mut self) {
        let target = self.typing_target();
        if self.typing.target != target {
            self.typing.target = target;
            self.typing.desired = false;
            self.typing.dirty = false;
            self.typing.failures = 0;
            self.typing.started = None;
            self.typing.due = Instant::now();
            self.typing.seen.clear();
            self.typing.suppressed.clear();
            self.typing.initialized = false;
            if let Some(panel) = self.snapshot.message_panel.as_mut() {
                panel.typing = Page::empty();
            }
        }
        self.observe_typing_messages();
        if self
            .pending
            .iter()
            .any(|p| matches!(p.purpose, Purpose::Typing(_, _)))
            || Instant::now() < self.typing.due
        {
            return;
        }
        let next = match (self.typing.current, target) {
            (Some(old), new) if Some(old) != new => {
                Some((old, Work::Stop, ProtocolRequest::StopTyping))
            }
            (None, Some(context)) => Some((
                context,
                Work::Start,
                ProtocolRequest::WatchTyping(context.conversation),
            )),
            (Some(context), Some(_)) if self.typing.dirty => Some((
                context,
                Work::Activity(self.typing.desired, self.typing.revision),
                ProtocolRequest::TypingActivity {
                    conversation: context.conversation,
                    active: self.typing.desired,
                },
            )),
            (Some(context), Some(_)) => Some((
                context,
                Work::Poll,
                ProtocolRequest::PollTyping(context.conversation),
            )),
            _ => None,
        };
        if let Some((context, work, command)) = next
            && self
                .request(context.account, command, Purpose::Typing(context, work))
                .is_err()
        {
            if matches!(work, Work::Stop) && self.current(context.account).is_err() {
                self.typing.current = None;
            }
            self.typing.due = Instant::now() + Duration::from_secs(2);
        }
    }
    fn observe_typing_messages(&mut self) {
        if self.typing.target.is_none() {
            return;
        }
        for message in self.snapshot.messages.as_slice() {
            if self.typing.initialized
                && !self.typing.seen.contains(&message.local_id)
                && !message.own
                && let Some(key) = &message.metadata.author_key
                && self.typing.suppressed.len() < 16
                && !self.typing.suppressed.contains(key)
            {
                self.typing.suppressed.push(key.clone());
            }
        }
        self.typing.seen = self
            .snapshot
            .messages
            .as_slice()
            .iter()
            .rev()
            .take(100)
            .map(|m| m.local_id)
            .collect();
        self.typing.initialized = true;
        if let Some(panel) = self
            .snapshot
            .message_panel
            .as_mut()
            .filter(|panel| Some(panel.context) == self.typing.target)
        {
            let retained = page(
                panel
                    .typing
                    .as_slice()
                    .iter()
                    .filter(|person| !self.typing.suppressed.contains(&person.author_key))
                    .cloned(),
            );
            if panel.typing != retained {
                panel.typing = retained;
                self.touch();
            }
        }
    }
    fn typing_retry(&mut self) {
        self.typing.failures = self.typing.failures.saturating_add(1).min(4);
        self.typing.due = Instant::now() + Duration::from_secs(2u64 << self.typing.failures);
    }
    pub(super) fn complete_typing(
        &mut self,
        context: ConversationRef,
        work: Work,
        result: Result<RuntimeReply, IdentityError>,
    ) {
        let accepted = matches!(
            &result,
            Ok(RuntimeReply::Protocol(ProtocolReply::TypingAccepted))
        );
        match work {
            Work::Start => {
                if accepted {
                    self.typing.current = Some(context);
                    self.typing.started = Some(Instant::now());
                    self.typing.due = Instant::now();
                } else {
                    self.typing_retry();
                }
            }
            Work::Stop => {
                self.typing.current = None;
                self.typing.due = Instant::now();
            }
            Work::Activity(active, revision) => {
                if (accepted || !active)
                    && self.typing.revision == revision
                    && self.typing.desired == active
                {
                    self.typing.dirty = false;
                }
                self.typing.due =
                    Instant::now() + Duration::from_millis(if accepted { 250 } else { 1000 });
            }
            Work::Poll => {
                self.typing.due = Instant::now() + Duration::from_millis(750);
                if self.typing.target != Some(context) || self.snapshot.selected != Some(context) {
                    return;
                }
                if let Ok(RuntimeReply::Protocol(ProtocolReply::Typing(observation))) = result {
                    if observation.conversation != context.conversation {
                        return;
                    }
                    if !observation.running {
                        self.typing.current = None;
                        self.typing_retry();
                    } else if self
                        .typing
                        .started
                        .is_some_and(|at| at.elapsed() >= Duration::from_secs(15))
                    {
                        self.typing.failures = 0;
                    }
                    self.typing.suppressed.retain(|key| {
                        observation
                            .people
                            .as_slice()
                            .iter()
                            .any(|person| &person.author_key == key)
                    });
                    let people = page(
                        observation
                            .people
                            .as_slice()
                            .iter()
                            .filter(|person| !self.typing.suppressed.contains(&person.author_key))
                            .map(|person| TypingPerson {
                                name: person.name.clone(),
                                author_key: person.author_key.clone(),
                            }),
                    );
                    if let Some(panel) = self
                        .snapshot
                        .message_panel
                        .as_mut()
                        .filter(|panel| panel.context == context)
                        && panel.typing != people
                    {
                        panel.typing = people;
                        self.touch();
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn observation(
        context: ConversationRef,
        names: &[(&str, &str)],
    ) -> Result<RuntimeReply, IdentityError> {
        Ok(RuntimeReply::Protocol(ProtocolReply::Typing(
            protocol::TypingObservation {
                conversation: context.conversation,
                running: true,
                people: page(names.iter().map(|(name, key)| protocol::TypingPerson {
                    name: text(name),
                    author_key: text(key),
                })),
            },
        )))
    }
    #[test]
    fn new_message_clears_that_author_until_the_signal_disappears() {
        let mut model = super::super::tests::model();
        model.snapshot = super::super::super::fixtures::snapshot(Some("chat-typing"));
        let context = model.snapshot.selected.unwrap();
        model.typing.target = Some(context);
        model.typing.current = Some(context);
        model.observe_typing_messages();
        model.complete_typing(
            context,
            Work::Poll,
            observation(
                context,
                &[("Mara", "fixture-mara"), ("Jonas", "fixture-jonas")],
            ),
        );
        assert_eq!(
            model
                .snapshot
                .message_panel
                .as_ref()
                .unwrap()
                .typing
                .as_slice()
                .len(),
            2
        );
        let mut message = model.snapshot.messages.as_slice()[0].clone();
        message.local_id = 999;
        message.own = false;
        message.metadata.author_key = Some(text("fixture-mara"));
        model.snapshot.messages = page(
            model
                .snapshot
                .messages
                .as_slice()
                .iter()
                .cloned()
                .chain([message]),
        );
        model.observe_typing_messages();
        let shown = model
            .snapshot
            .message_panel
            .as_ref()
            .unwrap()
            .typing
            .as_slice();
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].name.as_str(), "Jonas");
        model.complete_typing(
            context,
            Work::Poll,
            observation(context, &[("Mara", "fixture-mara")]),
        );
        assert!(
            model
                .snapshot
                .message_panel
                .as_ref()
                .unwrap()
                .typing
                .as_slice()
                .is_empty()
        );
        model.complete_typing(context, Work::Poll, observation(context, &[]));
        model.complete_typing(
            context,
            Work::Poll,
            observation(context, &[("Mara", "fixture-mara")]),
        );
        assert_eq!(
            model
                .snapshot
                .message_panel
                .as_ref()
                .unwrap()
                .typing
                .as_slice()
                .len(),
            1
        );
    }
    #[test]
    fn stale_room_and_superseded_activity_cannot_restore_old_state() {
        let mut model = super::super::tests::model();
        model.snapshot = super::super::super::fixtures::snapshot(Some("chat-typing"));
        let context = model.snapshot.selected.unwrap();
        model.typing.target = Some(context);
        model.typing.current = Some(context);
        model.typing.desired = false;
        model.typing.revision = 8;
        model.typing.dirty = true;
        model.complete_typing(
            context,
            Work::Activity(true, 7),
            Ok(RuntimeReply::Protocol(ProtocolReply::TypingAccepted)),
        );
        assert!(model.typing.dirty);
        model.snapshot.selected = None;
        let before = model
            .snapshot
            .message_panel
            .as_ref()
            .unwrap()
            .typing
            .clone();
        model.complete_typing(
            context,
            Work::Poll,
            observation(context, &[("Other", "other")]),
        );
        assert_eq!(
            model.snapshot.message_panel.as_ref().unwrap().typing,
            before
        );
    }
}
