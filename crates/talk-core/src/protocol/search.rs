//! Account-scoped unified message search. Identities are owner-issued handles.
use super::{MessageId, conversation::RecipientId};
use crate::{
    bounded::{Page, Text},
    ids::ConversationId,
};
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Query {
    pub term: Text<256>,
    pub conversation: Option<ConversationId>,
    pub person: Option<RecipientId>,
    pub from_self: bool,
    pub since: Option<Text<32>>,
    pub until: Option<Text<32>>,
    pub cursor: Option<Text<1024>>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Hit {
    pub message: MessageId,
    pub title: Text<256>,
    pub excerpt: Text<4096>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Results {
    pub hits: Page<Hit, 50>,
    pub cursor: Option<Text<1024>>,
}
