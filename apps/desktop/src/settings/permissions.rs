//! Permission checks never allocate capture. One operation, no queued blanket requests.
//! Explicit requests may change OS authorization; cancellation never asserts rollback.
use super::native_process;
use std::{
    process::Command,
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use talk_core::preferences::{RequestId, RuntimeState, permissions::*};

#[cfg(target_os = "linux")]
mod screen;

static GENERATION: AtomicU64 = AtomicU64::new(1);
struct State {
    snapshot: PermissionSnapshot,
    pending: Option<PermissionIntent>,
    last_request: RequestId,
    change: u64,
    screen_requested: bool,
}
struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}
impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}
pub struct PermissionRuntime {
    shared: Arc<Shared>,
}
impl PermissionRuntime {
    /// Memory-only initialization. Composition submits a Check for each visible row.
    pub fn start() -> Result<Self, PermissionError> {
        let generation = GENERATION
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| PermissionError::Unavailable)?;
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                snapshot: PermissionSnapshot {
                    generation,
                    runtime: RuntimeState::Running,
                    microphone: PermissionObservation::unknown(PermissionKind::Microphone),
                    camera: PermissionObservation::unknown(PermissionKind::Camera),
                    screen_sharing: PermissionObservation::unknown(PermissionKind::ScreenSharing),
                    operation: PermissionOperation::Idle,
                },
                pending: None,
                last_request: RequestId(0),
                change: 1,
                screen_requested: false,
            }),
            wake: Condvar::new(),
        });
        let worker = shared.clone();
        std::thread::Builder::new()
            .name("talk-permissions".into())
            .spawn(move || run(worker))
            .map_err(|_| PermissionError::Unavailable)?;
        Ok(Self { shared })
    }
    pub fn snapshot(&self) -> PermissionSnapshot {
        self.shared.lock().snapshot.clone()
    }
    pub fn poll(&self, cursor: u64) -> Option<(u64, PermissionSnapshot)> {
        let state = self.shared.lock();
        (state.change != cursor).then(|| (state.change, state.snapshot.clone()))
    }
    pub fn submit(&self, intent: PermissionIntent) -> Result<(), PermissionError> {
        let mut state = self.shared.lock();
        if state.snapshot.runtime != RuntimeState::Running {
            return Err(PermissionError::Closed);
        }
        if intent.generation != state.snapshot.generation || intent.request <= state.last_request {
            return Err(PermissionError::Stale);
        }
        if matches!(
            state.snapshot.operation,
            PermissionOperation::Pending { .. }
        ) {
            return Err(PermissionError::Busy);
        }
        let observation = observation(&mut state.snapshot, intent.kind);
        if (intent.action == PermissionAction::Request && !observation.can_request)
            || (intent.action == PermissionAction::OpenSystemSettings
                && !observation.can_open_settings)
        {
            return Err(PermissionError::Unsupported);
        }
        if intent.kind == PermissionKind::ScreenSharing
            && intent.action == PermissionAction::Request
        {
            if state.screen_requested {
                return Err(PermissionError::Unsupported);
            }
            state.screen_requested = true;
        }
        state.last_request = intent.request;
        state.pending = Some(intent);
        state.snapshot.operation = PermissionOperation::Pending {
            request: intent.request,
            kind: intent.kind,
            action: intent.action,
        };
        state.change = state.change.saturating_add(1);
        self.shared.wake.notify_one();
        Ok(())
    }
    pub fn request_shutdown(&self) {
        let mut state = self.shared.lock();
        if state.snapshot.runtime == RuntimeState::Running {
            state.snapshot.runtime = RuntimeState::Draining;
            state.pending = None;
            state.change = state.change.saturating_add(1);
            self.shared.wake.notify_one();
        }
    }
}
impl Drop for PermissionRuntime {
    fn drop(&mut self) {
        self.request_shutdown();
    }
}
fn observation(
    snapshot: &mut PermissionSnapshot,
    kind: PermissionKind,
) -> &mut PermissionObservation {
    match kind {
        PermissionKind::Microphone => &mut snapshot.microphone,
        PermissionKind::Camera => &mut snapshot.camera,
        PermissionKind::ScreenSharing => &mut snapshot.screen_sharing,
    }
}
fn run(shared: Arc<Shared>) {
    loop {
        let intent = {
            let mut state = shared.lock();
            while state.pending.is_none() && state.snapshot.runtime == RuntimeState::Running {
                state = shared.wake.wait(state).unwrap_or_else(|e| e.into_inner());
            }
            if state.snapshot.runtime != RuntimeState::Running {
                state.snapshot.runtime = RuntimeState::Stopped;
                state.change = state.change.saturating_add(1);
                return;
            }
            state.pending.take()
        };
        let Some(intent) = intent else {
            continue;
        };
        let result = platform(intent, || {
            shared.lock().snapshot.runtime != RuntimeState::Running
        });
        let mut state = shared.lock();
        if state.snapshot.runtime != RuntimeState::Running {
            continue;
        }
        match result {
            Ok(value) => {
                if let Some(mut value) = value {
                    if value.kind == PermissionKind::ScreenSharing {
                        let previous = state.snapshot.screen_sharing;
                        value.can_request &= !state.screen_requested;
                        if value.status == PermissionStatus::Unknown
                            && previous.guidance == PermissionGuidance::RestartRequired
                        {
                            value.guidance = PermissionGuidance::RestartRequired;
                        }
                    }
                    *observation(&mut state.snapshot, intent.kind) = value;
                }
                state.snapshot.operation = PermissionOperation::Finished {
                    request: intent.request,
                    kind: intent.kind,
                    action: intent.action,
                };
            }
            Err(error) => {
                state.snapshot.operation = PermissionOperation::Failed {
                    request: intent.request,
                    kind: intent.kind,
                    action: intent.action,
                    error,
                }
            }
        }
        state.change = state.change.saturating_add(1);
    }
}
fn platform(
    intent: PermissionIntent,
    cancelled: impl Fn() -> bool,
) -> Result<Option<PermissionObservation>, PermissionError> {
    if cancelled() {
        return Err(PermissionError::Cancelled);
    }
    let kind = intent.kind;
    if intent.action == PermissionAction::OpenSystemSettings {
        let mut command = if cfg!(target_os = "macos") {
            let mut command = Command::new("/usr/bin/open");
            command.args(["-b", "com.apple.systempreferences"]);
            command
        } else if cfg!(windows) {
            let uri = match kind {
                PermissionKind::Microphone => "ms-settings:privacy-microphone",
                PermissionKind::Camera => "ms-settings:privacy-webcam",
                PermissionKind::ScreenSharing => return Err(PermissionError::Unsupported),
            };
            let mut command = Command::new("cmd.exe");
            command.args(["/C", "start", "", uri]);
            command
        } else {
            return Err(PermissionError::Unsupported);
        };
        native_process::launch(&mut command, Duration::from_secs(3), cancelled)?;
        return Ok(None); // Opening Settings is not a grant. A fresh Check is required.
    }
    #[cfg(target_os = "macos")]
    {
        let native_kind = match kind {
            PermissionKind::Microphone => gpui::MacPermissionKind::Microphone,
            PermissionKind::Camera => gpui::MacPermissionKind::Camera,
            PermissionKind::ScreenSharing => gpui::MacPermissionKind::Screen,
        };
        let status = super::macos_access::observe(
            native_kind,
            intent.action == PermissionAction::Request,
            cancelled,
        )?;
        let mut observation = mac_observation(kind, native_status(status));
        if status == gpui::MacPermissionStatus::RestartRequired {
            observation.guidance = PermissionGuidance::RestartRequired;
            observation.can_request = false;
        }
        Ok(Some(observation))
    }
    #[cfg(not(target_os = "macos"))]
    {
        if intent.action == PermissionAction::Request {
            return Err(PermissionError::Unsupported);
        }
        #[cfg(target_os = "linux")]
        {
            if kind == PermissionKind::ScreenSharing {
                return screen::check(cancelled).map(Some);
            }
            let sandboxed = std::path::Path::new("/.flatpak-info").exists()
                || std::env::var_os("SNAP").is_some();
            Ok(Some(PermissionObservation {
                kind,
                status: if sandboxed {
                    PermissionStatus::Unknown
                } else {
                    PermissionStatus::NotRequired
                },
                guidance: PermissionGuidance::DeviceAccessCheckedAtOpen,
                can_request: false,
                can_open_settings: false,
            }))
        }
        // Windows picker consent is per share, not a durable setting. Without the
        // admitted native capture adapter no successful IsSupported observation exists.
        #[cfg(not(target_os = "linux"))]
        Ok(Some(PermissionObservation {
            kind,
            status: PermissionStatus::Unknown,
            guidance: if kind == PermissionKind::ScreenSharing {
                PermissionGuidance::SelectForEachShare
            } else {
                PermissionGuidance::SystemSettings
            },
            can_request: false,
            can_open_settings: cfg!(windows) && kind != PermissionKind::ScreenSharing,
        }))
    }
}

#[cfg(target_os = "macos")]
fn native_status(status: gpui::MacPermissionStatus) -> PermissionStatus {
    match status {
        gpui::MacPermissionStatus::NotDetermined => PermissionStatus::NotDetermined,
        gpui::MacPermissionStatus::Granted => PermissionStatus::Granted,
        gpui::MacPermissionStatus::Denied => PermissionStatus::Denied,
        gpui::MacPermissionStatus::Restricted => PermissionStatus::Restricted,
        gpui::MacPermissionStatus::Unknown | gpui::MacPermissionStatus::RestartRequired => {
            PermissionStatus::Unknown
        }
    }
}

#[cfg(any(target_os = "macos", test))]
fn mac_observation(kind: PermissionKind, status: PermissionStatus) -> PermissionObservation {
    PermissionObservation {
        kind,
        status,
        guidance: if kind == PermissionKind::ScreenSharing {
            PermissionGuidance::RestartAfterGrant
        } else {
            PermissionGuidance::SystemSettings
        },
        can_request: status == PermissionStatus::NotDetermined
            || (kind == PermissionKind::ScreenSharing && status == PermissionStatus::Unknown),
        can_open_settings: true,
    }
}

#[cfg(test)]
mod tests;
