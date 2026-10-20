//! Bounded observations from the device-local release owner. No paths, URLs or secrets.
use super::UpdateChannel;
use crate::bounded::Text;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpdateError {
    Busy,
    Network,
    RateLimited,
    InvalidResponse,
    Unavailable,
    UnsupportedInstall,
    PackageManagerUnavailable,
    PackageManagerFailed,
    Storage,
    TooLarge,
    Integrity,
    UnsafeArchive,
    InstallFailed,
    InstallUncertain,
    ActiveCall,
    Stale,
    Cancelled,
    Stopped,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReleaseInfo {
    pub version: Text<64>,
    pub prerelease: bool,
    pub bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpdateState {
    Idle,
    Checking,
    Current,
    Downloading {
        release: ReleaseInfo,
        received: u64,
        total: u64,
    },
    Verifying(ReleaseInfo),
    /// The selected archive has passed integrity, layout and install-route checks.
    Ready(ReleaseInfo),
    /// Composition sealed new calls and is draining application work.
    Reserved(ReleaseInfo),
    Applying(ReleaseInfo),
    /// Composition may now restart; installation success alone never quits the app.
    Installed(ReleaseInfo),
    Failed(UpdateError),
    Stopped,
}

/// The API check result is kept even if the later download or install fails.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateCheckResult {
    pub at_unix_seconds: u64,
    pub result: Result<Option<ReleaseInfo>, UpdateError>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateSnapshot {
    pub revision: u64,
    /// Correlates the channel/selection with activation, independently of progress revisions.
    pub generation: u64,
    pub current_version: Text<64>,
    pub channel: UpdateChannel,
    pub next_check_unix_seconds: Option<u64>,
    pub last_check: Option<UpdateCheckResult>,
    pub state: UpdateState,
}
