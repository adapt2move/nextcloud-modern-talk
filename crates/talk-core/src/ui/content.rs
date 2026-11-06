//! Bounded content observations. Image bytes and authenticated routes stay with composition.
use super::conversation::{AccountPersonRef, Availability, RemoteState};
use super::presentation::{Completion, ConversationRef, RequestId};
use super::{AccountScope, Page, Text, UnavailableReason};
/// One native file promise's bounded observation, retained until acknowledged.
/// This correlation grants no authority to read the former account or room.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileExportView {
    pub request: RequestId,
    pub context: ConversationRef,
    pub saved: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttachmentRef {
    pub context: ConversationRef,
    pub message: u128,
    pub placeholder: Text<128>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AssetTarget {
    SelfAvatar,
    ConversationAvatar(ConversationRef),
    MessageAvatar {
        context: ConversationRef,
        message: u128,
    },
    ParticipantAvatar {
        context: ConversationRef,
        participant: u128,
    },
    PersonAvatar(AccountPersonRef),
    CloudFile {
        account: AccountScope,
        path: Text<16384>,
        file: u64,
    },
    Thumbnail(AttachmentRef),
    Image(AttachmentRef),
}
impl AssetTarget {
    pub fn context(&self) -> Option<ConversationRef> {
        match self {
            Self::ConversationAvatar(c)
            | Self::MessageAvatar { context: c, .. }
            | Self::ParticipantAvatar { context: c, .. } => Some(*c),
            Self::Thumbnail(a) | Self::Image(a) => Some(a.context),
            Self::SelfAvatar | Self::PersonAvatar(_) | Self::CloudFile { .. } => None,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssetState {
    Loading,
    Ready { key: u64 },
    Unsupported,
    Denied,
    Missing,
    Failed,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetView {
    pub target: AssetTarget,
    pub state: AssetState,
    pub emoji: Option<Text<64>>,
    pub color: Option<Text<16>>,
    /// Present only for the native PDF first-page renderer's observed page count.
    pub document_pages: Option<u32>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PollOption {
    pub id: u16,
    pub label: Text<1000>,
    pub votes: Option<u32>,
    pub own: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PollVoter {
    pub name: Text<256>,
    pub option: u16,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PollView {
    pub id: u64,
    pub question: Text<4000>,
    pub options: Page<PollOption, 100>,
    pub closed: bool,
    /// Hidden-result polls reveal only aggregates after closure; never voter identities.
    pub hidden_results: bool,
    pub max_votes: u16,
    pub voters: Option<u32>,
    pub details: Page<PollVoter, 100>,
    pub details_complete: bool,
    pub vote: Availability,
    pub close: Availability,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PollDraft {
    pub question: Text<4000>,
    pub options: Page<Text<1000>, 100>,
    pub multiple: bool,
    pub hidden_results: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DownloadState {
    Transferring,
    Committing,
    Saved,
    Cancelled,
    Failed(UnavailableReason),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DownloadView {
    pub request: RequestId,
    pub attachment: AttachmentRef,
    pub name: Text<255>,
    pub received: u64,
    pub total: Option<u64>,
    pub state: DownloadState,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlaybackState {
    Loading,
    Paused,
    Playing,
    Ended,
    Stopping,
    Failed(UnavailableReason),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlaybackView {
    pub attachment: AttachmentRef,
    pub generation: u64,
    pub state: PlaybackState,
    pub position_ms: u64,
    pub duration_ms: Option<u64>,
    pub stop_unconfirmed: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CloudFileView {
    pub name: Text<255>,
    pub path: Text<16384>,
    pub directory: bool,
    pub size: Option<u64>,
    pub modified: Option<Text<256>>,
    pub modified_unix: Option<i64>,
    pub mime: Option<Text<256>>,
    pub file_id: Option<Text<256>>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileBrowserView {
    pub account: AccountScope,
    pub path: Text<16384>,
    pub section: FileSection,
    pub sort: FileSort,
    pub entries: Page<CloudFileView, 1000>,
    /// DAV page offset and total, including the folder itself. No server token.
    pub pagination: Option<(u64, u64)>,
    pub loading: bool,
    pub offline: bool,
    pub error: Option<UnavailableReason>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrowserDownloadView {
    pub request: RequestId,
    pub name: Text<255>,
    pub received: u64,
    pub total: Option<u64>,
    pub state: DownloadState,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileSection {
    All,
    Recent,
    Favorites,
    Search,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileSort {
    Name,
    Modified,
    Size,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContentPanel {
    pub file_browser: Option<FileBrowserView>,
    pub browser_download: Option<BrowserDownloadView>,
    pub self_name: Option<Text<256>>,
    pub organization: Option<Text<256>>,
    pub account: Option<AccountScope>,
    pub context: Option<ConversationRef>,
    pub assets: Page<AssetView, 128>,
    /// Composition-owned avatar images may outlive mounted target aliases.
    /// These keys retain renderer resources; they authorize no visible target.
    pub retained_avatar_keys: Page<u64, 128>,
    pub polls: Page<PollView, 20>,
    pub poll_create: Availability,
    pub poll_state: RemoteState,
    pub preview: Option<AttachmentRef>,
    pub playback: Option<PlaybackView>,
    pub transfer: Option<DownloadView>,
    pub result: Option<(RequestId, Completion)>,
}
impl Default for ContentPanel {
    fn default() -> Self {
        Self {
            file_browser: None,
            browser_download: None,
            self_name: None,
            organization: None,
            account: None,
            context: None,
            assets: Page::empty(),
            retained_avatar_keys: Page::empty(),
            polls: Page::empty(),
            poll_create: Availability::UNKNOWN,
            poll_state: RemoteState::Idle,
            preview: None,
            playback: None,
            transfer: None,
            result: None,
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContentAction {
    ListFiles(Text<16384>),
    ListFilesPage {
        path: Text<16384>,
        offset: u64,
    },
    RecentFiles,
    FavoriteFiles,
    SearchFiles(Text<256>),
    SortFiles(FileSort),
    OpenBrowserFile(Text<16384>),
    DownloadBrowserFile(Text<16384>),
    CancelBrowserDownload,
    DismissBrowserDownload,
    CloseFileBrowser,
    StageCloudFile {
        context: ConversationRef,
        path: Text<16384>,
    },
    StageCloudFiles {
        context: ConversationRef,
        paths: Page<Text<16384>, 8>,
    },
    DismissDownload,
    Load(AssetTarget),
    Preview(AttachmentRef),
    ClosePreview,
    PlayPreview {
        attachment: AttachmentRef,
        generation: u64,
    },
    PausePreview {
        attachment: AttachmentRef,
        generation: u64,
    },
    SeekPreview {
        attachment: AttachmentRef,
        generation: u64,
        position_ms: u64,
    },
    OpenCloud(AttachmentRef),
    RefreshPoll(u64),
    CreatePoll(PollDraft),
    Vote {
        poll: u64,
        options: Page<u16, 100>,
    },
    ClosePoll(u64),
    CancelDownload,
    Acknowledge,
}
