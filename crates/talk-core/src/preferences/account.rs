//! Per-account server preferences. Never encode these in the computer preference file.
use super::RequestId;
use crate::{bounded::Text, ids::AccountScope};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NextcloudFolder(Text<2048>);
impl NextcloudFolder {
    /// A path in the authenticated user's NextCloud files, not an OS path or URL.
    /// The server additionally validates ownership, shares and writability.
    pub fn new(path: &str) -> Result<Self, AccountPreferenceError> {
        if !path.starts_with('/')
            || path.contains('\\')
            || path.chars().any(char::is_control)
            || path != "/"
                && path
                    .split('/')
                    .skip(1)
                    .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(AccountPreferenceError::InvalidFolder);
        }
        Text::new(path)
            .map(Self)
            .map_err(|_| AccountPreferenceError::InvalidFolder)
    }
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountPreferences {
    pub scope: AccountScope,
    /// None means absent/unknown capability, never an enabled toggle.
    pub send_read_receipts: Option<bool>,
    pub share_typing_status: Option<bool>,
    pub attachment_folder: Option<NextcloudFolder>,
    pub attachments_allowed: Option<bool>,
    /// The server may derive conversation subfolders below the selected base folder.
    pub conversation_subfolders: Option<bool>,
    /// Missing capability or status never implies online.
    pub user_status_enabled: Option<bool>,
    pub user_presence: Option<StatusChoice>,
    /// False means this response did not observe status and cannot clear a cached value.
    pub user_status_observed: bool,
}
impl AccountPreferences {
    pub fn show_read_indicators(&self) -> bool {
        self.send_read_receipts == Some(true)
    }
    pub fn send_typing(&self) -> bool {
        self.share_typing_status == Some(true)
    }
    pub fn show_typing_indicators(&self) -> bool {
        self.share_typing_status == Some(true)
    }
}
/// Bounded own-status values; offline is an observation, not a picker choice.
/// Unknown status is represented by None.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatusChoice {
    Online,
    Away,
    DoNotDisturb,
    Invisible,
    /// Server observation only; the picker does not offer this choice.
    Offline,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AccountPreferenceChange {
    SendReadReceipts(bool),
    ShareTypingStatus(bool),
    AttachmentFolder(NextcloudFolder),
    UserPresence(StatusChoice),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountPreferenceResult {
    pub request: RequestId,
    pub values: AccountPreferences,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountPreferenceError {
    Unavailable,
    Stale,
    Busy,
    PermissionDenied,
    AuthenticationRequired,
    Network,
    Uncertain,
    InvalidFolder,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountPreferenceState {
    Unavailable,
    Loading {
        request: RequestId,
    },
    Ready,
    Saving {
        request: RequestId,
    },
    Failed {
        request: RequestId,
        error: AccountPreferenceError,
    },
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountPreferenceSnapshot {
    pub scope: AccountScope,
    pub values: Option<AccountPreferences>,
    pub state: AccountPreferenceState,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AccountPreferenceCommand {
    Load {
        scope: AccountScope,
        request: RequestId,
    },
    Set {
        scope: AccountScope,
        request: RequestId,
        change: AccountPreferenceChange,
    },
}
