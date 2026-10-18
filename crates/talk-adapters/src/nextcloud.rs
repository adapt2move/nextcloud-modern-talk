//! Owner of OCS/DAV/signaling wire formats and host-bound authenticated transport.
//! Endpoint resolution and credentials will remain private to the adapters crate.

use talk_core::ports::{
    Availability, NextcloudPort, OperationContext, PortError, ProtocolReply, ProtocolRequest, Task,
};

pub struct UnconfiguredNextcloud;

impl NextcloudPort for UnconfiguredNextcloud {
    fn availability(&self) -> Availability {
        Availability::Unconfigured
    }

    fn request(
        &mut self,
        _context: OperationContext,
        _request: ProtocolRequest,
    ) -> Task<'_, ProtocolReply> {
        Box::pin(async { Err(PortError::unconfigured()) })
    }
}
