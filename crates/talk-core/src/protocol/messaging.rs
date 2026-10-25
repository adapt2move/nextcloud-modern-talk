//! Bounded message actions and observations. All cursors belong to one conversation.
use super::conversation::ActionAvailability;
use super::{Message, MessageId, SendReceipt};
use crate::bounded::{Page, Text};
use crate::ids::ConversationId;
use crate::policy::Decision;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reaction {
    pub emoji: Text<64>,
    pub count: u32,
    pub own: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RichObject {
    pub id: Option<Text<1024>>,
    pub mime_type: Option<Text<256>>,
    pub size: Option<u64>,
    pub placeholder: Text<128>,
    pub kind: Text<128>,
    pub name: Text<1024>,
    /// Untrusted destination text; renderer requires deliberate activation and URL policy.
    pub link: Option<Text<2048>>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplyContext {
    pub id: MessageId,
    pub author: Text<256>,
    pub text: Text<4096>,
    pub unavailable: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageDetails {
    pub message: Message,
    /// Opaque actor identity within this conversation; display names are not identity.
    pub author_key: Option<Text<256>>,
    /// Server-policy eligibility for this fresh poll observation only. The native owner
    /// must additionally enforce freshness, visibility, preview policy and deduplication.
    /// Unknown and Denied never authorize a notification; history is always Denied.
    pub notify: Decision,
    pub own: bool,
    pub replyable: Option<bool>,
    pub parent: Option<ReplyContext>,
    pub thread: Option<MessageId>,
    pub reactions: Page<Reaction, 64>,
    pub rich: Page<RichObject, 32>,
    pub system: Option<Text<128>>,
    pub edit: Decision,
    pub delete: Decision,
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ComposeOptions {
    pub reply_to: Option<MessageId>,
    pub thread: Option<MessageId>,
    pub thread_title: Option<Text<256>>,
    pub silent: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Mention {
    pub id: Text<512>,
    pub label: Text<256>,
    pub source: Text<64>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageReminder {
    pub message: MessageId,
    pub timestamp: u64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduledMessage {
    pub id: MessageId,
    pub text: Text<131072>,
    pub send_at: u64,
    pub options: ComposeOptions,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Thread {
    pub root: MessageId,
    pub title: Text<256>,
    pub replies: u32,
    pub unread: Option<u32>,
    pub notification_level: Option<i8>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchHit {
    pub message: MessageId,
    pub title: Text<256>,
    pub excerpt: Text<4096>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessagingFeature {
    Read,
    Send,
    Reply,
    Edit,
    Delete,
    React,
    Mentions,
    Silent,
    Search,
    Pins,
    Reminders,
    Schedule,
    Threads,
    Forward,
    Files,
    Typing,
    ReadReceipts,
    Translation,
    Summary,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessagingCapabilities {
    pub actions: Page<(MessagingFeature, ActionAvailability), 32>,
    /// Changing shared pins is separate from reading the shared pin list.
    pub pin_change: Decision,
    pub message_limit: Option<u32>,
    pub file_limit: u64,
    pub read_receipts: Option<bool>,
    pub typing_status: Option<bool>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MessagingCommand {
    Capabilities,
    History {
        before: Option<MessageId>,
        thread: Option<MessageId>,
    },
    /// Finite future poll. Does not mark read or clear notifications.
    Poll {
        after: MessageId,
        thread: Option<MessageId>,
    },
    Context(MessageId),
    /// Read-only recovery of one durable send correlation after restart. Absence
    /// in the bounded search is uncertain and never authorizes a second POST.
    ReconcileSend,
    Send {
        text: Text<131072>,
        options: ComposeOptions,
    },
    Edit {
        message: MessageId,
        text: Text<131072>,
    },
    Delete(MessageId),
    React {
        message: MessageId,
        emoji: Text<64>,
        add: bool,
    },
    ReactionAuthors {
        message: MessageId,
        emoji: Text<64>,
    },
    MarkUnread,
    MarkRead(MessageId),
    Typing(bool),
    Mentions(Text<256>),
    Search {
        query: Text<256>,
        cursor: Option<Text<1024>>,
    },
    Pin {
        message: MessageId,
        until: u64,
    },
    Unpin(MessageId),
    HidePin(MessageId),
    Pins,
    Reminder {
        message: MessageId,
        timestamp: Option<u64>,
    },
    ReadReminder(MessageId),
    Reminders,
    Scheduled,
    Schedule {
        text: Text<131072>,
        send_at: u64,
        options: ComposeOptions,
    },
    EditScheduled {
        id: MessageId,
        text: Text<131072>,
        send_at: u64,
        silent: bool,
    },
    CancelScheduled(MessageId),
    Threads,
    Thread(MessageId),
    RenameThread {
        root: MessageId,
        title: Text<256>,
    },
    ThreadNotifications {
        root: MessageId,
        level: i8,
    },
    /// Caller must display destination and outgoing text before explicit submission.
    Forward {
        source: MessageId,
        destination: ConversationId,
    },
    /// A durable forward retains the preview that the user approved. A changed
    /// source must be shown again, never silently substituted during replay.
    ForwardApproved {
        source: MessageId,
        destination: ConversationId,
        author: Text<256>,
        body: Text<4096>,
    },
    SharedFiles {
        before: Option<MessageId>,
    },
    /// Upload only; a separate explicit ShareFile publishes it to the room.
    UploadFile {
        local_path: Text<16384>,
        name: Text<255>,
    },
    DownloadFile {
        message: MessageId,
        placeholder: Text<128>,
        destination: Text<16384>,
        progress: super::content::TransferProgress,
    },
    ShareFile {
        path: Text<16384>,
        caption: Text<4096>,
    },
    Translate {
        message: MessageId,
        language: Text<64>,
    },
    Summarize {
        from: MessageId,
    },
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MessagingResult {
    Capabilities(MessagingCapabilities),
    Messages {
        /// Room capabilities observed by the same read, avoiding a second probe.
        capabilities: Option<MessagingCapabilities>,
        messages: Page<MessageDetails, 50>,
        cursor: Option<MessageId>,
        common_read: Option<MessageId>,
    },
    Message(Box<MessageDetails>),
    Send(SendReceipt),
    Mentions(Page<Mention, 100>),
    ReactionAuthors(Page<Text<256>, 100>),
    /// Authoritative counts returned by an admitted add/remove operation.
    Reactions {
        message: MessageId,
        reactions: Page<Reaction, 64>,
    },
    Search {
        hits: Page<SearchHit, 50>,
        cursor: Option<Text<1024>>,
    },
    Reminder(Option<MessageReminder>),
    Reminders(Page<MessageReminder, 100>),
    Scheduled(Page<ScheduledMessage, 100>),
    Schedule(Box<ScheduledMessage>),
    Threads(Page<Thread, 50>),
    Thread(Box<Thread>),
    Uploaded {
        path: Text<16384>,
        bytes: u32,
    },
    FileSaved {
        bytes: u32,
    },
    Changed,
}
