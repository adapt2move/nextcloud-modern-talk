//! Transport-neutral invalidations for the existing account scheduler. Hints
//! confer no membership, notification, policy or device authority.
use crate::{ErrorKind, models::RoomToken, scope::SessionScope};
use std::collections::VecDeque;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LiveEvent {
    RefreshAll,
    RoomChanged { room: RoomToken },
    ChatAvailable { room: RoomToken },
    CallChanged { room: RoomToken },
    MembershipChanged { room: RoomToken },
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveState {
    Internal,
    Connecting,
    Connected,
    Reconnecting,
    Stopped(ErrorKind),
}
pub struct LiveEventBatch {
    pub scope: SessionScope,
    pub state: LiveState,
    /// First failure retained across the finite reconnect budget; no wire text.
    pub first_transport_failure: Option<ErrorKind>,
    /// At most 32 coalesced invalidations, never wire messages or media frames.
    pub events: Vec<LiveEvent>,
}

#[derive(Default)]
pub(crate) struct Mailbox(VecDeque<LiveEvent>);
impl Mailbox {
    pub(crate) fn push(&mut self, event: LiveEvent) {
        if self.0.contains(&LiveEvent::RefreshAll) || self.0.contains(&event) {
            return;
        }
        // A full authoritative refresh subsumes every invalidation. Coalescing
        // preserves required reconciliation without keeping an unbounded queue.
        if self.0.len() == 32 || event == LiveEvent::RefreshAll {
            self.0.clear();
            self.0.push_back(LiveEvent::RefreshAll);
        } else {
            self.0.push_back(event);
        }
    }
    pub(crate) fn drain(&mut self) -> Vec<LiveEvent> {
        self.0.drain(..).collect()
    }
}
