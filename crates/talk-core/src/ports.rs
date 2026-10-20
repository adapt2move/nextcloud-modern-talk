//! Worker-side service seams. Constructing/polling these tasks must never block GPUI.
//! Cancellation stops admission; dropping a task does not prove vault writer settlement.

use std::future::Future;
use std::pin::Pin;

use crate::bounded::{Page, Text};
use crate::identity::{
    AttemptCommand, AttemptReport, IdentityStatus, MaintenanceCommand, MaintenanceContext,
    MaintenanceReport, PendingAttemptContext, RecoveryContext, RestoredIdentity,
};
use crate::ids::OperationId;
use crate::ids::{AccountId, AccountScope, CallScope, ConversationId};
use crate::lifecycle::Cancellation;
use crate::policy::{Moment, PolicyInputs};
use crate::preferences::{
    RequestId,
    account::{AccountPreferenceChange, AccountPreferenceResult},
};
use crate::protocol::{
    AccountProbe, CallObservation, Conversation, ConversationList, ConversationTag,
    ConversationTagId, ConversationTags, HistoryPage, MessageId, Participants, SendMessage,
    SendReceipt,
};

pub type Task<'a, T> = Pin<Box<dyn Future<Output = Result<T, PortError>> + Send + 'a>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Availability {
    Unconfigured,
    Unsupported,
    Ready,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    Unconfigured,
    Unsupported,
    Cancelled,
    Deadline,
    Busy,
    PermissionDenied,
    ReauthenticationRequired,
    Malformed,
    Uncertain,
    StorageUnavailable,
    WrongScope,
    Transport,
    RedirectRejected,
    TooLarge,
    NotFound,
    Conflict,
    RateLimited,
    ServerUnavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Retry {
    Never,
    UserAction,
    AfterReconciliation,
    AfterDelay { millis: u32 },
}

/// Deliberately excludes URLs, wire bodies, provider messages and tokens.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PortError {
    pub kind: ErrorKind,
    pub account: Option<AccountId>,
    pub retry: Retry,
}

impl PortError {
    pub const fn unconfigured() -> Self {
        Self {
            kind: ErrorKind::Unconfigured,
            account: None,
            retry: Retry::Never,
        }
    }
}

pub struct OperationContext {
    pub scope: AccountScope,
    pub deadline: Moment,
    pub cancellation: Cancellation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProtocolRequest {
    Messaging {
        conversation: ConversationId,
        operation: OperationId,
        command: crate::protocol::messaging::MessagingCommand,
    },
    ConversationSettings {
        conversation: ConversationId,
        operation: OperationId,
        command: crate::protocol::conversation::ConversationCommand,
    },
    PollIncomingCalls,
    Meeting(crate::calls::Command),
    ProbeAccount,
    RefreshCallPolicy(CallScope),
    ReconcileConversation(ConversationId),
    Leave(CallScope),
    InspectAccount,
    ReadPreferences {
        request: RequestId,
    },
    SetPreference {
        request: RequestId,
        change: AccountPreferenceChange,
    },
    SetArchived {
        conversation: ConversationId,
        archived: bool,
    },
    SetFavorite {
        conversation: ConversationId,
        favorite: bool,
    },
    ReadTags,
    CreateTag {
        name: Text<1000>,
    },
    RenameTag {
        tag: ConversationTagId,
        name: Text<1000>,
    },
    DeleteTag(ConversationTagId),
    SetTagCollapsed {
        tag: ConversationTagId,
        collapsed: bool,
    },
    ReorderTags {
        tags: Page<ConversationTagId, 102>,
    },
    SetConversationTags {
        conversation: ConversationId,
        tags: Page<ConversationTagId, 20>,
    },
    ListConversations,
    Participants(ConversationId),
    History {
        conversation: ConversationId,
        before: Option<MessageId>,
    },
    Send(SendMessage),
    SendStatus(OperationId),
    MarkRead(MessageId),
    CreateGroup {
        name: Text<256>,
    },
    FindPeople {
        query: Text<256>,
    },
    CreateDirect {
        operation: OperationId,
        recipient: crate::protocol::conversation::RecipientId,
    },
    LeaveConversation(ConversationId),
    ActivateConversation(ConversationId),
    DeactivateConversation(ConversationId),
    ObserveCall(CallScope),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProtocolReply {
    Messaging {
        conversation: ConversationId,
        operation: OperationId,
        result: crate::protocol::messaging::MessagingResult,
    },
    ConversationSettings {
        conversation: ConversationId,
        operation: OperationId,
        result: crate::protocol::conversation::ConversationResult,
    },
    IncomingCalls(crate::calls::IncomingCalls),
    Meeting(crate::calls::Snapshot),
    AccountValid(AccountScope),
    Policy(Box<PolicyInputs>),
    LeaveAcknowledged(CallScope),
    Account(AccountProbe),
    Preferences(AccountPreferenceResult),
    Tags(ConversationTags),
    TagCreated(ConversationTag),
    Conversations(ConversationList),
    Conversation(Conversation),
    People(Page<crate::protocol::conversation::Recipient, 100>),
    Participants(Participants),
    History(HistoryPage),
    Send(SendReceipt),
    ReadAcknowledged(MessageId),
    ConversationLeft(ConversationId),
    ConversationActive(ConversationId),
    ConversationInactive(ConversationId),
    Call(CallObservation),
}

pub trait NextcloudPort: Send {
    fn availability(&self) -> Availability;
    fn request(
        &mut self,
        context: OperationContext,
        request: ProtocolRequest,
    ) -> Task<'_, ProtocolReply>;
}

/// One identity owner, with distinct pre-runtime, authenticated and maintenance seams.
/// Calls do bounded nonblocking admission before returning an observation task. Accepted
/// ledger/provider work belongs to the supervisor, not to the task: dropping it cannot
/// discard a dispatched writer, its durable intent or later settlement. Cancellation
/// closes activation/admission immediately; it never manufactures a retained-P choice.
/// The supervisor keeps 8 descriptors/account (7 ordinary, 1 reserved for suppression),
/// 1 provider operation/account and at most 2 provider workers globally. Overflow rejects
/// new work; local closure never queues. Expired slices retain unresolved durable markers.
/// No implementation of that supervisor or durability exists in this bootstrap.
pub trait IdentityPort: Send {
    fn availability(&self) -> Availability;

    /// Authorize/stage/activate a pending attempt. No runtime is required as input.
    /// Stored activation rechecks confirmed N and commits Eligible(N) before issuing a
    /// runtime. Session-only activation verifies the receipt's current ledger revision
    /// and still-live attempt; receipt delivery or writer completion never activates.
    fn attempt(
        &mut self,
        context: PendingAttemptContext,
        command: AttemptCommand,
    ) -> Task<'_, AttemptReport>;

    /// Cold restoration from intact committed eligibility plus exact confirmed item and
    /// fresh canonical account validation. Never enumerate to choose a credential.
    /// Cancellation/drop prevents activation; missing/corrupt metadata requires recovery.
    fn restore(&mut self, context: RecoveryContext) -> Task<'_, RestoredIdentity>;

    /// Authenticated operation/status resolution. Secret borrowing remains private to
    /// the identity/validated transport owner; no reusable authentication object returns.
    fn resolve(&mut self, context: OperationContext) -> Task<'_, IdentityStatus>;

    /// Only the supervisor invokes this seam. Check context deadline/account, correlate
    /// exact intent/N/P/writer and transition IDs, then serialize/revalidate ledger state.
    /// No runtime token is consulted for maintenance authority. A referenced retained
    /// runtime is independently revalidated as evidence, not used as job cancellation.
    /// Cancel/retain invalidate the attempt locally before waiting for durable commit.
    /// Settlement/deletion preserve disposition; late results cannot reopen admission.
    fn maintain(
        &mut self,
        context: MaintenanceContext,
        command: MaintenanceCommand,
    ) -> Task<'_, MaintenanceReport>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StopReason {
    UserLeave,
    AccountRejected,
    PolicyExpired,
    RestrictionObserved,
    Sleep,
    Shutdown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StopReport {
    pub call: CallScope,
    pub publication_closed_at: Moment,
    /// None means unconfirmed release, never success.
    pub devices_released_at: Option<Moment>,
    pub worker_terminated: bool,
}

pub trait MediaPort: Send {
    fn availability(&self) -> Availability;
    /// Immediate local atomic closure, independent of worker/control queues and remote I/O.
    fn close_publication(&self, scope: AccountScope, reason: StopReason);
    /// Priority stop must not be abandoned merely because the account was cancelled.
    fn stop(
        &mut self,
        call: CallScope,
        reason: StopReason,
        deadline: Moment,
    ) -> Task<'_, StopReport>;
}
