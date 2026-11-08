//! Deliberate room entry; navigation generations fence every completion.
use super::*;
use talk_core::ui::conversation::RemoteState;

impl Accounts {
    pub(super) fn public_room(&self, context: ConversationRef) -> bool {
        self.snapshot
            .conversations
            .as_slice()
            .iter()
            .any(|row| row.context == context && row.kind == ConversationKind::GuestAccessible)
    }

    pub(super) fn start_open(
        &mut self,
        context: ConversationRef,
        password: Option<Text<512>>,
    ) -> Result<(), SubmitError> {
        self.current(context.account)?;
        if self.snapshot.selected != Some(context) {
            return Err(SubmitError::StaleScope);
        }
        if self
            .pending
            .iter()
            .any(|p| matches!(p.purpose, Purpose::Open(c, g) if c == context && g == self.room_open_generation))
        {
            return Err(SubmitError::Busy);
        }
        self.room_open_generation = self.room_open_generation.saturating_add(1);
        self.request(
            context.account,
            ProtocolRequest::OpenConversation {
                conversation: context.conversation,
                password,
            },
            Purpose::Open(context, self.room_open_generation),
        )?;
        self.snapshot.content = ContentState::Loading;
        self.touch();
        Ok(())
    }

    pub(super) fn complete_open(
        &mut self,
        context: ConversationRef,
        generation: u64,
        result: Result<RuntimeReply, IdentityError>,
    ) {
        if self.current(context.account).is_err() {
            return;
        }
        self.apply_open(context, generation, result);
    }

    fn apply_open(
        &mut self,
        context: ConversationRef,
        generation: u64,
        result: Result<RuntimeReply, IdentityError>,
    ) {
        if self.snapshot.selected != Some(context) || generation != self.room_open_generation {
            return;
        }
        match result {
            Ok(RuntimeReply::Protocol(ProtocolReply::Conversation(room)))
                if room.id == context.conversation =>
            {
                if self
                    .discovery_join
                    .as_ref()
                    .is_some_and(|join| join.context == context)
                {
                    self.discovery_join = None;
                }
                let row = conversation(&room, context.account);
                self.snapshot.conversations =
                    page(self.snapshot.conversations.as_slice().iter().map(|old| {
                        if old.context == context {
                            row.clone()
                        } else {
                            old.clone()
                        }
                    }));
                self.observe_created_room(context.account, room);
                if let Some(panel) = &mut self.snapshot.message_panel {
                    panel.history = RemoteState::Idle;
                }
                let _ = self.start_messages(context);
            }
            result => {
                let reason = match result {
                    Err(IdentityError::Request(ErrorKind::NotFound)) => {
                        UnavailableReason::PermissionDenied
                    }
                    Err(error) => match submit_error(error) {
                        SubmitError::Unavailable(reason) => reason,
                        SubmitError::Busy => UnavailableReason::Busy,
                        _ => UnavailableReason::PolicyStale,
                    },
                    _ => UnavailableReason::PolicyStale,
                };
                if !matches!(
                    reason,
                    UnavailableReason::PasswordRequired
                        | UnavailableReason::Lobby
                        | UnavailableReason::Connection(_)
                ) && self
                    .discovery_join
                    .as_ref()
                    .is_some_and(|join| join.context == context)
                {
                    self.discovery_join = None;
                }
                if matches!(reason, UnavailableReason::Connection(_)) {
                    if let Some(panel) = &mut self.snapshot.message_panel {
                        panel.history = RemoteState::Failed(RequestId(0), reason);
                    }
                    self.snapshot.content = ContentState::Offline;
                    self.touch();
                    return;
                }
                self.invalidate_history(
                    context,
                    if reason == UnavailableReason::PermissionDenied {
                        ContentState::Denied
                    } else {
                        ContentState::Failed
                    },
                );
                if let Some(panel) = &mut self.snapshot.message_panel {
                    panel.history = RemoteState::Failed(RequestId(0), reason);
                }
                self.snapshot.conversations = page(
                    self.snapshot
                        .conversations
                        .as_slice()
                        .iter()
                        .cloned()
                        .map(|mut row| {
                            if row.context == context {
                                row.send = Decision::Unknown;
                                row.attach = Decision::Unknown;
                            }
                            row
                        }),
                );
                self.snapshot.content = if reason == UnavailableReason::PermissionDenied {
                    ContentState::Denied
                } else {
                    ContentState::Failed
                };
            }
        }
        self.touch();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_navigation_cannot_apply_room_entry_failures() {
        let mut model = super::super::tests::model();
        model.snapshot = crate::composition::fixtures::snapshot(Some("chat"));
        let context = model.snapshot.selected.unwrap();
        model.room_open_generation = 3;
        let before = model.snapshot.clone();
        model.apply_open(
            context,
            2,
            Err(IdentityError::Request(ErrorKind::PermissionDenied)),
        );
        assert_eq!(model.snapshot, before);
        model.clear_content();
        model.apply_open(
            context,
            3,
            Err(IdentityError::Request(ErrorKind::PasswordRequired)),
        );
        assert!(model.snapshot.message_panel.is_none());
    }

    #[test]
    fn password_failure_discards_old_history_and_keeps_a_specific_retry_state() {
        let mut model = super::super::tests::model();
        model.snapshot = crate::composition::fixtures::snapshot(Some("chat"));
        let context = model.snapshot.selected.unwrap();
        model.apply_open(
            context,
            0,
            Err(IdentityError::Request(ErrorKind::PasswordRequired)),
        );
        assert_eq!(model.snapshot.content, ContentState::Failed);
        assert!(model.snapshot.messages.as_slice().is_empty());
        assert_eq!(
            model.snapshot.message_panel.as_ref().unwrap().history,
            RemoteState::Failed(RequestId(0), UnavailableReason::PasswordRequired)
        );
    }
}
