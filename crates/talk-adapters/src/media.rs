//! Owner of native engine, capture, device lifetime and shared frame surfaces.
//! No engine, worker, device, frame or platform API is present yet.

use talk_core::ids::{AccountScope, CallScope};
use talk_core::policy::Moment;
use talk_core::ports::{Availability, MediaPort, PortError, StopReason, StopReport, Task};

pub struct UnconfiguredMedia;

impl MediaPort for UnconfiguredMedia {
    fn availability(&self) -> Availability {
        Availability::Unconfigured
    }

    fn close_publication(&self, _scope: AccountScope, _reason: StopReason) {
        // There is no publication path. Do not fabricate a device-release acknowledgement.
    }

    fn stop(
        &mut self,
        _call: CallScope,
        _reason: StopReason,
        _deadline: Moment,
    ) -> Task<'_, StopReport> {
        Box::pin(async { Err(PortError::unconfigured()) })
    }
}
