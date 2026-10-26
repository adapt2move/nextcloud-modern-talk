//! Room-scoped settings and membership values. Wire identifiers stay in the adapter.
use super::{Conversation, FeatureSupport, Role};
use crate::bounded::{Page, Text};
use crate::ids::{AccountScope, ConversationId};
use crate::policy::Decision;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActionAvailability {
    pub implemented: bool,
    pub server: FeatureSupport,
    pub allowed: Decision,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Setting {
    Metadata,
    Avatar,
    ReadOnly,
    Notifications,
    Important,
    Sensitive,
    Expiration,
    Preserve,
    GuestAccess,
    Password,
    Discoverability,
    Mentions,
    Permissions,
    Lobby,
    RecordingConsent,
    Delete,
    ClearHistory,
    Keep,
    Members,
    Invite,
    CsvImport,
    Roles,
    Bots,
    Bans,
    Breakouts,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotificationLevel {
    Default,
    Always,
    Mention,
    Never,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Discoverability {
    None,
    Users,
    Everyone,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecipientSource {
    User,
    Group,
    Team,
    Email,
    Federated,
    Phone,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemberId {
    pub conversation: ConversationId,
    pub local: u128,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecipientId {
    pub scope: AccountScope,
    pub local: u128,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntegrationId {
    pub conversation: ConversationId,
    pub local: u128,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Member {
    pub id: MemberId,
    /// Opaque account-local user identity from this fresh member observation.
    /// It grants no access and is never projected into serialized UI state.
    pub avatar_identity: Option<[u8; 32]>,
    pub name: Text<256>,
    pub role: Role,
    pub source: RecipientSource,
    pub permissions: Option<u16>,
    pub is_self: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Recipient {
    pub id: RecipientId,
    pub name: Text<256>,
    pub label: Text<512>,
    pub source: RecipientSource,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoomBot {
    pub id: IntegrationId,
    pub name: Text<256>,
    pub description: Text<4000>,
    pub enabled: Option<bool>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoomBan {
    pub id: IntegrationId,
    pub name: Text<256>,
    pub note: Text<4000>,
    pub timestamp: u64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversationDetails {
    pub conversation: Conversation,
    pub description: Option<Text<4000>>,
    pub read_only: Option<bool>,
    pub notifications: Option<NotificationLevel>,
    pub call_notifications: Option<bool>,
    pub important: Option<bool>,
    pub preserved: Option<bool>,
    pub message_expiration: Option<u32>,
    pub listable: Option<Discoverability>,
    pub mention_all: Option<bool>,
    pub default_permissions: Option<u16>,
    /// Inherited participant defaults from the server capability configuration.
    /// A moderator's effective permissions are not a substitute.
    pub server_default_permissions: Option<u16>,
    pub effective_permissions: Option<u16>,
    pub lobby: Option<bool>,
    pub lobby_timer: Option<u64>,
    pub recording_consent: Option<bool>,
    pub has_password: Option<bool>,
    pub participant_count: Option<u32>,
    pub controls: Page<(Setting, ActionAvailability), 32>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConversationCommand {
    Read,
    SetName(Text<256>),
    SetDescription(Text<4000>),
    SetEmojiAvatar {
        emoji: Text<64>,
        color: Option<Text<32>>,
    },
    RemoveAvatar,
    SetReadOnly(bool),
    SetNotifications(NotificationLevel),
    SetCallNotifications(bool),
    SetImportant(bool),
    SetSensitive(bool),
    SetExpiration(u32),
    SetPreserved(bool),
    SetGuestAccess {
        enabled: bool,
        password: Text<512>,
    },
    SetPassword(Text<512>),
    SetDiscoverability(Discoverability),
    SetMentionAll(bool),
    SetDefaultPermissions(u16),
    SetMemberPermissions {
        member: MemberId,
        permissions: u16,
    },
    SetLobby {
        enabled: bool,
        timer: Option<u64>,
    },
    SetRecordingConsent(bool),
    Delete,
    Leave,
    ClearHistory,
    Keep,
    GuestLink,
    Members,
    Directory {
        search: Text<256>,
        source: RecipientSource,
    },
    Invite(RecipientId),
    RemoveMember(MemberId),
    SetRole {
        member: MemberId,
        role: Role,
    },
    Bots,
    SetBotEnabled {
        bot: IntegrationId,
        enabled: bool,
    },
    Bans,
    Ban {
        member: MemberId,
        note: Text<4000>,
    },
    Unban(IntegrationId),
    ResendInvitations(Option<MemberId>),
    /// Local validation/preview only; explicit Invite operations perform each invitation.
    PreviewEmailCsv(Text<65536>),
    BreakoutRooms,
    ConfigureBreakouts {
        amount: u8,
        automatic: bool,
    },
    StartBreakouts,
    StopBreakouts,
    RemoveBreakouts,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConversationResult {
    Details(Box<ConversationDetails>),
    Members(Page<Member, 100>),
    Directory(Page<Recipient, 100>),
    Bots(Page<RoomBot, 100>),
    Bans(Page<RoomBan, 100>),
    EmailCsvPreview {
        recipients: Page<Recipient, 100>,
        duplicate_count: u16,
        invalid_count: u16,
    },
    BreakoutRooms(Page<Conversation, 20>),
    GuestLink(Text<2048>),
    Changed,
    Deleted,
    Left,
    HistoryCleared,
}
