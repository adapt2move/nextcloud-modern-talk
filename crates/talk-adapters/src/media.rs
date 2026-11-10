//! Maintained native WebRTC engine and scoped application media handoff.
//! The desktop owns physical devices; the authenticated meeting owner supplies
//! expiring publication authority. The old IPC supervisor is diagnostic only.

// These private interfaces are exercised by the native diagnostic driver. They
// are intentionally not reachable from presentation or a production call grant.
pub(crate) mod e2ee;
#[cfg(unix)]
#[allow(dead_code)]
pub(crate) mod runtime;
#[cfg(unix)]
#[allow(dead_code)]
pub(crate) mod supervisor;
#[allow(dead_code)]
pub(crate) mod wire;

static CALL_STOP: std::sync::Mutex<Option<&'static str>> = std::sync::Mutex::new(None);
/// Remember the step at which a call owner stopped, for the local diagnostic
/// log only. It is a fixed label and never a decision input.
pub(crate) fn note_call_stop(step: &'static str) {
    if let Ok(mut slot) = CALL_STOP.lock() {
        slot.get_or_insert(step);
    }
}
/// Take the step recorded by the most recently stopped call owner.
pub fn take_call_stop() -> Option<&'static str> {
    CALL_STOP.lock().ok()?.take()
}

use talk_core::ids::{AccountScope, CallScope};
use talk_core::policy::Moment;
use talk_core::ports::{Availability, MediaPort, PortError, StopReason, StopReport, Task};

/// Bootstrap/fixture owner for compositions without an admitted call service.
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

pub(crate) mod engine;
pub(crate) mod service;
pub use service::{CaptureHandle, MediaService, VideoImage};
