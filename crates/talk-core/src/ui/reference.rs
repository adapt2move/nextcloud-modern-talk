//! Resolved link presentation contains no URL or image bytes.
use super::{Page, Text, presentation::ConversationRef};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Preview {
    pub context: ConversationRef,
    pub message: u128,
    pub ordinal: u16,
    pub title: Text<512>,
    pub description: Option<Text<2048>>,
}
pub type Previews = Page<Preview, 64>;
