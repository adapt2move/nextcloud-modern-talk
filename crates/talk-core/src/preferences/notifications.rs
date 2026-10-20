//! Typed local notification policy. No event, source flag or action grants account/media authority.
//! Composition supplies fresh authenticated source evidence, eligibility and sanitized copy.
use super::{NotificationPreferences, RequestId};
use crate::{
    bounded::Text,
    ids::{AccountScope, ActorId, CallScope, ConversationId},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotificationCategory {
    Messages,
    Important,
    IncomingCalls,
    Calendar,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceAvailability {
    Unknown,
    Unavailable,
    Available,
}
/// A revisioned observation from the admitted account owner, not saved preferences.
/// Calendar availability requires its actual notification integration; DAV alone is insufficient.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NotificationSources {
    pub scope: AccountScope,
    pub revision: u64,
    pub messages: SourceAvailability,
    pub important: SourceAvailability,
    pub incoming_calls: SourceAvailability,
    pub calendar: SourceAvailability,
    /// Currently observed direct invitation. Clearing/replacing it cancels old ringing work.
    pub ringing: Option<CallScope>,
}
impl NotificationSources {
    pub fn availability(self, category: NotificationCategory) -> SourceAvailability {
        match category {
            NotificationCategory::Messages => self.messages,
            NotificationCategory::Important => self.important,
            NotificationCategory::IncomingCalls => self.incoming_calls,
            NotificationCategory::Calendar => self.calendar,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NotificationId(pub u64);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotificationKind {
    Message {
        scope: AccountScope,
        conversation: ConversationId,
        id: NotificationId,
    },
    /// An observed participant entering an owned conference call, never room membership alone.
    ConferenceParticipantJoined {
        call: CallScope,
        participant: ActorId,
        is_self: bool,
        id: NotificationId,
    },
    /// An actual personal server reminder, never a timer inferred from a message timestamp.
    MessageReminder {
        scope: AccountScope,
        conversation: ConversationId,
        id: NotificationId,
    },
    IncomingCall {
        call: CallScope,
    },
    Calendar {
        scope: AccountScope,
        id: NotificationId,
    },
    /// Explicit settings test. It carries no target, invitation or account authority.
    Test {
        category: NotificationCategory,
        scope: Option<AccountScope>,
    },
}
impl NotificationKind {
    pub fn category(self) -> NotificationCategory {
        match self {
            Self::Message { .. } => NotificationCategory::Messages,
            Self::ConferenceParticipantJoined { .. } | Self::MessageReminder { .. } => {
                NotificationCategory::Important
            }
            Self::IncomingCall { .. } => NotificationCategory::IncomingCalls,
            Self::Calendar { .. } => NotificationCategory::Calendar,
            Self::Test { category, .. } => category,
        }
    }
    pub fn scope(self) -> Option<AccountScope> {
        match self {
            Self::Message { scope, .. }
            | Self::MessageReminder { scope, .. }
            | Self::Calendar { scope, .. } => Some(scope),
            Self::IncomingCall { call } | Self::ConferenceParticipantJoined { call, .. } => {
                Some(call.account)
            }
            Self::Test { scope, .. } => scope,
        }
    }
    pub fn is_consistent(self) -> bool {
        match self {
            Self::Message {
                scope,
                conversation,
                ..
            }
            | Self::MessageReminder {
                scope,
                conversation,
                ..
            } => scope.account == conversation.account,
            Self::IncomingCall { call } | Self::ConferenceParticipantJoined { call, .. } => {
                call.is_consistent()
            }
            _ => true,
        }
    }
    /// Only routing intent; a future action handler must revalidate current account/target/call.
    pub fn activation(self) -> Option<NotificationActivation> {
        match self {
            Self::Message {
                scope,
                conversation,
                ..
            }
            | Self::MessageReminder {
                scope,
                conversation,
                ..
            } => Some(NotificationActivation::OpenConversation {
                scope,
                conversation,
            }),
            Self::ConferenceParticipantJoined { call, .. } => {
                Some(NotificationActivation::OpenConferencePrejoin(call))
            }
            Self::IncomingCall { call } => Some(NotificationActivation::AcceptDirectCall(call)),
            Self::Calendar { scope, id } => {
                Some(NotificationActivation::OpenCalendar { scope, id })
            }
            Self::Test { .. } => None,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotificationActivation {
    OpenConversation {
        scope: AccountScope,
        conversation: ConversationId,
    },
    OpenConferencePrejoin(CallScope),
    AcceptDirectCall(CallScope),
    OpenCalendar {
        scope: AccountScope,
        id: NotificationId,
    },
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotificationError {
    Unavailable,
    PermissionDenied,
    Failed,
    TimedOut,
    Busy,
    Closed,
    InvalidContent,
    SourceUnavailable,
    Stale,
    Duplicate,
    Coalesced,
    Cancelled,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryOutcome {
    Suppressed,
    SubmittedToSystem,
    Cancelled,
    Failed(NotificationError),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotificationState {
    Idle,
    Pending(RequestId),
    Finished {
        request: RequestId,
        banner: DeliveryOutcome,
        sound: DeliveryOutcome,
    },
    Stopped,
}
/// Bounded completion queue belongs to the worker. Composition drains it; snapshot is latest only.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NotificationCompletion {
    pub request: RequestId,
    pub category: NotificationCategory,
    pub banner: DeliveryOutcome,
    pub sound: DeliveryOutcome,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotificationEvent {
    pub request: RequestId,
    pub kind: NotificationKind,
    pub title: Text<128>,
    pub body: Text<512>,
    pub minimized: bool,
    /// Upstream combines current admission/membership, shouldNotify, mute, server/OS DND,
    /// and privacy. The worker cannot discover those facts from the event's kind.
    pub eligible: bool,
}
impl NotificationEvent {
    pub fn test(
        request: RequestId,
        category: NotificationCategory,
        title: Text<128>,
        body: Text<512>,
    ) -> Self {
        Self {
            request,
            kind: NotificationKind::Test {
                category,
                scope: None,
            },
            title,
            body,
            minimized: true,
            eligible: true,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NotificationPlan {
    pub banner: bool,
    pub sound: bool,
}
pub fn plan(
    values: NotificationPreferences,
    event: &NotificationEvent,
    source: SourceAvailability,
) -> NotificationPlan {
    let enabled = event.eligible
        && source == SourceAvailability::Available
        && event.kind.is_consistent()
        && !matches!(
            event.kind,
            NotificationKind::ConferenceParticipantJoined { is_self: true, .. }
        );
    let delivery = values.delivery(event.kind.category());
    NotificationPlan {
        banner: enabled
            && delivery.banners
            && (event.minimized || event.kind.category() == NotificationCategory::IncomingCalls),
        sound: enabled && delivery.sounds,
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BannerBackend {
    Freedesktop,
    UserNotifications,
    AppleScript,
    Unavailable,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SoundBackend {
    SystemNotification,
    PulseAudio,
    Afplay,
    WindowsSoundPlayer,
    Unavailable,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeNotificationCapabilities {
    /// Implemented routes, not permission grants or runtime delivery observations.
    pub banners: BannerBackend,
    pub sounds: SoundBackend,
    pub native_call_ui: bool,
    pub native_call_actions: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeCallAction {
    Accept,
    Decline,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NativeCallActions {
    pub accept: bool,
    pub decline: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeCallActionCompletion {
    pub call: CallScope,
    pub action: NativeCallAction,
    /// Ok means the admitted dispatcher accepted the intent, not media connected.
    pub result: Result<(), NotificationError>,
}
