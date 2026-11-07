//! Bounded account search projection. Source identities remain owner-issued handles.
use super::{
    AccountScope, Page, Text,
    conversation::{AccountPersonRef, RemoteState},
    presentation::{ConversationRef, RequestId},
};
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Filters {
    pub conversation: Option<ConversationRef>,
    pub person: Option<AccountPersonRef>,
    pub since: Option<Text<32>>,
    pub until: Option<Text<32>>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Hit {
    pub context: ConversationRef,
    pub message: u128,
    pub title: Text<256>,
    pub excerpt: Text<4096>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Panel {
    pub account: AccountScope,
    pub request: RequestId,
    pub term: Text<256>,
    pub filters: Filters,
    pub state: RemoteState,
    pub hits: Page<Hit, 100>,
    pub loaded: bool,
    pub more: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    Search { term: Text<256>, filters: Filters },
    More,
}
