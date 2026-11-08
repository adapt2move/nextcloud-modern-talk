//! Talk capability/room source mapping. Server roles never substitute for effective
//! permissions. Every observation retains request-start based freshness.
use nextcloud_talk_api::{
    self as api,
    meeting::{Observation, ObservedReply},
    models::*,
    signaling::PeerSession,
};
use talk_core::calls::PolicyEvidence as Evidence;
use talk_core::{
    ids::{CallScope, ClockEpoch},
    policy::*,
};

pub(crate) struct Bundle {
    pub inputs: PolicyInputs,
    pub exists: Observed<bool>,
    pub settings: SignalingSettings,
    pub peers: Vec<Peer>,
    pub invitees: Vec<Invitee>,
    pub hand_supported: bool,
    pub reactions: talk_core::bounded::Page<talk_core::bounded::Text<64>, 32>,
    pub ring_supported: bool,
    pub end_supported: bool,
    pub mute_supported: bool,
    pub recording_control: Option<bool>,
}
pub(crate) struct Invitee {
    pub attendee: u64,
    pub ring_eligible: bool,
    pub name: talk_core::bounded::Text<256>,
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
    pub(crate) fn finish(self, joined: bool) -> Result<Bundle, Evidence> {
        let account = self.account.ok_or(Evidence::Account)?;
        let capabilities = self.capabilities.ok_or(Evidence::Capabilities)?;
        let room = self.room.ok_or(Evidence::Room)?;
        let participants = self.participants.ok_or(Evidence::Participants)?;
        let settings = self.settings.ok_or(Evidence::SignalingSettings)?;
        let lease = |r: &ObservedReply, source| {
            Lease::from_qualified_refresh(self.scope, source, moment(r.started), moment(r.received))
                .map_err(|_| Evidence::Freshness)
        };
        let al = lease(&account, Source::AccountProbe)?;
        let cl = lease(&capabilities, Source::HostCapabilities)?;
        let gl = lease(&capabilities, Source::LocalCapabilities)?;
        let rl = lease(&room, Source::Conversation)?;
        let pl = lease(&participants, Source::ConnectedParticipants)?;
        let sl = lease(&settings, Source::SignalingSettings)?;
        let api::Reply::Account(a) = account.reply else {
            return Err(Evidence::Account);
        };
        let api::Reply::Capabilities(c) = capabilities.reply else {
            return Err(Evidence::Capabilities);
        };
        let api::Reply::Room(r) = room.reply else {
            return Err(Evidence::Room);
        };
        let api::Reply::Participants(people) = participants.reply else {
            return Err(Evidence::Participants);
        };
        let api::Reply::SignalingSettings(settings) = settings.reply else {
            return Err(Evidence::SignalingSettings);
        };
        if !c.supports("conversation-v4") {
            return Err(Evidence::ConversationApi);
        }
        if !c.supports("signaling-v3") {
            return Err(Evidence::SignalingApi);
        }
        if r.remote {
            return Err(Evidence::FederatedCall);
        }
        let has = |name: &str| c.features.iter().any(|f| f == name);
        let hand_supported = has("raise-hand");
        let mut reactions = talk_core::bounded::Page::empty();
        for reaction in c.call_reactions.as_deref().unwrap_or_default() {
            let Ok(value) = talk_core::bounded::Text::new(reaction) else {
                continue;
            };
            if !reactions.as_slice().contains(&value) {
                let _ = reactions.try_push(value);
            }
        }

        let attributes = r.attributes.ok_or(Evidence::Attributes)?;
        if attributes & !31 != 0 {
            return Err(Evidence::Attributes);
        }
        if attributes & 4 != 0 && !has("classified-conversations") {
            return Err(Evidence::ClassifiedCapability);
        }
        if attributes & 8 != 0 && !has("announcement-preset") {
            return Err(Evidence::ChannelCapability);
        }
        let permissions = r.permissions.ok_or(Evidence::Permissions)?;
        let own = people
            .iter()
            .filter(|p| {
                p.actor_type.as_deref() == Some("users")
                    && p.actor_id.as_deref() == Some(a.id.as_str())
            })
            .collect::<Vec<_>>();
        if own.len() != 1 {
            return Err(Evidence::Participants);
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
                _ => return Err(Evidence::RoomType),
            },
            permissions: EffectivePermissions {
                start_call: (if r.has_call != Some(true) {
                    r.can_start_call.ok_or(Evidence::CanStart)?
                } else {
                    r.can_start_call.unwrap_or(false)
                }) && permissions & 2 != 0,
                join_call: permissions & 4 != 0,
                bypass_lobby: permissions & 8 != 0,
                publish_audio: permissions & 16 != 0,
                publish_video: permissions & 32 != 0,
                publish_screen: permissions & 64 != 0,
            },
            read_only: match r.read_only {
                Some(0) => false,
                Some(1) => true,
                _ => return Err(Evidence::ReadOnly),
            },
            lobby_closed: match r.lobby_state {
                Some(0) => false,
                Some(1) => true,
                _ => return Err(Evidence::Lobby),
            },
            classified: attributes & 4 != 0,
            channel: attributes & 8 != 0,
            // Unknown consent never implies agreement. Ask explicitly when
            // per-room policy applies; classified rooms still prohibit recording.
            consent_required: r.recording_consent != Some(0),
            recording,
        };
        let configuration = CallConfiguration {
            audio_supported: has("audio"),
            video_supported: has("video"),
            calls_enabled: if r.has_call == Some(false) {
                c.call_enabled.ok_or(Evidence::CallsEnabled)?
            } else {
                c.call_enabled.unwrap_or(false)
            },
            recording_enabled: c.recording_enabled.unwrap_or(false),
            consent: match c.recording_consent {
                Some(0) => ConsentMode::NotRequired,
                Some(1) => ConsentMode::Required,
                Some(2) => ConsentMode::PerConversation,
                _ => ConsentMode::Required,
            },
            encryption_supported: has("call-end-to-end-encryption"),
            // Official E2EE.isEnabled: no capability => off; advertised
            // capability with omitted configuration => on. Support alone is
            // not a requirement when configuration explicitly disables it.
            encryption_required: has("call-end-to-end-encryption")
                && c.encryption_required != Some(false),
        };
        let maximum = c
            .grid_limit
            .and_then(|v| u16::try_from(v).ok())
            .filter(|v| *v > 0)
            // Unknown grid policy cannot authorize a remote video subscription.
            // Audio admission remains independent of optional video layout data.
            .unwrap_or(0);
        let profile = match settings.mode {
            SignalingMode::Internal => SignalingProfile::Internal,
            SignalingMode::External => SignalingProfile::External,
            _ => return Err(Evidence::SignalingSettings),
        };
        let mut invitees = Vec::with_capacity(people.len().min(100));
        for person in &people {
            if person.in_call != Some(0) {
                continue;
            }
            let (Some(attendee), Some(name)) = (
                person.attendee_id.filter(|id| *id > 0),
                person.display_name.as_deref(),
            ) else {
                continue;
            };
            if person.actor_type.as_deref() == Some("users")
                && person.actor_id.as_deref() == Some(a.id.as_str())
            {
                continue;
            }
            if invitees
                .iter()
                .any(|existing: &Invitee| existing.attendee == attendee)
            {
                continue;
            }
            let Ok(name) = talk_core::bounded::Text::new(name) else {
                continue;
            };
            let ring_eligible = person.actor_type.as_deref() == Some("users")
                && person.actor_id.as_deref().is_some_and(|id| !id.is_empty());
            invitees.push(Invitee {
                attendee,
                ring_eligible,
                name,
            });
        }
        let mut peers = Vec::with_capacity(16);
        let mut connected = own_connected;
        if let Some(value) = self.peers {
            let api::Reply::CallParticipants(list) = value.reply else {
                return Err(Evidence::CallParticipants);
            };
            if joined {
                connected = connected
                    && list
                        .iter()
                        .any(|p| PeerSession::parse(&p.session).is_ok_and(|s| s == self.own));
            }
            for p in list {
                let session =
                    PeerSession::parse(&p.session).map_err(|_| Evidence::CallParticipants)?;
                if session == self.own {
                    continue;
                }
                if peers.len() >= 16 || peers.iter().any(|p: &Peer| p.session == session) {
                    return Err(Evidence::CallParticipants);
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
                    return Err(Evidence::CallParticipants);
                }
                let flags = matching[0]
                    .in_call
                    .filter(|v| *v & !7 == 0 && *v & 1 != 0)
                    .ok_or(Evidence::CallParticipants)?;
                peers.push(Peer {
                    session,
                    publish_media: flags & 6 != 0,
                    name: talk_core::bounded::Text::new(
                        // A missing display label cannot revoke call authority.
                        matching[0].display_name.as_deref().unwrap_or(""),
                    )
                    .map_err(|_| Evidence::CallParticipants)?,
                    microphone: flags & 2 != 0,
                    camera: flags & 4 != 0,
                });
            }
        } else if joined {
            return Err(Evidence::CallParticipants);
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
                        enforced: c.grid_limit_enforced.unwrap_or(true),
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
                value: r.has_call.ok_or(Evidence::CallExists)?,
                lease: rl,
            },
            settings,
            peers,
            invitees,
            mute_supported: has("force-mute")
                && matches!(r.participant_type, Some(1 | 2))
                && r.has_call == Some(true)
                && connected,
            end_supported: matches!(r.participant_type, Some(1 | 2 | 6))
                && r.object_type.as_deref() != Some("room")
                && r.has_call == Some(true)
                && connected,
            ring_supported: has("send-call-notification")
                && permissions & 2 != 0
                && r.has_call == Some(true)
                && connected,
            recording_control: if has("recording-v1")
                && c.recording_enabled == Some(true)
                && matches!(r.participant_type, Some(1 | 2))
                && attributes & 4 == 0
                && r.has_call == Some(true)
                && connected
            {
                match recording {
                    RecordingState::Inactive => Some(true),
                    RecordingState::Active => Some(false),
                    _ => None,
                }
            } else {
                None
            },
            hand_supported,
            reactions,
        })
    }
}

/// Source names only; the diagnostic boundary never receives the payload.
pub(crate) fn evidence(observation: Observation) -> Evidence {
    match observation {
        Observation::Account => Evidence::Account,
        Observation::Capabilities => Evidence::Capabilities,
        Observation::Room => Evidence::Room,
        Observation::ConnectedParticipants => Evidence::Participants,
        Observation::SignalingSettings => Evidence::SignalingSettings,
        Observation::CallParticipants => Evidence::CallParticipants,
    }
}
