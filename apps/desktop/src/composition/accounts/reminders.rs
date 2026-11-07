//! Bounded account reminder observations. A selected room is not required.
use super::*;
use talk_core::ui::presentation::{AccountReminderView, AccountRemindersView};

struct Entry {
    scope: AccountScope,
    observation: Option<protocol::messaging::AccountReminders>,
    due: Instant,
    failures: u8,
    version: u64,
}

#[derive(Default)]
pub(super) struct Reminders {
    entries: Vec<Entry>,
}
impl Reminders {
    pub(super) fn retain_accounts(&mut self, scopes: &[AccountScope]) {
        self.entries.retain(|entry| scopes.contains(&entry.scope));
    }
    fn entry(&mut self, scope: AccountScope) -> Option<&mut Entry> {
        if !self.entries.iter().any(|entry| entry.scope == scope) {
            if self.entries.len() == 8 {
                return None;
            }
            self.entries.push(Entry {
                scope,
                observation: None,
                due: Instant::now(),
                failures: 0,
                version: 0,
            });
        }
        self.entries.iter_mut().find(|entry| entry.scope == scope)
    }
}

impl Accounts {
    pub(super) fn refresh_account_reminders(&mut self, scope: AccountScope) {
        if let Some(entry) = self.reminders.entry(scope) {
            entry.due = Instant::now();
        }
    }

    pub(super) fn reminder_changed(&mut self, scope: AccountScope) {
        if let Some(entry) = self.reminders.entry(scope) {
            entry.version = entry.version.saturating_add(1);
            entry.due = Instant::now();
        }
    }

    pub(super) fn observe_reminder_result(
        &mut self,
        context: ConversationRef,
        message: u128,
        result: &Result<protocol::messaging::MessagingResult, Completion>,
    ) {
        self.reminder_changed(context.account);
        let Ok(protocol::messaging::MessagingResult::Reminder(value)) = result else {
            return;
        };
        let Some(entry) = self.reminders.entry(context.account) else {
            return;
        };
        let Some(observation) = entry.observation.as_mut() else {
            return;
        };
        if value.is_some()
            && observation.reminders.as_slice().len() == 100
            && !observation.reminders.as_slice().iter().any(|r| {
                r.message.conversation == context.conversation
                    && u128::from(r.message.sequence) == message
            })
        {
            observation.complete = false;
        }
        observation.reminders = page(
            observation
                .reminders
                .as_slice()
                .iter()
                .filter(|r| {
                    r.message.conversation != context.conversation
                        || u128::from(r.message.sequence) != message
                })
                .take(if value.is_some() { 99 } else { 100 })
                .cloned()
                .chain(value.iter().cloned()),
        );
    }

    pub(super) fn tick_account_reminders(&mut self) {
        self.project_account_reminders();
        let Some(scope) = self.snapshot.active_account else {
            return;
        };
        if self.snapshot.conversation_list.content != ContentState::Ready
            || self.pending.len() >= 12
            || self.pending.iter().any(
                |pending| matches!(pending.purpose, Purpose::AccountReminders(s, _) if s == scope),
            )
        {
            return;
        }
        let now = Instant::now();
        let Some(entry) = self.reminders.entry(scope) else {
            return;
        };
        if now < entry.due {
            return;
        }
        // Admission failure is retried quietly and cannot spin on every UI tick.
        entry.due = now + Duration::from_secs(5);
        let version = entry.version;
        let _ = self.request(
            scope,
            ProtocolRequest::UpcomingReminders,
            Purpose::AccountReminders(scope, version),
        );
    }

    pub(super) fn complete_account_reminders(
        &mut self,
        scope: AccountScope,
        version: u64,
        result: Result<RuntimeReply, IdentityError>,
    ) {
        if self.current(scope).is_err() {
            return;
        }
        let Some(entry) = self.reminders.entry(scope) else {
            return;
        };
        if entry.version != version {
            entry.due = Instant::now();
            return;
        }
        match result {
            Ok(RuntimeReply::Protocol(ProtocolReply::UpcomingReminders(observation)))
                if observation.scope == scope
                    && observation.reminders.as_slice().iter().all(|r| {
                        r.message.conversation.account == scope.account && r.message.sequence > 0
                    }) =>
            {
                entry.failures = 0;
                entry.due = Instant::now()
                    + if observation.supported {
                        Duration::from_secs(60)
                    } else {
                        Duration::from_secs(3600)
                    };
                entry.observation = Some(observation);
            }
            _ => {
                entry.failures = entry.failures.saturating_add(1).min(6);
                entry.due = Instant::now() + Duration::from_secs((5u64 << entry.failures).min(300));
            }
        }
        self.project_account_reminders();
    }

    pub(super) fn project_account_reminders(&mut self) {
        let projected = self.snapshot.active_account.and_then(|scope| {
            let observation = self
                .reminders
                .entries
                .iter()
                .find(|e| e.scope == scope)?
                .observation
                .as_ref()?;
            let rooms = self.snapshot.conversations.as_slice();
            let mut complete = observation.complete;
            let mut reminders = Vec::new();
            for reminder in observation.reminders.as_slice() {
                let context = ConversationRef {
                    account: scope,
                    conversation: reminder.message.conversation,
                };
                if !context.is_consistent() || !rooms.iter().any(|room| room.context == context) {
                    complete = false;
                    continue;
                }
                if !reminders.iter().any(|r: &AccountReminderView| {
                    r.context == context && r.message == u128::from(reminder.message.sequence)
                }) {
                    reminders.push(AccountReminderView {
                        context,
                        message: u128::from(reminder.message.sequence),
                        timestamp: reminder.timestamp,
                    });
                }
            }
            self.outbox.project_reminders(scope, rooms, &mut reminders);
            reminders.sort_by_key(|reminder| reminder.timestamp);
            Some(AccountRemindersView {
                scope,
                support: if observation.supported {
                    FeatureSupport::Supported
                } else {
                    FeatureSupport::Unsupported
                },
                state: ContentState::Ready,
                complete,
                reminders: page(reminders),
            })
        });
        if self.snapshot.account_reminders != projected {
            self.snapshot.account_reminders = projected;
            self.touch();
        }
    }

    pub(super) fn open_reminder(
        &mut self,
        request: RequestId,
        context: ConversationRef,
        message: u128,
    ) -> Result<(), SubmitError> {
        if !self
            .snapshot
            .account_reminders
            .as_ref()
            .is_some_and(|view| {
                view.scope == context.account
                    && view
                        .reminders
                        .as_slice()
                        .iter()
                        .any(|r| r.context == context && r.message == message)
            })
        {
            return Err(SubmitError::StaleScope);
        }
        self.open_message_target(request, context, message)
    }

    pub(super) fn open_notification_message(
        &mut self,
        request: RequestId,
        context: ConversationRef,
        message: u128,
    ) -> Result<(), SubmitError> {
        self.current(context.account)?;
        if !context.is_consistent()
            || !self
                .background
                .account(context.account)
                .is_some_and(|account| {
                    account
                        .rooms
                        .as_slice()
                        .iter()
                        .any(|room| room.context == context)
                })
        {
            return Err(SubmitError::StaleScope);
        }
        self.select(Some(context.account));
        self.open_message_target(request, context, message)
    }

    pub(super) fn open_message_target(
        &mut self,
        request: RequestId,
        context: ConversationRef,
        message: u128,
    ) -> Result<(), SubmitError> {
        if !context.is_consistent() || u64::try_from(message).ok().is_none_or(|id| id == 0) {
            return Err(SubmitError::StaleScope);
        }
        self.open(context)?;
        if self
            .snapshot
            .messages
            .as_slice()
            .iter()
            .any(|row| row.local_id == message)
            && let Some(panel) = self
                .snapshot
                .message_panel
                .as_mut()
                .filter(|panel| panel.context == context)
        {
            panel.located = Some((request, message));
            self.touch();
            return Ok(());
        }
        self.message_action(
            request,
            context,
            talk_core::ui::messaging::MessageAction::Context(message),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::protocol::messaging::{AccountReminders, MessageReminder};

    #[test]
    fn projection_is_account_scoped_sorted_and_reports_unknown_rooms() {
        let mut model = super::super::tests::model();
        model.snapshot = super::super::super::fixtures::snapshot(Some("chat-ux"));
        let scope = model.snapshot.active_account.unwrap();
        let rooms = model.snapshot.conversations.as_slice();
        let first = rooms[0].context.conversation;
        let second = rooms[1].context.conversation;
        let unknown = super::super::super::fixtures::context(999).conversation;
        model.reminders.entries.push(Entry {
            scope,
            observation: Some(AccountReminders {
                scope,
                supported: true,
                complete: true,
                reminders: page([
                    MessageReminder {
                        message: protocol::MessageId {
                            conversation: first,
                            sequence: 7,
                        },
                        timestamp: 300,
                    },
                    MessageReminder {
                        message: protocol::MessageId {
                            conversation: second,
                            sequence: 8,
                        },
                        timestamp: 200,
                    },
                    MessageReminder {
                        message: protocol::MessageId {
                            conversation: unknown,
                            sequence: 9,
                        },
                        timestamp: 100,
                    },
                ]),
            }),
            due: Instant::now(),
            failures: 0,
            version: 0,
        });
        model.project_account_reminders();
        let view = model.snapshot.account_reminders.as_ref().unwrap();
        assert!(!view.complete);
        assert_eq!(
            view.reminders
                .as_slice()
                .iter()
                .map(|r| r.message)
                .collect::<Vec<_>>(),
            vec![8, 7]
        );
        model.snapshot.active_account = None;
        model.project_account_reminders();
        assert!(model.snapshot.account_reminders.is_none());
    }
}
