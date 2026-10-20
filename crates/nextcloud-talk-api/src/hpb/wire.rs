//! Protocol shape follows Talk 25.0.6 eed360d176edcb6888dad60965cb694e6bf62ae3
//! and signaling 2.1.1 e7a8fb7aa9b99ad9f41e0e1793ce147ed36ecc62.
//! In particular, subscriber SIDs come from the SFU; a requestoffer SID is not
//! a client-created negotiation ID. HTTP and HPB session IDs are distinct.
use crate::{
    ErrorKind,
    events::LiveEvent,
    models::RoomToken,
    signaling::{
        DescriptionKind, IceCandidate, Negotiation, PeerSession, SessionDescription, Signal,
    },
};
use serde_json::{Value, json};

pub(super) const FRAME_BYTES: usize = 128 * 1024;
pub(super) fn text(value: &Value, max: usize) -> Result<&str, ErrorKind> {
    let s = value.as_str().ok_or(ErrorKind::Malformed)?;
    if s.is_empty() || s.len() > max || s.chars().any(char::is_control) {
        return Err(ErrorKind::Malformed);
    }
    Ok(s)
}
pub(super) fn features(v: &Value) -> Result<Vec<String>, ErrorKind> {
    let Some(v) = v.as_array() else {
        return if v.is_null() {
            Ok(Vec::new())
        } else {
            Err(ErrorKind::Malformed)
        };
    };
    if v.len() > 64 {
        return Err(ErrorKind::TooLarge);
    }
    v.iter().map(|v| text(v, 128).map(str::to_owned)).collect()
}
pub(super) fn failure(v: &Value) -> ErrorKind {
    match v["code"].as_str() {
        Some("not_allowed" | "auth_failed" | "invalid_ticket" | "invalid_token") => {
            ErrorKind::PermissionDenied
        }
        Some("token_expired" | "token_not_valid_yet") => ErrorKind::ReauthenticationRequired,
        Some("no_such_room" | "no_such_session" | "client_not_found") => ErrorKind::NotFound,
        Some("too_many_requests") => ErrorKind::RateLimited,
        Some("processing_failed") => ErrorKind::ServerUnavailable,
        _ => ErrorKind::Malformed,
    }
}
#[derive(Clone)]
pub(super) struct Member {
    pub sfu: PeerSession,
    pub talk: PeerSession,
}
pub(super) fn member(value: &Value) -> Result<Member, ErrorKind> {
    if value["federated"].as_bool() == Some(true) {
        return Err(ErrorKind::Unsupported);
    }
    Ok(Member {
        sfu: PeerSession::parse(text(&value["sessionid"], 512)?)?,
        talk: PeerSession::parse(text(&value["roomsessionid"], 512)?)?,
    })
}
pub(super) fn hint(
    value: &Value,
    current: Option<&RoomToken>,
) -> Result<Option<LiveEvent>, ErrorKind> {
    let target = text(&value["target"], 32)?;
    let kind = text(&value["type"], 64)?;
    let room = |value: &Value| RoomToken::parse(text(value, 30)?);
    Ok(match (target, kind) {
        ("roomlist", "invite" | "disinvite" | "update") => {
            let token = room(&value[kind]["roomid"])?;
            Some(LiveEvent::RoomChanged { room: token })
        }
        ("participants", "update") => {
            let token = room(&value["update"]["roomid"])?;
            Some(LiveEvent::CallChanged { room: token })
        }
        ("room", "message") => {
            let token = room(&value["message"]["roomid"])?;
            if value["message"]["data"]["type"].as_str() == Some("chat") {
                Some(LiveEvent::ChatAvailable { room: token })
            } else {
                Some(LiveEvent::RoomChanged { room: token })
            }
        }
        ("room", "join" | "leave" | "change") => current
            .cloned()
            .map(|room| LiveEvent::MembershipChanged { room }),
        ("room", "switchto") => Some(LiveEvent::RefreshAll),
        _ => None,
    })
}
pub(super) fn signal(
    value: &Value,
) -> Result<Option<(PeerSession, Negotiation, Signal)>, ErrorKind> {
    let data = &value["data"];
    let Some(kind) = data["type"].as_str() else {
        return Ok(None);
    };
    if !matches!(kind, "offer" | "answer" | "candidate") {
        return Ok(None);
    }
    if value["sender"]["type"].as_str() != Some("session") {
        return Err(ErrorKind::WrongScope);
    }
    let from = PeerSession::parse(text(&value["sender"]["sessionid"], 512)?)?;
    if data.get("from").is_some() && data["from"].as_str() != Some(from.0.as_str()) {
        return Err(ErrorKind::WrongScope);
    }
    if data["roomType"].as_str() != Some("video") {
        return Err(ErrorKind::Unsupported);
    }
    let sid = Negotiation::parse(text(&data["sid"], 64)?)?;
    let payload = &data["payload"];
    let signal = match kind {
        "offer" | "answer" => {
            if payload["type"].as_str() != Some(kind) {
                return Err(ErrorKind::Malformed);
            }
            Signal::Description(
                SessionDescription::new(
                    if kind == "offer" {
                        DescriptionKind::Offer
                    } else {
                        DescriptionKind::Answer
                    },
                    payload["sdp"].as_str().ok_or(ErrorKind::Malformed)?,
                )
                .map_err(|_| ErrorKind::Malformed)?,
            )
        }
        "candidate" => {
            let c = &payload["candidate"];
            if c.is_null() || c["candidate"].as_str() == Some("") {
                return Ok(None);
            }
            Signal::Candidate(
                IceCandidate::new(
                    text(&c["sdpMid"], 32)?,
                    c["sdpMLineIndex"]
                        .as_u64()
                        .and_then(|n| u8::try_from(n).ok())
                        .ok_or(ErrorKind::Malformed)?,
                    text(&c["candidate"], 2048)?,
                )
                .map_err(|_| ErrorKind::Malformed)?,
            )
        }
        _ => return Ok(None),
    };
    Ok(Some((from, sid, signal)))
}
pub(super) fn message(
    peer: &PeerSession,
    sid: Option<&Negotiation>,
    signal: Option<Signal>,
) -> Value {
    let mut data = match signal {
        Some(Signal::Description(s)) => {
            let kind = if s.kind() == DescriptionKind::Offer {
                "offer"
            } else {
                "answer"
            };
            json!({"type":kind,"payload":{"type":kind,"sdp":s.as_str()}})
        }
        Some(Signal::Candidate(c)) => json!({"type":"candidate","payload":{"candidate":{
            "candidate":c.candidate(),"sdpMid":c.mid(),"sdpMLineIndex":c.index()
        }}}),
        None => json!({"type":"requestoffer"}),
    };
    data["roomType"] = json!("video");
    data["to"] = json!(peer.0.as_str());
    if let Some(sid) = sid {
        data["sid"] = json!(sid.0);
    }
    json!({"type":"message","message":{"recipient":{"type":"session","sessionid":peer.0.as_str()},"data":data}})
}
