//! Local nonsecret identities. Wire identifiers stay in the protocol adapter.
//! Production allocation requires collision-resistant randomness; no allocator exists yet.

macro_rules! local_id {
    ($($name:ident),+ $(,)?) => {$(
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub struct $name(u128);

        impl $name {
            /// Import an already allocated local identity, never a credential or wire token.
            pub const fn from_local(value: u128) -> Self { Self(value) }
        }
    )+};
}

local_id!(
    ProfileId,
    AccountId,
    HostId,
    ActorId,
    RuntimeGeneration,
    LoginAttemptId,
    VaultGeneration,
    OperationId,
    CallGeneration,
    ClockEpoch,
);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ConversationId {
    pub account: AccountId,
    pub host: HostId,
    pub local: u128,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct AccountScope {
    pub account: AccountId,
    pub runtime: RuntimeGeneration,
}

/// A local attempt/session generation, not an asserted server call-instance ID.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CallScope {
    pub account: AccountScope,
    pub conversation: ConversationId,
    pub call: CallGeneration,
}

impl CallScope {
    pub fn is_consistent(self) -> bool {
        self.account.account == self.conversation.account
    }
}
