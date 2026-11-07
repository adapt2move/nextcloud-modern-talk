//! Finite messaging projection. The composition owner reconciles canonical IDs.
use super::conversation::{Availability, RemoteState};
use super::presentation::{Completion, ConversationRef, RequestId};
use super::{ClipboardImage, Decision, Page, Text, UnavailableReason};
use crate::preferences::navigation::NavigationRoomKey;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageFeature {
    React,
    Reply,
    PrivateReply,
    Edit,
    Delete,
    Threads,
    Mentions,
    Silent,
    Search,
    Pins,
    Reminders,
    Schedules,
    Forward,
    Files,
    Voice,
    Typing,
    ReadReceipts,
    MarkUnread,
    UnreadFrom,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reaction {
    pub emoji: Text<64>,
    pub count: u32,
    pub own: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReactionAuthors {
    pub message: u128,
    pub emoji: Text<64>,
    pub authors: Page<Text<256>, 100>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReactionAuthorsLoad {
    pub request: RequestId,
    pub message: u128,
    pub emoji: Text<64>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ThreadDetails {
    pub root: u128,
    pub title: Text<256>,
    pub notification_level: Option<i8>,
    pub rename: Decision,
}
/// Recent room thread activity, scoped by the enclosing message panel.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ThreadActivity {
    pub root: u128,
    pub replies: u16,
    pub last_initials: Option<Text<16>>,
    pub recent_initials: Page<Text<16>, 3>,
    pub last_time: Option<Text<32>>,
    pub expires: Option<u64>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RichObject {
    pub id: Option<Text<1024>>,
    pub mime_type: Option<Text<256>>,
    pub size: Option<u64>,
    pub placeholder: Text<128>,
    pub kind: Text<128>,
    pub name: Text<1024>,
    pub link: Option<Text<2048>>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reply {
    pub id: u128,
    pub external: bool,
    pub author: Text<256>,
    pub text: Text<4096>,
    pub unavailable: bool,
}
/// Domain classification from an observed system tag, never from display text.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SystemEvent {
    /// No classification was observed, including older durable cache entries.
    #[default]
    Unknown,
    Other,
    CallIncoming,
    CallOutgoing,
    CallJoined,
    CallLeft,
    CallEnded,
    CallMissed,
    CallUnanswered,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageMetadata {
    /// Opaque correlation for the local outbox and a validated own-message
    /// reference. It never establishes identity from text or timestamps.
    pub local_operation: Option<u128>,
    /// Session-local admission order for pending text and attachment rows.
    pub local_order: Option<u64>,
    /// Opaque, room-scoped actor identity; display names do not identify senders.
    /// Missing or oversized protocol identities deliberately prevent grouping.
    pub author_key: Option<Text<256>>,
    pub expires_at: Option<u64>,
    pub timestamp: u64,
    pub truncated: bool,
    pub edited: bool,
    pub deleted: bool,
    pub system: bool,
    /// The server explicitly marked this attachment as a voice message.
    pub voice_message: bool,
    pub system_event: SystemEvent,
    /// Quiet pin and poll activity, classified from the untranslated system
    /// tag. Canonical history retains it; the visible timeline omits it. The
    /// field name is retained for compatibility with stored history.
    pub pin_notice: bool,
    /// Effective receipt target for this painted row, extending only through
    /// adjacent hidden pin activity with observed server-page continuity.
    /// Derived by composition; durable content never restores this evidence.
    pub read_through: Option<u128>,
    /// Exact adjacent predecessor, proven by a live server page in this runtime.
    pub unread_after: Option<u128>,
    pub replyable: bool,
    pub parent: Option<Reply>,
    pub thread: Option<u128>,
    pub reactions: Page<Reaction, 64>,
    pub rich: Page<RichObject, 32>,
    pub edit: Decision,
    pub delete: Decision,
}
impl Default for MessageMetadata {
    fn default() -> Self {
        Self {
            local_operation: None,
            local_order: None,
            author_key: None,
            expires_at: None,
            timestamp: 0,
            truncated: false,
            edited: false,
            deleted: false,
            system: false,
            voice_message: false,
            system_event: SystemEvent::Unknown,
            pin_notice: false,
            read_through: None,
            unread_after: None,
            replyable: false,
            parent: None,
            thread: None,
            reactions: Page::empty(),
            rich: Page::empty(),
            edit: Decision::Unknown,
            delete: Decision::Unknown,
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchResult {
    pub id: u128,
    pub title: Text<256>,
    pub excerpt: Text<4096>,
}
/// Ephemeral signaling observation, scoped by the containing message panel.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TypingPerson {
    pub name: Text<256>,
    pub author_key: Text<256>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Mention {
    pub identifier: Text<512>,
    pub author_key: Option<Text<256>>,
    pub label: Text<256>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reminder {
    pub message: u128,
    pub timestamp: u64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Scheduled {
    pub id: u128,
    pub body: Text<4096>,
    pub timestamp: u64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UploadedFile {
    pub path: Text<16384>,
    pub name: Text<255>,
    pub bytes: u64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadFailure {
    TooMany,
    Missing,
    TooLarge,
    Denied,
    Connection,
    Unsupported,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadState {
    Selected,
    Uploading,
    Sharing,
    Confirming,
    Shared,
    Failed(UploadFailure),
    Uncertain,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UploadView {
    pub id: u64,
    pub name: Text<255>,
    pub bytes: Option<u64>,
    pub state: UploadState,
    pub submitted: bool,
    pub transferred: u64,
    pub uploaded: bool,
    /// A reference-correlated canonical message, never guessed from its name.
    pub server_message: Option<u128>,
}
impl UploadView {
    pub fn message_id(&self) -> u128 {
        (1u128 << 126) | u128::from(self.id)
    }
    pub fn is_message_id(id: u128) -> bool {
        id & !u128::from(u64::MAX) == 1u128 << 126
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextLoad {
    pub target: u128,
    pub state: RemoteState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageActionError {
    Failed(UnavailableReason),
    /// A possibly applied write is retained for read-only reconciliation.
    Uncertain,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailedDraftKind {
    Send,
    Edit(u128),
    Schedule(u64),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FailedDraft {
    pub operation: u128,
    pub body: Text<4096>,
    pub target: ComposeTarget,
    pub kind: FailedDraftKind,
    pub retrying: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessagePanel {
    pub failed_drafts: Page<FailedDraft, 64>,
    /// Local durable or session-memory admission, never a server permission grant.
    pub queue_available: bool,
    pub queued_actions: u32,
    /// A pending room unread mutation keeps automatic read receipts suspended.
    pub unread_queued: bool,
    /// Latest scoped outbox failure, identified independently from user request
    /// counters so a late rejection cannot complete another action.
    pub action_error: Option<(u64, MessageActionError)>,
    pub pinned_messages: Page<super::presentation::MessageView, 50>,
    pub pins_state: RemoteState,
    pub pins_loaded: bool,
    pub pin_change: Decision,
    pub shared_files: Page<super::presentation::MessageView, 50>,
    pub file_limit: u64,
    pub media_caption: bool,
    pub uploaded: Option<UploadedFile>,
    pub uploads: Page<UploadView, 8>,
    pub uploads_running: bool,
    /// Local batch admission; the composer may clear without claiming delivery.
    pub files_queued: Option<RequestId>,
    pub files_sent: Option<RequestId>,
    pub file_error: Option<UploadFailure>,
    pub upload_name: Option<Text<255>>,
    pub file_saved: Option<u64>,
    pub thread_root: Option<u128>,
    pub thread_state: RemoteState,
    /// Independent root lookup when it lies outside the retained room history.
    pub thread_root_state: RemoteState,
    pub thread_root_message: Option<super::presentation::MessageView>,
    pub thread_messages: Page<super::presentation::MessageView, 50>,
    /// Server cursor for the previous reply page; pages stay bounded.
    pub thread_older: Option<u128>,
    pub thread_has_newer: bool,
    pub thread_activity: Page<ThreadActivity, 50>,
    pub thread_details: Option<ThreadDetails>,
    /// Prior observation for the single admitted optimistic thread change.
    pub thread_change_prior: Option<(RequestId, ThreadDetails, Option<bool>)>,
    /// Pending local subscription intent; this does not grant server authority.
    pub thread_following_intent: Option<bool>,
    pub thread_details_request: Option<RequestId>,
    pub context: ConversationRef,
    /// Foreground mutation state; targeted reads settle their own request slots.
    pub state: RemoteState,
    /// One admitted reaction mutation, cleared only by its correlated result.
    pub reaction_pending: Option<u128>,
    pub reaction_authors: Page<ReactionAuthors, 16>,
    pub reaction_authors_load: Option<ReactionAuthorsLoad>,
    pub history: RemoteState,
    pub older: Option<u128>,
    pub latest: Option<u128>,
    pub common_read: Option<u128>,
    pub features: Page<(MessageFeature, Availability), 32>,
    pub message_limit: u32,
    pub search: Page<SearchResult, 50>,
    pub search_loaded: bool,
    pub search_more: bool,
    pub located: Option<(RequestId, u128)>,
    /// One message-navigation request, independent of mutation and history state.
    pub context_load: Option<ContextLoad>,
    pub mentions: Page<Mention, 100>,
    pub typing: Page<TypingPerson, 16>,
    pub reminders: Page<Reminder, 100>,
    pub scheduled: Page<Scheduled, 100>,
    pub result: Option<(RequestId, Completion)>,
}
impl MessagePanel {
    pub fn new(context: ConversationRef) -> Self {
        Self {
            failed_drafts: Page::empty(),
            queue_available: false,
            queued_actions: 0,
            unread_queued: false,
            action_error: None,
            pinned_messages: Page::empty(),
            pins_state: RemoteState::Idle,
            pins_loaded: false,
            pin_change: Decision::Unknown,
            shared_files: Page::empty(),
            file_limit: 0,
            media_caption: false,
            uploaded: None,
            uploads: Page::empty(),
            uploads_running: false,
            files_queued: None,
            files_sent: None,
            file_error: None,
            upload_name: None,
            file_saved: None,
            thread_root: None,
            thread_state: RemoteState::Idle,
            thread_root_state: RemoteState::Idle,
            thread_root_message: None,
            thread_messages: Page::empty(),
            thread_older: None,
            thread_has_newer: false,
            thread_activity: Page::empty(),
            thread_details: None,
            thread_change_prior: None,
            thread_following_intent: None,
            thread_details_request: None,
            context,
            state: RemoteState::Idle,
            reaction_pending: None,
            reaction_authors: Page::empty(),
            reaction_authors_load: None,
            history: RemoteState::Idle,
            older: None,
            latest: None,
            common_read: None,
            features: Page::empty(),
            message_limit: 4096,
            search: Page::empty(),
            search_loaded: false,
            search_more: false,
            located: None,
            context_load: None,
            mentions: Page::empty(),
            typing: Page::empty(),
            reminders: Page::empty(),
            scheduled: Page::empty(),
            result: None,
        }
    }
    pub fn availability(&self, feature: MessageFeature) -> Availability {
        self.features
            .as_slice()
            .iter()
            .find(|(key, _)| *key == feature)
            .map_or(Availability::UNKNOWN, |(_, value)| *value)
    }
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ComposeTarget {
    pub reply_to: Option<u128>,
    pub private_reply: Option<PrivateReplyTarget>,
    pub thread: Option<u128>,
    /// Create a thread whose root is this new message.
    pub new_thread: bool,
    pub silent: bool,
}
/// Displayed quote and durable lookup key; neither grants access to the source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrivateReplyTarget {
    pub source: NavigationRoomKey,
    pub message: u128,
    pub author: Text<128>,
    pub body: Text<512>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MessageAction {
    RetryQueued(u128),
    DiscardQueued(u128),
    PickUpload,
    StageFiles(Page<Text<16384>, 8>),
    StageClipboardImage {
        name: Text<255>,
        image: ClipboardImage,
    },
    SendFiles {
        caption: Text<4096>,
        target: ComposeTarget,
    },
    RemoveFile(u64),
    RetryFile(u64),
    ExportFile {
        attachment: super::content::AttachmentRef,
        source: Page<RichObject, 1>,
        destination: Text<16384>,
    },
    Download {
        message: u128,
        placeholder: Text<128>,
        name: Text<255>,
    },
    ShareUpload {
        caption: Text<4096>,
    },
    DismissUpload,
    Files,
    Thread(u128),
    ThreadOlder(u128),
    ThreadRoot(u128),
    ThreadInfo(u128),
    RenameThread {
        root: u128,
        title: Text<256>,
    },
    ThreadNotifications {
        root: u128,
        level: i8,
    },
    Older,
    Refresh,
    Context(u128),
    Send {
        body: Text<4096>,
        target: ComposeTarget,
    },
    Edit {
        message: u128,
        body: Text<4096>,
    },
    Delete(u128),
    React {
        message: u128,
        emoji: Text<64>,
        add: bool,
    },
    ReactionAuthors {
        message: u128,
        emoji: Text<64>,
    },
    MarkRead(u128),
    MarkUnread,
    MarkUnreadFrom(u128),
    /// Locate the first visible message after the read marker; zero means start.
    LocateUnreadAfter(u128),
    Typing(bool),
    Mentions(Text<256>),
    Search(Text<256>),
    SearchNext,
    Pin(u128),
    Unpin(u128),
    Pins,
    Reminder {
        message: u128,
        timestamp: Option<u64>,
    },
    Reminders,
    Schedule {
        body: Text<4096>,
        timestamp: u64,
        target: ComposeTarget,
    },
    Scheduled,
    CancelScheduled(u128),
    Forward {
        message: u128,
        destination: ConversationRef,
    },
    ShareFile {
        path: Text<16384>,
        caption: Text<4096>,
    },
}
