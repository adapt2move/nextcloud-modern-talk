//! Safe protocol failure classification; never includes URLs, bodies or credentials.
use crate::scope::BindingId;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    Unconfigured,
    Unsupported,
    Cancelled,
    Deadline,
    Busy,
    PermissionDenied,
    ReauthenticationRequired,
    Malformed,
    Uncertain,
    WrongScope,
    Transport,
    RedirectRejected,
    TooLarge,
    NotFound,
    Conflict,
    RateLimited,
    ServerUnavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Retry {
    Never,
    UserAction,
    AfterReconciliation,
    AfterDelay { millis: u32 },
}

/// Deliberately excludes URLs, wire bodies, provider messages and tokens.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PortError {
    pub kind: ErrorKind,
    pub account: Option<BindingId>,
    pub retry: Retry,
}

impl PortError {
    pub const fn unconfigured() -> Self {
        Self {
            kind: ErrorKind::Unconfigured,
            account: None,
            retry: Retry::Never,
        }
    }
}
