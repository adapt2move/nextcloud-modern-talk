//! Public, bounded update observations. No installer or account credentials.
use super::UpdateChannel;
use crate::bounded::Text;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpdateError {
    Busy,
    Network,
    RateLimited,
    InvalidResponse,
    Unavailable,
    BrowserUnavailable,
    Stopped,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReleaseInfo {
    pub version: Text<64>,
    pub prerelease: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpdateState {
    Idle,
    Checking,
    Current,
    Available(ReleaseInfo),
    Failed(UpdateError),
    Stopped,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateSnapshot {
    pub revision: u64,
    pub current_version: Text<64>,
    pub channel: UpdateChannel,
    pub state: UpdateState,
}
