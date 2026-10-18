//! Owner of browser authorization, qualified OS vault and nonsecret restoration journal.
//! No secret storage exists in this bootstrap, including no test/in-memory fallback.

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
