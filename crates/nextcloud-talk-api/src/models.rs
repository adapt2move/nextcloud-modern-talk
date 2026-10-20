//! Bounded protocol observations. Unknown optional values remain absent, never permission grants.
use crate::error::ErrorKind;
use serde_json::Value;

pub(crate) fn integer(value: Option<&Value>) -> Option<u64> {
    value.and_then(|v| {
        v.as_u64().or_else(|| {
            v.as_str()
                .filter(|s| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|s| s.parse().ok())
        })
    })
}
pub(crate) fn string(v: Option<&Value>, max: usize) -> Result<Option<String>, ErrorKind> {
    match v.and_then(Value::as_str) {
        Some(s) if s.len() > max => Err(ErrorKind::TooLarge),
        Some(s) => Ok(Some(s.to_owned())),
        None => Ok(None),
    }
}
#[derive(Clone, Eq, PartialEq)]
pub struct RoomToken(String);
impl RoomToken {
    pub fn parse(token: &str) -> Result<Self, ErrorKind> {
        if !(4..=30).contains(&token.len())
            || !token
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        {
            return Err(ErrorKind::Malformed);
        }
        Ok(Self(token.to_owned()))
    }
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}
impl std::fmt::Debug for RoomToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RoomToken([opaque])")
    }
}
/// These are server protocol identifiers, never app navigation IDs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TagId(String);
impl TagId {
    pub fn parse(id: &str) -> Result<Self, ErrorKind> {
        if id.is_empty()
            || id.len() > 32
            || id.starts_with('0')
            || !id.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(ErrorKind::Malformed);
        }
        Ok(Self(id.into()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
#[derive(Clone, Debug)]
pub struct Account {
    pub id: String,
    pub display_name: Option<String>,
}
impl Account {
    pub(crate) fn decode(v: &Value) -> Result<Self, ErrorKind> {
        if !v.is_object() {
            return Err(ErrorKind::Malformed);
        }
        let mut v = v.clone();
        if v.get("displayname").is_none() {
            v["displayname"] = v["display-name"].clone();
        }
        let v = &v;
        let result = Self {
            id: v
                .get("id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control))
                .ok_or(ErrorKind::Malformed)?
                .to_owned(),
            display_name: string(v.get("displayname"), 256)?,
        };
        if result.id.is_empty() || result.id.chars().any(char::is_control) {
            return Err(ErrorKind::Malformed);
        }
        Ok(result)
    }
}
#[derive(Clone, Debug)]
pub struct Room {
    pub token: String,
    pub server_id: Option<u64>,
    pub name: String,
    pub kind: Option<u64>,
    pub participant_type: Option<u64>,
    pub attributes: Option<u64>,
    pub permissions: Option<u64>,
    pub read_only: Option<u64>,
    pub lobby_state: Option<u64>,
    pub object_type: Option<String>,
    pub can_leave: Option<bool>,
    pub favorite: Option<bool>,
    pub archived: Option<bool>,
    pub unread: Option<u64>,
    pub last_activity: Option<u64>,
    pub has_call: Option<bool>,
    pub sensitive: Option<bool>,
    pub remote: bool,
    pub tag_ids: Option<Vec<String>>,
    pub description: Option<String>,
    pub default_permissions: Option<u64>,
    pub call_permissions: Option<u64>,
    pub lobby_timer: Option<u64>,
    pub recording_consent: Option<u64>,
    pub sip_enabled: Option<u64>,
    pub object_id: Option<String>,
    pub notification_level: Option<i64>,
    pub notification_calls: Option<i64>,
    pub important: Option<bool>,
    pub message_expiration: Option<u64>,
    pub listable: Option<u64>,
    pub mention_permissions: Option<u64>,
    pub has_password: Option<bool>,
    pub can_delete: Option<bool>,
    pub can_start_call: Option<bool>,
    pub call_flag: Option<u64>,
    pub recording_status: Option<u64>,
    pub last_pinned: Option<u64>,
    pub hidden_pinned: Option<u64>,
}
impl Room {
    pub(crate) fn decode(v: &Value) -> Result<Self, ErrorKind> {
        if !v.is_object() {
            return Err(ErrorKind::Malformed);
        }
        let mut v = v.clone();
        if v.get("displayName").is_none() {
            v["displayName"] = v["name"].clone();
        }
        let v = &v;
        let result = Self {
            token: string(v.get("token"), 30)?.ok_or(ErrorKind::Malformed)?,
            server_id: v.get("id").and_then(Value::as_u64).filter(|n| *n > 0),
            name: string(v.get("displayName"), 256)?.ok_or(ErrorKind::Malformed)?,
            kind: integer(v.get("type")),
            participant_type: integer(v.get("participantType")),
            attributes: integer(v.get("attributes")),
            permissions: integer(v.get("permissions")),
            read_only: integer(v.get("readOnly")),
            lobby_state: integer(v.get("lobbyState")),
            object_type: string(v.get("objectType"), 256)?,
            can_leave: v.get("canLeaveConversation").and_then(Value::as_bool),
            favorite: v.get("isFavorite").and_then(Value::as_bool),
            archived: v.get("isArchived").and_then(Value::as_bool),
            unread: integer(v.get("unreadMessages")),
            last_activity: integer(v.get("lastActivity")),
            has_call: v.get("hasCall").and_then(Value::as_bool),
            sensitive: v.get("isSensitive").and_then(Value::as_bool),
            remote: ["remoteServer", "remoteToken"]
                .iter()
                .any(|key| v.get(key).is_some_and(|v| v.as_str() != Some(""))),
            tag_ids: strings(v.get("tagIds"), 20, true)?,
            description: string(v.get("description"), 4000)?,
            default_permissions: integer(v.get("defaultPermissions")),
            call_permissions: integer(v.get("callPermissions")),
            lobby_timer: integer(v.get("lobbyTimer")),
            recording_consent: integer(v.get("recordingConsent")),
            sip_enabled: integer(v.get("sipEnabled")),
            object_id: string(v.get("objectId"), 1024)?,
            notification_level: v.get("notificationLevel").and_then(Value::as_i64),
            notification_calls: v.get("notificationCalls").and_then(Value::as_i64),
            important: v.get("isImportant").and_then(Value::as_bool),
            message_expiration: integer(v.get("messageExpiration")),
            listable: integer(v.get("listable")),
            mention_permissions: integer(v.get("mentionPermissions")),
            has_password: v.get("hasPassword").and_then(Value::as_bool),
            can_delete: v.get("canDeleteConversation").and_then(Value::as_bool),
            can_start_call: v.get("canStartCall").and_then(Value::as_bool),
            call_flag: integer(v.get("callFlag")),
            recording_status: integer(v.get("callRecording")),
            last_pinned: integer(v.get("lastPinnedId")),
            hidden_pinned: integer(v.get("hiddenPinnedId")),
        };
        RoomToken::parse(&result.token)?;
        Ok(result)
    }
}
#[derive(Clone, Debug)]
pub struct Participant {
    pub display_name: Option<String>,
    pub participant_type: Option<u64>,
    pub actor_type: Option<String>,
    pub actor_id: Option<String>,
    pub in_call: Option<u64>,
    pub sessions: Option<Vec<String>>,
    pub attendee_id: Option<u64>,
    pub permissions: Option<u64>,
    pub room_token: Option<String>,
}
impl Participant {
    pub(crate) fn decode(v: &Value) -> Result<Self, ErrorKind> {
        if !v.is_object() {
            return Err(ErrorKind::Malformed);
        }
        let result = Self {
            display_name: string(v.get("displayName"), 256)?,
            participant_type: integer(v.get("participantType")),
            actor_type: string(v.get("actorType"), 128)?,
            actor_id: string(v.get("actorId"), 256)?,
            in_call: integer(v.get("inCall")),
            sessions: strings(v.get("sessionIds"), 17, false)?,
            attendee_id: integer(v.get("attendeeId")),
            permissions: integer(v.get("permissions")),
            room_token: string(v.get("roomToken"), 30)?,
        };
        Ok(result)
    }
}
#[derive(Clone, Debug)]
pub struct Message {
    pub token: String,
    pub id: u64,
    pub kind: Option<String>,
    pub reference: Option<String>,
    pub actor_type: Option<String>,
    pub actor_id: Option<String>,
    pub author: String,
    pub message: Option<String>,
    pub timestamp: u64,
    pub expires: Option<u64>,
    pub edited: Option<u64>,
    pub replyable: Option<bool>,
    pub parent: Option<crate::chat::Parent>,
    pub thread_id: Option<u64>,
    pub thread_title: Option<String>,
    pub thread_replies: Option<u64>,
    pub system: Option<String>,
    pub silent: Option<bool>,
    pub reactions: Vec<crate::chat::Reaction>,
    pub rich: Vec<crate::chat::RichObject>,
    /// Canonical parent supplied by a hidden edit/delete/reaction event.
    pub update: Option<Box<Message>>,
}
impl Message {
    pub(crate) fn decode(v: &Value) -> Result<Self, ErrorKind> {
        if !v.is_object() {
            return Err(ErrorKind::Malformed);
        }
        let result = Self {
            token: string(v.get("token"), 30)?.ok_or(ErrorKind::Malformed)?,
            id: integer(v.get("id")).ok_or(ErrorKind::Malformed)?,
            kind: string(v.get("messageType"), 128)?,
            reference: string(v.get("referenceId"), 256)?,
            actor_type: string(v.get("actorType"), 128)?,
            actor_id: string(v.get("actorId"), 256)?,
            author: string(v.get("actorDisplayName"), 256)?.ok_or(ErrorKind::Malformed)?,
            message: string(v.get("message"), 131072)?,
            timestamp: integer(v.get("timestamp")).ok_or(ErrorKind::Malformed)?,
            expires: integer(v.get("expirationTimestamp")),
            edited: integer(v.get("lastEditTimestamp")),
            replyable: v.get("isReplyable").and_then(Value::as_bool),
            parent: crate::chat::Parent::optional(v.get("parent"))?,
            thread_id: integer(v.get("threadId")).filter(|n| *n > 0),
            thread_title: string(v.get("threadTitle"), 256)?,
            thread_replies: integer(v.get("threadReplies")),
            system: string(v.get("systemMessage"), 128)?,
            // Talk 24/25 Message::toArray omits false. An explicitly malformed
            // value still conveys no notification permission.
            silent: match v.get("silent") {
                None => Some(false),
                Some(value) => value.as_bool(),
            },
            reactions: crate::chat::reactions(v)?,
            rich: crate::chat::rich(v.get("messageParameters"))?,
            update: if matches!(
                v["systemMessage"].as_str(),
                Some("message_edited" | "message_deleted" | "reaction" | "reaction_revoked")
            ) {
                if let Some(parent) = v.get("parent").filter(|p| p.is_object()) {
                    if parent.get("parent").is_some() {
                        return Err(ErrorKind::Malformed);
                    }
                    let mut parent = parent.clone();
                    if parent.get("token").is_none() {
                        parent["token"] = v["token"].clone();
                    }
                    if parent.get("timestamp").is_none() {
                        parent["timestamp"] = v["timestamp"].clone();
                    }
                    if parent.get("actorDisplayName").is_none() {
                        parent["actorDisplayName"] = Value::String(String::new());
                    }
                    if v["systemMessage"].as_str() == Some("message_deleted")
                        || parent["deleted"].as_bool() == Some(true)
                    {
                        parent["messageType"] = Value::String("comment_deleted".into());
                        parent["message"] = Value::String(String::new());
                        parent["messageParameters"] = serde_json::json!({});
                        parent["reactions"] = serde_json::json!({});
                    }
                    parent["systemMessage"] = Value::String(String::new());
                    Some(Box::new(Self::decode(&parent)?))
                } else {
                    None
                }
            } else {
                None
            },
        };
        RoomToken::parse(&result.token)?;
        if result
            .parent
            .as_ref()
            .is_some_and(|p| p.token.as_deref().is_some_and(|t| t != result.token))
        {
            return Err(ErrorKind::WrongScope);
        }
        if result.id == 0 {
            return Err(ErrorKind::Malformed);
        }
        Ok(result)
    }
}
#[derive(Clone, Debug)]
pub struct CallParticipant {
    pub session: String,
    pub actor_type: Option<String>,
    pub actor_id: Option<String>,
}
impl CallParticipant {
    pub(crate) fn decode(v: &Value) -> Result<Self, ErrorKind> {
        if !v.is_object() {
            return Err(ErrorKind::Malformed);
        }
        let result = Self {
            session: string(v.get("sessionId"), 512)?.ok_or(ErrorKind::Malformed)?,
            actor_type: string(v.get("actorType"), 128)?,
            actor_id: string(v.get("actorId"), 256)?,
        };
        Ok(result)
    }
}
#[derive(Clone, Debug)]
pub struct Tag {
    pub id: String,
    pub name: String,
    pub kind: Option<String>,
    pub order: u64,
    pub collapsed: bool,
}
impl Tag {
    pub(crate) fn decode(v: &Value) -> Result<Self, ErrorKind> {
        if !v.is_object() {
            return Err(ErrorKind::Malformed);
        }
        let result = Self {
            id: string(v.get("id"), 32)?.ok_or(ErrorKind::Malformed)?,
            name: string(v.get("name"), 1000)?.ok_or(ErrorKind::Malformed)?,
            kind: string(v.get("type"), 128)?,
            order: integer(v.get("sortOrder")).ok_or(ErrorKind::Malformed)?,
            collapsed: v
                .get("collapsed")
                .and_then(Value::as_bool)
                .ok_or(ErrorKind::Malformed)?,
        };
        TagId::parse(&result.id)?;
        Ok(result)
    }
}
fn strings(v: Option<&Value>, max: usize, numbers: bool) -> Result<Option<Vec<String>>, ErrorKind> {
    let Some(values) = v.and_then(Value::as_array) else {
        return Ok(None);
    };
    if values.len() > max {
        return Err(ErrorKind::TooLarge);
    }
    values
        .iter()
        .map(|v| match v {
            Value::String(s) if s.len() <= 512 => Ok(s.clone()),
            Value::Number(n) if numbers && n.as_u64().is_some() => Ok(n.to_string()),
            _ => Err(ErrorKind::Malformed),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}
#[derive(Clone, Debug)]
pub struct Capabilities {
    pub notification_endpoints: Vec<String>,
    pub server_major: Option<u64>,
    pub features: Vec<String>,
    pub read_privacy: Option<u64>,
    pub typing_privacy: Option<u64>,
    pub attachment_folder: Option<String>,
    pub attachments_allowed: Option<bool>,
    pub conversation_subfolders: Option<bool>,
    pub can_create: Option<bool>,
    pub message_limit: Option<u64>,
    pub server_default_permissions: Option<u16>,
    pub call_enabled: Option<bool>,
    pub recording_enabled: Option<bool>,
    pub breakouts_enabled: Option<bool>,
    pub grid_limit: Option<u64>,
    pub grid_limit_enforced: Option<bool>,
    pub recording_consent: Option<u64>,
    pub encryption_required: Option<bool>,
}
impl Capabilities {
    /// Tested server families with the required conversation API; no policy grant.
    pub fn supported_protocol(&self) -> bool {
        matches!(self.server_major, Some(34 | 35)) && self.supports("conversation-v4")
    }
    pub fn supports(&self, feature: &str) -> bool {
        self.features.iter().any(|f| f == feature)
    }
    pub(crate) fn decode(data: &Value) -> Result<Self, ErrorKind> {
        let s = &data["capabilities"]["spreed"];
        let features = match s.get("features").and_then(Value::as_array) {
            Some(f) if f.len() <= 256 => f
                .iter()
                .map(|v| string(Some(v), 128)?.ok_or(ErrorKind::Malformed))
                .collect::<Result<Vec<_>, _>>()?,
            Some(_) => return Err(ErrorKind::TooLarge),
            None if s.is_null() => Vec::new(),
            None => return Err(ErrorKind::Malformed),
        };
        Ok(Self {
            notification_endpoints: strings(
                data["capabilities"]["notifications"].get("ocs-endpoints"),
                32,
                false,
            )?
            .unwrap_or_default(),
            server_major: integer(data.get("version").and_then(|v| v.get("major"))),
            features,
            read_privacy: integer(s["config"]["chat"].get("read-privacy")),
            typing_privacy: integer(s["config"]["chat"].get("typing-privacy")),
            attachment_folder: string(s["config"]["attachments"].get("folder"), 16384)?,
            attachments_allowed: s["config"]["attachments"]["allowed"].as_bool(),
            conversation_subfolders: s["config"]["attachments"]["conversation-subfolders"]
                .as_bool(),
            can_create: s["config"]["conversations"]["can-create"].as_bool(),
            message_limit: integer(s["config"]["chat"].get("max-length")),
            server_default_permissions: integer(s["config"]["permissions"].get("default"))
                .filter(|mask| mask & !511 == 0)
                .and_then(|mask| u16::try_from(mask).ok()),
            call_enabled: s["config"]["call"]["enabled"].as_bool(),
            recording_enabled: s["config"]["call"]["recording"].as_bool(),
            breakouts_enabled: s["config"]["call"]["breakout-rooms"].as_bool(),
            grid_limit: integer(s["config"]["call"].get("grid-limit")),
            grid_limit_enforced: s["config"]["call"]["grid-limit-enforced"].as_bool(),
            recording_consent: integer(s["config"]["call"].get("recording-consent")),
            encryption_required: s["config"]["call"]["end-to-end-encryption"].as_bool(),
        })
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignalingMode {
    Internal,
    External,
    Unknown,
}
/// Engine-neutral ICE data. Credentials are redacted from Debug.
#[derive(Clone)]
pub struct IceServer {
    pub urls: Vec<String>,
    pub username: zeroize::Zeroizing<String>,
    pub credential: zeroize::Zeroizing<String>,
}
#[derive(Clone)]
pub struct SignalingSettings {
    pub mode: SignalingMode,
    pub ice_servers: Vec<IceServer>,
    /// Opaque server-issued credentials consumed only by this account's HPB owner.
    pub external: Option<crate::hpb::ExternalSettings>,
}
impl std::fmt::Debug for SignalingSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignalingSettings")
            .field("mode", &self.mode)
            .field("ice_servers", &self.ice_servers.len())
            .finish()
    }
}
impl SignalingSettings {
    pub(crate) fn decode(v: &Value) -> Result<Self, ErrorKind> {
        let mode = match v["signalingMode"].as_str() {
            Some("internal") => SignalingMode::Internal,
            Some("external") => SignalingMode::External,
            _ => SignalingMode::Unknown,
        };
        let mut ice_servers = Vec::new();
        for name in ["stunservers", "turnservers"] {
            let entries = v
                .get(name)
                .and_then(Value::as_array)
                .ok_or(ErrorKind::Malformed)?;
            for entry in entries {
                if ice_servers.len() >= 8 {
                    return Err(ErrorKind::TooLarge);
                }
                let urls = strings(entry.get("urls"), 4, false)?.ok_or(ErrorKind::Malformed)?;
                if urls.is_empty()
                    || urls.iter().any(|s| {
                        s.len() > 2048
                            || s.bytes().any(|b| b.is_ascii_control() || b == b' ')
                            || !(s.starts_with("stun:")
                                || s.starts_with("stuns:")
                                || s.starts_with("turn:")
                                || s.starts_with("turns:"))
                    })
                {
                    return Err(ErrorKind::Malformed);
                }
                ice_servers.push(IceServer {
                    urls,
                    username: zeroize::Zeroizing::new(
                        string(entry.get("username"), 1024)?.unwrap_or_default(),
                    ),
                    credential: zeroize::Zeroizing::new(
                        string(entry.get("credential"), 2048)?.unwrap_or_default(),
                    ),
                });
            }
        }
        let external = if mode == SignalingMode::External {
            Some(crate::hpb::ExternalSettings::decode(v)?)
        } else {
            None
        };
        Ok(Self {
            mode,
            ice_servers,
            external,
        })
    }
}
/// Session IDs are network-only; never Debug/Clone or included in a core/UI snapshot.
pub struct RoomSession(pub(crate) zeroize::Zeroizing<String>);
impl RoomSession {
    pub fn into_secret(self) -> zeroize::Zeroizing<String> {
        self.0
    }
}
#[derive(Clone, Copy, Debug)]
pub struct ParticipantFlags(u8);
impl ParticipantFlags {
    pub fn new(audio: bool, video: bool) -> Self {
        Self(1 | if audio { 2 } else { 0 } | if video { 4 } else { 0 })
    }
    pub fn bits(self) -> u8 {
        self.0
    }
}
pub enum Preference {
    ReadReceipts(bool),
    TypingStatus(bool),
    AttachmentFolder(String),
}

#[cfg(test)]
mod message_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn silent_omission_matches_talk_without_trusting_malformed_values() {
        let mut message = json!({
            "id": 42, "token": "testroom", "actorDisplayName": "Bob", "timestamp": 1
        });
        assert_eq!(Message::decode(&message).unwrap().silent, Some(false));
        for (value, expected) in [
            (json!(false), Some(false)),
            (json!(true), Some(true)),
            (Value::Null, None),
            (json!("false"), None),
            (json!(0), None),
        ] {
            message["silent"] = value;
            assert_eq!(Message::decode(&message).unwrap().silent, expected);
        }
    }
}
