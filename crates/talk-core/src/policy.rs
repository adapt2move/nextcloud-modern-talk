//! Conservative policy primitives. No server parser or production policy resolver exists.
//! A lease is usable only after the adapter's source profile is independently qualified.

use crate::ids::{CallScope, ClockEpoch};

pub const MAX_SOURCE_AGE_MS: u64 = 15_000; // F
pub const MAX_REFRESH_INTERVAL_MS: u64 = 10_000; // R, including scheduler tolerance
pub const REQUEST_DEADLINE_MS: u64 = 3_000; // D
pub const SOURCE_ALLOWANCE_MS: u64 = 2_000; // S, a qualification requirement

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Moment {
    pub epoch: ClockEpoch,
    pub millis: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Source {
    AccountProbe,
    LocalCapabilities,
    HostCapabilities,
    Conversation,
    ConnectedParticipants,
    SignalingSettings,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyError {
    WrongScope,
    ClockDiscontinuity,
    RequestExpired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Lease {
    scope: CallScope,
    source: Source,
    valid_from: Moment,
    expires: Moment,
}

impl Lease {
    /// Source qualification is an explicit precondition, never inferred from HTTP success.
    /// The bootstrap adapters cannot issue a qualified refresh.
    pub fn from_qualified_refresh(
        scope: CallScope,
        source: Source,
        started: Moment,
        received: Moment,
    ) -> Result<Self, PolicyError> {
        if !scope.is_consistent() {
            return Err(PolicyError::WrongScope);
        }
        if started.epoch != received.epoch || received.millis < started.millis {
            return Err(PolicyError::ClockDiscontinuity);
        }
        if received.millis - started.millis > REQUEST_DEADLINE_MS {
            return Err(PolicyError::RequestExpired);
        }
        // Equivalent to (request start - S) + F; never receipt time + F.
        let expires = started
            .millis
            .checked_add(MAX_SOURCE_AGE_MS - SOURCE_ALLOWANCE_MS)
            .ok_or(PolicyError::ClockDiscontinuity)?;
        Ok(Self {
            scope,
            source,
            valid_from: received,
            expires: Moment {
                epoch: started.epoch,
                millis: expires,
            },
        })
    }

    pub fn is_fresh(self, scope: CallScope, now: Moment) -> bool {
        scope.is_consistent()
            && self.scope == scope
            && self.expires.epoch == now.epoch
            && now.millis >= self.valid_from.millis
            && now.millis < self.expires.millis
    }

    pub fn expires(self) -> Moment {
        self.expires
    }

    pub fn source(self) -> Source {
        self.source
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Fact {
    Yes,
    No,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Decision {
    Allowed,
    Denied,
    Unknown,
    Stale,
}

/// A normalized source value. Unknown never carries a lease that grants authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Observed<T> {
    Known { value: T, lease: Lease },
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConversationKind {
    Direct,
    Group,
    Public,
    NoteToSelf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConsentMode {
    NotRequired,
    Required,
    PerConversation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CallConfiguration {
    pub audio_supported: bool,
    pub video_supported: bool,
    pub calls_enabled: bool,
    pub recording_enabled: bool,
    pub consent: ConsentMode,
    pub encryption_supported: bool,
    pub encryption_required: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EffectivePermissions {
    pub start_call: bool,
    pub join_call: bool,
    pub bypass_lobby: bool,
    pub publish_audio: bool,
    pub publish_video: bool,
    pub publish_screen: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordingState {
    Inactive,
    Starting,
    Active,
    Stopping,
    Failed,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RoomPolicy {
    pub kind: ConversationKind,
    pub permissions: EffectivePermissions,
    pub read_only: bool,
    pub lobby_closed: bool,
    pub classified: bool,
    pub channel: bool,
    pub consent_required: bool,
    pub recording: RecordingState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GridPolicy {
    pub enforced: bool,
    pub maximum_remote_videos: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignalingProfile {
    Internal,
    External,
}

/// Bounded source bundle, not a grant. A future resolver must check every source's
/// expected provenance/scope, earliest expiry, current session continuity, local user
/// consent, permissions and actual native encryption. There is no resolver yet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyInputs {
    pub scope: CallScope,
    pub account_valid: Observed<bool>,
    pub host_configuration: Observed<CallConfiguration>,
    pub local_grid: Observed<GridPolicy>,
    pub room: Observed<RoomPolicy>,
    pub own_session_connected: Observed<bool>,
    pub signaling_profile: Observed<SignalingProfile>,
}

/// A minimum boundary algebra: server version and role names never grant permission.
/// Rich call/consent/encryption resolution remains future work under ADR-004.
pub struct Gate {
    pub capability: Fact,
    pub configured: Fact,
    pub effective_permission: Fact,
    pub invariant_allows: Fact,
}

impl Gate {
    pub fn resolve(&self, lease: Option<Lease>, scope: CallScope, now: Moment) -> Decision {
        let facts = [
            self.capability,
            self.configured,
            self.effective_permission,
            self.invariant_allows,
        ];
        if facts.contains(&Fact::No) {
            return Decision::Denied;
        }
        if facts.contains(&Fact::Unknown) || lease.is_none() {
            return Decision::Unknown;
        }
        if !lease.is_some_and(|value| value.is_fresh(scope, now)) {
            return Decision::Stale;
        }
        Decision::Allowed
    }
}
