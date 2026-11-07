//! Short-lived alerts from the account's admitted Notifications observation.
//! Selection does not confer eligibility and switching rooms does not discard alerts.
use super::*;
use std::collections::VecDeque;
use talk_core::preferences::{
    Locale,
    notifications::{
        NotificationEvent, NotificationId, NotificationKind, NotificationSources,
        SourceAvailability,
    },
};
use talk_core::ui::conversation::NotificationLevel;

const CAPACITY: usize = 16;
const LIFETIME: Duration = Duration::from_secs(5);
struct MessageNotice {
    scope: AccountScope,
    value: protocol::MessageNotice,
    received: Instant,
}
#[derive(Default)]
pub(super) struct Notices {
    messages: VecDeque<MessageNotice>,
}
impl Notices {
    pub(super) fn retain_accounts(&mut self, scopes: &[AccountScope]) {
        self.messages.retain(|n| scopes.contains(&n.scope));
    }
    pub(super) fn remove_room(&mut self, context: ConversationRef) {
        self.messages
            .retain(|n| n.scope != context.account || n.value.conversation != context.conversation);
    }
    fn remove_account(&mut self, scope: AccountScope) {
        self.messages.retain(|n| n.scope != scope);
    }
    fn push(&mut self, scope: AccountScope, value: protocol::MessageNotice, now: Instant) {
        if value.conversation.account != scope.account
            || self.messages.iter().any(|n| {
                n.scope == scope
                    && n.value.conversation == value.conversation
                    && n.value.message == value.message
                    && n.value.reminder == value.reminder
                    && (!value.reminder || n.value.notification_id == value.notification_id)
            })
        {
            return;
        }
        if self.messages.len() == CAPACITY {
            self.messages.pop_front();
        }
        self.messages.push_back(MessageNotice {
            scope,
            value,
            received: now,
        });
    }
}
impl Accounts {
    pub(super) fn complete_message_notifications(
        &mut self,
        scope: AccountScope,
        result: Result<RuntimeReply, IdentityError>,
    ) {
        let failure = match &result {
            Err(IdentityError::Request(kind)) => Some(*kind),
            _ => None,
        };
        let observed = match result {
            Ok(RuntimeReply::Protocol(ProtocolReply::MessageNotifications(value)))
                if value.scope == scope && self.current(scope).is_ok() =>
            {
                Some(value)
            }
            _ => None,
        };
        if let Some(value) = observed {
            self.background.notification_source(
                scope,
                background::Observation::Messages,
                value.source,
                Instant::now(),
            );
            if value.source != protocol::NotificationSourceState::Available {
                self.notices.remove_account(scope);
                return;
            }
            if value.changed {
                self.background
                    .invalidate(scope, background::Observation::Rooms, Instant::now());
            }
            for notice in value.notices.as_slice() {
                self.notices.push(scope, notice.clone(), Instant::now());
            }
        } else {
            if let Some(kind) = failure {
                let source = if kind == ErrorKind::Unsupported {
                    protocol::NotificationSourceState::Unsupported
                } else {
                    protocol::NotificationSourceState::Paused {
                        reason: kind,
                        retry_after_millis: None,
                    }
                };
                self.background.notification_source(
                    scope,
                    background::Observation::Messages,
                    source,
                    Instant::now(),
                );
            } else {
                self.background
                    .observed(scope, background::Observation::Messages, false);
            }
            self.notices.remove_account(scope);
        }
    }
    pub(in crate::composition) fn notification_observations(&self) -> Vec<NotificationSources> {
        self.snapshot
            .accounts
            .as_slice()
            .iter()
            .take(8)
            .filter(|a| self.current(a.scope).is_ok())
            .map(|a| NotificationSources {
                scope: a.scope,
                revision: 0,
                messages: if self.background.source(
                    a.scope,
                    background::Observation::Messages,
                    Instant::now(),
                ) {
                    SourceAvailability::Available
                } else {
                    SourceAvailability::Unavailable
                },
                important: if self.background.source(
                    a.scope,
                    background::Observation::Messages,
                    Instant::now(),
                ) {
                    SourceAvailability::Available
                } else {
                    SourceAvailability::Unavailable
                },
                incoming_calls: if self.incoming_source_available(a.scope) {
                    SourceAvailability::Available
                } else {
                    SourceAvailability::Unavailable
                },
                calendar: SourceAvailability::Unavailable,
                ringing: self.ringing_call().filter(|call| call.account == a.scope),
            })
            .collect()
    }
    pub(in crate::composition) fn message_notification(
        &mut self,
        background: bool,
        locale: Locale,
    ) -> Option<NotificationEvent> {
        loop {
            let notice = self.notices.messages.front()?;
            let current = self
                .snapshot
                .conversation_panel
                .as_ref()
                .filter(|p| {
                    p.context.account == notice.scope
                        && p.context.conversation == notice.value.conversation
                })
                .and_then(|p| p.details.as_ref());
            let eligible = notice.received.elapsed() < LIFETIME
                && self.current(notice.scope).is_ok()
                && self.background.source(
                    notice.scope,
                    background::Observation::Messages,
                    Instant::now(),
                )
                && background
                && (notice.value.reminder
                    || !current.is_some_and(|d| d.notifications == Some(NotificationLevel::Never)));
            if !eligible {
                self.notices.messages.pop_front();
                continue;
            }
            let redacted =
                notice.value.redacted || current.is_some_and(|d| d.sensitive || d.classified);
            let localized = |key| crate::presentation::localization::text(locale, key);
            return Some(NotificationEvent {
                request: talk_core::preferences::RequestId(0),
                kind: if notice.value.reminder {
                    NotificationKind::MessageReminder {
                        scope: notice.scope,
                        conversation: notice.value.conversation,
                        id: NotificationId(notice.value.notification_id),
                        message: notice.value.message,
                    }
                } else {
                    NotificationKind::Message {
                        scope: notice.scope,
                        conversation: notice.value.conversation,
                        id: NotificationId(notice.value.message),
                    }
                },
                title: if notice.value.reminder {
                    if redacted {
                        text(localized("Erinnerung"))
                    } else {
                        text(&format!(
                            "{} · {}",
                            localized("Erinnerung"),
                            notice.value.title.as_str()
                        ))
                    }
                } else if redacted {
                    text(localized("Neue Nachricht"))
                } else {
                    notice.value.title.clone()
                },
                body: if redacted {
                    text(localized("Öffne das Gespräch, um die Nachricht zu lesen."))
                } else {
                    notice.value.body.clone()
                },
                minimized: true,
                eligible: true,
            });
        }
    }
    pub(in crate::composition) fn settle_message_notification(&mut self, kind: NotificationKind) {
        let matches = self.notices.messages.front().is_some_and(|n| match kind {
            NotificationKind::Message {
                scope,
                conversation,
                id,
            } => {
                !n.value.reminder
                    && n.scope == scope
                    && n.value.conversation == conversation
                    && n.value.message == id.0
            }
            NotificationKind::MessageReminder {
                scope,
                conversation,
                id,
                message,
            } => {
                n.value.reminder
                    && n.scope == scope
                    && n.value.conversation == conversation
                    && n.value.notification_id == id.0
                    && n.value.message == message
            }
            _ => false,
        });
        if matches {
            self.notices.messages.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::ids::RuntimeGeneration;
    fn notice(context: ConversationRef, message: u64) -> protocol::MessageNotice {
        protocol::MessageNotice {
            conversation: context.conversation,
            message,
            notification_id: message,
            reminder: false,
            title: text("Other room"),
            body: text("New message"),
            redacted: false,
        }
    }
    #[test]
    fn background_queue_is_bounded_deduplicated_and_removed_by_exact_runtime_or_room() {
        let context = super::super::super::fixtures::context(1);
        let mut other = context;
        other.conversation.local = 2;
        let mut queue = Notices::default();
        for id in 1..=20 {
            queue.push(context.account, notice(context, id), Instant::now());
        }
        assert_eq!(queue.messages.len(), CAPACITY);
        queue.push(context.account, notice(context, 20), Instant::now());
        assert_eq!(queue.messages.len(), CAPACITY);
        queue.push(other.account, notice(other, 20), Instant::now());
        queue.remove_room(context);
        assert_eq!(queue.messages.len(), 1);
        assert_eq!(queue.messages[0].value.conversation, other.conversation);
        queue.retain_accounts(&[AccountScope {
            runtime: RuntimeGeneration::from_local(99),
            ..context.account
        }]);
        assert!(queue.messages.is_empty());
    }
    #[test]
    fn reminders_keep_their_own_event_identity_when_the_same_message_was_notified() {
        let context = super::super::super::fixtures::context(1);
        let mut queue = Notices::default();
        queue.push(context.account, notice(context, 42), Instant::now());
        let mut reminder = notice(context, 42);
        reminder.reminder = true;
        reminder.notification_id = 100;
        queue.push(context.account, reminder.clone(), Instant::now());
        queue.push(context.account, reminder.clone(), Instant::now());
        reminder.notification_id = 101;
        queue.push(context.account, reminder, Instant::now());
        assert_eq!(queue.messages.len(), 3);
    }

    #[test]
    fn switching_selection_does_not_drop_a_background_notice_but_stale_completion_does() {
        let mut model = super::super::tests::model();
        let context = super::super::super::fixtures::context(1);
        model
            .notices
            .push(context.account, notice(context, 42), Instant::now());
        model.clear_content();
        assert_eq!(model.notices.messages.len(), 1);
        let result = protocol::MessageNotifications {
            source: protocol::NotificationSourceState::Available,
            scope: context.account,
            notices: page([notice(context, 43)]),
            changed: true,
        };
        model.complete_message_notifications(
            context.account,
            Ok(RuntimeReply::Protocol(ProtocolReply::MessageNotifications(
                result,
            ))),
        );
        assert!(
            model.notices.messages.is_empty(),
            "an unadmitted runtime cannot supply alerts"
        );
    }
    #[test]
    fn repeated_live_hints_neither_emit_nor_clear_queued_notifications() {
        let mut model = super::super::tests::model();
        let context = super::super::super::fixtures::context(1);
        let now = Instant::now();
        model.background.sync(&[context.account], now);
        model
            .notices
            .push(context.account, notice(context, 42), now);
        for _ in 0..100 {
            model.apply_live_events(
                context.account,
                Ok(RuntimeReply::Protocol(ProtocolReply::LiveEvents(
                    protocol::AccountLiveEvents {
                        scope: context.account,
                        state: protocol::AccountLiveState::Reconnecting,
                        first_transport_failure: Some(ErrorKind::Transport),
                        changes: page([
                            protocol::AccountChange::IncomingCalls,
                            protocol::AccountChange::Messages,
                            protocol::AccountChange::Conversations,
                        ]),
                    },
                ))),
                now,
            );
        }
        assert_eq!(model.notices.messages.len(), 1);
        assert_eq!(model.notices.messages[0].value.message, 42);
    }
}
