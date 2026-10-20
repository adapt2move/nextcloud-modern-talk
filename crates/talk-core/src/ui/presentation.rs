//! Native presentation projection. Producers own policy, identity and operation lifetimes.
//! All collections/text are bounded. No transport DTO, endpoint, secret or frame is exported.
use super::{
    AccountId, AccountScope, CallScope, ConversationId, Decision, MediaIntent, Page, SubmitError,
    Text, UnavailableReason, UserIntent,
};
use crate::preferences::navigation::{NavigationAccountKey, NavigationRoomKey};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConversationRef {
    pub account: AccountScope,
    pub conversation: ConversationId,
}

impl ConversationRef {
    pub fn is_consistent(self) -> bool {
        self.account.account == self.conversation.account
    }
}

/// Window-local correlation, never authentication or operation authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestId(pub u64);

/// Untrusted address typed by the user, *not* a validated origin. The identity owner
/// must normalize HTTPS, validate host/base path/TLS and bind the canonical origin.
/// This wrapper rejects credential/query/fragment entry before it crosses the seam.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerInput(Text<2048>);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddressInputError {
    Empty,
    TooLong,
    CredentialsOrParameters,
    InsecureScheme,
}

impl ServerInput {
    pub fn new(value: &str) -> Result<Self, AddressInputError> {
        if value.len() > 2048 {
            return Err(AddressInputError::TooLong);
        }
        let value = value.trim();
        if value.is_empty() {
            return Err(AddressInputError::Empty);
        }
        if value
            .chars()
            .any(|c| c.is_control() || matches!(c, '@' | '?' | '#'))
        {
            return Err(AddressInputError::CredentialsOrParameters);
        }
        if value.contains("://") && !value.starts_with("https://") {
            return Err(AddressInputError::InsecureScheme);
        }
        Ok(Self(
            Text::new(value).map_err(|_| AddressInputError::TooLong)?,
        ))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageChoice {
    NativeStore,
    PrivateFile,
    SessionOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageStatus {
    NotSaved,
    Saving,
    NativeStored,
    PrivateFileStored,
    SessionOnly,
    CleanupPending,
    RecoveryRequired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeStoreState {
    Unknown,
    Available,
    Missing,
    Locked,
    Denied,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoginFailure {
    Expired,
    BrowserUnavailable,
    Network,
    Tls,
    Denied,
    Storage,
    Unconfigured,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoginPhase {
    EnterServer,
    Validating,
    BrowserWaiting,
    ChooseStorage,
    Ready,
    Failed(LoginFailure),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoginView {
    pub phase: LoginPhase,
    pub attempt: Option<RequestId>,
    pub server_label: Text<128>,
    pub account_label: Text<128>,
    pub native_store: NativeStoreState,
    pub storage: StorageStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentState {
    Loading,
    Ready,
    Offline,
    Denied,
    Failed,
    Unconfigured,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Presence {
    Available,
    Away,
    DoNotDisturb,
    Offline,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountView {
    pub scope: AccountScope,
    pub navigation_key: Option<NavigationAccountKey>,
    pub connection: AccountConnectionState,
    pub label: Text<128>,
    pub server_label: Text<128>,
    pub self_name: Text<128>,
    pub initials: Text<16>,
    pub presence: Presence,
    pub storage: StorageStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountConnectionState {
    Connected,
    Guest,
    Disconnected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SavedAccountPhase {
    Dormant,
    Restoring,
    WaitingForBrowser,
    AwaitingStorageChoice,
    Active,
    ReauthenticationRequired,
    RecoveryRequired,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SavedAccountView {
    pub id: AccountId,
    pub label: Option<Text<128>>,
    pub server_label: Option<Text<128>>,
    pub phase: SavedAccountPhase,
    pub storage: StorageStatus,
    pub cleanup_pending: bool,
    pub suppression_unconfirmed: bool,
    pub failure: Option<LoginFailure>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConversationKind {
    Direct,
    Group,
    GuestAccessible,
    NoteToSelf,
    Unknown,
}

/// Capability absence is distinct from an unobserved/failed capability response.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub enum FeatureSupport {
    #[default]
    Unknown,
    Unsupported,
    Supported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConferencePreset {
    Standard,
    Webinar,
    Presentation,
}

/// Authoritative attributes of an existing conversation, never title heuristics.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub enum ConversationCategory {
    #[default]
    Unknown,
    Channel,
    Conference(ConferencePreset),
    Huddle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConversationOrganization {
    pub category: ConversationCategory,
    pub classified: Option<bool>,
    pub favorites: FeatureSupport,
    pub favorite_change: Decision,
    pub tags: FeatureSupport,
    pub tag_change: Decision,
}
impl Default for ConversationOrganization {
    fn default() -> Self {
        Self {
            category: ConversationCategory::Unknown,
            classified: None,
            favorites: FeatureSupport::Unknown,
            favorite_change: Decision::Unknown,
            tags: FeatureSupport::Unknown,
            tag_change: Decision::Unknown,
        }
    }
}

/// Opaque, runtime-scoped presentation handle. The owner revalidates every mutation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TagRef {
    pub account: AccountScope,
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
pub struct TagView {
    pub id: TagRef,
    pub name: Text<1000>,
    pub order: u32,
    pub collapsed: bool,
    pub kind: TagKind,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TagEdit {
    Create(Text<1000>),
    Rename {
        tag: TagRef,
        name: Text<1000>,
    },
    Delete(TagRef),
    Collapse {
        tag: TagRef,
        collapsed: bool,
    },
    Reorder(Page<TagRef, 102>),
    Assign {
        context: ConversationRef,
        tags: Page<TagRef, 20>,
    },
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OrganizationResult {
    pub request: RequestId,
    pub account: AccountScope,
    pub result: Completion,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversationView {
    pub context: ConversationRef,
    pub navigation_key: Option<NavigationRoomKey>,
    pub title: Text<256>,
    pub initials: Text<16>,
    pub subtitle: Text<256>,
    pub updated_label: Text<32>,
    pub kind: ConversationKind,
    pub unread: u32,
    pub favorite: Option<bool>,
    pub organization: ConversationOrganization,
    pub tags: Option<Page<TagRef, 20>>,
    /// Authoritative server state. Unknown is not an active/non-archived room.
    pub archived: Option<bool>,
    pub archive: Decision,
    pub presence: Presence,
    pub participants: Option<u16>,
    pub send: Decision,
    pub attach: Decision,
    /// Explicit native privacy-policy projection, not a generic server permission.
    /// Classified/unknown restrictions MUST project Denied/Unknown; never infer from send.
    pub draft_clipboard: Decision,
    pub call: Decision,
    /// Allocated/revalidated by the call owner; a row click never creates a call.
    pub call_scope: Option<CallScope>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryState {
    Received,
    Pending,
    ServerAccepted,
    Failed,
    Uncertain,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileView {
    pub local_id: u128,
    pub name: Text<256>,
    pub detail: Text<128>,
    pub download: Decision,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageView {
    pub local_id: u128,
    pub author: Text<128>,
    pub initials: Text<16>,
    pub time_label: Text<32>,
    pub body: Text<4096>,
    pub own: bool,
    pub delivery: DeliveryState,
    pub attachments: Page<FileView, 8>,
    pub reply_count: u16,
    pub metadata: super::messaging::MessageMetadata,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParticipantView {
    pub local_id: u128,
    pub name: Text<128>,
    pub initials: Text<16>,
    pub role_label: Text<64>,
    pub presence: Presence,
    pub self_user: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequiredGate {
    MicrophonePermission,
    CameraPermission,
    ScreenPermission,
    RecordingConsent,
    SecurityPolicy,
    Lobby,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CallStage {
    Incoming,
    Outgoing,
    Connecting,
    Active,
    Reconnecting,
    Required(RequiredGate),
    Ended,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaState {
    /// No native media implementation is attached. Never render as an active track.
    Unavailable,
    Off,
    Requested,
    Active,
    Denied,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CallView {
    pub scope: CallScope,
    pub title: Text<256>,
    pub direct: bool,
    pub stage: CallStage,
    pub accept: Decision,
    pub microphone: MediaState,
    pub camera: MediaState,
    pub share: MediaState,
    pub participants: Page<ParticipantView, 16>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Completion {
    /// Server accepted the request. This does not mean recipient delivery/read.
    ServerAccepted,
    Failed(UnavailableReason),
    Uncertain,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SubmissionResult {
    pub request: RequestId,
    pub context: ConversationRef,
    pub result: Completion,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConversationMutationResult {
    pub request: RequestId,
    pub context: ConversationRef,
    pub result: Completion,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresentationSnapshot {
    pub create_panel: Option<super::conversation::CreatePanel>,
    pub revision: u64,
    pub login: LoginView,
    pub accounts: Page<AccountView, 8>,
    /// Durable records have no live scope until the account owner admits one.
    pub saved_accounts: Page<SavedAccountView, 8>,
    pub active_account: Option<AccountScope>,
    pub conversations: Page<ConversationView, 100>,
    pub conversation_list: ConversationListState,
    pub tags: Page<TagView, 102>,
    pub tag_state: ContentState,
    pub organization_result: Option<OrganizationResult>,
    /// History/files/participants below belong to exactly this runtime+host context.
    pub selected: Option<ConversationRef>,
    pub content: ContentState,
    pub messages: Page<MessageView, 100>,
    pub files: Page<FileView, 100>,
    pub participants: Page<ParticipantView, 100>,
    pub conversation_panel: Option<super::conversation::ConversationPanel>,
    pub message_panel: Option<super::messaging::MessagePanel>,
    pub call: Option<CallView>,
    /// Keep this correlated result until acknowledged; do not lose it by coalescing.
    pub submission: Option<SubmissionResult>,
    pub mutation: Option<ConversationMutationResult>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConversationListState {
    pub content: ContentState,
    /// False for partial/unknown lists; absence is not an authoritative empty list.
    pub complete: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UiIntent {
    Create {
        account: AccountScope,
        request: RequestId,
        action: super::conversation::CreateAction,
    },
    AcknowledgeCreation {
        account: AccountScope,
        request: RequestId,
    },
    Conversation {
        request: RequestId,
        context: ConversationRef,
        action: super::conversation::ConversationAction,
    },
    Message {
        request: RequestId,
        context: ConversationRef,
        action: super::messaging::MessageAction,
    },
    AcknowledgeRoom {
        request: RequestId,
        context: ConversationRef,
    },
    AcknowledgeMessage {
        request: RequestId,
        context: ConversationRef,
    },
    EditTags {
        request: RequestId,
        account: AccountScope,
        edit: TagEdit,
    },
    AcknowledgeOrganization {
        request: RequestId,
        account: AccountScope,
    },
    ReloadConversations(AccountScope),
    NewLogin,
    RestoreAccount(AccountId),
    Reauthenticate(AccountId),
    BeginLogin {
        request: RequestId,
        server: ServerInput,
    },
    CancelLogin(RequestId),
    ReopenBrowser(RequestId),
    ChooseStorage {
        attempt: RequestId,
        choice: StorageChoice,
    },
    SelectAccount(AccountScope),
    OpenConversation(ConversationRef),
    SetArchived {
        request: RequestId,
        context: ConversationRef,
        archived: bool,
    },
    SetFavorite {
        request: RequestId,
        context: ConversationRef,
        favorite: bool,
    },
    Refresh(ConversationRef),
    SendDraft {
        request: RequestId,
        context: ConversationRef,
        body: Text<4096>,
    },
    AcknowledgeSubmission {
        request: RequestId,
        context: ConversationRef,
    },
    AcknowledgeMutation {
        request: RequestId,
        context: ConversationRef,
    },
    PickAttachment(ConversationRef),
    DownloadFile {
        context: ConversationRef,
        local_id: u128,
    },
    Search {
        account: AccountScope,
        query: Text<256>,
    },
    StartDirect {
        call: CallScope,
        media: MediaIntent,
    },
    /// Independent media wishes; mandatory admission belongs to the application.
    JoinConference {
        call: CallScope,
        microphone: bool,
        camera: bool,
    },
    DeclineDirect(CallScope),
    SetMicrophone {
        call: CallScope,
        enabled: bool,
    },
    SetCamera {
        call: CallScope,
        enabled: bool,
    },
    ShareScreen(CallScope),
    StopScreen(CallScope),
    ReconnectCall(CallScope),
    ResolveRequiredGate {
        call: CallScope,
        gate: RequiredGate,
    },
    /// Preserve the existing typed call/navigation contract; no string dispatcher.
    Legacy(UserIntent),
}

/// Additive native seam. Nonblocking admission is distinct from asynchronous success.
/// The owner revalidates every scope/policy. Renderer clicks confer no authority.
/// One coalesced snapshot and at most 64 queued intents; priority leave/logout must
/// remain independently admissible as required by ADR-002.
pub trait NativeDesktopPort {
    fn try_submit(&mut self, intent: UiIntent) -> Result<(), SubmitError>;
    fn take_snapshot(&mut self) -> Option<PresentationSnapshot>;
}

#[cfg(test)]
mod tests {
    use super::{AddressInputError, ServerInput};

    #[test]
    fn address_input_preserves_installation_path_but_carries_no_authority() {
        let address = ServerInput::new("  cloud.example.test/installation/  ").unwrap();
        assert_eq!(address.as_str(), "cloud.example.test/installation/");
        // Full host/TLS/origin validation belongs to identity, including this malformed host.
        assert!(ServerInput::new("not a host").is_ok());
    }
    #[test]
    fn address_input_rejects_credentials_parameters_control_bytes_and_oversize() {
        for address in [
            "user@cloud.example.test",
            "cloud.example.test/?token=x",
            "cloud.example.test/#secret",
        ] {
            assert_eq!(
                ServerInput::new(address),
                Err(AddressInputError::CredentialsOrParameters)
            );
        }
        assert_eq!(
            ServerInput::new("cloud.\nexample.test"),
            Err(AddressInputError::CredentialsOrParameters)
        );
        assert_eq!(ServerInput::new("  "), Err(AddressInputError::Empty));
        assert_eq!(
            ServerInput::new("http://"),
            Err(AddressInputError::InsecureScheme)
        );
        assert_eq!(
            ServerInput::new(&"a".repeat(2049)),
            Err(AddressInputError::TooLong)
        );
    }
}
