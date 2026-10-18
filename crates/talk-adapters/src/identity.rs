//! Owner of browser authorization, qualified OS vault and nonsecret restoration journal.
//! No secret storage exists in this bootstrap, including no test/in-memory fallback.

use talk_core::ports::{
    Availability, IdentityCommand, IdentityPort, IdentityReport, OperationContext, PortError, Task,
};

pub struct UnconfiguredIdentity;

impl IdentityPort for UnconfiguredIdentity {
    fn availability(&self) -> Availability {
        Availability::Unconfigured
    }

    fn transition(
        &mut self,
        _context: OperationContext,
        _command: IdentityCommand,
    ) -> Task<'_, IdentityReport> {
        Box::pin(async { Err(PortError::unconfigured()) })
    }
}
