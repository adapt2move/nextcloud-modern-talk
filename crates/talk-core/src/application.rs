//! Minimal composition shell. No actor, account, connection or service is started.

use crate::bounded::Page;
use crate::identity::PersistenceState;
use crate::ports::{Availability, IdentityPort, MediaPort, NextcloudPort};
use crate::ui::{DesktopPort, SubmitError, UnavailableReason, UserIntent, ViewSnapshot};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntegrationState {
    pub protocol: Availability,
    pub identity: Availability,
    pub media: Availability,
}

pub struct Bootstrap<N, I, M> {
    _protocol: N,
    _identity: I,
    _media: M,
    state: IntegrationState,
    initial_snapshot_pending: bool,
}

impl<N: NextcloudPort, I: IdentityPort, M: MediaPort> Bootstrap<N, I, M> {
    pub fn new(protocol: N, identity: I, media: M) -> Self {
        let state = IntegrationState {
            protocol: protocol.availability(),
            identity: identity.availability(),
            media: media.availability(),
        };
        Self {
            _protocol: protocol,
            _identity: identity,
            _media: media,
            state,
            initial_snapshot_pending: true,
        }
    }

    pub fn integration_state(&self) -> IntegrationState {
        self.state
    }
}

impl<N: NextcloudPort, I: IdentityPort, M: MediaPort> DesktopPort for Bootstrap<N, I, M> {
    fn try_submit(&mut self, _intent: UserIntent) -> Result<(), SubmitError> {
        Err(SubmitError::Unavailable(UnavailableReason::Unconfigured))
    }

    fn take_snapshot(&mut self) -> Option<ViewSnapshot> {
        if !self.initial_snapshot_pending {
            return None;
        }
        self.initial_snapshot_pending = false;
        Some(ViewSnapshot {
            revision: 0,
            account: None,
            availability: Some(UnavailableReason::Unconfigured),
            persistence: PersistenceState::Unconfigured,
            conversations: Page::empty(),
            messages: Page::empty(),
        })
    }
}
