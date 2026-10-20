//! Nonsecret identity contracts, not a login, journal, vault or supervisor implementation.
//! IDs and records are not capabilities. The owner must validate bindings and serialized
//! revisions; constructors below check only the stated correlation/lifetime predicates.

use crate::ids::{AccountId, AccountScope, LoginAttemptId, OperationId, VaultGeneration};
use crate::lifecycle::Cancellation;
use crate::policy::Moment;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestoreDisposition {
    Suppressed,
    Unresolved,
    Eligible(VaultGeneration),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Settlement {
    WriterPending,
    SettledPresent,
    SettledAbsent,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoreIntent {
    pub account: AccountId,
    pub attempt: LoginAttemptId,
    /// Original provider-write ledger identity, preserved across maintenance operations.
    pub operation: OperationId,
    pub attempted: VaultGeneration,
    pub retained: Option<VaultGeneration>,
}

impl StoreIntent {
    pub fn has_distinct_generation(self) -> bool {
        self.retained != Some(self.attempted)
    }
}

/// Only the attempted item is within abandonment cleanup authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CleanupMarker {
    pub intent: StoreIntent,
    pub settlement: Settlement,
}

impl CleanupMarker {
    pub fn may_delete(self, account: AccountId, generation: VaultGeneration) -> bool {
        self.intent.has_distinct_generation()
            && self.intent.account == account
            && self.intent.attempted == generation
            && matches!(self.settlement, Settlement::SettledPresent)
    }

    pub fn complete(self) -> bool {
        self.intent.has_distinct_generation()
            && matches!(self.settlement, Settlement::SettledAbsent)
    }
}

/// Checked after a committed record was loaded. Missing/corrupt records are not this type.
/// An explicit cancel-and-retain may leave attempted-item cleanup pending independently.
pub fn can_restore(disposition: RestoreDisposition, confirmed_item: VaultGeneration) -> bool {
    disposition == RestoreDisposition::Eligible(confirmed_item)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersistenceState {
    Unconfigured,
    Stored(VaultGeneration),
    SessionOnly,
    SessionOnlyCleanupPending,
    RestartSuppressionUnconfirmed,
    RecoveryRequired,
}

/// Pending browser authorization/storage, before any runtime is issued. Reserving N and
/// the provider operation ID does not dispatch a write or assert validated authorization.
pub struct PendingAttemptContext {
    pub intent: StoreIntent,
    pub deadline: Moment,
    pub cancellation: Cancellation,
}

impl PendingAttemptContext {
    /// Correlation/time checks only. The identity owner also checks its current-attempt
    /// registry, canonical binding and validation evidence immediately before activation.
    pub fn admits(&self, intent: StoreIntent, now: Moment) -> bool {
        self.intent == intent
            && intent.has_distinct_generation()
            && !self.cancellation.is_cancelled()
            && now.epoch == self.deadline.epoch
            && now.millis < self.deadline.millis
    }
}

/// Cold startup needs an account lookup, not an invented authorized runtime.
pub struct RecoveryContext {
    pub account: AccountId,
    pub operation: OperationId,
    pub deadline: Moment,
    pub cancellation: Cancellation,
}

/// One bounded maintenance dispatch/observation slice; expiry never settles a writer.
pub const MAINTENANCE_SLICE_MS: u64 = 3_000;

/// Owned by the identity supervisor, independent of runtime and caller cancellation.
/// It permits only maintenance commands, never new login or runtime activation.
/// The constructor does not establish supervisor ownership; the adapter enforces that.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MaintenanceContext {
    account: AccountId,
    operation: OperationId,
    started: Moment,
    deadline: Moment,
}

impl MaintenanceContext {
    pub fn for_supervisor(account: AccountId, operation: OperationId, now: Moment) -> Option<Self> {
        Some(Self {
            account,
            operation,
            started: now,
            deadline: Moment {
                epoch: now.epoch,
                millis: now.millis.checked_add(MAINTENANCE_SLICE_MS)?,
            },
        })
    }

    pub fn account(self) -> AccountId {
        self.account
    }

    pub fn operation(self) -> OperationId {
        self.operation
    }

    pub fn deadline(self) -> Moment {
        self.deadline
    }

    pub fn admits(self, account: AccountId, now: Moment) -> bool {
        account == self.account
            && now.epoch == self.started.epoch
            && now.millis >= self.started.millis
            && now.millis < self.deadline.millis
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetentionValidation {
    /// Must still be admitted and bound to P at commit; a rejected runtime never qualifies.
    CurrentRuntime(AccountScope),
    /// Explicit cold-recovery choice: validate the named stored P privately before commit.
    /// This requests validation; it is not evidence of validity or permission to activate.
    RevalidateStored,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AbandonChoice {
    /// Abandon the store, suppress all generations; optional memory activation is separate.
    SessionOnly,
    /// Cancel the attempt, suppress all generations; never infer retention of P.
    Cancel,
    RetainPrevious {
        generation: VaultGeneration,
        validation: RetentionValidation,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityContractError {
    SameGeneration,
    WrongAccount,
    WrongRetainedGeneration,
    WrongAcknowledgement,
}

/// Requested atomic disposition plus exact-N abandonment/cleanup. Not a durable result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Abandonment {
    intent: StoreIntent,
    choice: AbandonChoice,
}

impl Abandonment {
    pub fn new(intent: StoreIntent, choice: AbandonChoice) -> Result<Self, IdentityContractError> {
        if !intent.has_distinct_generation() {
            return Err(IdentityContractError::SameGeneration);
        }
        if let AbandonChoice::RetainPrevious {
            generation,
            validation,
        } = choice
        {
            if intent.retained != Some(generation) {
                return Err(IdentityContractError::WrongRetainedGeneration);
            }
            if let RetentionValidation::CurrentRuntime(scope) = validation
                && scope.account != intent.account
            {
                return Err(IdentityContractError::WrongAccount);
            }
        }
        Ok(Self { intent, choice })
    }

    pub fn intent(self) -> StoreIntent {
        self.intent
    }

    pub fn choice(self) -> AbandonChoice {
        self.choice
    }

    /// A requested value only. UI/storage claims must use the committed acknowledgement.
    pub fn requested_disposition(self) -> RestoreDisposition {
        match self.choice {
            AbandonChoice::SessionOnly | AbandonChoice::Cancel => RestoreDisposition::Suppressed,
            AbandonChoice::RetainPrevious { generation, .. } => {
                RestoreDisposition::Eligible(generation)
            }
        }
    }
}

/// Owner-issued only after durable commit or intact committed-record load. Neither a
/// requested disposition nor a provider completion may be reported as this record.
/// Public fields are a test seam, not a proof that any storage I/O actually happened.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommittedRestoration {
    pub account: AccountId,
    pub revision: u64,
    pub disposition: RestoreDisposition,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IdentityStatus {
    pub persistence: PersistenceState,
    pub settlement: Settlement,
    /// None means no confirmed durable record, never implicit suppression/eligibility.
    pub restoration: Option<CommittedRestoration>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptCommand {
    /// System-browser flow and identity validation; holds pending secrets privately.
    Authorize,
    Stage,
    ActivateStored,
    ActivateSessionOnly(SessionOnlyCommit),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptReport {
    Authorized(StoreIntent),
    Staged {
        intent: StoreIntent,
        status: IdentityStatus,
    },
    Activated {
        intent: StoreIntent,
        scope: AccountScope,
        status: IdentityStatus,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RestoredIdentity {
    pub operation: OperationId,
    pub scope: AccountScope,
    pub generation: VaultGeneration,
    pub restoration: CommittedRestoration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SuppressionCause {
    /// Refers to the already invalidated runtime; it supplies no authenticated authority.
    AccountRejected(AccountScope),
    Logout(AccountScope),
    AccountRemoval,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaintenanceCommand {
    Abandon(Abandonment),
    Suppress(SuppressionCause),
    Reconcile(StoreIntent),
    DeleteAbandonedAttempt(StoreIntent),
}

impl MaintenanceCommand {
    /// Reject mismatched account/attempt descriptors before any ledger/provider effect.
    pub fn is_scoped_to(self, account: AccountId) -> bool {
        match self {
            Self::Abandon(request) => request.intent().account == account,
            Self::Suppress(
                SuppressionCause::AccountRejected(scope) | SuppressionCause::Logout(scope),
            ) => scope.account == account,
            Self::Suppress(SuppressionCause::AccountRemoval) => true,
            Self::Reconcile(intent) | Self::DeleteAbandonedAttempt(intent) => {
                intent.account == account && intent.has_distinct_generation()
            }
        }
    }
}

/// Maintenance never returns a runtime grant. Its result can outlive the requesting task.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MaintenanceReport {
    /// This transition's identity, distinct from StoreIntent.operation's provider writer.
    pub operation: OperationId,
    pub outcome: MaintenanceOutcome,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaintenanceOutcome {
    Abandoned {
        request: Abandonment,
        restoration: CommittedRestoration,
        settlement: Settlement,
    },
    Suppressed {
        cause: SuppressionCause,
        restoration: CommittedRestoration,
    },
    Reconciled {
        intent: StoreIntent,
        restoration: CommittedRestoration,
        settlement: Settlement,
    },
    Cleanup {
        marker: CleanupMarker,
        restoration: CommittedRestoration,
    },
}

impl MaintenanceReport {
    /// Correlates the exact selected transition with an owner-reported durable result.
    /// This predicate cannot certify the owner's storage/validation preconditions.
    pub fn acknowledges_abandonment(
        self,
        context: MaintenanceContext,
        expected: Abandonment,
    ) -> bool {
        match self.outcome {
            MaintenanceOutcome::Abandoned {
                request,
                restoration,
                ..
            } => {
                self.operation == context.operation()
                    && request == expected
                    && request.intent().account == context.account()
                    && restoration.account == context.account()
                    && restoration.disposition == expected.requested_disposition()
            }
            _ => false,
        }
    }
}

/// Receipt of an explicit committed session-only choice, not an activation grant.
/// Only a matching Abandoned acknowledgement can produce it; settlement cannot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionOnlyCommit {
    intent: StoreIntent,
    revision: u64,
}

impl SessionOnlyCommit {
    pub fn from_acknowledgement(
        context: MaintenanceContext,
        expected: Abandonment,
        report: MaintenanceReport,
    ) -> Result<Self, IdentityContractError> {
        if let MaintenanceOutcome::Abandoned { restoration, .. } = report.outcome
            && expected.choice() == AbandonChoice::SessionOnly
            && report.acknowledges_abandonment(context, expected)
        {
            return Ok(Self {
                intent: expected.intent(),
                revision: restoration.revision,
            });
        }
        Err(IdentityContractError::WrongAcknowledgement)
    }

    pub fn revision(self) -> u64 {
        self.revision
    }

    /// Necessary checks only. Activation must also compare the latest ledger revision,
    /// selected choice, live attempt registry and canonical validation inside the owner.
    pub fn matches_live_attempt(self, context: &PendingAttemptContext, now: Moment) -> bool {
        context.admits(self.intent, now)
    }
}

/// Qualified persistent routing, independent of transient crate instance IDs.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum StorageBackend {
    MacKeychain,
    WindowsCredential,
    LinuxSecretService,
    PrivateFile,
}
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ProviderBinding {
    pub backend: StorageBackend,
    pub profile: crate::ids::ProfileId,
    /// Random registry key for one immutable OS-user/store configuration.
    pub store: u128,
}
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct QualifiedItem {
    pub provider: ProviderBinding,
    pub account: AccountId,
    pub generation: VaultGeneration,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageFailure {
    Missing,
    Locked,
    AccessDenied,
    Corrupt,
    TemporarilyUnavailable,
    Unavailable,
    Unsupported,
    Cancelled,
    Conflict,
    Other,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageChoice {
    Native,
    PrivateFile,
    SessionOnly,
}
