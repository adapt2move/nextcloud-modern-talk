//! Exact Talk24/25 source mapping. Server roles never substitute for effective
//! permissions. Every observation retains request-start based freshness.
use nextcloud_talk_api::{
    self as api,
    meeting::{Observation, ObservedReply},
    models::*,
    signaling::PeerSession,
};
use talk_core::{
    ids::{CallScope, ClockEpoch},
    policy::*,
};

pub(crate) struct Bundle {
    pub inputs: PolicyInputs,
    pub exists: Observed<bool>,
    pub settings: SignalingSettings,
    pub peers: Vec<Peer>,
}
pub(crate) struct Peer {
    pub session: PeerSession,
    pub publish_media: bool,
    pub name: talk_core::bounded::Text<256>,
    pub microphone: bool,
    pub camera: bool,
}
pub(crate) struct Sources {
    scope: CallScope,
    own: PeerSession,
    account: Option<ObservedReply>,
    capabilities: Option<ObservedReply>,
    room: Option<ObservedReply>,
    participants: Option<ObservedReply>,
    settings: Option<ObservedReply>,
    peers: Option<ObservedReply>,
}
pub(crate) fn moment(m: api::scope::Moment) -> Moment {
    Moment {
        epoch: ClockEpoch::from_local(m.epoch.key()),
        millis: m.millis,
    }
}
impl Sources {
    pub(crate) fn new(scope: CallScope, own: PeerSession) -> Self {
        Self {
            scope,
            own,
            account: None,
            capabilities: None,
            room: None,
            participants: None,
            settings: None,
            peers: None,
        }
    }
    pub(crate) fn add(&mut self, what: Observation, value: ObservedReply) {
        match what {
            Observation::Account => self.account = Some(value),
            Observation::Capabilities => self.capabilities = Some(value),
            Observation::Room => self.room = Some(value),
            Observation::ConnectedParticipants => self.participants = Some(value),
            Observation::SignalingSettings => self.settings = Some(value),
            Observation::CallParticipants => self.peers = Some(value),
        }
    }
    pub(crate) fn finish(self, joined: bool) -> Result<Bundle, api::ErrorKind> {
        let unknown = api::ErrorKind::Unsupported;
        let account = self.account.ok_or(unknown)?;
        let capabilities = self.capabilities.ok_or(unknown)?;
        let room = self.room.ok_or(unknown)?;
        let participants = self.participants.ok_or(unknown)?;
        let settings = self.settings.ok_or(unknown)?;
        let lease = |r: &ObservedReply, source| {
            Lease::from_qualified_refresh(self.scope, source, moment(r.started), moment(r.received))
                .map_err(|_| api::ErrorKind::Deadline)
        };
        let al = lease(&account, Source::AccountProbe)?;
        let cl = lease(&capabilities, Source::HostCapabilities)?;
        let gl = lease(&capabilities, Source::LocalCapabilities)?;
        let rl = lease(&room, Source::Conversation)?;
        let pl = lease(&participants, Source::ConnectedParticipants)?;
        let sl = lease(&settings, Source::SignalingSettings)?;
        let api::Reply::Account(a) = account.reply else {
            return Err(unknown);
        };
        let api::Reply::Capabilities(c) = capabilities.reply else {
            return Err(unknown);
        };
        let api::Reply::Room(r) = room.reply else {
            return Err(unknown);
        };
        let api::Reply::Participants(people) = participants.reply else {
            return Err(unknown);
        };
        let api::Reply::SignalingSettings(settings) = settings.reply else {
            return Err(unknown);
        };
        if !matches!(c.server_major, Some(34 | 35)) || r.remote {
            return Err(unknown);
        }
        let has = |name: &str| c.features.iter().any(|f| f == name);
        let attributes = r.attributes.ok_or(unknown)?;
        if attributes & !31 != 0 {
            return Err(unknown);
        }
        if attributes & 4 != 0 && !has("classified-conversations") {
            return Err(unknown);
        }
        if attributes & 8 != 0 && !has("announcement-preset") {
            return Err(unknown);
        }
        let permissions = r
            .permissions
            .filter(|v| *v & !511 == 0 && *v != 0)
            .ok_or(unknown)?;
        let own = people
            .iter()
            .filter(|p| {
                p.actor_type.as_deref() == Some("users")
                    && p.actor_id.as_deref() == Some(a.id.as_str())
            })
            .collect::<Vec<_>>();
        if own.len() != 1 {
            return Err(unknown);
        }
        let own_connected = own[0].sessions.as_ref().is_some_and(|sessions| {
            sessions
                .iter()
                .any(|s| PeerSession::parse(s).is_ok_and(|s| s == self.own))
        });
        let recording = match r.recording_status {
            Some(0) => RecordingState::Inactive,
            Some(1 | 2) => RecordingState::Active,
            Some(3 | 4) => RecordingState::Starting,
            Some(5) => RecordingState::Failed,
            _ => RecordingState::Unknown,
        };
        let room_policy = RoomPolicy {
            kind: match r.kind {
                Some(1) => ConversationKind::Direct,
                Some(2) => ConversationKind::Group,
                Some(3) => ConversationKind::Public,
                Some(6) => ConversationKind::NoteToSelf,
                _ => return Err(unknown),
            },
            permissions: EffectivePermissions {
                start_call: r.can_start_call.ok_or(unknown)? && permissions & 2 != 0,
                join_call: permissions & 4 != 0,
                bypass_lobby: permissions & 8 != 0,
                publish_audio: permissions & 16 != 0,
                publish_video: permissions & 32 != 0,
                publish_screen: permissions & 64 != 0,
            },
            read_only: match r.read_only {
                Some(0) => false,
                Some(1) => true,
                _ => return Err(unknown),
            },
            lobby_closed: match r.lobby_state {
                Some(0) => false,
                Some(1) => true,
                _ => return Err(unknown),
            },
            classified: attributes & 4 != 0,
            channel: attributes & 8 != 0,
            consent_required: match r.recording_consent {
                Some(0) => false,
                Some(1) => true,
                _ => return Err(unknown),
            },
            recording,
        };
        let configuration = CallConfiguration {
            audio_supported: has("audio"),
            video_supported: has("video"),
            calls_enabled: c.call_enabled.ok_or(unknown)?,
            recording_enabled: c.recording_enabled.ok_or(unknown)?,
            consent: match c.recording_consent {
                Some(0) => ConsentMode::NotRequired,
                Some(1) => ConsentMode::Required,
                Some(2) => ConsentMode::PerConversation,
                _ => return Err(unknown),
            },
            encryption_supported: has("call-end-to-end-encryption"),
            encryption_required: c.encryption_required.ok_or(unknown)?,
        };
        let maximum = c
            .grid_limit
            .and_then(|v| u16::try_from(v).ok())
            .filter(|v| *v > 0)
            .ok_or(unknown)?;
        let profile = match settings.mode {
            SignalingMode::Internal => SignalingProfile::Internal,
            SignalingMode::External => SignalingProfile::External,
            _ => return Err(unknown),
        };
        let mut peers = Vec::with_capacity(16);
        let mut connected = own_connected;
        if let Some(value) = self.peers {
            let api::Reply::CallParticipants(list) = value.reply else {
                return Err(unknown);
            };
            if joined {
                connected = connected
                    && list
                        .iter()
                        .any(|p| PeerSession::parse(&p.session).is_ok_and(|s| s == self.own));
            }
            for p in list {
                let session = PeerSession::parse(&p.session)?;
                if session == self.own {
                    continue;
                }
                if peers.len() >= 16 || peers.iter().any(|p: &Peer| p.session == session) {
                    return Err(api::ErrorKind::TooLarge);
                }
                let matching = people
                    .iter()
                    .filter(|person| {
                        person
                            .sessions
                            .as_ref()
                            .is_some_and(|s| s.iter().any(|s| s == &p.session))
                    })
                    .collect::<Vec<_>>();
                if matching.len() != 1 {
                    return Err(unknown);
                }
                let flags = matching[0]
                    .in_call
                    .filter(|v| *v & !7 == 0 && *v & 1 != 0)
                    .ok_or(unknown)?;
                peers.push(Peer {
                    session,
                    publish_media: flags & 6 != 0,
                    name: talk_core::bounded::Text::new(
                        matching[0].display_name.as_deref().ok_or(unknown)?,
                    )
                    .map_err(|_| unknown)?,
                    microphone: flags & 2 != 0,
                    camera: flags & 4 != 0,
                });
            }
        } else if joined {
            return Err(unknown);
        }
        Ok(Bundle {
            inputs: PolicyInputs {
                scope: self.scope,
                account_valid: Observed::Known {
                    value: true,
                    lease: al,
                },
                host_configuration: Observed::Known {
                    value: configuration,
                    lease: cl,
                },
                local_grid: Observed::Known {
                    value: GridPolicy {
                        enforced: c.grid_limit_enforced.ok_or(unknown)?,
                        maximum_remote_videos: maximum.min(16),
                    },
                    lease: gl,
                },
                room: Observed::Known {
                    value: room_policy,
                    lease: rl,
                },
                own_session_connected: Observed::Known {
                    value: connected,
                    lease: pl,
                },
                signaling_profile: Observed::Known {
                    value: profile,
                    lease: sl,
                },
            },
            exists: Observed::Known {
                value: r.has_call.ok_or(unknown)?,
                lease: rl,
            },
            settings,
            peers,
        })
    }
}
