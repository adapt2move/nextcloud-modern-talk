//! Bounded conversation presentation values. Handles are scoped, never wire identities.
use super::AccountScope;
use super::presentation::{Completion, ConversationRef, FeatureSupport, RequestId};
use super::{Decision, Page, Text, UnavailableReason};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Availability {
    pub implemented: bool,
    pub server: FeatureSupport,
    pub allowed: Decision,
}
impl Availability {
    pub const UNKNOWN: Self = Self {
        implemented: true,
        server: FeatureSupport::Unknown,
        allowed: Decision::Unknown,
    };
    pub fn enabled(self) -> bool {
        self.implemented
            && self.server == FeatureSupport::Supported
            && self.allowed == Decision::Allowed
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoteState {
    Idle,
    Loading(RequestId),
    Saving(RequestId),
    Failed(RequestId, UnavailableReason),
    Uncertain(RequestId),
}
impl RemoteState {
    pub fn pending(self) -> bool {
        matches!(
            self,
            Self::Loading(_) | Self::Saving(_) | Self::Uncertain(_)
        )
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoomSetting {
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
pub enum MemberRole {
    Owner,
    Moderator,
    Member,
    Guest,
    Following,
    GuestModerator,
    Unknown,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectorySource {
    User,
    Group,
    Team,
    Email,
    Federated,
    Phone,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PersonRef {
    pub context: ConversationRef,
    pub local: u128,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccountPersonRef {
    pub account: AccountScope,
    pub local: u128,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountPerson {
    pub id: AccountPersonRef,
    pub name: Text<256>,
    pub label: Text<512>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CreateAction {
    Refresh,
    FindPeople(Text<256>),
    Group(Text<256>),
    Direct(AccountPersonRef),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CreationResult {
    pub request: RequestId,
    pub completion: Completion,
    pub conversation: Option<ConversationRef>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreatePanel {
    pub account: AccountScope,
    pub group: Decision,
    pub state: RemoteState,
    pub uncertain: bool,
    pub people: Page<AccountPerson, 100>,
    pub people_loaded: bool,
    pub result: Option<CreationResult>,
}
impl CreatePanel {
    pub fn new(account: AccountScope) -> Self {
        Self {
            account,
            group: Decision::Unknown,
            state: RemoteState::Idle,
            uncertain: false,
            people: Page::empty(),
            people_loaded: false,
            result: None,
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Member {
    pub id: PersonRef,
    pub name: Text<256>,
    pub role: MemberRole,
    pub source: DirectorySource,
    pub permissions: Option<u16>,
    pub self_user: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Recipient {
    pub id: PersonRef,
    pub name: Text<256>,
    pub label: Text<512>,
    pub source: DirectorySource,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Bot {
    pub id: PersonRef,
    pub name: Text<256>,
    pub description: Text<4000>,
    pub enabled: Option<bool>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Ban {
    pub id: PersonRef,
    pub name: Text<256>,
    pub note: Text<4000>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoomDetails {
    pub name: Text<256>,
    pub description: Option<Text<4000>>,
    pub read_only: Option<bool>,
    pub notifications: Option<NotificationLevel>,
    pub call_notifications: Option<bool>,
    pub important: Option<bool>,
    pub sensitive: bool,
    pub classified: bool,
    pub preserved: Option<bool>,
    pub message_expiration: Option<u32>,
    pub default_permissions: Option<u16>,
    pub server_default_permissions: Option<u16>,
    pub effective_permissions: Option<u16>,
    pub lobby: Option<bool>,
    pub has_password: Option<bool>,
    pub participant_count: Option<u32>,
    pub guest_access: Option<bool>,
    pub leave: Decision,
    pub controls: Page<(RoomSetting, Availability), 32>,
}
impl RoomDetails {
    pub fn availability(&self, setting: RoomSetting) -> Availability {
        self.controls
            .as_slice()
            .iter()
            .find(|(key, _)| *key == setting)
            .map_or(Availability::UNKNOWN, |(_, value)| *value)
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RoomResult {
    pub request: RequestId,
    pub completion: Completion,
}
/// A local file operation is separate from CSV preview and invitation submission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CsvFileError {
    TooLarge,
    InvalidUtf8,
    NotRegularFile,
    ReadFailed,
    PickerUnavailable,
    TimedOut,
    Unavailable(UnavailableReason),
}
#[derive(Clone, Eq, PartialEq)]
pub enum CsvFileOutcome {
    Loaded(Text<65536>),
    Cancelled,
    Failed(CsvFileError),
}
impl std::fmt::Debug for CsvFileOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Loaded(_) => f.write_str("Loaded(<redacted>)"),
            Self::Cancelled => f.write_str("Cancelled"),
            Self::Failed(error) => f.debug_tuple("Failed").field(error).finish(),
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CsvFileResult {
    pub request: RequestId,
    /// Presentation must also match its current page session and edit generation.
    pub edit_generation: u64,
    pub outcome: CsvFileOutcome,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversationPanel {
    pub guest_link: Option<Text<2048>>,
    pub context: ConversationRef,
    pub details: Option<RoomDetails>,
    /// Restored observations are display-only until a fresh detail response.
    pub details_stale: bool,
    pub state: RemoteState,
    pub members: Page<Member, 100>,
    pub members_loaded: bool,
    pub directory: Page<Recipient, 100>,
    pub directory_loaded: bool,
    pub bots: Page<Bot, 100>,
    pub bots_loaded: bool,
    pub bots_state: RemoteState,
    pub bans: Page<Ban, 100>,
    pub bans_loaded: bool,
    pub csv_duplicates: u16,
    pub csv_invalid: u16,
    pub csv_file: Option<CsvFileResult>,
    pub result: Option<RoomResult>,
}
impl ConversationPanel {
    pub fn new(context: ConversationRef) -> Self {
        Self {
            context,
            guest_link: None,
            details: None,
            details_stale: false,
            state: RemoteState::Idle,
            members: Page::empty(),
            members_loaded: false,
            directory: Page::empty(),
            directory_loaded: false,
            bots: Page::empty(),
            bots_loaded: false,
            bots_state: RemoteState::Idle,
            bans: Page::empty(),
            bans_loaded: false,
            csv_duplicates: 0,
            csv_invalid: 0,
            csv_file: None,
            result: None,
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConversationAction {
    Read,
    SetName(Text<256>),
    SetDescription(Text<4000>),
    SetEmojiAvatar(Text<64>),
    RemoveAvatar,
    SetReadOnly(bool),
    SetNotifications(NotificationLevel),
    SetCallNotifications(bool),
    SetImportant(bool),
    SetSensitive(bool),
    SetExpiration(u32),
    SetGuestAccess(bool),
    GuestLink,
    SetPassword(Text<512>),
    SetDefaultPermissions(u16),
    SetLobby(bool),
    Delete,
    ClearHistory,
    Leave,
    Members,
    Directory {
        search: Text<256>,
        source: DirectorySource,
    },
    Invite(PersonRef),
    RemoveMember(PersonRef),
    SetRole {
        member: PersonRef,
        role: MemberRole,
    },
    Bots,
    SetBotEnabled {
        bot: PersonRef,
        enabled: bool,
    },
    Bans,
    Unban(PersonRef),
    PreviewCsv(Text<65536>),
    PickCsv {
        edit_generation: u64,
    },
    CancelCsvPick {
        pick_request: RequestId,
    },
    ConfigureBreakouts {
        amount: u8,
        automatic: bool,
    },
}
impl ConversationAction {
    pub fn reads(&self) -> bool {
        matches!(
            self,
            Self::Read
                | Self::GuestLink
                | Self::Members
                | Self::Directory { .. }
                | Self::Bots
                | Self::Bans
                | Self::PreviewCsv(_)
                | Self::PickCsv { .. }
                | Self::CancelCsvPick { .. }
        )
    }
}
