//! One local attachment player. The authenticated content owner supplies finite
//! bytes; this worker accepts no URL, path, credential, call grant or UI entity.
//! Native codec/reference allocations are not a qualified process-memory bound.
//!
//! Keep the owner until `reap()` confirms worker exit before replacing it. Stop
//! revokes frame access immediately. If native disposal stalls, the process-wide
//! lease stays occupied, including after this Rust handle is dropped; elapsed
//! teardown is reported honestly rather than allowing overlapping playback.
#![forbid(unsafe_code)]

mod pipeline;

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use talk_core::protocol::content::{MAX_VIDEO_BYTES, VideoBytes};

const TICK: Duration = Duration::from_millis(20);
const START_LIMIT: Duration = Duration::from_secs(10);
const STOP_LIMIT: Duration = Duration::from_secs(3);
const LIFETIME: Duration = Duration::from_secs(30 * 60);
const MAX_POSITION_MS: u64 = 24 * 60 * 60 * 1_000;
static OCCUPIED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Error {
    Busy,
    Unsupported,
    Malformed,
    TooLarge,
    Decoder,
    AudioUnavailable,
    Deadline,
    ControlFailed,
    WorkerUnavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Phase {
    Starting,
    Paused,
    Playing,
    Ended,
    Failed(Error),
    Stopping,
    Stopped,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Snapshot {
    pub phase: Phase,
    pub position_ms: u64,
    pub duration_ms: Option<u64>,
    /// Stop has not settled within three seconds. Capacity remains reserved.
    pub stop_unconfirmed: bool,
}

/// Tightly packed, straight-alpha BGRA. The composition converts it to a
/// RenderImage off the UI thread and rechecks its own attachment generation.
pub(super) struct Frame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
    pub sequence: u64,
}

struct Shared {
    stop: AtomicBool,
    playing: AtomicBool,
    seek: Mutex<Option<u64>>,
    frame: Mutex<Option<Frame>>,
    snapshot: Mutex<Snapshot>,
    stop_at: Mutex<Option<Instant>>,
    // Two completions after a test submits a request cover an in-flight
    // iteration that may already have sampled controls before that request.
    #[cfg(test)]
    completed_iterations: std::sync::atomic::AtomicUsize,
}
impl Shared {
    fn new() -> Self {
        Self {
            stop: AtomicBool::new(false),
            playing: AtomicBool::new(false),
            seek: Mutex::new(None),
            frame: Mutex::new(None),
            snapshot: Mutex::new(Snapshot {
                phase: Phase::Starting,
                position_ms: 0,
                duration_ms: None,
                stop_unconfirmed: false,
            }),
            stop_at: Mutex::new(None),
            #[cfg(test)]
            completed_iterations: std::sync::atomic::AtomicUsize::new(0),
        }
    }
    fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        self.playing.store(false, Ordering::Release);
        if let Ok(mut at) = self.stop_at.lock() {
            at.get_or_insert_with(Instant::now);
        }
        if let Ok(mut frame) = self.frame.lock() {
            *frame = None;
        }
        if let Ok(mut snapshot) = self.snapshot.lock()
            && !matches!(snapshot.phase, Phase::Stopped | Phase::Failed(_))
        {
            snapshot.phase = Phase::Stopping;
        }
    }
    fn phase(&self, phase: Phase) {
        if let Ok(mut snapshot) = self.snapshot.lock() {
            // A user Stop cannot be overwritten by a late Paused/Playing poll.
            if self.stop.load(Ordering::Acquire)
                && !matches!(phase, Phase::Stopped | Phase::Failed(_))
            {
                snapshot.phase = Phase::Stopping;
            } else {
                snapshot.phase = phase;
            }
        }
    }
    fn snapshot(&self) -> Snapshot {
        let mut value = self.snapshot.lock().map_or(
            Snapshot {
                phase: Phase::Failed(Error::WorkerUnavailable),
                position_ms: 0,
                duration_ms: None,
                stop_unconfirmed: false,
            },
            |value| *value,
        );
        value.stop_unconfirmed = value.phase == Phase::Stopping
            && self
                .stop_at
                .lock()
                .is_ok_and(|at| at.is_some_and(|at| at.elapsed() >= STOP_LIMIT));
        value
    }
}

/// No native API runs on the caller. Controls are replaceable memory slots;
/// Stop is irreversible and bypasses both ordinary control slots.
pub(super) struct Player {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}
impl Player {
    pub fn start(bytes: VideoBytes) -> Result<Self, Error> {
        if bytes.as_slice().is_empty() {
            return Err(Error::Malformed);
        }
        if bytes.as_slice().len() > MAX_VIDEO_BYTES {
            return Err(Error::TooLarge);
        }
        let container = pipeline::Container::detect(bytes.as_slice())?;
        OCCUPIED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Error::Busy)?;
        struct Lease;
        impl Drop for Lease {
            fn drop(&mut self) {
                OCCUPIED.store(false, Ordering::Release);
            }
        }
        let lease = Lease;
        let shared = Arc::new(Shared::new());
        let state = shared.clone();
        let worker = thread::Builder::new()
            .name("content-video".into())
            .spawn(move || {
                // Declared first: release only after worker/native disposal.
                let _lease = lease;
                run(bytes, container, &state);
            })
            .map_err(|_| Error::WorkerUnavailable)?;
        Ok(Self {
            shared,
            worker: Some(worker),
        })
    }
    pub fn snapshot(&self) -> Snapshot {
        self.shared.snapshot()
    }
    pub fn set_playing(&self, playing: bool) -> Result<(), Error> {
        self.admit_control()?;
        self.shared.playing.store(playing, Ordering::Release);
        self.wake();
        Ok(())
    }
    pub fn seek_ms(&self, position_ms: u64) -> Result<(), Error> {
        self.admit_control()?;
        let position = clamp_seek(position_ms, self.snapshot().duration_ms);
        *self
            .shared
            .seek
            .lock()
            .map_err(|_| Error::WorkerUnavailable)? = Some(position);
        self.wake();
        Ok(())
    }
    fn admit_control(&self) -> Result<(), Error> {
        if self.shared.stop.load(Ordering::Acquire)
            || self.worker.as_ref().is_none_or(JoinHandle::is_finished)
        {
            Err(Error::ControlFailed)
        } else {
            Ok(())
        }
    }
    pub fn take_frame(&self) -> Option<Frame> {
        if self.shared.stop.load(Ordering::Acquire) {
            return None;
        }
        self.shared.frame.lock().ok()?.take()
    }
    pub fn stop(&self) {
        self.shared.stop();
        self.wake();
    }
    fn wake(&self) {
        if let Some(worker) = &self.worker {
            worker.thread().unpark();
        }
    }
    /// Joining an already finished worker cannot wait for native teardown.
    /// `false` includes overdue/unconfirmed teardown; do not replace yet.
    pub fn reap(&mut self) -> bool {
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
        {
            return false;
        }
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            self.shared.phase(Phase::Failed(Error::WorkerUnavailable));
        }
        true
    }
    /// Native disposal and worker exit have completed. Safe for an immutable
    /// shutdown observer after the UI's ordinary polling loop has stopped.
    pub fn is_stopped(&self) -> bool {
        self.worker.as_ref().is_none_or(JoinHandle::is_finished)
    }
}
impl Drop for Player {
    fn drop(&mut self) {
        self.stop();
        // Do not block the UI owner on native teardown. The worker retains its
        // shared state, original backing and exclusive lease until disposal.
        let _ = self.reap();
    }
}

fn clamp_seek(position: u64, duration: Option<u64>) -> u64 {
    position
        .min(MAX_POSITION_MS)
        .min(duration.map_or(MAX_POSITION_MS, |d| d.saturating_sub(1)))
}

fn run(bytes: VideoBytes, container: pipeline::Container, shared: &Arc<Shared>) {
    let started = Instant::now();
    if shared.stop.load(Ordering::Acquire) {
        shared.phase(Phase::Stopped);
        return;
    }
    let pipeline = match pipeline::Pipeline::new(bytes, container, shared.clone()) {
        Ok(pipeline) => pipeline,
        Err(error) => {
            shared.phase(Phase::Failed(error));
            return;
        }
    };
    run_pipeline(pipeline, shared, started);
}

fn run_pipeline(mut pipeline: pipeline::Pipeline, shared: &Arc<Shared>, started: Instant) {
    let mut failure = pipeline.set_playing(false).err();
    let mut desired_playing = false;
    let mut preroll = false;
    let mut settled = false;
    let mut transition_started = Instant::now();
    let mut sequence = 0u64;
    loop {
        if failure.is_some() {
            break;
        }
        if shared.stop.load(Ordering::Acquire) {
            break;
        }
        if let Some(error) = pipeline.error() {
            failure = Some(error);
            break;
        }
        if started.elapsed() >= LIFETIME
            || (!settled && transition_started.elapsed() >= START_LIMIT)
        {
            failure = Some(Error::Deadline);
            break;
        }
        if pipeline.clock_lost() {
            // GStreamer selects a replacement clock on PAUSED -> PLAYING.
            // Re-read user intent below; clock recovery must never grant Play
            // after a newer Pause or Stop. Native producer threads only signal.
            if pipeline.set_playing(false).is_err() {
                failure = Some(Error::ControlFailed);
                break;
            }
            desired_playing = false;
            if settled {
                transition_started = Instant::now();
            }
            settled = false;
        }
        if let Err(error) = pipeline.update_latency() {
            failure = Some(error);
            break;
        }
        if shared.stop.load(Ordering::Acquire) {
            break;
        }
        if pipeline.eos() && shared.snapshot().phase != Phase::Ended {
            shared.playing.store(false, Ordering::Release);
            desired_playing = false;
            if pipeline.set_playing(false).is_err() {
                failure = Some(Error::ControlFailed);
                break;
            }
            shared.phase(Phase::Ended);
        }
        let playing = shared.playing.load(Ordering::Acquire);
        let seek = shared.seek.lock().ok().and_then(|mut seek| seek.take());
        let ended = shared.snapshot().phase == Phase::Ended;
        if let Some(position) = seek.or_else(|| (ended && playing).then_some(0)) {
            if pipeline.seek(position).is_err() {
                failure = Some(Error::ControlFailed);
                break;
            }
            preroll = false;
            settled = false;
            transition_started = Instant::now();
            // A seek prerolls a fresh frame even while remaining paused.
            // Retire Ended immediately: otherwise every worker tick repeats the
            // implicit replay seek before its first new frame can arrive.
            shared.phase(Phase::Starting);
        }
        // The first video frame does not complete audio preroll. Keep early
        // Play coalesced until discovery and the whole PAUSED transition settle.
        if playing != desired_playing && (settled || !playing) {
            if pipeline.set_playing(playing).is_err() {
                failure = Some(Error::ControlFailed);
                break;
            }
            desired_playing = playing;
            transition_started = Instant::now();
        }
        match pipeline.frame(!desired_playing && !preroll) {
            Ok(Some(mut frame)) => {
                sequence = sequence.saturating_add(1);
                frame.sequence = sequence;
                if !shared.stop.load(Ordering::Acquire)
                    && let Ok(mut latest) = shared.frame.lock()
                {
                    *latest = Some(frame);
                }
                preroll = true;
            }
            Ok(None) => {}
            Err(error) => {
                failure = Some(error);
                break;
            }
        }
        let observation = pipeline.observation();
        if let Ok(mut snapshot) = shared.snapshot.lock() {
            if let Some(position) = observation.position_ms {
                snapshot.position_ms = position;
            }
            snapshot.duration_ms = observation.duration_ms;
        }
        settled = preroll && observation.settled_state.is_some();
        if !pipeline.eos() {
            if settled && observation.settled_state == Some(gstreamer::State::Playing) {
                shared.phase(Phase::Playing);
            } else if settled && observation.settled_state == Some(gstreamer::State::Paused) {
                shared.phase(Phase::Paused);
            } else {
                shared.phase(Phase::Starting);
            }
        }
        #[cfg(test)]
        shared.completed_iterations.fetch_add(1, Ordering::Release);
        thread::park_timeout(TICK);
    }
    shared.stop();
    // Native state changes can themselves stall. The observer computes the
    // deadline without needing this worker to return from a native call.
    pipeline.stop();
    drop(pipeline);
    shared.phase(failure.map_or(Phase::Stopped, Phase::Failed));
}

#[cfg(test)]
mod tests;
