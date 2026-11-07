//! Account-scoped invitation handles. Wire IDs and remote credentials stay in the adapter.
use crate::{
    bounded::{Page, Text},
    ids::AccountScope,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Invitation {
    pub id: u128,
    pub room: Text<256>,
    pub inviter: Text<256>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Invitations {
    pub scope: AccountScope,
    pub supported: bool,
    pub items: Page<Invitation, 100>,
}
