//! Owner of browser authorization, qualified OS vault and private restoration metadata.
//! Production implementation keeps secrets, storage and owned runtime workers private.
mod avatars;
pub(crate) mod browser;
mod connection;
mod data;
mod error;
mod ledger;
mod login;
mod private_fs;
mod provider;
mod runtime;
pub use connection::{
    AccountConnection, CONNECTION_CAPACITY, CONNECTION_PAGE, ConnectionAccount,
    ConnectionCompletion, ConnectionCursor, ConnectionPending, ConnectionRequestId,
    ConnectionSnapshot, ConnectionState, ConnectionUpdate, SavedAccountState,
};
pub use data::{
    CACHE_HISTORY_LIMIT, CACHE_LIFETIME, CACHE_ROOM_LIMIT, CACHE_THUMBNAIL_BYTES,
    CACHE_THUMBNAIL_LIFETIME, CACHE_THUMBNAIL_LIMIT, CacheHistory, CacheRoom, CacheThumbnail,
    CachedAttachment, CachedMessage, CachedReaction, CachedReply, CachedRoomKind, DataCommand,
    DataError, DataResult, DataSnapshot, OUTBOX_LIMIT, OutboxEntry, OutboxIntent, OutboxKind,
    OutboxOptions, OutboxPhase, OutboxRecord,
};
pub use error::IdentityError;
pub use runtime::{
    AccountPhase, AccountSummary, IdentityRuntime, Observation, RecoverySelection, RuntimeCommand,
    RuntimeConfig, RuntimeReply, StoredItemStatus, StoredState,
};

/// Safe presentation of the selected HTTPS destination before browser handoff.
/// This is only a label; Login Flow still validates and binds its own full origin.
pub fn selected_server_origin(input: &str) -> Result<talk_core::bounded::Text<512>, IdentityError> {
    login::Origin::parse(input, false)?.display_origin()
}

use talk_core::identity::{
    AttemptCommand, AttemptReport, IdentityStatus, MaintenanceCommand, MaintenanceContext,
    MaintenanceReport, PendingAttemptContext, RecoveryContext, RestoredIdentity,
};
use talk_core::ports::{Availability, IdentityPort, OperationContext, PortError, Task};

pub struct UnconfiguredIdentity;

impl IdentityPort for UnconfiguredIdentity {
    fn availability(&self) -> Availability {
        Availability::Unconfigured
    }

    fn attempt(
        &mut self,
        _context: PendingAttemptContext,
        _command: AttemptCommand,
    ) -> Task<'_, AttemptReport> {
        Box::pin(async { Err(PortError::unconfigured()) })
    }

    fn restore(&mut self, _context: RecoveryContext) -> Task<'_, RestoredIdentity> {
        Box::pin(async { Err(PortError::unconfigured()) })
    }

    fn resolve(&mut self, _context: OperationContext) -> Task<'_, IdentityStatus> {
        Box::pin(async { Err(PortError::unconfigured()) })
    }

    fn maintain(
        &mut self,
        _context: MaintenanceContext,
        _command: MaintenanceCommand,
    ) -> Task<'_, MaintenanceReport> {
        Box::pin(async { Err(PortError::unconfigured()) })
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod test_server;
