//! Nonsecret restoration contract. These records do not persist themselves.

use crate::ids::{AccountId, LoginAttemptId, OperationId, VaultGeneration};

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
