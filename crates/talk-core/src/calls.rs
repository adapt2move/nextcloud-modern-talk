//! The local call lifecycle. This module owns user intent and policy admission;
//! HTTP membership, an OS permission and an engine callback are never authority
//! on their own. The adapter must close its publication latch when `advance`
//! removes a grant, before awaiting any remote leave or device disposal.

use crate::ids::CallScope;
use crate::policy::{
    ConsentMode, ConversationKind, Lease, Moment, Observed, PolicyInputs, RecordingState,
    SignalingProfile, Source,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MediaWishes {
    pub microphone: bool,
    pub camera: bool,
    pub screen: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Entry {
    OutgoingDirect,
    IncomingDirect,
    GroupAudio,
    PreparedConference,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Gate {
    PolicyUnknown,
    PolicyStale,
    CallEnded,
    CallForbidden,
    Lobby,
    RecordingConsent,
    RequiredEncryption,
    SignalingUnsupported,
    MediaUnavailable,
    MembershipLost,
    WrongScope,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    Ringing,
    Preparing,
    Connecting,
    Active,
    Waiting(Gate),
    Reconnecting,
    Ending,
    Ended,
}

/// Actual implementation availability; capability advertisements cannot fill it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NativeSupport {
    pub audio: bool,
    pub video: bool,
    pub screen: bool,
    pub external_signaling: bool,
    pub end_to_end_encryption: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DevicePermissions {
    pub microphone: bool,
    pub camera: bool,
    /// A grant for this selected screen session, never the OS-settings button.
    pub screen_session: bool,
}

/// A grant is scoped to this attempt and the earliest contributing source lease.
/// It is data for the media owner, not a token that can extend its own deadline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Grant {
    scope: CallScope,
    expires: Moment,
    publish: MediaWishes,
    remote_video_limit: u16,
}
impl Grant {
    pub fn scope(self) -> CallScope {
        self.scope
    }
    pub fn expires(self) -> Moment {
        self.expires
    }
    pub fn publication(self) -> MediaWishes {
        self.publish
    }
    pub fn remote_video_limit(self) -> u16 {
        self.remote_video_limit
    }
    pub fn is_fresh(self, scope: CallScope, now: Moment) -> bool {
        self.scope == scope && now.epoch == self.expires.epoch && now.millis < self.expires.millis
    }
}

/// One cohesive owner, with no queue, timer, network payload or native device.
#[derive(Clone)]
pub struct CallSession {
    scope: CallScope,
    entry: Entry,
    phase: Phase,
    wishes: MediaWishes,
    consent: bool,
    consent_required: Option<bool>,
    accepted: bool,
    joined: bool,
    grant: Option<Grant>,
}
impl CallSession {
    pub fn new(scope: CallScope, entry: Entry) -> Result<Self, Gate> {
        if !scope.is_consistent() {
            return Err(Gate::WrongScope);
        }
        Ok(Self {
            scope,
            entry,
            phase: if entry == Entry::IncomingDirect {
                Phase::Ringing
            } else {
                Phase::Preparing
            },
            wishes: MediaWishes::default(),
            consent: false,
            consent_required: None,
            accepted: false,
            joined: false,
            grant: None,
        })
    }
    pub fn scope(&self) -> CallScope {
        self.scope
    }
    pub fn phase(&self) -> Phase {
        self.phase
    }
    pub fn recording_consent(&self) -> bool {
        self.consent
    }
    pub fn entry(&self) -> Entry {
        self.entry
    }
    pub fn accepted(&self) -> bool {
        self.accepted
    }
    pub fn wishes(&self) -> MediaWishes {
        self.wishes
    }
    pub fn grant(&self) -> Option<Grant> {
        self.grant
    }
    /// Called only for an explicit start, accept or final prepared join.
    /// Duplicate acceptance is idempotent and cannot add camera/screen intent.
    pub fn accept(&mut self, scope: CallScope, mut wishes: MediaWishes) -> Result<(), Gate> {
        self.check_live(scope)?;
        if self.accepted {
            return Ok(());
        }
        if self.entry == Entry::GroupAudio {
            wishes.camera = false;
        }
        wishes.screen = false;
        self.accepted = true;
        self.wishes = wishes;
        self.phase = Phase::Connecting;
        Ok(())
    }
    /// Consent is explicit and only accepted while the current policy asks for it.
    pub fn consent(&mut self, scope: CallScope) -> Result<(), Gate> {
        self.check_live(scope)?;
        if self.phase != Phase::Waiting(Gate::RecordingConsent) {
            return Err(Gate::PolicyUnknown);
        }
        self.consent = true;
        self.phase = Phase::Connecting;
        Ok(())
    }
    pub fn set_wishes(&mut self, scope: CallScope, wishes: MediaWishes) -> Result<(), Gate> {
        self.check_live(scope)?;
        if !self.accepted {
            return Err(Gate::CallForbidden);
        }
        // Every expansion must pass advance again. A muted track closes now.
        self.wishes = wishes;
        self.grant = None;
        Ok(())
    }
    pub fn joined(&mut self, scope: CallScope) -> Result<(), Gate> {
        self.check_live(scope)?;
        if !self.accepted || self.grant.is_none() {
            return Err(Gate::CallForbidden);
        }
        self.joined = true;
        self.phase = Phase::Connecting;
        Ok(())
    }
    /// A successful REST join is not a connected media path.
    pub fn connected(&mut self, scope: CallScope, now: Moment) -> Result<(), Gate> {
        self.check_live(scope)?;
        if !self.joined || !self.grant.is_some_and(|g| g.is_fresh(scope, now)) {
            return Err(Gate::PolicyStale);
        }
        self.phase = Phase::Active;
        Ok(())
    }
    /// Observed control loss invalidates admission immediately, not at a new
    /// sliding deadline. Recovery never revives a microphone, camera or share.
    pub fn disconnected(&mut self, scope: CallScope) -> Result<(), Gate> {
        self.check_live(scope)?;
        self.grant = None;
        self.wishes = MediaWishes::default();
        self.phase = Phase::Reconnecting;
        Ok(())
    }
    /// Local end is independent of whether HTTP leave succeeds.
    pub fn end(&mut self, scope: CallScope) -> Result<(), Gate> {
        if scope != self.scope {
            return Err(Gate::WrongScope);
        }
        self.grant = None;
        self.wishes = MediaWishes::default();
        self.consent = false;
        self.phase = Phase::Ending;
        Ok(())
    }
    /// The adapter calls this only after its own cleanup observation; a server
    /// response or hidden tile alone must not be used as that observation.
    pub fn ended(&mut self, scope: CallScope) -> Result<(), Gate> {
        if scope != self.scope {
            return Err(Gate::WrongScope);
        }
        self.end(scope)?;
        self.phase = Phase::Ended;
        Ok(())
    }
    pub fn expire(&mut self, now: Moment) -> bool {
        if self.grant.is_some_and(|g| !g.is_fresh(self.scope, now)) {
            self.grant = None;
            self.wishes = MediaWishes::default();
            self.phase = Phase::Waiting(Gate::PolicyStale);
            return true;
        }
        false
    }
    fn check_live(&self, scope: CallScope) -> Result<(), Gate> {
        if scope != self.scope {
            Err(Gate::WrongScope)
        } else if matches!(self.phase, Phase::Ending | Phase::Ended) {
            Err(Gate::CallEnded)
        } else {
            Ok(())
        }
    }
    /// Evaluate complete fresh evidence. A refresh can narrow current intent but
    /// cannot create intent, transfer consent or revive an ended generation.
    pub fn advance(
        &mut self,
        inputs: &PolicyInputs,
        call_exists: Observed<bool>,
        support: NativeSupport,
        devices: DevicePermissions,
        now: Moment,
    ) -> Result<Grant, Gate> {
        self.check_live(inputs.scope)?;
        let result = self.resolve(inputs, call_exists, support, devices, now);
        match result {
            Ok(grant) => {
                self.grant = Some(grant);
                if matches!(self.phase, Phase::Waiting(_) | Phase::Reconnecting) {
                    self.phase = Phase::Connecting;
                }
                Ok(grant)
            }
            Err(gate) => {
                self.grant = None;
                if self.accepted {
                    self.phase = Phase::Waiting(gate);
                }
                Err(gate)
            }
        }
    }
    fn resolve(
        &mut self,
        p: &PolicyInputs,
        call_exists: Observed<bool>,
        support: NativeSupport,
        devices: DevicePermissions,
        now: Moment,
    ) -> Result<Grant, Gate> {
        if !self.accepted {
            return Err(Gate::CallForbidden);
        }
        let mut expires = None;
        let mut read = |lease: Lease, source| {
            if lease.source() != source {
                return Err(Gate::PolicyUnknown);
            }
            if !lease.is_fresh(self.scope, now) {
                return Err(Gate::PolicyStale);
            }
            let deadline = lease.expires();
            expires = Some(expires.map_or(deadline, |old: Moment| {
                if old.millis <= deadline.millis {
                    old
                } else {
                    deadline
                }
            }));
            Ok(())
        };
        macro_rules! known {
            ($observed:expr, $source:expr) => {
                match $observed {
                    Observed::Known { value, lease } => {
                        read(lease, $source)?;
                        value
                    }
                    Observed::Unknown => return Err(Gate::PolicyUnknown),
                }
            };
        }
        if !known!(p.account_valid, Source::AccountProbe) {
            return Err(Gate::CallForbidden);
        }
        let config = known!(p.host_configuration, Source::HostCapabilities);
        let room = known!(p.room, Source::Conversation);
        let grid = known!(p.local_grid, Source::LocalCapabilities);
        let connected = known!(p.own_session_connected, Source::ConnectedParticipants);
        let signaling = known!(p.signaling_profile, Source::SignalingSettings);
        let exists = known!(call_exists, Source::Conversation);
        if !connected {
            return Err(Gate::MembershipLost);
        }
        if !config.calls_enabled
            || room.channel
            || room.read_only
            || room.kind == ConversationKind::NoteToSelf
            || !room.permissions.join_call
            || (!exists && !room.permissions.start_call)
        {
            return Err(Gate::CallForbidden);
        }
        if self.entry == Entry::IncomingDirect && !exists {
            return Err(Gate::CallEnded);
        }
        if room.lobby_closed && !room.permissions.bypass_lobby {
            return Err(Gate::Lobby);
        }
        if config.encryption_required && !support.end_to_end_encryption {
            return Err(Gate::RequiredEncryption);
        }
        if signaling == SignalingProfile::External && !support.external_signaling {
            return Err(Gate::SignalingUnsupported);
        }
        if !support.audio {
            return Err(Gate::MediaUnavailable);
        }
        // Classified is only accepted from the adapter's reviewed source mapping.
        // Its no-recording invariant takes precedence over consent configuration.
        if room.classified && room.recording != RecordingState::Inactive {
            return Err(Gate::CallForbidden);
        }
        let consent_required = !room.classified
            && match config.consent {
                ConsentMode::Required => true,
                ConsentMode::PerConversation => room.consent_required,
                ConsentMode::NotRequired => false,
            };
        if self.consent_required != Some(consent_required) {
            self.consent = false;
            self.consent_required = Some(consent_required);
        }
        if consent_required && !self.consent {
            return Err(Gate::RecordingConsent);
        }
        let publish = MediaWishes {
            microphone: self.wishes.microphone
                && devices.microphone
                && config.audio_supported
                && room.permissions.publish_audio,
            camera: self.wishes.camera
                && devices.camera
                && support.video
                && config.video_supported
                && room.permissions.publish_video,
            screen: self.wishes.screen
                && devices.screen_session
                && support.screen
                && room.permissions.publish_screen,
        };
        Ok(Grant {
            scope: self.scope,
            expires: expires.ok_or(Gate::PolicyUnknown)?,
            publish,
            remote_video_limit: grid.maximum_remote_videos.min(16),
        })
    }
}

/// Commands accepted only by the current authenticated account owner. Snapshots
/// carry local correlation; no SDP, ICE addresses, session tokens or raw frames.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    Start {
        call: CallScope,
        entry: Entry,
        wishes: MediaWishes,
    },
    Change {
        call: CallScope,
        wishes: MediaWishes,
    },
    Consent(CallScope),
    Reconnect(CallScope),
    Stop(CallScope),
    Observe(CallScope),
}
impl Command {
    pub fn scope(&self) -> CallScope {
        match self {
            Self::Start { call, .. } | Self::Change { call, .. } => *call,
            Self::Consent(call)
            | Self::Reconnect(call)
            | Self::Stop(call)
            | Self::Observe(call) => *call,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceState {
    Off,
    Requested,
    Active,
    Unavailable,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MediaCounters {
    pub audio_sent: u64,
    pub audio_received: u64,
    pub video_sent: u64,
    pub video_received: u64,
    pub data_received: u64,
    pub offers_sent: u64,
    pub answers_sent: u64,
    pub policy_refreshes: u64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Snapshot {
    pub call: CallScope,
    pub entry: Entry,
    pub participants: crate::bounded::Page<Participant, 16>,
    pub phase: Phase,
    pub microphone: DeviceState,
    pub camera: DeviceState,
    pub screen: DeviceState,
    pub peers: u16,
    pub recording: RecordingState,
    pub counters: MediaCounters,
    pub revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Participant {
    pub local: u128,
    pub name: crate::bounded::Text<256>,
    pub microphone: bool,
    pub camera: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ids::*,
        policy::{CallConfiguration, EffectivePermissions, GridPolicy, RoomPolicy},
    };
    fn scope() -> CallScope {
        let account = AccountId::from_local(1);
        CallScope {
            account: AccountScope {
                account,
                runtime: RuntimeGeneration::from_local(2),
            },
            conversation: ConversationId {
                account,
                host: HostId::from_local(3),
                local: 4,
            },
            call: CallGeneration::from_local(5),
        }
    }
    fn now(ms: u64) -> Moment {
        Moment {
            epoch: ClockEpoch::from_local(1),
            millis: ms,
        }
    }
    fn known<T>(value: T, source: Source, started: u64) -> Observed<T> {
        Observed::Known {
            value,
            lease: Lease::from_qualified_refresh(scope(), source, now(started), now(started + 10))
                .unwrap(),
        }
    }
    fn support() -> NativeSupport {
        NativeSupport {
            audio: true,
            video: true,
            screen: true,
            external_signaling: false,
            end_to_end_encryption: false,
        }
    }
    fn devices() -> DevicePermissions {
        DevicePermissions {
            microphone: true,
            camera: true,
            screen_session: true,
        }
    }
    fn inputs() -> PolicyInputs {
        PolicyInputs {
            scope: scope(),
            account_valid: known(true, Source::AccountProbe, 0),
            host_configuration: known(
                CallConfiguration {
                    audio_supported: true,
                    video_supported: true,
                    calls_enabled: true,
                    recording_enabled: true,
                    consent: ConsentMode::NotRequired,
                    encryption_supported: false,
                    encryption_required: false,
                },
                Source::HostCapabilities,
                0,
            ),
            local_grid: known(
                GridPolicy {
                    enforced: true,
                    maximum_remote_videos: 4,
                },
                Source::LocalCapabilities,
                0,
            ),
            room: known(
                RoomPolicy {
                    kind: ConversationKind::Group,
                    permissions: EffectivePermissions {
                        start_call: true,
                        join_call: true,
                        bypass_lobby: false,
                        publish_audio: true,
                        publish_video: true,
                        publish_screen: true,
                    },
                    read_only: false,
                    lobby_closed: false,
                    classified: false,
                    channel: false,
                    consent_required: false,
                    recording: RecordingState::Inactive,
                },
                Source::Conversation,
                0,
            ),
            own_session_connected: known(true, Source::ConnectedParticipants, 0),
            signaling_profile: known(SignalingProfile::Internal, Source::SignalingSettings, 0),
        }
    }
    fn exists() -> Observed<bool> {
        known(true, Source::Conversation, 0)
    }
    fn accepted(entry: Entry) -> CallSession {
        let mut call = CallSession::new(scope(), entry).unwrap();
        call.accept(
            scope(),
            MediaWishes {
                microphone: true,
                camera: true,
                screen: true,
            },
        )
        .unwrap();
        call
    }
    #[test]
    fn group_entry_and_duplicate_accept_do_not_add_capture() {
        let mut c = accepted(Entry::GroupAudio);
        assert_eq!(
            c.wishes(),
            MediaWishes {
                microphone: true,
                camera: false,
                screen: false
            }
        );
        c.accept(
            scope(),
            MediaWishes {
                microphone: true,
                camera: true,
                screen: true,
            },
        )
        .unwrap();
        assert!(!c.wishes().camera && !c.wishes().screen);
    }
    #[test]
    fn expiry_closes_and_clears_all_wishes() {
        let mut c = accepted(Entry::PreparedConference);
        let g = c
            .advance(&inputs(), exists(), support(), devices(), now(100))
            .unwrap();
        assert_eq!(g.expires(), now(13000));
        assert!(c.expire(now(13000)));
        assert_eq!(c.wishes(), MediaWishes::default());
        assert!(c.grant().is_none());
    }
    #[test]
    fn channel_and_readonly_never_grant_even_with_all_permissions() {
        for channel in [false, true] {
            let mut p = inputs();
            if let Observed::Known { value, .. } = &mut p.room {
                value.channel = channel;
                value.read_only = !channel;
                value.permissions.bypass_lobby = true;
            }
            assert_eq!(
                accepted(Entry::PreparedConference).advance(
                    &p,
                    exists(),
                    support(),
                    devices(),
                    now(100)
                ),
                Err(Gate::CallForbidden)
            );
        }
    }
    #[test]
    fn consent_is_current_explicit_and_classified_recording_stays_forbidden() {
        let mut p = inputs();
        if let Observed::Known { value, .. } = &mut p.host_configuration {
            value.consent = ConsentMode::Required;
        }
        let mut c = accepted(Entry::PreparedConference);
        assert!(c.consent(scope()).is_err());
        assert_eq!(
            c.advance(&p, exists(), support(), devices(), now(100)),
            Err(Gate::RecordingConsent)
        );
        c.consent(scope()).unwrap();
        assert!(
            c.advance(&p, exists(), support(), devices(), now(101))
                .is_ok()
        );
        if let Observed::Known { value, .. } = &mut p.room {
            value.classified = true;
        }
        let mut classified = accepted(Entry::PreparedConference);
        assert!(
            classified
                .advance(&p, exists(), support(), devices(), now(100))
                .is_ok()
        );
        assert!(!classified.recording_consent());
        if let Observed::Known { value, .. } = &mut p.room {
            value.recording = RecordingState::Active;
        }
        assert_eq!(
            classified.advance(&p, exists(), support(), devices(), now(100)),
            Err(Gate::CallForbidden)
        );
    }
    #[test]
    fn required_encryption_external_and_lobby_are_distinct() {
        let mut p = inputs();
        if let Observed::Known { value, .. } = &mut p.host_configuration {
            value.encryption_required = true;
        }
        assert_eq!(
            accepted(Entry::PreparedConference).advance(
                &p,
                exists(),
                support(),
                devices(),
                now(100)
            ),
            Err(Gate::RequiredEncryption)
        );
        p = inputs();
        p.signaling_profile = known(SignalingProfile::External, Source::SignalingSettings, 0);
        assert_eq!(
            accepted(Entry::PreparedConference).advance(
                &p,
                exists(),
                support(),
                devices(),
                now(100)
            ),
            Err(Gate::SignalingUnsupported)
        );
        p = inputs();
        if let Observed::Known { value, .. } = &mut p.room {
            value.lobby_closed = true;
        }
        assert_eq!(
            accepted(Entry::PreparedConference).advance(
                &p,
                exists(),
                support(),
                devices(),
                now(100)
            ),
            Err(Gate::Lobby)
        );
    }
    #[test]
    fn reconnect_does_not_revive_capture_and_end_is_irreversible() {
        let mut c = accepted(Entry::OutgoingDirect);
        c.advance(&inputs(), exists(), support(), devices(), now(100))
            .unwrap();
        c.joined(scope()).unwrap();
        c.connected(scope(), now(100)).unwrap();
        c.disconnected(scope()).unwrap();
        let g = c
            .advance(&inputs(), exists(), support(), devices(), now(101))
            .unwrap();
        assert_eq!(g.publication(), MediaWishes::default());
        c.ended(scope()).unwrap();
        assert_eq!(
            c.advance(&inputs(), exists(), support(), devices(), now(102)),
            Err(Gate::CallEnded)
        );
    }
    #[test]
    fn wrong_source_and_old_scope_never_revalidate() {
        let mut p = inputs();
        p.account_valid = known(true, Source::Conversation, 0);
        assert!(
            accepted(Entry::PreparedConference)
                .advance(&p, exists(), support(), devices(), now(100))
                .is_err()
        );
        let mut p = inputs();
        p.scope.call = CallGeneration::from_local(6);
        assert_eq!(
            accepted(Entry::PreparedConference).advance(
                &p,
                exists(),
                support(),
                devices(),
                now(100)
            ),
            Err(Gate::WrongScope)
        );
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IncomingCalls {
    pub scope: crate::ids::AccountScope,
    pub call: Option<Snapshot>,
    pub title: Option<crate::bounded::Text<256>>,
    pub eligible: bool,
}
