//! Compatibility projection into the existing conservative application policy decoder.
//! Network ingress is decoded once by the API; this bounded value is local, never sent.
use nextcloud_talk_api::models::*;
use serde_json::{Value, json};
pub(super) trait Project {
    fn project(self) -> Value;
}
impl Project for Account {
    fn project(self) -> Value {
        json!({"id":self.id,"displayname":self.display_name})
    }
}
impl Project for Room {
    fn project(self) -> Value {
        json!({"token":self.token,"id":self.server_id,"displayName":self.name,"type":self.kind,"participantType":self.participant_type,"attributes":self.attributes,"permissions":self.permissions,"readOnly":self.read_only,"lobbyState":self.lobby_state,"objectType":self.object_type,"canLeaveConversation":self.can_leave,"isFavorite":self.favorite,"isArchived":self.archived,"unreadMessages":self.unread,"lastActivity":self.last_activity,"hasCall":self.has_call,"isSensitive":self.sensitive,"remoteServer":if self.remote{"remote"}else{""},"tagIds":self.tag_ids,"description":self.description,"defaultPermissions":self.default_permissions,"callPermissions":self.call_permissions,"lobbyTimer":self.lobby_timer,"recordingConsent":self.recording_consent,"sipEnabled":self.sip_enabled,"objectId":self.object_id,"notificationLevel":self.notification_level,"notificationCalls":self.notification_calls,"isImportant":self.important,"messageExpiration":self.message_expiration,"listable":self.listable,"mentionPermissions":self.mention_permissions,"hasPassword":self.has_password,"canDeleteConversation":self.can_delete,"canStartCall":self.can_start_call,"callFlag":self.call_flag,"callRecording":self.recording_status,"lastPinnedId":self.last_pinned,"hiddenPinnedId":self.hidden_pinned})
    }
}
impl Project for Participant {
    fn project(self) -> Value {
        json!({"displayName":self.display_name,"participantType":self.participant_type,"actorType":self.actor_type,"actorId":self.actor_id,"inCall":self.in_call,"sessionIds":self.sessions,"attendeeId":self.attendee_id,"permissions":self.permissions,"roomToken":self.room_token})
    }
}
impl Project for Message {
    fn project(self) -> Value {
        json!({"token":self.token,"id":self.id,"messageType":self.kind,"referenceId":self.reference,"actorType":self.actor_type,"actorId":self.actor_id,"actorDisplayName":self.author,"message":self.message,"timestamp":self.timestamp,"expirationTimestamp":self.expires,"lastEditTimestamp":self.edited})
    }
}
impl Project for CallParticipant {
    fn project(self) -> Value {
        json!({"sessionId":self.session,"actorType":self.actor_type,"actorId":self.actor_id})
    }
}
impl Project for Tag {
    fn project(self) -> Value {
        json!({"id":self.id,"name":self.name,"type":self.kind,"sortOrder":self.order,"collapsed":self.collapsed})
    }
}
impl Project for Capabilities {
    fn project(self) -> Value {
        json!({"version":{"major":self.server_major},"capabilities":{"spreed":{"features":self.features,"config":{"chat":{"read-privacy":self.read_privacy,"typing-privacy":self.typing_privacy,"max-length":self.message_limit},"attachments":{"folder":self.attachment_folder,"allowed":self.attachments_allowed,"conversation-subfolders":self.conversation_subfolders},"conversations":{"can-create":self.can_create},"permissions":{"default":self.server_default_permissions},"call":{"enabled":self.call_enabled,"recording":self.recording_enabled,"breakout-rooms":self.breakouts_enabled,"grid-limit":self.grid_limit,"grid-limit-enforced":self.grid_limit_enforced,"recording-consent":self.recording_consent,"end-to-end-encryption":self.encryption_required}}}}})
    }
}
impl Project for SignalingSettings {
    fn project(self) -> Value {
        json!({"signalingMode":match self.mode {SignalingMode::Internal=>"internal",SignalingMode::External=>"external",SignalingMode::Unknown=>"unknown"}})
    }
}
