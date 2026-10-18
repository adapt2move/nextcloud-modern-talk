//! Pure admission and cooperative cancellation primitives, not an async runtime.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::ids::AccountScope;

pub const CAPTURE_RELEASE_DEADLINE_MS: u64 = 1_000;
pub const POLICY_ENFORCEMENT_DEADLINE_MS: u64 = 250;

#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// Owned by the serialized account actor. Handles alone never grant access.
pub struct Admission {
    scope: AccountScope,
    open: bool,
    cancellation: Cancellation,
}

impl Admission {
    /// Call only after independently validated authorization. This does no login.
    pub fn for_validated_scope(scope: AccountScope) -> Self {
        Self {
            scope,
            open: true,
            cancellation: Cancellation::default(),
        }
    }

    pub fn admits(&self, scope: AccountScope) -> bool {
        self.open && self.scope == scope && !self.cancellation.is_cancelled()
    }

    pub fn cancellation(&self) -> Cancellation {
        self.cancellation.clone()
    }

    /// No I/O: closure precedes all later durable or remote cleanup.
    pub fn close(&mut self) {
        self.open = false;
        self.cancellation.cancel();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RejectionEvidence {
    /// Adapter verified the same-origin, uncached account probe's OCS rejection.
    ConfirmedAccountProbe,
    ActionDenied,
    ConversationOrSignalingExpired,
    TransportFailure,
    Ambiguous,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RejectionAction {
    ShutDownAccount,
    RestrictAction,
    ReconcileSession,
    DegradeAndProbe,
}

pub fn classify(evidence: RejectionEvidence) -> RejectionAction {
    match evidence {
        RejectionEvidence::ConfirmedAccountProbe => RejectionAction::ShutDownAccount,
        RejectionEvidence::ActionDenied => RejectionAction::RestrictAction,
        RejectionEvidence::ConversationOrSignalingExpired => RejectionAction::ReconcileSession,
        RejectionEvidence::TransportFailure | RejectionEvidence::Ambiguous => {
            RejectionAction::DegradeAndProbe
        }
    }
}
