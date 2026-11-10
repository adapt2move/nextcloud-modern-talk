//! Composition-owned bridge between an admitted call and selected native devices.
//! Every callback rechecks the irreversible lifetime and current policy deadline.
//! This module never chooses or opens a device and never manufactures call scope.
use super::engine::VideoFrame;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    },
    time::Instant,
};
use talk_core::{
    calls::{Grant, MediaWishes, Snapshot},
    ids::{AccountScope, CallScope},
    lifecycle::Cancellation,
    policy::Moment,
};

struct Wake(Mutex<Option<Box<dyn Fn() + Send + Sync>>>);
impl Wake {
    fn notify(&self) {
        if let Ok(wake) = self.0.try_lock()
            && let Some(wake) = wake.as_ref()
        {
            wake();
        }
    }
}
pub struct MediaService {
    wake: Arc<Wake>,
    origin: Instant,
    slot: Mutex<Option<Weak<Session>>>,
}
pub(crate) struct Session {
    pub(crate) scope: CallScope,
    wake: Arc<Wake>,
    origin: Instant,
    account: Cancellation,
    alive: AtomicBool,
    publish: AtomicU8,
    blocked: AtomicU8,
    deadline: AtomicU64,
    pub(crate) snapshot: Mutex<Snapshot>,
    audio: Mutex<VecDeque<[i16; 480]>>,
    video: Mutex<Option<VideoFrame>>,
    output: Mutex<VecDeque<[i16; 480]>>,
    received: Mutex<Vec<VideoImage>>,
    microphone_seen: AtomicBool,
    camera_seen: AtomicBool,
    control_degraded: AtomicBool,
    screen_generation: AtomicU64,
    screen_deadline: AtomicU64,
    screen_pending: AtomicBool,
    screen: Mutex<Option<(u64, VideoFrame)>>,
}
/// At most one frame per admitted stream (16 maximum): cameras decode at 640x360,
/// the sole admitted remote screen at up to 1920x1080. The sole consumer
/// takes ownership; identity is retained through the renderer, never inferred
/// from whichever peer happened to decode most recently.
/// Reserved participant id of the local sender within a call generation.
pub const LOCAL_PARTICIPANT: u128 = 0;
pub struct VideoImage {
    pub participant: u128,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
#[derive(Clone)]
pub struct CaptureHandle {
    session: Weak<Session>,
}
impl MediaService {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            origin: Instant::now(),
            slot: Mutex::new(None),
            wake: Arc::new(Wake(Mutex::new(None))),
        })
    }
    pub fn set_device_wake(&self, wake: impl Fn() + Send + Sync + 'static) {
        if let Ok(mut slot) = self.wake.0.lock() {
            *slot = Some(Box::new(wake));
        }
    }
    pub(crate) fn admit(
        &self,
        snapshot: Snapshot,
        account: Cancellation,
    ) -> Result<Arc<Session>, ()> {
        let mut slot = self.slot.lock().map_err(|_| ())?;
        if slot
            .as_ref()
            .and_then(Weak::upgrade)
            .is_some_and(|s| s.live())
        {
            return Err(());
        }
        let s = Arc::new(Session {
            scope: snapshot.call,
            wake: self.wake.clone(),
            origin: self.origin,
            account,
            alive: AtomicBool::new(true),
            publish: AtomicU8::new(0),
            blocked: AtomicU8::new(0),
            deadline: AtomicU64::new(0),
            snapshot: Mutex::new(snapshot),
            audio: Mutex::new(VecDeque::with_capacity(4)),
            video: Mutex::new(None),
            output: Mutex::new(VecDeque::with_capacity(4)),
            received: Mutex::new(Vec::with_capacity(16)),
            microphone_seen: AtomicBool::new(false),
            camera_seen: AtomicBool::new(false),
            control_degraded: AtomicBool::new(false),
            screen_generation: AtomicU64::new(1),
            screen_deadline: AtomicU64::new(0),
            screen_pending: AtomicBool::new(false),
            screen: Mutex::new(None),
        });
        *slot = Some(Arc::downgrade(&s));
        self.wake.notify();
        Ok(s)
    }
    pub fn capture(&self) -> Option<CaptureHandle> {
        let s = self.slot.lock().ok()?.as_ref()?.upgrade()?;
        s.live().then_some(CaptureHandle {
            session: Arc::downgrade(&s),
        })
    }
    pub fn close_call(&self, scope: CallScope) {
        if let Some(s) = self
            .slot
            .lock()
            .ok()
            .and_then(|s| s.as_ref().and_then(Weak::upgrade))
            && s.scope == scope
        {
            s.close();
        }
    }
    pub fn close_account(&self, scope: AccountScope) {
        if let Some(s) = self
            .slot
            .lock()
            .ok()
            .and_then(|s| s.as_ref().and_then(Weak::upgrade))
            && s.scope.account == scope
        {
            s.close();
        }
    }
    pub fn snapshot(&self) -> Option<Snapshot> {
        let s = self.slot.lock().ok()?.as_ref()?.upgrade()?;
        let result = s.snapshot.lock().ok()?.clone();
        Some(result)
    }
}
impl Session {
    pub(crate) fn set_control_degraded(&self, degraded: bool) {
        self.control_degraded.store(degraded, Ordering::Release);
    }
    pub(crate) fn control_degraded(&self) -> bool {
        self.control_degraded.load(Ordering::Acquire)
    }
    pub(crate) fn live(&self) -> bool {
        self.alive.load(Ordering::Acquire) && !self.account.is_cancelled()
    }
    fn valid(&self) -> bool {
        self.live()
            && self.origin.elapsed().as_millis() < u128::from(self.deadline.load(Ordering::Acquire))
    }
    pub(crate) fn grant(&self, grant: Grant, now: Moment) {
        if !self.live() || !grant.is_fresh(self.scope, now) {
            self.suspend();
            return;
        }
        let remaining = grant.expires().millis - now.millis;
        let elapsed = u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX);
        let deadline = elapsed.saturating_add(remaining);
        self.deadline.store(deadline, Ordering::Release);
        let w = grant.publication();
        let requested =
            u8::from(w.microphone) | (u8::from(w.camera) * 2) | (u8::from(w.screen) * 4);
        self.blocked.fetch_and(requested, Ordering::AcqRel);
        let publication = requested & !self.blocked.load(Ordering::Acquire);
        if publication & 4 != 0 {
            self.screen_deadline.store(deadline, Ordering::Release);
        } else if !w.screen && self.screen_pending.load(Ordering::Acquire) {
            self.stop_screen();
        }
        if self.publish.swap(publication, Ordering::AcqRel) != publication {
            self.wake.notify();
        }
        if !w.microphone {
            if let Ok(mut a) = self.audio.lock() {
                a.clear();
            }
            self.microphone_seen.store(false, Ordering::Release);
        }
        if !w.camera {
            if let Ok(mut v) = self.video.lock() {
                *v = None;
            }
            self.camera_seen.store(false, Ordering::Release);
        }
    }
    pub(crate) fn narrow(&self, wishes: MediaWishes) {
        let keep = u8::from(wishes.microphone)
            | (u8::from(wishes.camera) * 2)
            | (u8::from(wishes.screen) * 4);
        self.publish.fetch_and(keep, Ordering::AcqRel);
        if !wishes.microphone {
            if let Ok(mut q) = self.audio.lock() {
                q.clear();
            }
            self.microphone_seen.store(false, Ordering::Release);
        }
        if !wishes.camera {
            if let Ok(mut q) = self.video.lock() {
                *q = None;
            }
            self.camera_seen.store(false, Ordering::Release);
        }
        if !wishes.screen {
            self.stop_screen();
        }
        self.wake.notify();
    }
    pub(crate) fn suspend(&self) {
        self.publish.store(0, Ordering::Release);
        self.deadline.store(0, Ordering::Release);
        if let Ok(mut v) = self.video.lock() {
            *v = None;
        }
        if let Ok(mut a) = self.audio.lock() {
            a.clear();
        }
        if let Ok(mut a) = self.output.lock() {
            a.clear();
        }
        if let Ok(mut v) = self.received.lock() {
            v.clear();
        }
        self.microphone_seen.store(false, Ordering::Release);
        self.camera_seen.store(false, Ordering::Release);
    }
    pub(crate) fn close(&self) {
        self.alive.store(false, Ordering::Release);
        self.stop_screen();
        self.suspend();
        self.wake.notify();
    }
    pub(crate) fn publication(&self) -> MediaWishes {
        if !self.valid() {
            return MediaWishes::default();
        }
        let b = self.publish.load(Ordering::Acquire);
        MediaWishes {
            microphone: b & 1 != 0,
            camera: b & 2 != 0,
            screen: b & 4 != 0,
        }
    }
    pub(crate) fn failed(&self) -> (bool, bool) {
        let b = self.blocked.load(Ordering::Acquire);
        (b & 1 != 0, b & 2 != 0)
    }
    pub(crate) fn actual(&self) -> (bool, bool) {
        (
            self.valid() && self.microphone_seen.load(Ordering::Acquire),
            self.valid() && self.camera_seen.load(Ordering::Acquire),
        )
    }
    pub(crate) fn screen_capture_active(&self) -> bool {
        self.live()
            && self.blocked.load(Ordering::Acquire) & 4 == 0
            && self.origin.elapsed().as_millis()
                < u128::from(self.screen_deadline.load(Ordering::Acquire))
    }
    pub(crate) fn screen_generation_for_owner(&self) -> Option<u64> {
        self.publication()
            .screen
            .then(|| self.screen_generation.load(Ordering::Acquire))
    }
    pub(crate) fn screen_flowing(&self, generation: u64) {
        if self.screen_generation_for_owner() == Some(generation)
            && let Ok(mut s) = self.snapshot.lock()
        {
            if self.screen_generation_for_owner() != Some(generation) {
                return;
            }
            s.sharing = talk_core::calls::SharingState::Sharing;
            s.screen = talk_core::calls::DeviceState::Active;
        }
    }
    pub(crate) fn fail_screen(&self, reason: talk_core::calls::ShareFailure) {
        self.stop_screen();
        if let Ok(mut s) = self.snapshot.lock() {
            s.sharing = talk_core::calls::SharingState::Failed(reason);
            s.screen = talk_core::calls::DeviceState::Unavailable;
        }
    }
    pub(crate) fn start_screen(&self, expected: u64) -> bool {
        let Ok(mut snapshot) = self.snapshot.lock() else {
            return false;
        };
        if !self.live() || self.screen_epoch() != expected {
            return false;
        }
        self.retire_screen();
        self.blocked.fetch_and(!4, Ordering::AcqRel);
        self.screen_pending.store(true, Ordering::Release);
        snapshot.sharing = talk_core::calls::SharingState::Choosing;
        snapshot.screen = talk_core::calls::DeviceState::Requested;
        self.wake.notify();
        true
    }
    pub(crate) fn stop_screen(&self) {
        // Share starts, synchronous stops and capture failures use the same
        // barrier. An older queued start cannot reopen a stopped generation,
        // including when the Stop command itself encounters a full lane.
        let snapshot = self.snapshot.lock();
        self.retire_screen();
        if let Ok(mut s) = snapshot {
            s.sharing = talk_core::calls::SharingState::Idle;
            s.screen = talk_core::calls::DeviceState::Off;
        }
        self.wake.notify();
    }
    fn retire_screen(&self) {
        self.blocked.fetch_or(4, Ordering::AcqRel);
        self.screen_deadline.store(0, Ordering::Release);
        self.screen_pending.store(false, Ordering::Release);
        self.screen_generation.fetch_add(1, Ordering::AcqRel);
        self.publish.fetch_and(!4, Ordering::AcqRel);
        if let Ok(mut q) = self.screen.lock() {
            *q = None;
        }
    }
    pub(crate) fn screen_epoch(&self) -> u64 {
        self.screen_generation.load(Ordering::Acquire)
    }
    pub(crate) fn take_screen(&self) -> Option<(u64, VideoFrame)> {
        if !self.publication().screen {
            return None;
        }
        let (generation, frame) = self.screen.try_lock().ok()?.take()?;
        (generation == self.screen_epoch() && self.publication().screen)
            .then_some((generation, frame))
    }
    pub(crate) fn take_audio(&self) -> Option<[i16; 480]> {
        if !self.publication().microphone {
            return None;
        }
        self.audio.try_lock().ok()?.pop_front()
    }
    pub(crate) fn take_video(&self) -> Option<VideoFrame> {
        if !self.publication().camera {
            return None;
        }
        self.video.try_lock().ok()?.take()
    }
    pub(crate) fn play_audio(&self, samples: &[i16]) {
        if !self.valid() || samples.len() != 480 {
            return;
        }
        if let Ok(mut out) = self.output.try_lock() {
            if out.len() == 4 {
                out.pop_front();
            }
            let mut frame = [0; 480];
            frame.copy_from_slice(samples);
            out.push_back(frame);
        }
    }
    pub(crate) fn retain_video(&self, peers: &[u128]) {
        if let Ok(mut images) = self.received.try_lock() {
            images.retain(|i| i.participant == LOCAL_PARTICIPANT || peers.contains(&i.participant));
        }
    }
    /// The sender's own camera, for the self view only. Zero is the reserved
    /// local participant; it shares the 16-frame bound with the remote streams
    /// and never reaches the encoder or the wire.
    pub(crate) fn show_local(&self, width: u32, height: u32, rgba: &[u8]) {
        if !self.valid() {
            return;
        }
        if let Ok(mut images) = self.received.try_lock() {
            if images.len() >= 16 || images.iter().any(|i| i.participant == LOCAL_PARTICIPANT) {
                return;
            }
            images.push(VideoImage {
                participant: LOCAL_PARTICIPANT,
                width,
                height,
                rgba: rgba.to_vec(),
            });
        }
    }
    pub(crate) fn show_video(&self, participant: u128, frame: VideoFrame) {
        if !self.valid() {
            return;
        }
        if frame.width == 0
            || frame.height == 0
            || frame.width > 1920
            || frame.height > 1080
            || frame.rgba.len() != (frame.width * frame.height * 4) as usize
        {
            return;
        }
        if let Ok(mut images) = self.received.try_lock() {
            if images.len() == 16 || images.iter().any(|i| i.participant == participant) {
                return;
            }
            if self.valid() {
                images.push(VideoImage {
                    participant,
                    width: frame.width,
                    height: frame.height,
                    rgba: frame.rgba,
                });
            }
        }
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.close();
    }
}
impl CaptureHandle {
    pub fn scope(&self) -> Option<CallScope> {
        self.session.upgrade().filter(|s| s.live()).map(|s| s.scope)
    }
    pub fn outgoing_wait(&self) -> bool {
        self.session.upgrade().is_some_and(|s| {
            s.valid()
                && s.snapshot.try_lock().is_ok_and(|v| {
                    v.entry == talk_core::calls::Entry::OutgoingDirect
                        && v.phase == talk_core::calls::Phase::Connecting
                })
        })
    }
    pub fn receive_allowed(&self) -> bool {
        self.session.upgrade().is_some_and(|s| s.valid())
    }
    pub fn device_failed(&self, microphone: bool, camera: bool) {
        if let Some(s) = self.session.upgrade() {
            let bits = u8::from(microphone) | (u8::from(camera) * 2);
            s.blocked.fetch_or(bits, Ordering::AcqRel);
            s.publish.fetch_and(!bits, Ordering::AcqRel);
        }
    }
    /// Lift a device failure after the selected device changed. Publication
    /// resumes with the next grant, if the call still wishes for that medium.
    pub fn device_retry(&self, microphone: bool, camera: bool) {
        if let Some(s) = self.session.upgrade() {
            let bits = u8::from(microphone) | (u8::from(camera) * 2);
            s.blocked.fetch_and(!bits, Ordering::AcqRel);
        }
    }
    pub fn requested(&self) -> MediaWishes {
        self.session
            .upgrade()
            .map_or(MediaWishes::default(), |s| s.publication())
    }
    pub fn microphone(&self, samples: &[i16; 480]) -> bool {
        let Some(s) = self.session.upgrade() else {
            return false;
        };
        if !s.publication().microphone {
            return false;
        }
        let Ok(mut audio) = s.audio.try_lock() else {
            return false;
        };
        if audio.len() == 4 {
            audio.pop_front();
        }
        if !s.publication().microphone {
            return false;
        }
        audio.push_back(*samples);
        s.microphone_seen.store(true, Ordering::Release);
        true
    }
    pub fn camera_rgba(&self, width: u32, height: u32, rgba: &[u8]) -> bool {
        let Some(s) = self.session.upgrade() else {
            return false;
        };
        if !s.publication().camera
            || width == 0
            || height == 0
            || width > 1280
            || height > 720
            || rgba.len() != (width * height * 4) as usize
        {
            return false;
        }
        let Ok(mut video) = s.video.try_lock() else {
            return false;
        };
        if video.is_some() {
            return false;
        }
        let frame = VideoFrame {
            width,
            height,
            rgba: rgba.to_vec(),
        };
        if !s.publication().camera {
            return false;
        }
        *video = Some(frame);
        s.camera_seen.store(true, Ordering::Release);
        drop(video);
        s.show_local(width, height, rgba);
        true
    }
    pub fn screen_generation(&self) -> Option<u64> {
        let s = self.session.upgrade()?;
        s.screen_capture_active()
            .then(|| s.screen_generation.load(Ordering::Acquire))
    }
    pub fn screen_state(&self, generation: u64, state: talk_core::calls::SharingState) {
        // Only the media owner can observe a transmitted encoded sample.
        if matches!(
            state,
            talk_core::calls::SharingState::Sharing | talk_core::calls::SharingState::Idle
        ) {
            return;
        }
        let Some(s) = self.session.upgrade() else {
            return;
        };
        if s.screen_generation.load(Ordering::Acquire) != generation || !s.screen_capture_active() {
            return;
        }
        if let Ok(mut snapshot) = s.snapshot.lock() {
            if s.screen_generation.load(Ordering::Acquire) != generation {
                return;
            }
            snapshot.sharing = state;
            snapshot.screen = match state {
                talk_core::calls::SharingState::Sharing => talk_core::calls::DeviceState::Active,
                talk_core::calls::SharingState::Failed(_) => {
                    talk_core::calls::DeviceState::Unavailable
                }
                _ => talk_core::calls::DeviceState::Requested,
            };
            // Serialize failure effects with the stop/start snapshot barrier.
            // A delayed old callback cannot block a replacement generation.
            if matches!(state, talk_core::calls::SharingState::Failed(_)) {
                s.blocked.fetch_or(4, Ordering::AcqRel);
                s.publish.fetch_and(!4, Ordering::AcqRel);
                if let Ok(mut q) = s.screen.lock() {
                    *q = None;
                }
            }
        }
    }
    pub fn screen_rgba(&self, generation: u64, width: u32, height: u32, rgba: &[u8]) -> bool {
        if self.screen_generation() != Some(generation)
            || !self.requested().screen
            || width == 0
            || height == 0
            || width > 1920
            || height > 1080
            || !width.is_multiple_of(2)
            || !height.is_multiple_of(2)
            || rgba.len() != (width * height * 4) as usize
        {
            return false;
        }
        let Some(s) = self.session.upgrade() else {
            return false;
        };
        let Ok(mut q) = s.screen.try_lock() else {
            return false;
        };
        if self.screen_generation() != Some(generation) {
            return false;
        }
        *q = Some((
            generation,
            VideoFrame {
                width,
                height,
                rgba: rgba.to_vec(),
            },
        ));
        true
    }
    pub fn output_audio(&self, out: &mut [i16; 480]) -> bool {
        out.fill(0);
        let Some(s) = self.session.upgrade() else {
            return false;
        };
        if !s.valid() {
            return false;
        }
        if let Some(frame) = s.output.try_lock().ok().and_then(|mut q| q.pop_front()) {
            *out = frame;
            true
        } else {
            false
        }
    }
    pub fn take_video(&self) -> Option<VideoImage> {
        let s = self.session.upgrade()?;
        if !s.valid() {
            return None;
        }
        let mut images = s.received.try_lock().ok()?;
        if images.is_empty() {
            None
        } else {
            Some(images.remove(0))
        }
    }
}

#[cfg(test)]
mod screen_tests {
    use super::*;
    use talk_core::{
        calls::{DeviceState, Entry, MediaCounters, Phase, SharingState},
        ids::*,
        policy::RecordingState,
    };
    fn admitted() -> (Arc<MediaService>, Arc<Session>, CaptureHandle) {
        let account = AccountId::from_local(1);
        let call = CallScope {
            account: AccountScope {
                account,
                runtime: RuntimeGeneration::from_local(1),
            },
            conversation: ConversationId {
                account,
                host: HostId::from_local(1),
                local: 1,
            },
            call: CallGeneration::from_local(1),
        };
        let service = MediaService::new();
        let session = service
            .admit(
                Snapshot {
                    wishes: Default::default(),
                    call,
                    entry: Entry::PreparedConference,
                    participants: talk_core::bounded::Page::empty(),
                    phase: Phase::Connecting,
                    microphone: DeviceState::Off,
                    local_speaking: false,
                    camera: DeviceState::Off,
                    screen: DeviceState::Off,
                    sharing: SharingState::Idle,
                    peers: 0,
                    recording: RecordingState::Unknown,
                    recording_control: None,
                    recording_pending: false,
                    recording_uncertain: false,
                    encrypted: false,
                    hand_supported: false,
                    hand_raised: false,
                    reactions: talk_core::bounded::Page::empty(),
                    latest_reaction: None,
                    invitees_loaded: false,
                    invitees: talk_core::bounded::Page::empty(),
                    ring_status: None,
                    mute_status: None,
                    can_end_everyone: false,
                    end_pending: false,
                    end_failed: false,
                    counters: MediaCounters::default(),
                    revision: 0,
                },
                Cancellation::default(),
            )
            .unwrap();
        let handle = service.capture().unwrap();
        assert!(session.start_screen(session.screen_epoch()));
        session.deadline.store(60_000, Ordering::Release);
        session.publish.store(4, Ordering::Release);
        session.screen_deadline.store(60_000, Ordering::Release);
        (service, session, handle)
    }
    #[test]
    fn the_own_camera_reaches_the_self_view_once_and_survives_peer_retention() {
        let (_service, session, handle) = admitted();
        let pixels = [9u8; 16];
        session.show_local(2, 2, &pixels);
        // One frame per sender until the consumer has taken it.
        session.show_local(2, 2, &[1u8; 16]);
        session.retain_video(&[]);
        let image = handle.take_video().expect("local frame");
        assert_eq!(image.participant, LOCAL_PARTICIPANT);
        assert_eq!(image.rgba, pixels);
        assert!(handle.take_video().is_none());
        session.show_local(2, 2, &pixels);
        assert!(handle.take_video().is_some());
    }
    #[test]
    fn a_changed_device_lifts_the_failure_of_the_previous_one() {
        let (_service, session, handle) = admitted();
        handle.device_failed(true, true);
        assert_eq!(session.failed(), (true, true));
        // Only the medium whose device changed is tried again.
        handle.device_retry(true, false);
        assert_eq!(session.failed(), (false, true));
        handle.device_retry(false, true);
        assert_eq!(session.failed(), (false, false));
    }
    #[test]
    fn synchronous_stop_retires_starts_queued_before_control_overload() {
        let (_service, session, _) = admitted();
        let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
        for _ in 0..8 {
            sender.try_send(session.screen_epoch()).unwrap();
        }
        session.stop_screen();
        assert!(sender.try_send(session.screen_epoch()).is_err());
        while let Ok(old_start) = receiver.try_recv() {
            assert!(!session.start_screen(old_start));
            assert!(!session.screen_capture_active());
        }
        // A later deliberate action can choose a new source.
        assert!(session.start_screen(session.screen_epoch()));
        assert_eq!(
            session.snapshot.lock().unwrap().sharing,
            SharingState::Choosing
        );
    }
    #[test]
    fn stopped_and_replaced_generations_reject_frames_and_late_failure() {
        let (_service, session, handle) = admitted();
        let old = handle.screen_generation().unwrap();
        assert!(handle.screen_rgba(old, 2, 2, &[0; 16]));
        assert_eq!(
            session.snapshot.lock().unwrap().sharing,
            SharingState::Choosing
        );
        session.stop_screen();
        assert!(session.take_screen().is_none());
        assert!(session.start_screen(session.screen_epoch()));
        session.publish.store(4, Ordering::Release);
        session.screen_deadline.store(60_000, Ordering::Release);
        let new = handle.screen_generation().unwrap();
        assert_ne!(old, new);
        assert!(!handle.screen_rgba(old, 2, 2, &[0; 16]));
        handle.screen_state(
            old,
            SharingState::Failed(talk_core::calls::ShareFailure::SourceClosed),
        );
        assert_eq!(
            session.snapshot.lock().unwrap().sharing,
            SharingState::Choosing
        );
        assert!(handle.screen_rgba(new, 2, 2, &[0; 16]));
        session.screen_flowing(new);
        assert_eq!(
            session.snapshot.lock().unwrap().sharing,
            SharingState::Sharing
        );
        session.close();
        assert!(!handle.screen_rgba(new, 2, 2, &[0; 16]));
        assert!(session.take_screen().is_none());
    }
    #[test]
    fn source_failure_blocks_frames_and_queue_is_latest_only() {
        let (_service, session, handle) = admitted();
        let generation = handle.screen_generation().unwrap();
        handle.screen_state(generation, SharingState::Sharing);
        assert_eq!(
            session.snapshot.lock().unwrap().sharing,
            SharingState::Choosing
        );
        assert!(handle.screen_rgba(generation, 2, 2, &[1; 16]));
        assert!(handle.screen_rgba(generation, 2, 2, &[2; 16]));
        assert_eq!(session.take_screen().unwrap().1.rgba, [2; 16]);
        handle.screen_state(
            generation,
            SharingState::Failed(talk_core::calls::ShareFailure::SourceClosed),
        );
        assert!(!handle.screen_rgba(generation, 2, 2, &[3; 16]));
        assert!(!session.publication().screen);
    }
    #[test]
    fn queued_old_frame_and_flow_observation_cannot_activate_new_share() {
        let (_service, session, handle) = admitted();
        let old = handle.screen_generation().unwrap();
        assert!(session.start_screen(session.screen_epoch()));
        session.publish.store(4, Ordering::Release);
        session.screen_deadline.store(60_000, Ordering::Release);
        // Reproduce a callback that passed its first check just before Stop.
        *session.screen.lock().unwrap() = Some((
            old,
            VideoFrame {
                width: 2,
                height: 2,
                rgba: vec![7; 16],
            },
        ));
        assert!(session.take_screen().is_none());
        session.screen_flowing(old);
        assert_eq!(
            session.snapshot.lock().unwrap().sharing,
            SharingState::Choosing
        );
    }
}
