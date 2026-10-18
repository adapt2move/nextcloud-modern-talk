//! Renderer boundary: bounded display data and user intents, with no network/native types.
//! No media frame, credential, endpoint, authorization header or signaling payload belongs here.

use crate::bounded::{Page, Text};
use crate::identity::PersistenceState;
use crate::ids::{AccountId, AccountScope, CallScope, ConversationId};
use crate::policy::Decision;

pub const MAX_VISIBLE_CONVERSATIONS: usize = 100;
pub const MAX_VISIBLE_MESSAGES: usize = 100;
pub const MAX_INTENTS: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnavailableReason {
    Unconfigured,
    Unsupported,
    PermissionDenied,
    PolicyUnknown,
    PolicyStale,
    ReauthenticationRequired,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversationRow {
    pub id: ConversationId,
    pub label: Text<256>,
    pub can_join: Decision,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageRow {
    pub local_id: u128,
    pub excerpt: Text<4096>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ViewSnapshot {
    pub revision: u64,
    pub account: Option<AccountScope>,
    pub availability: Option<UnavailableReason>,
    pub persistence: PersistenceState,
    pub conversations: Page<ConversationRow, MAX_VISIBLE_CONVERSATIONS>,
    pub messages: Page<MessageRow, MAX_VISIBLE_MESSAGES>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaIntent {
    MicrophoneOffCameraOff,
    MicrophoneOnCameraOff,
    MicrophoneOnCameraOn,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UserIntent {
    SelectConversation(ConversationId),
    /// Opens voluntary device prejoin; it does not enable capture by itself.
    PrepareConference(CallScope),
    /// Direct acceptance has no device-prejoin interstitial. Mandatory gates still apply.
    AcceptDirect {
        call: CallScope,
        media: MediaIntent,
    },
    JoinPrepared {
        call: CallScope,
        media: MediaIntent,
    },
    Leave(CallScope),
    Logout(AccountId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmitError {
    Unavailable(UnavailableReason),
    Busy,
    StaleScope,
}

/// UI-thread methods must be bounded and nonblocking. Acceptance is not operation success.
pub trait DesktopPort {
    fn try_submit(&mut self, intent: UserIntent) -> Result<(), SubmitError>;
    /// One coalesced latest snapshot, never an unbounded event backlog.
    fn take_snapshot(&mut self) -> Option<ViewSnapshot>;
}
