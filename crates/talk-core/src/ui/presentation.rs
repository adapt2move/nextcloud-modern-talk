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
pub enum AccountLoadFailure {
    DeviceChanged,
    InUse,
    AccessDenied,
    Unavailable,
    UnsafeLocation,
    Incomplete,
    BindingMismatch,
    UnsupportedVersion,
    InvalidData,
    RecoveryRequired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoginFailure {
    AccountStorage(AccountLoadFailure),
    Connection(ConnectionFailure),
    Expired,
    BrowserUnavailable,
    Network,
    Tls,
    Denied,
    Storage,
    AccountExists,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageSaveFailure {
    NativeUnavailable,
    NativeLocked,
    NativeDenied,
    NativeCorrupt,
    NativeTemporarilyUnavailable,
    Cancelled,
    Conflict,
    Failed,
    LocalDenied,
    LocalCorrupt,
    LocalUnavailable,
    RecoveryRequired,
    SuppressionUnconfirmed,
    Uncertain,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoginView {
    pub phase: LoginPhase,
    pub attempt: Option<RequestId>,
    pub server_label: Text<512>,
    pub account_label: Text<128>,
    pub native_store: NativeStoreState,
    pub storage: StorageStatus,
    pub storage_failure: Option<StorageSaveFailure>,
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
    pub connection_failure: Option<ConnectionFailure>,
    pub label: Text<128>,
    pub server_label: Text<128>,
    pub self_name: Text<128>,
    pub initials: Text<16>,
    pub presence: Presence,
    pub storage: StorageStatus,
}

/// Safe account-wide connection observations, never raw server messages or URLs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionFailure {
    Unreachable,
    TimedOut,
    AccessDenied,
    ServerUnavailable,
    RateLimited,
    InvalidResponse,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountConnectionState {
    Connected,
    Connecting,
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
    /// Historical metadata; this flag never grants an authenticated scope.
    pub connected_once: bool,
    pub id: AccountId,
    pub navigation_key: NavigationAccountKey,
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
    pub unread_direct_mention: Option<bool>,
    pub last_read_message: Option<u128>,
    pub unread_mention: Option<bool>,
    pub notification_muted: Option<bool>,
    pub last_activity: Option<u64>,
    pub favorite: Option<bool>,
    pub organization: ConversationOrganization,
    pub tags: Option<Page<TagRef, 20>>,
    /// Authoritative server state. Unknown is not an active/non-archived room.
    pub archived: Option<bool>,
    pub archive: Decision,
    pub presence: Presence,
    /// Ephemeral status text returned for a direct conversation.
    pub status_icon: Option<Text<64>>,
    pub status_message: Option<Text<320>>,
    pub participants: Option<u16>,
    /// A small, session-only index of names observed with participant permission.
    pub participant_names: Page<Text<256>, 16>,
    pub send: Decision,
    pub attach: Decision,
    /// Explicit clipboard-export privacy policy, not a generic server permission
    /// or permission to paste local text into an editable draft.
    /// Classified/unknown restrictions MUST project Denied/Unknown; never infer from send.
    pub draft_clipboard: Decision,
    pub call: Decision,
    /// Server observation; unknown does not imply that a call is active.
    pub call_active: Option<bool>,
    /// Allocated/revalidated by the call owner; a row click never creates a call.
    pub call_scope: Option<CallScope>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryState {
    Received,
    /// Locally staged; no transport submission or server acceptance is implied.
    Queued,
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
    pub raised_hand: bool,
    /// Recent decoded audio exceeds the local indication threshold.
    pub speaking: bool,
    pub call_can_mute: bool,
    /// Last observed call flags, absent outside the call projection.
    pub call_microphone: Option<bool>,
    pub call_camera: Option<bool>,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CallTileKind {
    Camera,
    Screen,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ScreenShareState {
    #[default]
    Idle,
    Choosing,
    Starting,
    Sharing,
    Failed(ScreenShareFailure),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScreenShareFailure {
    Unavailable,
    Cancelled,
    SourceClosed,
    TimedOut,
    PermissionDenied,
}
/// Frame-free stream metadata. `local_id` addresses the media image handoff;
/// `participant` identifies the camera/participant tile belonging to this source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CallStreamView {
    pub local_id: u128,
    pub participant: u128,
    pub participant_name: Text<128>,
    pub kind: CallTileKind,
    pub dimensions: Option<(u32, u32)>,
    pub flowing: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CallTileId {
    pub local_id: u128,
    pub kind: CallTileKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CallScreenStage {
    pub tile: CallTileId,
    pub name: Text<128>,
    pub dimensions: Option<(u32, u32)>,
    pub flowing: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CallRecordingState {
    Inactive,
    Starting,
    Active,
    Stopping,
    Failed,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CallReactionView {
    pub sequence: u64,
    pub participant: Option<u128>,
    pub emoji: Text<64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CallInviteeView {
    pub local_id: u128,
    pub ring_allowed: bool,
    pub name: Text<128>,
    pub initials: Text<16>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CallRingOutcome {
    Pending,
    Requested,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CallRingStatus {
    pub target: u128,
    pub outcome: CallRingOutcome,
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
    pub sharing: ScreenShareState,
    pub streams: Page<CallStreamView, 16>,
    pub recording: CallRecordingState,
    /// Some(true) starts, Some(false) stops; None hides unsupported controls.
    pub recording_control: Option<bool>,
    pub recording_pending: bool,
    pub recording_uncertain: bool,
    pub hand_supported: bool,
    pub hand_raised: bool,
    pub reactions: Page<Text<64>, 32>,
    pub latest_reaction: Option<CallReactionView>,
    pub invitees_loaded: bool,
    pub invitees: Page<CallInviteeView, 100>,
    pub ring_status: Option<CallRingStatus>,
    pub mute_status: Option<CallRingStatus>,
    pub can_end_everyone: bool,
    pub end_pending: bool,
    pub end_failed: bool,
    /// Up to sixteen remote participants plus the local call participant.
    pub participants: Page<ParticipantView, 17>,
}

impl CallView {
    /// Resolve a participant's camera without conflating actor and stream IDs.
    pub fn camera_tile(&self, participant: u128) -> Option<CallTileId> {
        let stream = self.streams.as_slice().iter().find(|stream| {
            stream.kind == CallTileKind::Camera && stream.participant == participant
        })?;
        Some(CallTileId {
            local_id: stream.local_id,
            kind: stream.kind,
        })
    }
    /// A presentation projection of the admitted stream; never separate authority.
    pub fn screen_stage(&self) -> Option<CallScreenStage> {
        let stream = self
            .streams
            .as_slice()
            .iter()
            .find(|s| s.kind == CallTileKind::Screen)?;
        Some(CallScreenStage {
            tile: CallTileId {
                local_id: stream.local_id,
                kind: stream.kind,
            },
            name: stream.participant_name.clone(),
            dimensions: stream.dimensions,
            flowing: stream.flowing,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Completion {
    /// The local outbox accepted the intent. Normal saved accounts commit it
    /// durably; restricted/session-only contexts retain it for this session.
    /// No server acceptance, recipient delivery or read is implied.
    Queued,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountRemovalFailure {
    NativeLocked,
    NativeDenied,
    NativeUnavailable,
    NativeCancelled,
    NativeFailed,
    LocalDenied,
    LocalFile,
    LocalState,
    SuppressionUnconfirmed,
    CleanupPending,
    TimedOut,
    Busy,
    Cancelled,
    Changed,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccountRemovalResult {
    pub request: RequestId,
    pub account: AccountId,
    /// Local removal, never a claim that a server token was revoked.
    pub result: Result<(), AccountRemovalFailure>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresentationSnapshot {
    pub content_panel: super::content::ContentPanel,
    pub file_export: Option<super::content::FileExportView>,
    pub create_panel: Option<super::conversation::CreatePanel>,
    pub discover_panel: Option<super::discover::Panel>,
    pub search_panel: Option<super::search::Panel>,
    pub reference_previews: super::reference::Previews,
    pub revision: u64,
    pub login: LoginView,
    pub accounts: Page<AccountView, 8>,
    /// Durable records have no live scope until the account owner admits one.
    pub saved_accounts: Page<SavedAccountView, 8>,
    /// Correlated local removal/cleanup outcome, retained until acknowledged.
    pub account_removal: Option<AccountRemovalResult>,
    pub active_account: Option<AccountScope>,
    pub conversations: Page<ConversationView, 100>,
    pub threads: ThreadList,
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

/// Account-scoped, bounded subscribed-thread observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ThreadList {
    pub account: Option<AccountScope>,
    pub support: FeatureSupport,
    pub content: ContentState,
    pub entries: Page<ThreadView, 50>,
    pub offset: u32,
    pub has_next: bool,
}
impl Default for ThreadList {
    fn default() -> Self {
        Self {
            account: None,
            support: FeatureSupport::Unknown,
            content: ContentState::Unconfigured,
            entries: Page::empty(),
            offset: 0,
            has_next: false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ThreadView {
    pub context: ConversationRef,
    pub root: u128,
    pub title: Text<256>,
    pub replies: u32,
    /// Last reply when the server supplies one; zero-reply threads use their root.
    pub last_message: Option<u128>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConversationListState {
    pub content: ContentState,
    /// False for partial/unknown lists; absence is not an authoritative empty list.
    pub complete: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UiIntent {
    RefreshThreads(AccountScope),
    PageThreads {
        account: AccountScope,
        offset: u32,
    },
    /// Mark a listed room unread without changing the selected conversation.
    MarkRoomUnread {
        request: RequestId,
        context: ConversationRef,
    },
    Discover {
        account: AccountScope,
        request: RequestId,
        action: super::discover::Action,
    },
    AccountSearch {
        account: AccountScope,
        request: RequestId,
        action: super::search::Action,
    },
    /// Acknowledge or cancel only this native promise, including after navigation.
    FinishFileExport {
        request: RequestId,
        context: ConversationRef,
    },
    /// Deliberate activation of one link in the exact displayed message body.
    /// Composition revalidates the room, source and ordinal before OS handoff.
    OpenMessageLink {
        context: ConversationRef,
        message: u128,
        source: Text<4096>,
        ordinal: u16,
    },
    Content {
        request: RequestId,
        account: AccountScope,
        action: super::content::ContentAction,
    },
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
    /// Recent local composing activity; the account signaling owner throttles it.
    Typing {
        context: ConversationRef,
        active: bool,
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
    /// Remove this app's admitted account or dormant saved record. A live
    /// account requires its exact current scope; None never targets a live scope.
    RemoveAccount {
        request: RequestId,
        account: AccountId,
        scope: Option<AccountScope>,
    },
    AcknowledgeAccountRemoval {
        request: RequestId,
        account: AccountId,
    },
    Reauthenticate(AccountId),
    BeginLogin {
        request: RequestId,
        server: ServerInput,
    },
    /// Retry the failed attempt's complete installation address, retained by its owner.
    RetryLogin {
        request: RequestId,
    },
    /// Retry opening local accounts before any login destination was loaded.
    RetryAccountLoad,
    /// Deliberate recovery of the same legacy macOS directory after a mount change.
    RepairAccountBinding,
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
    EndCallForEveryone(CallScope),
    SetCallRecording {
        call: CallScope,
        start: bool,
    },
    MuteCallParticipant {
        call: CallScope,
        target: u128,
    },
    RingCallParticipant {
        call: CallScope,
        target: u128,
    },
    SetCallHand {
        call: CallScope,
        raised: bool,
    },
    SendCallReaction {
        call: CallScope,
        emoji: Text<64>,
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
