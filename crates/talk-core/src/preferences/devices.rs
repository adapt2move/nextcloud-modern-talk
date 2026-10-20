//! Device UI contract. Implemented by the application-owned physical device service,
//! never by a settings view. This interface does not claim a media provider is present.
use super::{RequestId, Revision, RuntimeState};
use crate::bounded::{Page, Text};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceId(Text<256>);
impl DeviceId {
    pub fn new(value: &str) -> Result<Self, DeviceError> {
        if value.is_empty() || value.chars().any(char::is_control) {
            return Err(DeviceError::InvalidSelection);
        }
        Text::new(value)
            .map(Self)
            .map_err(|_| DeviceError::InvalidSelection)
    }
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum DeviceSelection {
    #[default]
    SystemDefault,
    Specific(DeviceId),
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DevicePreferences {
    pub microphone: DeviceSelection,
    pub speaker: DeviceSelection,
    pub camera: DeviceSelection,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceKind {
    Microphone,
    Speaker,
    Camera,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceError {
    Unavailable,
    PermissionDenied,
    Removed,
    Busy,
    Stale,
    InvalidSelection,
    CaptureFailed,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionState {
    Unknown,
    PromptRequired,
    Granted,
    Denied,
    Unavailable,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceInfo {
    pub id: DeviceId,
    pub label: Text<128>,
    pub kind: DeviceKind,
    pub is_default: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PreviewSession(pub u64);
/// Opaque surface lease; frame bytes and provider handles are never in a snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VideoSurfaceId(pub u64);
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SelectionState {
    Unknown,
    Selected(DeviceId),
    /// Saved specific choice is retained, but the current effective device is explicit.
    DefaultFallback {
        missing: DeviceId,
        effective: Option<DeviceId>,
    },
    Unavailable(DeviceError),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreviewState {
    Stopped,
    Starting {
        request: RequestId,
    },
    Stopping {
        request: RequestId,
    },
    Live {
        request: RequestId,
        surface: Option<VideoSurfaceId>,
    },
    Failed {
        request: RequestId,
        error: DeviceError,
    },
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceSnapshot {
    pub runtime: RuntimeState,
    pub session: Option<PreviewSession>,
    pub inventory_revision: Revision,
    pub devices: Page<DeviceInfo, 64>,
    pub microphone_permission: PermissionState,
    pub camera_permission: PermissionState,
    pub microphone: SelectionState,
    pub speaker: SelectionState,
    pub camera: SelectionState,
    /// 0..=1000; None until an actual microphone sample has arrived.
    pub microphone_level: Option<u16>,
    /// Actual output samples submitted, not proof that a person heard them.
    pub speaker_level: Option<u16>,
    pub microphone_preview: PreviewState,
    pub camera_preview: PreviewState,
    pub speaker_test: PreviewState,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeviceIntent {
    Refresh {
        request: RequestId,
        kind: DeviceKind,
    },
    Enumerate {
        request: RequestId,
    },
    /// Reconcile before prejoin/call admission as well as settings preview; this
    /// command alone must never begin capture. Missing devices use explicit fallback.
    ApplySelection {
        request: RequestId,
        revision: Revision,
        values: DevicePreferences,
    },
    StartPreview {
        request: RequestId,
        session: PreviewSession,
        kind: DeviceKind,
    },
    TestSpeaker {
        request: RequestId,
        session: PreviewSession,
    },
    /// Stop one deliberately running check while retaining the other checks.
    StopDevice {
        session: PreviewSession,
        kind: DeviceKind,
    },
    /// Priority stop, never queued behind ordinary commands. Exit/drop/device loss
    /// invalidate the lease before stopping capture and releasing native resources.
    StopPreview {
        session: PreviewSession,
    },
}
pub trait DeviceSettingsPort: Send {
    fn snapshot(&self) -> DeviceSnapshot;
    /// Bounded admission: one preview session, at most one pending device selection.
    fn submit(&mut self, intent: DeviceIntent) -> Result<(), DeviceError>;
}
