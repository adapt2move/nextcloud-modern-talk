//! Finite messaging projection. The composition owner reconciles canonical IDs.
use super::conversation::{Availability, RemoteState};
use super::presentation::{Completion, ConversationRef, RequestId};
use super::{Decision, Page, Text};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageFeature {
    React,
    Reply,
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
    Typing,
    ReadReceipts,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reaction {
    pub emoji: Text<64>,
    pub count: u32,
    pub own: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RichObject {
    pub placeholder: Text<128>,
    pub kind: Text<128>,
    pub name: Text<1024>,
    pub link: Option<Text<2048>>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reply {
    pub id: u128,
    pub author: Text<256>,
    pub text: Text<4096>,
    pub unavailable: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageMetadata {
    pub timestamp: u64,
    pub truncated: bool,
    pub edited: bool,
    pub deleted: bool,
    pub system: bool,
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
            timestamp: 0,
            truncated: false,
            edited: false,
            deleted: false,
            system: false,
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
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Mention {
    pub identifier: Text<512>,
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
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessagePanel {
    pub pinned_messages: Page<super::presentation::MessageView, 50>,
    pub shared_files: Page<super::presentation::MessageView, 50>,
    pub file_limit: u64,
    pub uploaded: Option<UploadedFile>,
    pub upload_name: Option<Text<255>>,
    pub file_saved: Option<u64>,
    pub thread_root: Option<u128>,
    pub thread_messages: Page<super::presentation::MessageView, 50>,
    pub context: ConversationRef,
    pub state: RemoteState,
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
    pub mentions: Page<Mention, 100>,
    pub reminders: Page<Reminder, 100>,
    pub scheduled: Page<Scheduled, 100>,
    pub result: Option<(RequestId, Completion)>,
}
impl MessagePanel {
    pub fn new(context: ConversationRef) -> Self {
        Self {
            pinned_messages: Page::empty(),
            shared_files: Page::empty(),
            file_limit: 0,
            uploaded: None,
            upload_name: None,
            file_saved: None,
            thread_root: None,
            thread_messages: Page::empty(),
            context,
            state: RemoteState::Idle,
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
            mentions: Page::empty(),
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
    pub thread: Option<u128>,
    pub silent: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MessageAction {
    PickUpload,
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
    MarkRead(u128),
    MarkUnread,
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
