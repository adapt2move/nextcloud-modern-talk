//! Equality-only network owner/lease correlation, never application policy.
macro_rules! local_id {
    ($($name:ident),+ $(,)?) => {$(
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub struct $name(u128);

        impl $name {
            /// Import an already allocated local identity, never a credential or wire token.
            pub const fn from_key(value: u128) -> Self { Self(value) }
            /// Nonsecret stable local representation for the identity ledger only.
            pub const fn key(self) -> u128 { self.0 }
        }
    )+};
}

local_id!(BindingId, LeaseId, OriginId, ClockId, NegotiationId);
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SessionScope {
    pub account: BindingId,
    pub runtime: LeaseId,
}
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RoomKey {
    pub account: BindingId,
    pub host: OriginId,
    pub local: u128,
}
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CallKey {
    pub account: SessionScope,
    pub conversation: RoomKey,
    pub call: NegotiationId,
}
impl CallKey {
    pub fn is_consistent(self) -> bool {
        self.account.account == self.conversation.account
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Moment {
    pub epoch: ClockId,
    pub millis: u64,
}

/// A narrow bridge to an owner's irreversible cancellation signal. It cannot
/// inspect credentials or perform network work. The application adapter implements
/// this for its existing cancellation handle, without exposing a mutable atomic.
pub trait CancellationSignal: Send + Sync {
    fn cancel(&self);
    fn is_cancelled(&self) -> bool;
}
#[derive(Default)]
struct AtomicCancellation(std::sync::atomic::AtomicBool);
impl CancellationSignal for AtomicCancellation {
    fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }
    fn is_cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Acquire)
    }
}
/// Cloning shares closure; it never grants a fresh lease or reopens admission.
#[derive(Clone)]
pub struct Cancellation(std::sync::Arc<dyn CancellationSignal>);
impl Default for Cancellation {
    fn default() -> Self {
        Self::from_signal(AtomicCancellation::default())
    }
}
impl Cancellation {
    pub fn from_signal(signal: impl CancellationSignal + 'static) -> Self {
        Self(std::sync::Arc::new(signal))
    }
    pub fn cancel(&self) {
        self.0.cancel();
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }
}
pub struct OperationContext {
    pub scope: SessionScope,
    pub deadline: Moment,
    pub cancellation: Cancellation,
}
