//! Finite content operations on an admitted account. Images are transient worker values,
//! never serialized UI data. Progress is shared observation, not a second network owner.
use super::MessageId;
use super::conversation::{ActionAvailability, MemberId, RecipientId};
use crate::bounded::{LimitExceeded, Page, Text};
use crate::ids::ConversationId;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

pub const MAX_IMAGE_BYTES: usize = 4 * 1024 * 1024;
#[derive(Clone, Eq, PartialEq)]
pub struct ImageBytes(Arc<[u8]>);
impl std::fmt::Debug for ImageBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ImageBytes").field(&self.0.len()).finish()
    }
}
impl ImageBytes {
    pub fn new(bytes: Vec<u8>) -> Result<Self, LimitExceeded> {
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(LimitExceeded);
        }
        Ok(Self(bytes.into()))
    }
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AssetRequest {
    SelfAvatar,
    ConversationAvatar(ConversationId),
    MessageAvatar(MessageId),
    ParticipantAvatar(MemberId),
    PersonAvatar(RecipientId),
    Attachment {
        message: MessageId,
        placeholder: Text<128>,
        large: bool,
    },
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PollDraft {
    pub question: Text<4000>,
    pub options: Page<Text<1000>, 100>,
    pub multiple: bool,
    pub hidden_results: bool,
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
pub struct Poll {
    pub id: u64,
    pub question: Text<4000>,
    pub options: Page<PollOption, 100>,
    pub closed: bool,
    pub hidden_results: bool,
    pub max_votes: u16,
    pub voters: Option<u32>,
    pub details: Page<PollVoter, 100>,
    pub details_complete: bool,
    pub vote: ActionAvailability,
    pub close: ActionAvailability,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PollCommand {
    Capabilities,
    Read(u64),
    Create(PollDraft),
    Vote { poll: u64, options: Page<u16, 100> },
    Close(u64),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContentCommand {
    SelfProfile,
    Asset(AssetRequest),
    OpenCloud {
        message: MessageId,
        placeholder: Text<128>,
    },
    Poll {
        conversation: ConversationId,
        command: PollCommand,
    },
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContentResult {
    SelfProfile {
        name: Option<Text<256>>,
        organization: Option<Text<256>>,
    },
    Image(ImageBytes),
    Emoji {
        text: Text<64>,
        color: Option<Text<16>>,
    },
    /// An installation-relative file route constructed by the protocol owner; no credentials.
    CloudLink(Text<2048>),
    PollCapabilities(ActionAvailability),
    Poll(Box<Poll>),
}
#[derive(Default)]
struct TransferState {
    received: AtomicU64,
    total: AtomicU64,
    cancelled: AtomicBool,
    committing: AtomicBool,
    published: AtomicBool,
}
#[derive(Clone, Default)]
pub struct TransferProgress(Arc<TransferState>);
impl PartialEq for TransferProgress {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for TransferProgress {}
impl std::fmt::Debug for TransferProgress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TransferProgress")
    }
}
impl TransferProgress {
    pub fn update(&self, received: u64, total: Option<u64>) {
        self.0
            .total
            .store(total.map_or(0, |n| n.saturating_add(1)), Ordering::Release);
        self.0.received.fetch_max(received, Ordering::AcqRel);
    }
    pub fn snapshot(&self) -> (u64, Option<u64>, bool) {
        (
            self.0.received.load(Ordering::Acquire),
            self.0.total.load(Ordering::Acquire).checked_sub(1),
            self.0.committing.load(Ordering::Acquire),
        )
    }
    pub fn published(&self) {
        self.0.published.store(true, Ordering::Release);
    }
    pub fn is_published(&self) -> bool {
        self.0.published.load(Ordering::Acquire)
    }
    pub fn committing(&self) {
        self.0.committing.store(true, Ordering::Release);
    }
    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancelled_transfer_keeps_real_progress_and_distinct_publication() {
        let progress = TransferProgress::default();
        let reader = progress.clone();
        progress.update(12, Some(50));
        progress.update(10, Some(50));
        progress.cancel();
        assert_eq!(reader.snapshot(), (12, Some(50), false));
        assert!(reader.is_cancelled());
        assert!(!reader.is_published());
        progress.committing();
        progress.published();
        assert!(reader.is_published());
    }
    #[test]
    fn oversized_encoded_image_never_crosses_worker_boundary() {
        assert!(ImageBytes::new(vec![0; MAX_IMAGE_BYTES + 1]).is_err());
    }
}
