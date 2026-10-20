//! Observations and explicit permission actions; no permission DTO grants call/capture authority.
use super::{RequestId, RuntimeState};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionKind {
    Microphone,
    Camera,
    ScreenSharing,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionStatus {
    Unknown,
    NotDetermined,
    Granted,
    Denied,
    Restricted,
    PerSession,
    /// No global permission API on this native backend; opening a device may still fail.
    NotRequired,
    Unsupported,
    Unavailable,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionGuidance {
    CheckFirst,
    SystemSettings,
    RestartAfterGrant,
    SelectForEachShare,
    DeviceAccessCheckedAtOpen,
    HelperUnavailable,
    BackendUnavailable,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PermissionObservation {
    pub kind: PermissionKind,
    pub status: PermissionStatus,
    pub guidance: PermissionGuidance,
    pub can_request: bool,
    pub can_open_settings: bool,
}
impl PermissionObservation {
    pub fn unknown(kind: PermissionKind) -> Self {
        Self {
            kind,
            status: PermissionStatus::Unknown,
            guidance: PermissionGuidance::CheckFirst,
            can_request: false,
            can_open_settings: false,
        }
    }
    /// Only valid for an explicitly entered Devices page with a current preview lease.
    /// Screen observations never authorize capture, even when the OS reports Granted.
    pub fn allows_device_preview(self) -> bool {
        self.kind != PermissionKind::ScreenSharing
            && matches!(
                self.status,
                PermissionStatus::Granted | PermissionStatus::NotRequired
            )
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionAction {
    Check,
    Request,
    OpenSystemSettings,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PermissionIntent {
    pub generation: u64,
    pub request: RequestId,
    pub kind: PermissionKind,
    pub action: PermissionAction,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionError {
    Stale,
    Busy,
    Closed,
    Unsupported,
    Unavailable,
    Failed,
    TimedOut,
    Cancelled,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionOperation {
    Idle,
    Pending {
        request: RequestId,
        kind: PermissionKind,
        action: PermissionAction,
    },
    Finished {
        request: RequestId,
        kind: PermissionKind,
        action: PermissionAction,
    },
    Failed {
        request: RequestId,
        kind: PermissionKind,
        action: PermissionAction,
        error: PermissionError,
    },
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionSnapshot {
    pub generation: u64,
    pub runtime: RuntimeState,
    pub microphone: PermissionObservation,
    pub camera: PermissionObservation,
    pub screen_sharing: PermissionObservation,
    pub operation: PermissionOperation,
}
