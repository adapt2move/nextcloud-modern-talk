//! Composition-owned bridge between an admitted call and selected native devices.
//! Every callback rechecks the irreversible lifetime and current policy deadline.
//! This module never chooses or opens a device and never manufactures call scope.
use libwebrtc::{
    native::yuv_helper,
    video_frame::{I420Buffer, native::VideoFrameBufferExt},
};
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
    video: Mutex<Option<I420Buffer>>,
    output: Mutex<VecDeque<[i16; 480]>>,
    received: Mutex<Vec<VideoImage>>,
    microphone_seen: AtomicBool,
    camera_seen: AtomicBool,
}
/// At most one 640x360 frame per admitted peer (16 maximum). The sole consumer
/// takes ownership; identity is retained through the renderer, never inferred
/// from whichever peer happened to decode most recently.
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
    pub(crate) fn take_audio(&self) -> Option<[i16; 480]> {
        if !self.publication().microphone {
            return None;
        }
        self.audio.try_lock().ok()?.pop_front()
    }
    pub(crate) fn take_video(&self) -> Option<I420Buffer> {
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
            images.retain(|i| peers.contains(&i.participant));
        }
    }
    pub(crate) fn show_video(
        &self,
        participant: u128,
        frame: libwebrtc::video_frame::BoxVideoFrame,
    ) {
        if !self.valid() {
            return;
        }
        let (width, height) = (frame.buffer.width(), frame.buffer.height());
        if width == 0 || height == 0 || width > 1920 || height > 1080 {
            return;
        }
        if let Ok(mut images) = self.received.try_lock() {
            if images.len() == 16 || images.iter().any(|i| i.participant == participant) {
                return;
            }
            let ratio = (640.0 / f64::from(width))
                .min(360.0 / f64::from(height))
                .min(1.0);
            let width = ((f64::from(width) * ratio) as u32 & !1).max(2);
            let height = ((f64::from(height) * ratio) as u32 & !1).max(2);
            let mut buffer = frame.buffer.to_i420();
            let buffer = buffer.scale(width as i32, height as i32);
            let mut rgba = vec![0; (width * height * 4) as usize];
            buffer.to_argb(
                libwebrtc::video_frame::VideoFormatType::ABGR,
                &mut rgba,
                width * 4,
                width as i32,
                height as i32,
            );
            if self.valid() {
                images.push(VideoImage {
                    participant,
                    width,
                    height,
                    rgba,
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
        let mut frame = I420Buffer::new(width, height);
        let (sy, su, sv) = frame.strides();
        let (y, u, v) = frame.data_mut();
        yuv_helper::abgr_to_i420(
            rgba,
            width * 4,
            y,
            sy,
            u,
            su,
            v,
            sv,
            width as i32,
            height as i32,
        );
        if !s.publication().camera {
            return false;
        }
        *video = Some(frame);
        s.camera_seen.store(true, Ordering::Release);
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
