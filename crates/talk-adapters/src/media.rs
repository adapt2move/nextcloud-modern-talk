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

/// Where a call owner stopped and, if a request failed, which one and how.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CallStop {
    pub step: &'static str,
    pub source: Option<&'static str>,
    pub error: Option<talk_core::ports::ErrorKind>,
}
type CallFailure = (&'static str, talk_core::ports::ErrorKind);
static CALL_STOP: std::sync::Mutex<(Option<&'static str>, Option<CallFailure>)> =
    std::sync::Mutex::new((None, None));
/// Remember the step at which a call owner stopped, for the local diagnostic
/// log only. It is a fixed label and never a decision input.
pub(crate) fn note_call_stop(step: &'static str) {
    if let Ok(mut slot) = CALL_STOP.lock() {
        slot.0.get_or_insert(step);
    }
}
/// Commit a media failure together with its stop, so a concurrent successful
/// policy refresh cannot clear the cause before composition observes it.
pub(crate) fn note_call_stop_failure(
    step: &'static str,
    source: &'static str,
    error: talk_core::ports::ErrorKind,
) {
    if let Ok(mut slot) = CALL_STOP.lock()
        && slot.0.is_none()
    {
        *slot = (Some(step), Some((source, error)));
    }
}
/// Remember the request whose failure led to a stop; `None` clears it once
/// that request succeeds again. Fixed labels only.
pub(crate) fn note_call_failure(failure: Option<CallFailure>) {
    if let Ok(mut slot) = CALL_STOP.lock()
        && slot.0.is_none()
    {
        slot.1 = failure;
    }
}
/// Take the step recorded by the most recently stopped call owner.
pub fn take_call_stop() -> Option<CallStop> {
    let mut slot = CALL_STOP.lock().ok()?;
    let step = slot.0.take()?;
    let failure = slot.1.take();
    Some(CallStop {
        step,
        source: failure.map(|failure| failure.0),
        error: failure.map(|failure| failure.1),
    })
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
pub use service::{CaptureHandle, LOCAL_PARTICIPANT, MediaService, VideoImage};
