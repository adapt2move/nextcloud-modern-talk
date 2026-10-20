//! Private, bounded decoding and conservative capability/permission normalization.
use serde_json::Value;
use talk_core::bounded::Text;
use talk_core::ids::{AccountScope, ConversationId};
use talk_core::policy::Decision;
use talk_core::ports::ErrorKind;
use talk_core::protocol::{
    AccountProbe, Conversation, ConversationOrganization, FeatureSupport, Role, RoomActions,
    RoomKind, ServerFamily,
};

#[cfg(any(test, feature = "backend-test"))]
pub(super) fn json(bytes: &[u8]) -> Result<Value, ErrorKind> {
    nextcloud_talk_api::diagnostics::bounded_json(bytes).map_err(super::transport::kind)
}
pub(super) fn integer(value: Option<&Value>) -> Option<u64> {
    value.and_then(|v| {
        v.as_u64().or_else(|| {
            v.as_str().and_then(|s| {
                if !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) {
                    s.parse().ok()
                } else {
                    None
                }
            })
        })
    })
}
pub(super) fn text<const N: usize>(value: Option<&Value>) -> Result<Text<N>, ErrorKind> {
    Text::new(value.and_then(Value::as_str).ok_or(ErrorKind::Malformed)?)
        .map_err(|_| ErrorKind::TooLarge)
}
pub(super) fn role(value: Option<&Value>) -> Role {
    match integer(value) {
        Some(1) => Role::Owner,
        Some(2) => Role::Moderator,
        Some(3) => Role::Member,
        Some(4) => Role::Guest,
        Some(5) => Role::Following,
        Some(6) => Role::GuestModerator,
        _ => Role::Unknown,
    }
}
fn boolean(value: Option<&Value>) -> Decision {
    match value.and_then(Value::as_bool) {
        Some(true) => Decision::Allowed,
        Some(false) => Decision::Denied,
        None => Decision::Unknown,
    }
}

#[derive(Clone)]
pub(super) struct Capabilities {
    pub read_receipts: Option<bool>,
    pub typing_status: Option<bool>,
    pub attachment_folder: Option<talk_core::preferences::account::NextcloudFolder>,
    pub attachments_allowed: Option<bool>,
    pub conversation_subfolders: Option<bool>,
    pub features: Vec<String>,
    pub family: ServerFamily,
    pub can_create: Decision,
    pub message_limit: Option<u32>,
    pub server_default_permissions: Option<u16>,
    pub call_enabled: Decision,
    pub recording_consent: Option<u64>,
    pub recording_enabled: Option<bool>,
    pub breakouts_enabled: Option<bool>,
    pub encryption_required: Option<bool>,
}
impl Capabilities {
    pub fn decode(data: &Value) -> Result<Self, ErrorKind> {
        let major = integer(data.get("version").and_then(|v| v.get("major")));
        let family = match major {
            Some(34) => ServerFamily::Nextcloud34,
            Some(35) => ServerFamily::Nextcloud35,
            _ => ServerFamily::Unknown,
        };
        let spreed = &data["capabilities"]["spreed"];
        let mut features = Vec::new();
        if let Some(list) = spreed["features"].as_array() {
            if list.len() > 256 {
                return Err(ErrorKind::TooLarge);
            }
            for item in list {
                features.push(text::<128>(Some(item))?.as_str().to_owned());
            }
        } else if !spreed.is_null() {
            return Err(ErrorKind::Malformed);
        }
        Ok(Self {
            features,
            read_receipts: super::preferences::public_privacy(
                &spreed["config"]["chat"]["read-privacy"],
            ),
            typing_status: super::preferences::public_privacy(
                &spreed["config"]["chat"]["typing-privacy"],
            ),
            attachment_folder: spreed["config"]["attachments"]["folder"]
                .as_str()
                .and_then(|path| talk_core::preferences::account::NextcloudFolder::new(path).ok()),
            attachments_allowed: spreed["config"]["attachments"]["allowed"].as_bool(),
            conversation_subfolders: spreed["config"]["attachments"]["conversation-subfolders"]
                .as_bool(),
            family,
            can_create: boolean(spreed["config"]["conversations"].get("can-create")),
            message_limit: integer(spreed["config"]["chat"].get("max-length"))
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| *n > 0),
            server_default_permissions: integer(spreed["config"]["permissions"].get("default"))
                .filter(|mask| mask & !511 == 0)
                .and_then(|mask| u16::try_from(mask).ok()),
            call_enabled: boolean(spreed["config"]["call"].get("enabled")),
            recording_consent: integer(spreed["config"]["call"].get("recording-consent")),
            recording_enabled: spreed["config"]["call"]["recording"].as_bool(),
            breakouts_enabled: spreed["config"]["call"]["breakout-rooms"].as_bool(),
            encryption_required: spreed["config"]["call"]
                .get("end-to-end-encryption")
                .and_then(Value::as_bool),
        })
    }
    pub fn has(&self, name: &str) -> bool {
        self.features.iter().any(|s| s == name)
    }
    pub fn supported(&self) -> bool {
        self.family != ServerFamily::Unknown && self.has("conversation-v4")
    }
    pub fn feature(&self, name: &str) -> FeatureSupport {
        if !self.supported() {
            FeatureSupport::Unknown
        } else if self.has(name) {
            FeatureSupport::Supported
        } else {
            FeatureSupport::Unsupported
        }
    }
    pub fn probe(&self, scope: AccountScope, display_name: Text<256>) -> AccountProbe {
        AccountProbe {
            scope,
            display_name,
            family: self.family,
            talk_available: self.has("conversation-v4"),
            group_creation: if self.supported() {
                self.can_create
            } else {
                Decision::Unknown
            },
            message_limit: self.message_limit,
            reference_correlation: self.has("chat-reference-id"),
            channel_semantics: self.has("announcement-preset"),
        }
    }
}

pub(super) fn room(
    data: &Value,
    id: ConversationId,
    caps: &Capabilities,
) -> Result<Conversation, ErrorKind> {
    let kind = match integer(data.get("type")) {
        Some(1) => RoomKind::Direct,
        Some(2) => RoomKind::Group,
        Some(3) => RoomKind::Public,
        Some(4) => RoomKind::Changelog,
        Some(5) => RoomKind::FormerDirect,
        Some(6) => RoomKind::NoteToSelf,
        _ => RoomKind::Unknown,
    };
    let role = role(data.get("participantType"));
    let attributes = integer(data.get("attributes"));
    let permissions = integer(data.get("permissions"));
    let read_only = integer(data.get("readOnly"));
    let lobby = integer(data.get("lobbyState"));
    let attrs = attributes.unwrap_or(0);
    let classified = attrs & 4 != 0;
    let channel = attrs & 8 != 0;
    let moderator = matches!(role, Role::Owner | Role::Moderator | Role::GuestModerator);
    let remote = data
        .get("remoteServer")
        .is_some_and(|v| v.as_str() != Some(""))
        || data
            .get("remoteToken")
            .is_some_and(|v| v.as_str() != Some(""));
    let object = data.get("objectType").and_then(Value::as_str).unwrap_or("");
    let unknown = !caps.supported()
        || kind == RoomKind::Unknown
        || role == Role::Unknown
        || attributes.is_none()
        || attrs & !31 != 0
        || attrs != 0 && !caps.has("conversation-presets")
        || attrs & 2 != 0 && !caps.has("preserve-conversation")
        || classified && !caps.has("classified-conversations")
        || attrs & 24 != 0 && !caps.has("announcement-preset")
        || attrs & 16 != 0 && !channel
        || permissions.is_none_or(|v| v & !511 != 0)
        || !caps.has("conversation-permissions")
        || !caps.has("read-only-rooms")
        || !matches!(read_only, Some(0 | 1))
        || !matches!(lobby, Some(0 | 1))
        || remote
        || !matches!(
            object,
            "" | "file"
                | "event"
                | "room"
                | "phone"
                | "sample"
                | "extended_conversation"
                | "classified"
                | "classified_persist"
                | "external_call"
                | "share:password"
        );
    let mut actions = RoomActions {
        read_messages: Decision::Allowed,
        send_message: Decision::Allowed,
        list_participants: Decision::Allowed,
        leave_conversation: boolean(data.get("canLeaveConversation")),
        join_call: Decision::Denied,
    };
    if unknown {
        actions = RoomActions {
            read_messages: Decision::Unknown,
            send_message: Decision::Unknown,
            list_participants: Decision::Unknown,
            leave_conversation: Decision::Unknown,
            join_call: Decision::Unknown,
        };
    }
    // Classified handling needs the later privacy/cache owner; do not open content now.
    if classified {
        actions.read_messages = Decision::Denied;
        actions.send_message = Decision::Denied;
        actions.list_participants = Decision::Denied;
    }
    if !caps.has("chat-v2") {
        actions.read_messages = Decision::Unknown;
        actions.send_message = Decision::Unknown;
    }
    if read_only == Some(1)
        || permissions.is_some_and(|p| p & 128 == 0)
        || matches!(kind, RoomKind::Changelog | RoomKind::FormerDirect)
    {
        actions.send_message = Decision::Denied;
    }
    if lobby == Some(1) && permissions.is_none_or(|p| p & 8 == 0) {
        actions.read_messages = Decision::Denied;
        actions.send_message = Decision::Denied;
        actions.list_participants = Decision::Denied;
    }
    if channel && !moderator {
        actions.list_participants = Decision::Denied;
    }
    if attrs & 16 != 0 && !moderator {
        actions.leave_conversation = Decision::Denied;
    }
    if channel || classified || read_only == Some(1) {
        actions.join_call = Decision::Denied;
    }
    Ok(Conversation {
        id,
        call_scope: None,
        navigation_key: None,
        tag_ids: None,
        favorite: if caps.feature("favorites") == FeatureSupport::Supported {
            data.get("isFavorite").and_then(Value::as_bool)
        } else {
            None
        },
        organization: ConversationOrganization {
            favorites: caps.feature("favorites"),
            favorite_change: if caps.feature("favorites") != FeatureSupport::Supported
                || unknown
                || data.get("isFavorite").and_then(Value::as_bool).is_none()
            {
                Decision::Unknown
            } else if matches!(
                role,
                Role::Owner | Role::Moderator | Role::Member | Role::Following
            ) {
                Decision::Allowed
            } else {
                Decision::Denied
            },
            tags: caps.feature("conversation-tags"),
            tag_change: if caps.feature("conversation-tags") != FeatureSupport::Supported || unknown
            {
                Decision::Unknown
            } else if matches!(
                role,
                Role::Owner | Role::Moderator | Role::Member | Role::Following
            ) {
                Decision::Allowed
            } else {
                Decision::Denied
            },
            classified: (caps.supported()
                && caps.has("classified-conversations")
                && attributes.is_some()
                && attrs & !31 == 0)
                .then_some(classified),
            channel: (caps.supported()
                && caps.has("announcement-preset")
                && attributes.is_some()
                && attrs & !31 == 0)
                .then_some(channel),
            voice_room: (caps.supported()
                && caps.has("conversation-presets")
                && attributes.is_some()
                && attrs & !31 == 0)
                .then_some(attrs & 1 != 0),
        },
        name: text(data.get("displayName").or_else(|| data.get("name")))?,
        kind,
        role,
        archived: caps
            .has("archived-conversations-v2")
            .then(|| data.get("isArchived").and_then(Value::as_bool))
            .flatten(),
        archive: if !caps.has("archived-conversations-v2")
            || unknown
            || data.get("isArchived").and_then(Value::as_bool).is_none()
        {
            Decision::Unknown
        } else if matches!(
            role,
            Role::Owner | Role::Moderator | Role::Member | Role::Following
        ) {
            Decision::Allowed
        } else {
            Decision::Denied
        },
        actions,
        unread_messages: integer(data.get("unreadMessages")).and_then(|n| u32::try_from(n).ok()),
        last_activity: integer(data.get("lastActivity")),
        call_active: data.get("hasCall").and_then(Value::as_bool),
        sensitive: classified
            || data
                .get("isSensitive")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        classified,
        channel,
        unknown_restrictions: unknown,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inherited_permissions_are_bounded_capability_configuration() {
        for (value, expected) in [
            (serde_json::json!(502), Some(502)),
            (serde_json::json!(132), Some(132)),
            (serde_json::json!(0), Some(0)),
            (serde_json::json!(512), None),
            (serde_json::json!(-1), None),
            (serde_json::Value::Null, None),
        ] {
            let data = serde_json::json!({"capabilities":{"spreed":{"features":[],"config":{"permissions":{"default":value}}}}});
            assert_eq!(
                Capabilities::decode(&data)
                    .unwrap()
                    .server_default_permissions,
                expected
            );
        }
        assert_eq!(
            Capabilities::decode(&serde_json::json!({}))
                .unwrap()
                .server_default_permissions,
            None
        );
    }
    #[test]
    fn hostile_json_has_finite_structure_and_no_duplicate_policy_keys() {
        assert!(json(br#"{"permission":true,"permission":false}"#).is_err());
        assert!(json(format!("{}0{}", "[".repeat(66), "]".repeat(66)).as_bytes()).is_err());
        assert!(json(format!("[{}0]", "0,".repeat(4097)).as_bytes()).is_err());
        assert!(json(b"{} trailing").is_err());
        assert!(json(br#"{"newField":{"unknown":true},"text":"Unicode \u00fc"}"#).is_ok());
    }
}
