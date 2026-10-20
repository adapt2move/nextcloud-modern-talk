//! Short-lived message alerts from the existing admitted poll. No background history replay.
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
    context: ConversationRef,
    id: u64,
    title: Text<128>,
    body: Text<512>,
    redacted: bool,
    level: Option<NotificationLevel>,
    received: Instant,
}
#[derive(Default)]
pub(super) struct Notices {
    messages: VecDeque<MessageNotice>,
}
impl Notices {
    pub fn clear(&mut self) {
        self.messages.clear();
    }
}
impl Accounts {
    pub(super) fn observe_message_notices(
        &mut self,
        context: ConversationRef,
        messages: &Page<protocol::messaging::MessageDetails, 50>,
        after: Option<u128>,
    ) {
        let Some(after) = after else {
            return;
        };
        let Some(details) = self
            .snapshot
            .conversation_panel
            .as_ref()
            .filter(|p| p.context == context)
            .and_then(|p| p.details.as_ref())
        else {
            return;
        };
        if matches!(details.notifications, None | Some(NotificationLevel::Never)) {
            return;
        }
        let redacted = details.sensitive || details.classified;
        for message in messages.as_slice() {
            if !new_incoming(message, after) {
                continue;
            }
            if self.notices.messages.len() == CAPACITY {
                self.notices.messages.pop_front();
            }
            self.notices.messages.push_back(MessageNotice {
                context,
                id: message.message.id.sequence,
                title: clean(details.name.as_str()),
                body: clean(&format!(
                    "{}: {}",
                    message.message.author.as_str(),
                    message
                        .message
                        .text
                        .as_str()
                        .chars()
                        .take(512)
                        .collect::<String>()
                )),
                redacted,
                level: details.notifications,
                received: Instant::now(),
            });
        }
    }
    /// A source observation grants no eligibility: each alert still needs fresh server evidence.
    pub(in crate::composition) fn notification_observations(&self) -> Vec<NotificationSources> {
        self.snapshot
            .accounts
            .as_slice()
            .iter()
            .take(8)
            .filter(|a| self.current(a.scope).is_ok())
            .map(|a| {
                let messages = if self.snapshot.active_account == Some(a.scope)
                    && self.snapshot.content == ContentState::Ready
                    && self.snapshot.message_panel.is_some()
                {
                    SourceAvailability::Available
                } else {
                    SourceAvailability::Unavailable
                };
                NotificationSources {
                    scope: a.scope,
                    revision: 0,
                    messages,
                    important: SourceAvailability::Unavailable,
                    incoming_calls: if self.snapshot.active_account == Some(a.scope)
                        && self.incoming_source_available()
                    {
                        SourceAvailability::Available
                    } else {
                        SourceAvailability::Unavailable
                    },
                    calendar: SourceAvailability::Unavailable,
                    ringing: self.ringing_call().filter(|call| call.account == a.scope),
                }
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
                .filter(|p| p.context == notice.context)
                .and_then(|p| p.details.as_ref());
            let eligible = notice.received.elapsed() < LIFETIME
                && self.current(notice.context.account).is_ok()
                && self.snapshot.selected == Some(notice.context)
                && self.snapshot.content == ContentState::Ready
                && background
                && current.is_some_and(|d| {
                    d.notifications == notice.level
                        && d.notifications != Some(NotificationLevel::Never)
                });
            if !eligible {
                self.notices.messages.pop_front();
                continue;
            }
            let redacted = notice.redacted || current.is_some_and(|d| d.sensitive || d.classified);
            let localized = |key| crate::presentation::localization::text(locale, key);
            return Some(NotificationEvent {
                request: talk_core::preferences::RequestId(0),
                kind: NotificationKind::Message {
                    scope: notice.context.account,
                    conversation: notice.context.conversation,
                    id: NotificationId(notice.id),
                },
                title: if redacted {
                    clean(localized("Neue Nachricht"))
                } else {
                    notice.title.clone()
                },
                body: if redacted {
                    clean(localized("Öffne das Gespräch, um die Nachricht zu lesen."))
                } else {
                    notice.body.clone()
                },
                minimized: true,
                eligible: true,
            });
        }
    }
    pub(in crate::composition) fn settle_message_notification(&mut self, kind: NotificationKind) {
        if let NotificationKind::Message {
            scope,
            conversation,
            id,
        } = kind
            && self.notices.messages.front().is_some_and(|n| {
                n.context.account == scope && n.context.conversation == conversation && n.id == id.0
            })
        {
            self.notices.messages.pop_front();
        }
    }
}
fn clean<const N: usize>(value: &str) -> Text<N> {
    text(
        &value
            .chars()
            .filter(|c| !c.is_control())
            .take(N)
            .collect::<String>(),
    )
}
fn new_incoming(message: &protocol::messaging::MessageDetails, after: u128) -> bool {
    message.notify == Decision::Allowed
        && !message.own
        && u128::from(message.message.id.sequence) > after
        && message.message.kind == protocol::MessageKind::Comment
        && message.message.edited_at.is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notifications_require_new_authorized_incoming_comments() {
        let context = super::super::super::fixtures::context(1);
        let mut message = protocol::messaging::MessageDetails {
            message: protocol::Message {
                id: protocol::MessageId {
                    conversation: context.conversation,
                    sequence: 11,
                },
                author: text("Bob"),
                text: text("Hello"),
                kind: protocol::MessageKind::Comment,
                timestamp: 1,
                expires_at: None,
                edited_at: None,
                own_operation: None,
            },
            own: false,
            notify: Decision::Allowed,
            replyable: Some(true),
            parent: None,
            thread: None,
            reactions: Page::empty(),
            rich: Page::empty(),
            system: None,
            edit: Decision::Denied,
            delete: Decision::Denied,
        };
        assert!(new_incoming(&message, 10));
        assert!(!new_incoming(&message, 11));
        message.own = true;
        assert!(!new_incoming(&message, 10));
        message.own = false;
        message.notify = Decision::Unknown;
        assert!(!new_incoming(&message, 10));
        message.notify = Decision::Allowed;
        message.message.edited_at = Some(2);
        assert!(!new_incoming(&message, 10));
        message.message.edited_at = None;
        message.message.kind = protocol::MessageKind::Deleted;
        assert!(!new_incoming(&message, 10));
        message.message.kind = protocol::MessageKind::System;
        assert!(!new_incoming(&message, 10));
    }
}
