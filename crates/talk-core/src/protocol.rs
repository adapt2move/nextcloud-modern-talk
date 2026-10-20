//! Bounded worker-side protocol values. No wire routes, credentials or media authority.
//! Application owners project these values into `ui`; renderers never import this module.

use crate::bounded::{Page, Text};
use crate::ids::{AccountScope, CallScope, ConversationId, OperationId};
use crate::policy::Decision;
use crate::preferences::navigation::NavigationRoomKey;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeatureSupport {
    Unknown,
    Unsupported,
    Supported,
}

/// Only exact, capability-qualified metadata is projected. Call/chat restrictions
/// remain independent; a classified label never permits classified content access.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConversationOrganization {
    pub favorites: FeatureSupport,
    pub favorite_change: Decision,
    pub tags: FeatureSupport,
    pub tag_change: Decision,
    pub classified: Option<bool>,
    pub channel: Option<bool>,
    pub voice_room: Option<bool>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServerFamily {
    Nextcloud34,
    Nextcloud35,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountProbe {
    pub scope: AccountScope,
    pub display_name: Text<256>,
    pub family: ServerFamily,
    pub talk_available: bool,
    pub group_creation: Decision,
    pub message_limit: Option<u32>,
    pub reference_correlation: bool,
    pub channel_semantics: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoomKind {
    Direct,
    Group,
    Public,
    Changelog,
    FormerDirect,
    NoteToSelf,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    Owner,
    Moderator,
    Member,
    Guest,
    Following,
    GuestModerator,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RoomActions {
    pub read_messages: Decision,
    pub send_message: Decision,
    pub list_participants: Decision,
    pub leave_conversation: Decision,
    /// Always unavailable until the separate native policy/media owners qualify a path.
    pub join_call: Decision,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Conversation {
    pub id: ConversationId,
    pub call_scope: Option<CallScope>,
    /// Set only by the identity ledger owner from an authoritative server room ID.
    pub navigation_key: Option<NavigationRoomKey>,
    pub favorite: Option<bool>,
    pub organization: ConversationOrganization,
    /// Runtime-scoped owner handles. None means metadata is unavailable, not empty.
    pub tag_ids: Option<Page<ConversationTagId, 20>>,
    pub name: Text<256>,
    pub kind: RoomKind,
    pub role: Role,
    pub archived: Option<bool>,
    pub archive: Decision,
    pub actions: RoomActions,
    pub unread_messages: Option<u32>,
    pub last_activity: Option<u64>,
    pub call_active: Option<bool>,
    pub sensitive: bool,
    pub classified: bool,
    pub channel: bool,
    pub unknown_restrictions: bool,
}

/// Opaque local correlation only, never a server tag ID or permission grant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConversationTagId {
    pub scope: AccountScope,
    pub local: u128,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TagKind {
    Custom,
    Favorites,
    Other,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversationTag {
    pub id: ConversationTagId,
    /// Up to 250 Unicode scalar values, bounded to 1000 UTF-8 bytes.
    pub name: Text<1000>,
    pub order: u32,
    pub collapsed: bool,
    pub kind: TagKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversationTags {
    pub scope: AccountScope,
    pub tags: Page<ConversationTag, 102>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversationList {
    pub scope: AccountScope,
    /// Full reconciliation only. An oversized list fails; it is never silently truncated.
    pub rooms: Page<Conversation, 100>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageNotice {
    pub conversation: ConversationId,
    pub message: u64,
    pub title: Text<128>,
    pub body: Text<512>,
    pub redacted: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageNotifications {
    pub scope: AccountScope,
    pub notices: Page<MessageNotice, 4>,
    /// New source events also request a full navigation reconciliation, even if
    /// local/server notification policy suppresses every banner.
    pub changed: bool,
}

/// Coalescible hints from an admitted account transport (including HPB).
/// A hint requests a fresh read; it never grants message, room or call authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountChange {
    Messages,
    Conversations,
    IncomingCalls,
}

/// Informational transport state, never account, membership or media authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountLiveState {
    Internal,
    Connecting,
    Connected,
    Reconnecting,
    Stopped(crate::ports::ErrorKind),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountLiveEvents {
    pub scope: AccountScope,
    pub state: AccountLiveState,
    /// Safe first transport failure, retained independently of HTTP observations.
    pub first_transport_failure: Option<crate::ports::ErrorKind>,
    /// At most one hint per existing account observation slot; no wire room tokens.
    pub changes: Page<AccountChange, 3>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Participant {
    pub display_name: Text<256>,
    pub role: Role,
    pub is_self: bool,
    pub in_call: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Participants {
    pub conversation: ConversationId,
    pub people: Page<Participant, 100>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MessageId {
    pub conversation: ConversationId,
    pub sequence: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageKind {
    Comment,
    Deleted,
    System,
    Unsupported,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Message {
    pub id: MessageId,
    pub author: Text<256>,
    /// Plain, uninterpreted text; rich parameters/HTML are not executed or expanded.
    pub text: Text<131072>,
    pub kind: MessageKind,
    pub timestamp: u64,
    pub expires_at: Option<u64>,
    pub edited_at: Option<u64>,
    pub own_operation: Option<OperationId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryPage {
    pub conversation: ConversationId,
    pub messages: Page<Message, 50>,
    pub older: Option<MessageId>,
    pub common_read: Option<MessageId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SendMessage {
    pub conversation: ConversationId,
    /// Unique within the account runtime. Reusing an unresolved ID never sends again.
    pub operation: OperationId,
    pub text: Text<131072>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SendState {
    /// Submission may have reached the server. No automatic replay is safe.
    Uncertain,
    /// Server acknowledgement or a matching history record; not recipient delivery/read.
    ServerAccepted(MessageId),
    Rejected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SendReceipt {
    pub operation: OperationId,
    pub state: SendState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignalingBackend {
    Internal,
    External,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CallObservation {
    pub call: CallScope,
    pub connected_participants: u16,
    pub signaling: SignalingBackend,
    /// A settings read grants no join, capture, encryption or media readiness.
    pub native_media: Decision,
}

pub mod content;
pub mod conversation;
pub mod messaging;
