//! Worker-side service seams. Constructing/polling these tasks must never block GPUI.
//! Cancellation stops admission; dropping a task does not prove vault writer settlement.

use std::future::Future;
use std::pin::Pin;

use crate::identity::{PersistenceState, Settlement, StoreIntent};
use crate::ids::{AccountId, AccountScope, CallScope, ConversationId};
use crate::lifecycle::Cancellation;
use crate::policy::{Moment, PolicyInputs};

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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Retry {
    Never,
    UserAction,
    AfterReconciliation,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolRequest {
    ProbeAccount,
    RefreshCallPolicy(CallScope),
    ReconcileConversation(ConversationId),
    Leave(CallScope),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProtocolReply {
    AccountValid(AccountScope),
    Policy(Box<PolicyInputs>),
    LeaveAcknowledged(CallScope),
}

pub trait NextcloudPort: Send {
    fn availability(&self) -> Availability;
    fn request(
        &mut self,
        context: OperationContext,
        request: ProtocolRequest,
    ) -> Task<'_, ProtocolReply>;
}

/// The identity owner holds pending login material privately. A StoreIntent contains IDs
/// only; Stage must resolve a still-live attempt and commit metadata before provider I/O.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityCommand {
    Stage(StoreIntent),
    Activate(StoreIntent),
    Resolve(AccountScope),
    Reconcile(StoreIntent),
    DeleteAbandonedAttempt(StoreIntent),
    SuppressRestoration(AccountId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IdentityReport {
    pub persistence: PersistenceState,
    pub settlement: Settlement,
}

pub trait IdentityPort: Send {
    fn availability(&self) -> Availability;
    fn transition(
        &mut self,
        context: OperationContext,
        command: IdentityCommand,
    ) -> Task<'_, IdentityReport>;
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
