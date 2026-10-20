//! Local, explicitly requested device checks. One worker owns every native stream.
//! No call grant, server, recording, or serialized media is involved. Composition
//! saves DevicePreferences through SettingsRuntime, then applies the same values
//! here and to prejoin. A future MediaPort consumer must resolve selected_plan()
//! again after its own call authority checks; UnconfiguredMedia stays unconfigured.
//!
//! Start is memory-only. Enumerate explicitly on entering settings (and Refresh).
//! A live preview session refreshes inventory every two seconds. Poll outside render
//! at ~50ms while checks run; no frame polling/timer is needed after exit. StopPreview
//! revokes frame access immediately, but Stopped is published only after native
//! disposal. Keep the runtime alive through shutdown until runtime == Stopped.
//! CPAL and GStreamer's safe APIs contain native/unsafe dependency internals; this
//! module does not make a whole-process memory-safety or physical-device claim.
#![forbid(unsafe_code)]

mod audio;
mod call;
mod camera;
mod frames;
pub use frames::VideoFrame;

use std::sync::{
    Arc, Condvar, Mutex, MutexGuard,
    atomic::{AtomicBool, AtomicU8, AtomicU16, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};
use talk_core::bounded::{Page, Text};
use talk_core::preferences::{RequestId, Revision, RuntimeState, devices::*};

const KINDS: [DeviceKind; 3] = [
    DeviceKind::Microphone,
    DeviceKind::Speaker,
    DeviceKind::Camera,
];
const TICK: Duration = Duration::from_millis(33);
const REFRESH: Duration = Duration::from_secs(2);
static LEASE: AtomicU64 = AtomicU64::new(1);

/// Preferences, never a capture permission. Missing explicit devices are retained
/// and blocked, not silently replaced with a different physical device.
#[derive(Clone, Debug)]
pub struct SelectedDevicePlan {
    pub revision: Revision,
    pub saved: DevicePreferences,
    pub microphone: SelectionState,
    pub speaker: SelectionState,
    pub camera: SelectionState,
}

struct Signal {
    valid: AtomicBool,
    observed: AtomicBool,
    level: AtomicU16,
    error: AtomicU8,
    born: Instant,
    last_activity: AtomicU64,
}
impl Signal {
    fn new() -> Self {
        Self {
            valid: AtomicBool::new(true),
            observed: AtomicBool::new(false),
            level: AtomicU16::new(0),
            error: AtomicU8::new(0),
            born: Instant::now(),
            last_activity: AtomicU64::new(0),
        }
    }
    fn valid(&self) -> bool {
        self.valid.load(Ordering::Acquire)
    }
    fn revoke(&self) {
        self.valid.store(false, Ordering::Release);
    }
    fn touch(&self) {
        self.last_activity.store(
            self.born
                .elapsed()
                .as_millis()
                .min(u128::from(u64::MAX - 1)) as u64
                + 1,
            Ordering::Relaxed,
        );
    }
    fn stalled(&self) -> bool {
        let last = self.last_activity.load(Ordering::Relaxed);
        last != 0
            && self
                .born
                .elapsed()
                .as_millis()
                .saturating_sub(u128::from(last))
                > 3_000
    }
    fn fail(&self, error: DeviceError) {
        if self.valid() {
            self.error.store(error_code(error), Ordering::Release);
        }
    }
}
#[derive(Clone)]
struct Desired {
    request: RequestId,
    session: PreviewSession,
    serial: u64,
    signal: Arc<Signal>,
}
impl Desired {
    fn new(request: RequestId, session: PreviewSession) -> Result<Self, DeviceError> {
        let serial = LEASE
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1))
            .map_err(|_| DeviceError::Unavailable)?;
        Ok(Self {
            request,
            session,
            serial,
            signal: Arc::new(Signal::new()),
        })
    }
}
struct State {
    media: Option<Arc<talk_adapters::media::MediaService>>,
    snapshot: DeviceSnapshot,
    change: u64,
    preferences: DevicePreferences,
    revision: Revision,
    last_request: RequestId,
    last_session: u64,
    enumerate: u8,
    stop_all: bool,
    desired: [Option<Desired>; 3],
    frame: Option<Arc<VideoFrame>>,
}
impl State {
    fn changed(&mut self) {
        self.change = self.change.saturating_add(1);
    }
    fn stop(&mut self, index: usize) {
        if let Some(desired) = self.desired[index].take() {
            desired.signal.revoke();
            *preview_mut(&mut self.snapshot, index) = PreviewState::Stopping {
                request: desired.request,
            };
        }
        if index == 0 {
            self.snapshot.microphone_level = None;
        }
        if index == 1 {
            self.snapshot.speaker_level = None;
        }
        if index == 2 {
            self.frame = None;
        }
    }
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

pub struct DeviceRuntime {
    shared: Arc<Shared>,
}
impl DeviceRuntime {
    pub fn start() -> Result<Self, DeviceError> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                media: None,
                snapshot: empty_snapshot(),
                change: 1,
                preferences: DevicePreferences::default(),
                revision: Revision(0),
                last_request: RequestId(0),
                last_session: 0,
                enumerate: 0,
                stop_all: false,
                desired: [None, None, None],
                frame: None,
            }),
            wake: Condvar::new(),
        });
        let worker = shared.clone();
        std::thread::Builder::new()
            .name("talk-device-checks".into())
            .spawn(move || {
                // Unwinding attempts native disposal, but cannot certify its success.
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&worker))).is_err()
                {
                    let mut state = worker.lock();
                    for index in 0..3 {
                        if let Some(desired) = state.desired[index].take() {
                            desired.signal.revoke();
                            *preview_mut(&mut state.snapshot, index) = PreviewState::Failed {
                                request: desired.request,
                                error: DeviceError::CaptureFailed,
                            };
                        }
                    }
                    state.frame = None;
                    state.snapshot.microphone_level = None;
                    state.snapshot.speaker_level = None;
                    state.snapshot.runtime = RuntimeState::Draining;
                    state.changed();
                }
            })
            .map_err(|_| DeviceError::Unavailable)?;
        Ok(Self { shared })
    }
    pub fn attach_media(&self, media: Arc<talk_adapters::media::MediaService>) {
        let weak = Arc::downgrade(&self.shared);
        media.set_device_wake(move || {
            if let Some(shared) = weak.upgrade() {
                shared.wake.notify_all();
            }
        });
        self.shared.lock().media = Some(media);
        self.shared.wake.notify_all();
    }
    pub fn snapshot(&self) -> DeviceSnapshot {
        self.shared.lock().snapshot.clone()
    }
    pub fn poll(&self, cursor: u64) -> Option<(u64, DeviceSnapshot)> {
        let state = self.shared.lock();
        (cursor != state.change).then(|| (state.change, state.snapshot.clone()))
    }
    /// A lease contains only owned pixels. At most three frames exist; consumers
    /// retaining all three cause incoming frames to be dropped, never queued.
    /// Drop both this lease and the GPUI RenderImage on stop/account/view changes.
    pub fn latest_frame(
        &self,
        surface: VideoSurfaceId,
        after_sequence: u64,
    ) -> Option<Arc<VideoFrame>> {
        let state = self.shared.lock();
        state
            .frame
            .as_ref()
            .filter(|f| {
                f.surface() == surface && f.sequence() > after_sequence && !f.rgba().is_empty()
            })
            .cloned()
    }
    pub fn selected_plan(&self) -> SelectedDevicePlan {
        let state = self.shared.lock();
        SelectedDevicePlan {
            revision: state.revision,
            saved: state.preferences.clone(),
            microphone: state.snapshot.microphone.clone(),
            speaker: state.snapshot.speaker.clone(),
            camera: state.snapshot.camera.clone(),
        }
    }
    pub fn submit(&self, intent: DeviceIntent) -> Result<(), DeviceError> {
        let mut state = self.shared.lock();
        if state.snapshot.runtime != RuntimeState::Running {
            return Err(DeviceError::Unavailable);
        }
        match intent {
            DeviceIntent::StopPreview { session } => {
                if state.snapshot.session != Some(session) {
                    return Err(DeviceError::Stale);
                }
                for index in 0..3 {
                    state.stop(index);
                }
                state.stop_all = true;
            }
            DeviceIntent::StopDevice { session, kind } => {
                if state.snapshot.session != Some(session) {
                    return Err(DeviceError::Stale);
                }
                state.stop(kind_index(kind));
            }
            DeviceIntent::Refresh { request, kind } => {
                admit_request(&mut state, request)?;
                state.enumerate |= 1 << kind_index(kind);
            }
            DeviceIntent::Enumerate { request } => {
                admit_request(&mut state, request)?;
                state.enumerate = 7;
            }
            DeviceIntent::ApplySelection {
                request,
                revision,
                values,
            } => {
                if revision < state.revision {
                    return Err(DeviceError::Stale);
                }
                admit_request(&mut state, request)?;
                if values != state.preferences {
                    for index in 0..3 {
                        if selection(&values, index) != selection(&state.preferences, index) {
                            state.stop(index);
                            reset_permission(&mut state.snapshot, index);
                            // Reopen only after the disposed/Stopped acknowledgement
                            // and a fresh current-page permission/intent in composition.
                        }
                    }
                }
                state.preferences = values;
                state.revision = revision;
                state.enumerate = 7;
            }
            DeviceIntent::StartPreview {
                request,
                session,
                kind,
            } => {
                if kind == DeviceKind::Speaker {
                    return Err(DeviceError::InvalidSelection);
                }
                start_desired(&mut state, request, session, kind_index(kind))?;
            }
            DeviceIntent::TestSpeaker { request, session } => {
                start_desired(&mut state, request, session, 1)?
            }
        }
        state.changed();
        self.shared.wake.notify_one();
        Ok(())
    }
    pub fn request_shutdown(&self) {
        let mut state = self.shared.lock();
        if state.snapshot.runtime == RuntimeState::Running {
            for index in 0..3 {
                state.stop(index);
            }
            state.stop_all = true;
            state.snapshot.runtime = RuntimeState::Draining;
            state.changed();
            self.shared.wake.notify_one();
        }
    }
}
impl Drop for DeviceRuntime {
    fn drop(&mut self) {
        self.request_shutdown();
    }
}
impl DeviceSettingsPort for DeviceRuntime {
    fn snapshot(&self) -> DeviceSnapshot {
        DeviceRuntime::snapshot(self)
    }
    fn submit(&mut self, intent: DeviceIntent) -> Result<(), DeviceError> {
        DeviceRuntime::submit(self, intent)
    }
}

fn start_desired(
    state: &mut State,
    request: RequestId,
    session: PreviewSession,
    index: usize,
) -> Result<(), DeviceError> {
    if state.stop_all
        || matches!(
            preview_mut(&mut state.snapshot, index),
            PreviewState::Starting { .. }
                | PreviewState::Live { .. }
                | PreviewState::Stopping { .. }
        )
    {
        return Err(DeviceError::Busy);
    }
    if state
        .snapshot
        .session
        .is_some_and(|active| active != session)
    {
        return Err(DeviceError::Busy);
    }
    if state.snapshot.session.is_none() && session.0 <= state.last_session {
        return Err(DeviceError::Stale);
    }
    admit_request(state, request)?;
    let desired = Desired::new(request, session)?;
    state.stop(index);
    reset_permission(&mut state.snapshot, index);
    state.last_session = session.0;
    state.snapshot.session = Some(session);
    *preview_mut(&mut state.snapshot, index) = PreviewState::Starting { request };
    state.desired[index] = Some(desired);
    state.enumerate = 7;
    Ok(())
}
fn admit_request(state: &mut State, request: RequestId) -> Result<(), DeviceError> {
    if request <= state.last_request {
        return Err(DeviceError::Stale);
    }
    state.last_request = request;
    Ok(())
}

enum Capture {
    Audio(audio::Capture),
    Camera(camera::Capture),
}
struct Active {
    desired: Desired,
    capture: Capture,
    started: Instant,
}
impl Drop for Active {
    fn drop(&mut self) {
        self.desired.signal.revoke();
    }
}

fn run(shared: &Shared) {
    let mut calls = call::Calls::default();
    let mut active: [Option<Active>; 3] = [None, None, None];
    let mut inventory = Inventory::default();
    let mut frames = frames::FramePool::default();
    let mut refreshed = Instant::now();
    loop {
        let media = shared.lock().media.clone();
        let call_present = media.as_ref().and_then(|m| m.capture()).is_some();
        if call_present {
            let mut state = shared.lock();
            for index in 0..3 {
                state.stop(index);
            }
            if !calls.active() {
                state.enumerate |= 7;
            }
        }
        let draining = shared.lock().snapshot.runtime == RuntimeState::Draining;
        if draining && calls.stop().is_err() {
            release_failed(shared);
            return;
        }
        // Disposal happens before any new open or stop acknowledgement. No native
        // call occurs while holding the presentation/admission mutex.
        for (index, slot) in active.iter_mut().enumerate() {
            if slot
                .as_ref()
                .is_some_and(|item| !item.desired.signal.valid())
            {
                if dispose(slot.take()).is_err() {
                    release_failed(shared);
                    return;
                }
                if index == 2 {
                    frames.clear_released();
                }
            }
            let mut state = shared.lock();
            if slot.is_none()
                && state.desired[index].is_none()
                && matches!(
                    preview_mut(&mut state.snapshot, index),
                    PreviewState::Stopping { .. }
                )
            {
                *preview_mut(&mut state.snapshot, index) = PreviewState::Stopped;
                state.changed();
            }
        }
        let refresh = {
            let mut state = shared.lock();
            if state.stop_all && active.iter().all(Option::is_none) {
                state.snapshot.session = None;
                state.snapshot.microphone_preview = PreviewState::Stopped;
                state.snapshot.camera_preview = PreviewState::Stopped;
                state.snapshot.speaker_test = PreviewState::Stopped;
                state.stop_all = false;
                if state.snapshot.runtime == RuntimeState::Draining {
                    state.snapshot.runtime = RuntimeState::Stopped;
                    state.changed();
                    return;
                }
                state.changed();
            }
            let refresh = if (state.snapshot.session.is_some() || call_present)
                && refreshed.elapsed() >= REFRESH
            {
                7
            } else {
                state.enumerate
            };
            state.enumerate = 0;
            refresh
        };
        if refresh != 0 {
            inventory.refresh(refresh);
            refreshed = Instant::now();
            let mut state = shared.lock();
            let mut next = state.snapshot.clone();
            next.devices = inventory.page();
            next.microphone =
                inventory.resolve(&state.preferences.microphone, DeviceKind::Microphone);
            next.speaker = inventory.resolve(&state.preferences.speaker, DeviceKind::Speaker);
            next.camera = inventory.resolve(&state.preferences.camera, DeviceKind::Camera);
            if next.devices != state.snapshot.devices
                || next.microphone != state.snapshot.microphone
                || next.speaker != state.snapshot.speaker
                || next.camera != state.snapshot.camera
            {
                next.inventory_revision.0 = next.inventory_revision.0.saturating_add(1);
                state.snapshot = next;
                state.changed();
            }
        }
        if let Some(media) = &media {
            let preferences = shared.lock().preferences.clone();
            if calls.tick(media, &inventory, &preferences).is_err() {
                release_failed(shared);
                return;
            }
        }
        for (index, slot) in active.iter_mut().enumerate() {
            let desired = shared.lock().desired[index].clone();
            if slot.is_none()
                && let Some(desired) = desired.filter(|d| d.signal.valid())
            {
                {
                    let mut state = shared.lock();
                    if desired.signal.valid() {
                        *preview_mut(&mut state.snapshot, index) = PreviewState::Starting {
                            request: desired.request,
                        };
                        state.changed();
                    }
                }
                let choice = selection(&shared.lock().preferences, index).clone();
                let resolved = inventory.effective(&choice, KINDS[index]);
                let capture = resolved.and_then(|id| match index {
                    0 => inventory
                        .audio_backend
                        .as_ref()
                        .ok_or(DeviceError::Unavailable)?
                        .microphone(&id, desired.signal.clone())
                        .map(Capture::Audio),
                    1 => inventory
                        .audio_backend
                        .as_ref()
                        .ok_or(DeviceError::Unavailable)?
                        .tone(&id, desired.signal.clone())
                        .map(Capture::Audio),
                    _ => inventory
                        .start_camera(&id, desired.signal.clone())
                        .map(Capture::Camera),
                });
                match capture {
                    Ok(capture) if desired.signal.valid() => {
                        *slot = Some(Active {
                            desired,
                            capture,
                            started: Instant::now(),
                        });
                    }
                    Ok(capture) => {
                        if dispose(Some(Active {
                            desired,
                            capture,
                            started: Instant::now(),
                        }))
                        .is_err()
                        {
                            release_failed(shared);
                            return;
                        }
                    }
                    Err(error) => finish(shared, index, &desired, Some(error)),
                }
            }
            let Some(item) = slot.as_mut() else {
                continue;
            };
            if !item.desired.signal.valid() {
                continue;
            }
            let selected = selection(&shared.lock().preferences, index).clone();
            // On an observed hotplug/default-route change, revoke and require
            // another explicit start. OS mixer routing is outside this API.
            let current_id = match &item.capture {
                Capture::Audio(c) => c.id(),
                Capture::Camera(c) => c.id(),
            };
            if inventory.effective(&selected, KINDS[index]).as_ref() != Ok(current_id) {
                item.desired.signal.fail(DeviceError::Removed);
            }
            if let Capture::Camera(capture) = &mut item.capture {
                match capture.poll(&mut frames, VideoSurfaceId(item.desired.serial)) {
                    Ok(Some(frame)) => {
                        let mut state = shared.lock();
                        if item.desired.signal.valid() {
                            state.frame = Some(frame);
                            state.changed();
                        }
                    }
                    Ok(None) => {}
                    Err(error) => item.desired.signal.fail(error),
                }
            }
            let error =
                decode_error(item.desired.signal.error.load(Ordering::Acquire)).or_else(|| {
                    item.desired
                        .signal
                        .stalled()
                        .then_some(DeviceError::Removed)
                });
            let tone_done = index == 1 && item.started.elapsed() >= audio::TONE_DURATION;
            let starved = (tone_done || item.started.elapsed() >= Duration::from_secs(8))
                && !item.desired.signal.observed.load(Ordering::Acquire);
            if error.is_some() || tone_done || starved {
                let desired = item.desired.clone();
                // Drop first. A failed/timed-out start never becomes a live preview.
                if dispose(slot.take()).is_err() {
                    release_failed(shared);
                    return;
                }
                finish(
                    shared,
                    index,
                    &desired,
                    error.or(starved.then_some(DeviceError::CaptureFailed)),
                );
                if index == 2 {
                    frames.clear_released();
                }
                continue;
            }
            if item.desired.signal.observed.load(Ordering::Acquire) {
                let mut state = shared.lock();
                if !item.desired.signal.valid() {
                    continue;
                }
                let before = state.snapshot.clone();
                *preview_mut(&mut state.snapshot, index) = PreviewState::Live {
                    request: item.desired.request,
                    surface: (index == 2).then_some(VideoSurfaceId(item.desired.serial)),
                };
                if index == 0 {
                    state.snapshot.microphone_permission = PermissionState::Granted;
                    state.snapshot.microphone_level =
                        Some(item.desired.signal.level.load(Ordering::Relaxed).min(1000));
                }
                if index == 1 {
                    state.snapshot.speaker_level =
                        Some(item.desired.signal.level.load(Ordering::Relaxed).min(1000));
                }
                if index == 2 {
                    state.snapshot.camera_permission = PermissionState::Granted;
                }
                if before != state.snapshot {
                    state.changed();
                }
            }
        }
        let state = shared.lock();
        if state.enumerate != 0
            || state.stop_all
            || state
                .desired
                .iter()
                .any(|d| d.as_ref().is_some_and(|d| !d.signal.valid()))
        {
            continue;
        }
        if calls.active()
            || active.iter().any(Option::is_some)
            || state.desired.iter().any(Option::is_some)
        {
            drop(
                shared
                    .wake
                    .wait_timeout(state, TICK)
                    .unwrap_or_else(|e| e.into_inner()),
            );
        } else if state.snapshot.session.is_some() {
            drop(
                shared
                    .wake
                    .wait_timeout(state, REFRESH)
                    .unwrap_or_else(|e| e.into_inner()),
            );
        } else {
            drop(shared.wake.wait(state).unwrap_or_else(|e| e.into_inner()));
        }
    }
}

fn dispose(mut active: Option<Active>) -> Result<(), DeviceError> {
    let result = if let Some(Active {
        capture: Capture::Camera(camera),
        ..
    }) = active.as_mut()
    {
        camera.stop()
    } else {
        Ok(())
    };
    drop(active);
    result
}
fn release_failed(shared: &Shared) {
    let mut state = shared.lock();
    for index in 0..3 {
        if let Some(desired) = state.desired[index].take() {
            desired.signal.revoke();
            *preview_mut(&mut state.snapshot, index) = PreviewState::Failed {
                request: desired.request,
                error: DeviceError::CaptureFailed,
            };
        }
    }
    state.frame = None;
    state.snapshot.microphone_level = None;
    state.snapshot.speaker_level = None;
    // A native failure to reach NULL is not a successful stop acknowledgement.
    // Admission stays closed; composition must report the unresolved shutdown.
    state.snapshot.runtime = RuntimeState::Draining;
    state.changed();
}

fn finish(shared: &Shared, index: usize, desired: &Desired, error: Option<DeviceError>) {
    let mut state = shared.lock();
    if state.snapshot.session != Some(desired.session)
        || state.desired[index]
            .as_ref()
            .is_none_or(|d| d.serial != desired.serial)
    {
        return;
    }
    desired.signal.revoke();
    state.desired[index] = None;
    *preview_mut(&mut state.snapshot, index) = match error {
        Some(error) => PreviewState::Failed {
            request: desired.request,
            error,
        },
        None => PreviewState::Stopped,
    };
    if index == 0 {
        state.snapshot.microphone_level = None;
    }
    if index == 1 {
        state.snapshot.speaker_level = None;
    }
    if index == 2 {
        state.frame = None;
    }
    if error == Some(DeviceError::PermissionDenied) {
        if index == 0 {
            state.snapshot.microphone_permission = PermissionState::Denied;
        }
        if index == 2 {
            state.snapshot.camera_permission = PermissionState::Denied;
        }
    }
    state.changed();
}

#[derive(Default)]
struct Inventory {
    audio: Vec<DeviceInfo>,
    cameras: Vec<camera::Device>,
    audio_error: [Option<DeviceError>; 2],
    camera_error: Option<DeviceError>,
    audio_backend: Option<audio::Backend>,
}
impl Inventory {
    fn refresh(&mut self, kinds: u8) {
        for (index, kind) in [DeviceKind::Microphone, DeviceKind::Speaker]
            .into_iter()
            .enumerate()
        {
            if kinds & (1 << index) == 0 {
                continue;
            }
            let result = self
                .audio_backend
                .get_or_insert_with(audio::Backend::new)
                .enumerate(kind);
            self.audio.retain(|device| device.kind != kind);
            match result {
                Ok(devices) => {
                    self.audio.extend(devices);
                    self.audio_error[index] = None;
                }
                Err(error) => self.audio_error[index] = Some(error),
            }
        }
        if kinds & 4 != 0 {
            match camera::enumerate() {
                Ok(devices) => {
                    self.cameras = devices;
                    self.camera_error = None;
                }
                Err(error) => {
                    self.cameras.clear();
                    self.camera_error = Some(error);
                }
            }
        }
    }
    fn page(&self) -> Page<DeviceInfo, 64> {
        let mut page = Page::empty();
        for device in self
            .audio
            .iter()
            .chain(self.cameras.iter().map(|d| &d.info))
            .take(64)
        {
            let _ = page.try_push(device.clone());
        }
        page
    }
    fn effective(
        &self,
        selection: &DeviceSelection,
        kind: DeviceKind,
    ) -> Result<DeviceId, DeviceError> {
        self.audio
            .iter()
            .chain(self.cameras.iter().map(|d| &d.info))
            .find(|d| {
                d.kind == kind
                    && match selection {
                        DeviceSelection::SystemDefault => d.is_default,
                        DeviceSelection::Specific(id) => d.id == *id,
                    }
            })
            .map(|d| d.id.clone())
            .ok_or_else(|| match selection {
                DeviceSelection::Specific(_) => DeviceError::Removed,
                DeviceSelection::SystemDefault => if kind == DeviceKind::Camera {
                    self.camera_error
                } else {
                    self.audio_error[kind_index(kind)]
                }
                .unwrap_or(DeviceError::Unavailable),
            })
    }
    fn resolve(&self, choice: &DeviceSelection, kind: DeviceKind) -> SelectionState {
        match self.effective(choice, kind) {
            Ok(id) => SelectionState::Selected(id),
            // None explicitly means no effective fallback; keep the saved identity.
            Err(_) if matches!(choice, DeviceSelection::Specific(_)) => {
                let DeviceSelection::Specific(id) = choice else {
                    return SelectionState::Unknown;
                };
                SelectionState::DefaultFallback {
                    missing: id.clone(),
                    effective: None,
                }
            }
            Err(error) => SelectionState::Unavailable(error),
        }
    }
    fn start_camera(
        &self,
        id: &DeviceId,
        signal: Arc<Signal>,
    ) -> Result<camera::Capture, DeviceError> {
        self.cameras
            .iter()
            .find(|d| &d.info.id == id)
            .ok_or(DeviceError::Removed)
            .and_then(|d| camera::Capture::start(d, signal))
    }
}

fn safe_label(value: &str) -> Text<128> {
    let mut text = String::with_capacity(128);
    for ch in value.chars().take(512).filter(|c| {
        !c.is_control() && !matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
    }) {
        if text.len() + ch.len_utf8() > 128 {
            break;
        }
        text.push(ch);
    }
    Text::new(text.trim()).unwrap_or_else(|_| Text::new("").expect("empty bounded text"))
}
fn kind_index(kind: DeviceKind) -> usize {
    match kind {
        DeviceKind::Microphone => 0,
        DeviceKind::Speaker => 1,
        DeviceKind::Camera => 2,
    }
}
fn selection(values: &DevicePreferences, index: usize) -> &DeviceSelection {
    match index {
        0 => &values.microphone,
        1 => &values.speaker,
        _ => &values.camera,
    }
}
fn preview_mut(snapshot: &mut DeviceSnapshot, index: usize) -> &mut PreviewState {
    match index {
        0 => &mut snapshot.microphone_preview,
        1 => &mut snapshot.speaker_test,
        _ => &mut snapshot.camera_preview,
    }
}
fn reset_permission(snapshot: &mut DeviceSnapshot, index: usize) {
    match index {
        0 => snapshot.microphone_permission = PermissionState::Unknown,
        2 => snapshot.camera_permission = PermissionState::Unknown,
        _ => {}
    }
}
fn error_code(error: DeviceError) -> u8 {
    match error {
        DeviceError::PermissionDenied => 1,
        DeviceError::Removed => 2,
        DeviceError::Busy => 3,
        DeviceError::Unavailable => 4,
        _ => 5,
    }
}
fn decode_error(code: u8) -> Option<DeviceError> {
    match code {
        0 => None,
        1 => Some(DeviceError::PermissionDenied),
        2 => Some(DeviceError::Removed),
        3 => Some(DeviceError::Busy),
        4 => Some(DeviceError::Unavailable),
        _ => Some(DeviceError::CaptureFailed),
    }
}
fn empty_snapshot() -> DeviceSnapshot {
    DeviceSnapshot {
        session: None,
        inventory_revision: Revision(0),
        devices: Page::empty(),
        microphone_permission: PermissionState::Unknown,
        camera_permission: PermissionState::Unknown,
        microphone: SelectionState::Unknown,
        speaker: SelectionState::Unknown,
        camera: SelectionState::Unknown,
        microphone_level: None,
        speaker_level: None,
        microphone_preview: PreviewState::Stopped,
        camera_preview: PreviewState::Stopped,
        speaker_test: PreviewState::Stopped,
        runtime: RuntimeState::Running,
    }
}

#[cfg(test)]
mod tests;
