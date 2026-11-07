//! Bounded, account-scoped discovery display; a row never grants membership.
use super::{
    AccountScope, Decision, Page, Text,
    conversation::RemoteState,
    presentation::{ConversationRef, FeatureSupport, RequestId},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Room {
    pub context: ConversationRef,
    pub name: Text<256>,
    pub description: Option<Text<4000>>,
    pub participants: Option<u32>,
    pub join: Decision,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Panel {
    pub account: AccountScope,
    pub request: RequestId,
    pub query: Text<256>,
    pub support: FeatureSupport,
    pub state: RemoteState,
    pub rooms: Page<Room, 100>,
    pub loaded: bool,
    pub joining: Option<ConversationRef>,
    pub uncertain: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    List(Text<256>),
    Join(ConversationRef),
}
