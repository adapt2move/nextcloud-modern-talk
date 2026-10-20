//! Engine-neutral, bounded SDP/ICE values. No devices, codecs, IPC or peer connection.
use std::{fmt, io};
pub const SDP_MAX: usize = 65536;
pub const CANDIDATE_MAX: usize = 2048;
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid protocol signal")
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum DescriptionKind {
    Offer = 1,
    Answer = 2,
}

pub struct SessionDescription {
    kind: DescriptionKind,
    text: Box<str>,
}

impl fmt::Debug for SessionDescription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionDescription")
            .field("kind", &self.kind)
            .field("bytes", &self.text.len())
            .finish_non_exhaustive()
    }
}

impl SessionDescription {
    /// Validate borrowed bytes before an upstream owner reserves/copies payload.
    pub fn validate(text: &str) -> io::Result<()> {
        if text.len() > SDP_MAX
            || !text.starts_with("v=0\r\n")
            || text.as_bytes().contains(&0)
            || text.split('\n').any(|line| line.len() > 2048)
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn new(kind: DescriptionKind, text: &str) -> io::Result<Self> {
        Self::validate(text)?;
        Ok(Self {
            kind,
            text: text.into(),
        })
    }
    pub fn kind(&self) -> DescriptionKind {
        self.kind
    }
    pub fn as_str(&self) -> &str {
        &self.text
    }
}
pub struct IceCandidate {
    mid: Box<str>,
    index: u8,
    candidate: Box<str>,
}

impl fmt::Debug for IceCandidate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("IceCandidate([redacted])")
    }
}

impl IceCandidate {
    pub fn validate(mid: &str, index: u8, candidate: &str) -> io::Result<()> {
        if mid.is_empty()
            || mid.len() > 64
            || index >= 24
            || candidate.len() > CANDIDATE_MAX
            || !candidate.starts_with("candidate:")
            || !mid.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || candidate
                .bytes()
                .any(|b| b == 0 || b == b'\r' || b == b'\n')
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn new(mid: &str, index: u8, candidate: &str) -> io::Result<Self> {
        Self::validate(mid, index, candidate)?;
        Ok(Self {
            mid: mid.into(),
            index,
            candidate: candidate.into(),
        })
    }
    pub fn mid(&self) -> &str {
        &self.mid
    }
    pub fn index(&self) -> u8 {
        self.index
    }
    pub fn candidate(&self) -> &str {
        &self.candidate
    }
}

use crate::{
    Client, Error, ErrorKind,
    models::RoomToken,
    scope::{OperationContext, OriginId, SessionScope},
};
use serde_json::{Value, json};
use zeroize::Zeroizing;

/// Exact network session identity. It never authorizes a device or chooses a role.
#[derive(Clone, Eq, PartialEq)]
pub struct PeerSession(pub(crate) Zeroizing<String>);
impl PeerSession {
    pub fn parse(text: &str) -> Result<Self, ErrorKind> {
        if text.is_empty()
            || text == "0"
            || text.len() > 512
            || !text.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(ErrorKind::Malformed);
        }
        Ok(Self(Zeroizing::new(text.into())))
    }
    pub fn matches(&self, other: &Self) -> bool {
        self == other
    }
    pub fn local_should_offer(&self, peer: &Self) -> bool {
        peer.0.as_str() < self.0.as_str()
    }
}
#[derive(Clone, Eq, PartialEq)]
pub struct Negotiation(pub(crate) String);
impl Negotiation {
    pub fn parse(text: &str) -> Result<Self, ErrorKind> {
        if text.is_empty()
            || text.len() > 64
            || !text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        {
            return Err(ErrorKind::Malformed);
        }
        Ok(Self(text.into()))
    }
    pub fn matches(&self, other: &Self) -> bool {
        self == other
    }
}
/// Returned only after this client's successful HTTP join. It proves membership,
/// not capture permission, call policy freshness or end-to-end encryption support.
pub struct CallHandle {
    pub(crate) room: RoomToken,
    pub(crate) room_id: Option<u64>,
    pub(crate) origin: OriginId,
    pub(crate) owner: std::sync::Arc<()>,
    pub(crate) scope: SessionScope,
    pub(crate) generation: u64,
}
pub enum Signal {
    Description(SessionDescription),
    Candidate(IceCandidate),
}
pub enum Event {
    Signal {
        from: PeerSession,
        negotiation: Negotiation,
        signal: Signal,
    },
    Membership(Vec<Member>),
    /// Scoped hints are represented explicitly and never become media commands.
    Hint {
        from: PeerSession,
        kind: Hint,
    },
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Hint {
    MuteAudio,
    MuteVideo,
    UnmuteAudio,
    UnmuteVideo,
    EndOfCandidates,
}
pub struct Member {
    pub room_id: u64,
    pub session: PeerSession,
    pub in_call: Option<u32>,
    pub permissions: u64,
}
/// One completely validated response, at most 84 events / 192 KiB retained signal text.
pub struct SignalBatch {
    events: Vec<Event>,
}
impl SignalBatch {
    pub fn events(&self) -> &[Event] {
        &self.events
    }
    pub fn into_events(self) -> Vec<Event> {
        self.events
    }
}
/// HTTP receipt only: the server may discard an obsolete recipient/session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AcceptedForProcessing;

impl Client {
    fn validate_call(&self, c: &OperationContext, call: &CallHandle) -> Result<(), Error> {
        self.validate(c)?;
        if !std::sync::Arc::ptr_eq(&self.owner, &call.owner)
            || call.origin != self.transport.host()
            || call.scope != c.scope
            || self.call_generation != Some(call.generation)
            || self.active_room.as_ref() != Some(&call.room)
            || self.room_session.is_none()
        {
            return Err(self.error(ErrorKind::WrongScope));
        }
        Ok(())
    }
    pub async fn send_signal(
        &mut self,
        c: &OperationContext,
        call: &CallHandle,
        to: &PeerSession,
        negotiation: &Negotiation,
        signal: Signal,
    ) -> Result<AcceptedForProcessing, Error> {
        self.validate_call(c, call)?;
        let counter = if matches!(&signal, Signal::Description(_)) {
            &mut self.sent_descriptions
        } else {
            &mut self.sent_candidates
        };
        let max = if matches!(&signal, Signal::Description(_)) {
            2
        } else {
            64
        };
        if *counter >= max {
            self.call_generation = None;
            return Err(self.error(ErrorKind::TooLarge));
        }
        *counter += 1;
        let (kind, payload) = match signal {
            Signal::Description(s) => {
                let kind = if s.kind() == DescriptionKind::Offer {
                    "offer"
                } else {
                    "answer"
                };
                (kind, json!({"type":kind,"sdp":s.as_str()}))
            }
            Signal::Candidate(s) => (
                "candidate",
                json!({"candidate":{"candidate":s.candidate(),"sdpMid":s.mid(),"sdpMLineIndex":s.index()}}),
            ),
        };
        let inner = json!({"to":to.0.as_str(),"sid":negotiation.0,"roomType":"video","type":kind,"payload":payload});
        let messages=json!([{"ev":"message","sessionId":self.room_session.as_ref().expect("validated session").as_str(),"fn":inner.to_string()}]).to_string();
        if messages.len() > 128 * 1024 {
            return Err(self.error(ErrorKind::TooLarge));
        }
        let route = format!(
            "ocs/v2.php/apps/spreed/api/v3/signaling/{}",
            call.room.as_str()
        );
        self.call_generation = None;
        let result = self
            .transport
            .ocs(
                c,
                reqwest::Method::POST,
                &route,
                &[],
                &[("messages", messages)],
                false,
            )
            .await;
        if let Err(error) = result {
            self.call_generation = None;
            return Err(self.mutation_error(error));
        }
        self.validate(c)?;
        self.call_generation = Some(call.generation);
        Ok(AcceptedForProcessing)
    }
    /// A destructive server read with a separate 35-second HTTP ceiling and the caller's
    /// earlier deadline. Failure terminates this exchange; it is never automatically replayed.
    pub async fn poll_signals(
        &mut self,
        c: &OperationContext,
        call: &CallHandle,
    ) -> Result<SignalBatch, Error> {
        self.validate_call(c, call)?;
        let room_id = call
            .room_id
            .ok_or_else(|| self.error(ErrorKind::WrongScope))?;
        // Mark before await so dropping an admitted read retires this handle too.
        self.call_generation = None;
        let route = format!(
            "ocs/v2.php/apps/spreed/api/v3/signaling/{}",
            call.room.as_str()
        );
        let result = self
            .transport
            .signaling_poll(c, &route)
            .await
            .map_err(|_| self.error(ErrorKind::Uncertain))?;
        let local = self
            .room_session
            .as_ref()
            .ok_or_else(|| self.error(ErrorKind::WrongScope))?;
        let batch = parse_batch(&result.data, local, room_id).map_err(|kind| self.error(kind))?;
        self.validate(c)?;
        self.call_generation = Some(call.generation);
        Ok(batch)
    }
}
fn text(v: Option<&Value>, max: usize) -> Result<&str, ErrorKind> {
    let s = v.and_then(Value::as_str).ok_or(ErrorKind::Malformed)?;
    if s.len() > max {
        return Err(ErrorKind::TooLarge);
    }
    Ok(s)
}
pub(crate) fn parse_batch(
    value: &Value,
    local: &str,
    expected_room: u64,
) -> Result<SignalBatch, ErrorKind> {
    let rows = value.as_array().ok_or(ErrorKind::Malformed)?;
    if rows.len() > 84 {
        return Err(ErrorKind::TooLarge);
    }
    let mut events = Vec::with_capacity(rows.len());
    let (mut descriptions, mut candidates, mut snapshots, mut hints, mut retained) =
        (0, 0, 0, 0, 0usize);
    for row in rows {
        match row["type"].as_str() {
            Some("usersInRoom") => {
                snapshots += 1;
                if snapshots > 2 {
                    return Err(ErrorKind::TooLarge);
                }
                let users = row["data"].as_array().ok_or(ErrorKind::Malformed)?;
                if users.len() > 17 {
                    return Err(ErrorKind::TooLarge);
                }
                let mut members: Vec<Member> = Vec::with_capacity(users.len());
                for user in users {
                    let session = PeerSession::parse(text(user.get("sessionId"), 512)?)?;
                    if members.iter().any(|m| m.session == session) {
                        return Err(ErrorKind::Malformed);
                    }
                    let room_id = user["roomId"]
                        .as_u64()
                        .filter(|n| *n > 0)
                        .ok_or(ErrorKind::Malformed)?;
                    if room_id != expected_room {
                        return Err(ErrorKind::WrongScope);
                    }
                    let in_call = match user.get("inCall") {
                        None | Some(Value::Null) => None,
                        Some(v) => Some(
                            v.as_u64()
                                .and_then(|v| u32::try_from(v).ok())
                                .ok_or(ErrorKind::Malformed)?,
                        ),
                    };
                    let permissions = user["participantPermissions"]
                        .as_u64()
                        .ok_or(ErrorKind::Malformed)?;
                    if user["lastPing"].as_u64().is_none()
                        || ["userId", "actorType", "actorId"]
                            .iter()
                            .any(|k| user[k].as_str().is_none())
                    {
                        return Err(ErrorKind::Malformed);
                    }
                    retained += session.0.len();
                    members.push(Member {
                        room_id,
                        session,
                        in_call,
                        permissions,
                    });
                }
                events.push(Event::Membership(members));
            }
            Some("message") => {
                let raw = text(row.get("data"), 131072)?;
                let data = crate::json::json(raw.as_bytes())?;
                if data["roomType"].as_str() != Some("video") {
                    return Err(ErrorKind::Unsupported);
                }
                if data["to"].as_str() != Some(local) {
                    return Err(ErrorKind::WrongScope);
                }
                let from = PeerSession::parse(text(data.get("from"), 512)?)?;
                retained += from.0.len();
                let kind = text(data.get("type"), 32)?;
                if matches!(kind, "mute" | "unmute" | "endOfCandidates") {
                    hints += 1;
                    if hints > 16 {
                        return Err(ErrorKind::TooLarge);
                    }
                    if let Some(sid) = data.get("sid") {
                        Negotiation::parse(text(Some(sid), 64)?)?;
                    }
                    let hint = match (kind, data["payload"]["name"].as_str()) {
                        ("mute", Some("audio")) => Hint::MuteAudio,
                        ("mute", Some("video")) => Hint::MuteVideo,
                        ("unmute", Some("audio")) => Hint::UnmuteAudio,
                        ("unmute", Some("video")) => Hint::UnmuteVideo,
                        ("endOfCandidates", _)
                            if data.get("payload").is_none_or(|p| {
                                p.is_null() || p.as_object().is_some_and(|v| v.is_empty())
                            }) =>
                        {
                            Hint::EndOfCandidates
                        }
                        _ => return Err(ErrorKind::Unsupported),
                    };
                    events.push(Event::Hint { from, kind: hint });
                } else {
                    let negotiation = Negotiation::parse(text(data.get("sid"), 64)?)?;
                    retained += negotiation.0.len();
                    let payload = &data["payload"];
                    let signal = match kind {
                        "offer" | "answer" => {
                            descriptions += 1;
                            if descriptions > 2 {
                                return Err(ErrorKind::TooLarge);
                            }
                            if payload["type"].as_str() != Some(kind) {
                                return Err(ErrorKind::Malformed);
                            }
                            let sdp = text(payload.get("sdp"), SDP_MAX)?;
                            retained += sdp.len();
                            Signal::Description(
                                SessionDescription::new(
                                    if kind == "offer" {
                                        DescriptionKind::Offer
                                    } else {
                                        DescriptionKind::Answer
                                    },
                                    sdp,
                                )
                                .map_err(|_| ErrorKind::Malformed)?,
                            )
                        }
                        "candidate" => {
                            candidates += 1;
                            if candidates > 64 {
                                return Err(ErrorKind::TooLarge);
                            }
                            let p = &payload["candidate"];
                            let candidate = text(p.get("candidate"), CANDIDATE_MAX)?;
                            let mid = text(p.get("sdpMid"), 64)?;
                            let index = p["sdpMLineIndex"]
                                .as_u64()
                                .and_then(|n| u8::try_from(n).ok())
                                .ok_or(ErrorKind::Malformed)?;
                            retained += candidate.len() + mid.len();
                            Signal::Candidate(
                                IceCandidate::new(mid, index, candidate)
                                    .map_err(|_| ErrorKind::Malformed)?,
                            )
                        }
                        _ => return Err(ErrorKind::Unsupported),
                    };
                    events.push(Event::Signal {
                        from,
                        negotiation,
                        signal,
                    });
                }
            }
            _ => return Err(ErrorKind::Unsupported),
        }
        if retained > 192 * 1024 {
            return Err(ErrorKind::TooLarge);
        }
    }
    Ok(SignalBatch { events })
}
