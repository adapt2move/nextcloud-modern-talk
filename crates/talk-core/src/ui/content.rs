//! Bounded content observations. Image bytes and authenticated routes stay with composition.
use super::conversation::{AccountPersonRef, Availability, RemoteState};
use super::presentation::{Completion, ConversationRef, RequestId};
use super::{AccountScope, Page, Text, UnavailableReason};

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
            Self::SelfAvatar | Self::PersonAvatar(_) => None,
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
pub struct ContentPanel {
    pub self_name: Option<Text<256>>,
    pub organization: Option<Text<256>>,
    pub account: Option<AccountScope>,
    pub context: Option<ConversationRef>,
    pub assets: Page<AssetView, 128>,
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
            self_name: None,
            organization: None,
            account: None,
            context: None,
            assets: Page::empty(),
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
