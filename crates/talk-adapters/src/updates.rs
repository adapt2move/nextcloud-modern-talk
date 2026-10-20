//! Device-local release owner: one sleeping worker, one download slot and one reservation.
//!
//! Startup is offline and unarmed. Composition must pass the loaded channel through
//! `set_channel`; the persisted daily claim is committed before any automatic request.
//! Installing is a separate, explicit transition after composition has sealed calls
//! synchronously and drained drafts/transfers/media. The worker never ends a call.
mod archive;
#[cfg(feature = "backend-test")]
pub mod fixture;
mod install;
mod native;
mod schedule;
mod store;
mod transport;

use semver::Version;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use talk_core::{
    bounded::Text,
    lifecycle::Cancellation,
    preferences::{
        UpdateChannel,
        updates::{ReleaseInfo, UpdateCheckResult, UpdateError, UpdateSnapshot, UpdateState},
    },
};
use tokio::sync::watch;

fn digest_hex(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Optional additional publisher-signature policy. The mandatory exact GitHub
/// asset digest, archive bounds and package identity checks always run as well.
/// Implementations must verify data only; never execute an archive or release text.
pub trait ArtifactVerifier: Send + Sync {
    fn verify(&self, archive: &Path, release: &ReleaseInfo) -> Result<(), UpdateError>;
}

struct State {
    snapshot: UpdateSnapshot,
    armed: bool,
    check_requested: bool,
    install_requested: bool,
    busy: bool,
    reserved: bool,
    poisoned: bool,
    cancel: Cancellation,
    relaunch: Option<PathBuf>,
    previous_usable: bool,
}
struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    changed: watch::Sender<u64>,
    stop: AtomicBool,
    stopped: AtomicBool,
}

/// A single-use capability returned only after composition's authoritative call
/// check and admission seal succeeds. No constructor or Clone is exposed to views.
#[derive(Debug)]
pub struct InstallReservation {
    generation: u64,
}

pub struct UpdateRuntime {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}
impl UpdateRuntime {
    pub fn start(current_version: &str) -> Result<Self, UpdateError> {
        Self::start_with_verifier(current_version, None)
    }
    pub fn start_with_verifier(
        current_version: &str,
        verifier: Option<Arc<dyn ArtifactVerifier>>,
    ) -> Result<Self, UpdateError> {
        let current = Version::parse(current_version).map_err(|_| UpdateError::InvalidResponse)?;
        let (changed, _) = watch::channel(1);
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                snapshot: UpdateSnapshot {
                    revision: 1,
                    generation: 1,
                    current_version: Text::new(current_version)
                        .map_err(|_| UpdateError::InvalidResponse)?,
                    channel: UpdateChannel::Stable,
                    next_check_unix_seconds: None,
                    last_check: None,
                    state: UpdateState::Idle,
                },
                armed: false,
                check_requested: false,
                install_requested: false,
                busy: false,
                reserved: false,
                poisoned: false,
                cancel: Cancellation::default(),
                relaunch: None,
                previous_usable: false,
            }),
            wake: Condvar::new(),
            changed,
            stop: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
        });
        let owner = shared.clone();
        let worker = thread::Builder::new()
            .name("talk-updates".into())
            .spawn(move || {
                let result = worker(&owner, &current, verifier.as_deref());
                let mut state = lock(&owner);
                state.busy = false;
                state.snapshot.state = match result {
                    Ok(()) => UpdateState::Stopped,
                    Err(error) => UpdateState::Failed(error),
                };
                advance(&mut state);
                owner.stopped.store(true, Ordering::Release);
            })
            .map_err(|_| UpdateError::Unavailable)?;
        Ok(Self {
            shared,
            worker: Some(worker),
        })
    }
    pub fn snapshot(&self) -> UpdateSnapshot {
        lock(&self.shared).snapshot.clone()
    }
    pub fn poll(&self, after: u64) -> Option<(u64, UpdateSnapshot)> {
        let snapshot = self.snapshot();
        (snapshot.revision != after).then_some((snapshot.revision, snapshot))
    }
    /// The first confirmed preference arms scheduling, with no immediate request.
    /// A later channel change cancels any old result and requests one new check.
    pub fn set_channel(&self, channel: UpdateChannel) -> Result<(), UpdateError> {
        let mut state = lock(&self.shared);
        self.running()?;
        if state.reserved
            || state.install_requested
            || matches!(
                state.snapshot.state,
                UpdateState::Applying(_) | UpdateState::Installed(_)
            )
        {
            return Err(UpdateError::Busy);
        }
        if state.armed && state.snapshot.channel == channel {
            return Ok(());
        }
        let was_armed = state.armed;
        invalidate(&self.shared, &mut state);
        state.armed = true;
        state.snapshot.channel = channel;
        state.snapshot.state = UpdateState::Idle;
        state.check_requested = was_armed;
        state.poisoned = false;
        advance(&mut state);
        self.shared.wake.notify_one();
        Ok(())
    }
    /// One explicit Settings check; repeated clicks during a request are refused.
    pub fn check(&self, channel: UpdateChannel) -> Result<(), UpdateError> {
        self.set_channel(channel)?;
        let mut state = lock(&self.shared);
        self.running()?;
        if state.busy || state.reserved || state.install_requested {
            return Err(UpdateError::Busy);
        }
        invalidate(&self.shared, &mut state);
        state.check_requested = true;
        state.poisoned = false;
        state.snapshot.state = UpdateState::Checking;
        advance(&mut state);
        self.shared.wake.notify_one();
        Ok(())
    }
    /// `seal_calls` must consult the actual serialized call/admission owner,
    /// including incoming, pending, joining and ending calls, then seal new calls.
    /// It is invoked once synchronously; callers must not substitute a UI boolean.
    /// On Err no reservation is made. Composition owns releasing its seal on abort.
    pub fn reserve_install(
        &self,
        expected_generation: u64,
        seal_calls: impl FnOnce() -> Result<(), UpdateError>,
    ) -> Result<InstallReservation, UpdateError> {
        let mut state = lock(&self.shared);
        self.running()?;
        if expected_generation != state.snapshot.generation {
            return Err(UpdateError::Stale);
        }
        if state.busy || state.reserved || state.check_requested {
            return Err(UpdateError::Busy);
        }
        let UpdateState::Ready(release) = &state.snapshot.state else {
            return Err(UpdateError::Unavailable);
        };
        let release = release.clone();
        seal_calls()?;
        state.reserved = true;
        state.snapshot.state = UpdateState::Reserved(release);
        advance(&mut state);
        Ok(InstallReservation {
            generation: expected_generation,
        })
    }
    /// Captured from the verified shipping installation before any bundle swap.
    /// Keep this adapter-only path in composition and launch it only after Installed.
    pub fn relaunch_path(&self, reservation: &InstallReservation) -> Result<PathBuf, UpdateError> {
        let state = lock(&self.shared);
        self.running()?;
        validate_reservation(&state, reservation)?;
        if !matches!(state.snapshot.state, UpdateState::Reserved(_)) {
            return Err(UpdateError::Busy);
        }
        state
            .relaunch
            .clone()
            .ok_or(UpdateError::UnsupportedInstall)
    }
    /// Available only after a settled failure and full verification of the previous
    /// installation. Uncertain native transactions never grant relaunch authority.
    pub fn relaunch_after_failure(&self) -> Option<PathBuf> {
        let state = lock(&self.shared);
        (state.previous_usable && matches!(state.snapshot.state, UpdateState::Failed(_)))
            .then(|| state.relaunch.clone())
            .flatten()
    }
    /// Called only after composition has completed its durable-work/lifecycle drain.
    /// The updater remains alive during that drain. Installation never downloads.
    pub fn begin_install(&self, reservation: InstallReservation) -> Result<(), UpdateError> {
        let mut state = lock(&self.shared);
        self.running()?;
        validate_reservation(&state, &reservation)?;
        let UpdateState::Reserved(release) = &state.snapshot.state else {
            return Err(UpdateError::Stale);
        };
        let release = release.clone();
        state.install_requested = true;
        state.busy = true;
        state.snapshot.state = UpdateState::Applying(release);
        advance(&mut state);
        self.shared.wake.notify_one();
        Ok(())
    }
    pub fn abort_install(&self, reservation: InstallReservation) -> Result<(), UpdateError> {
        let mut state = lock(&self.shared);
        self.running()?;
        validate_reservation(&state, &reservation)?;
        let UpdateState::Reserved(release) = &state.snapshot.state else {
            return Err(UpdateError::Busy);
        };
        let release = release.clone();
        state.reserved = false;
        state.snapshot.state = UpdateState::Ready(release);
        advance(&mut state);
        self.shared.wake.notify_one();
        Ok(())
    }
    fn running(&self) -> Result<(), UpdateError> {
        if self.shared.stop.load(Ordering::Acquire) || self.is_stopped() {
            Err(UpdateError::Stopped)
        } else {
            Ok(())
        }
    }
    pub fn request_shutdown(&self) {
        self.shared.stop.store(true, Ordering::Release);
        let mut state = lock(&self.shared);
        // A committed native package transaction cannot safely be abandoned. Its
        // bounded completion/uncertainty owner runs to settlement before stopped.
        if !matches!(state.snapshot.state, UpdateState::Applying(_)) {
            state.cancel.cancel();
        }
        self.shared
            .changed
            .send_replace(state.snapshot.generation.saturating_add(1));
        advance(&mut state);
        self.shared.wake.notify_one();
    }
    pub fn is_stopped(&self) -> bool {
        self.shared.stopped.load(Ordering::Acquire)
    }
}
impl Drop for UpdateRuntime {
    fn drop(&mut self) {
        self.request_shutdown();
        // Composition normally waits for is_stopped before dropping Services.
        // Joining here preserves ownership even if a caller violates that order.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn validate_reservation(
    state: &State,
    reservation: &InstallReservation,
) -> Result<(), UpdateError> {
    if state.reserved && state.snapshot.generation == reservation.generation {
        Ok(())
    } else {
        Err(UpdateError::Stale)
    }
}
fn lock(shared: &Shared) -> std::sync::MutexGuard<'_, State> {
    shared.state.lock().unwrap_or_else(|e| e.into_inner())
}
fn advance(state: &mut State) {
    state.snapshot.revision = state.snapshot.revision.saturating_add(1);
}
fn invalidate(shared: &Shared, state: &mut State) {
    state.cancel.cancel();
    state.cancel = Cancellation::default();
    state.relaunch = None;
    state.previous_usable = false;
    state.snapshot.generation = state.snapshot.generation.saturating_add(1);
    shared.changed.send_replace(state.snapshot.generation);
}
fn publish(shared: &Shared, generation: u64, value: UpdateState) {
    let mut state = lock(shared);
    if state.snapshot.generation == generation && !shared.stop.load(Ordering::Acquire) {
        state.snapshot.state = value;
        advance(&mut state);
    }
}
async fn interruptible<T>(
    shared: &Shared,
    generation: u64,
    operation: impl std::future::Future<Output = Result<T, UpdateError>>,
) -> Result<T, UpdateError> {
    let mut changed = shared.changed.subscribe();
    if shared.stop.load(Ordering::Acquire) {
        return Err(UpdateError::Stopped);
    }
    if *changed.borrow_and_update() != generation {
        return Err(UpdateError::Stale);
    }
    tokio::select! {
        biased;
        _ = changed.changed() => Err(if shared.stop.load(Ordering::Acquire) { UpdateError::Stopped } else { UpdateError::Stale }),
        result = operation => result,
    }
}

fn worker(
    shared: &Shared,
    current: &Version,
    verifier: Option<&dyn ArtifactVerifier>,
) -> Result<(), UpdateError> {
    let mut state = lock(shared);
    while !state.armed && !shared.stop.load(Ordering::Acquire) {
        state = shared.wake.wait(state).unwrap_or_else(|e| e.into_inner());
    }
    if shared.stop.load(Ordering::Acquire) {
        return Ok(());
    }
    drop(state);
    let store = store::Store::open()?;
    let mut schedule = match store.schedule()? {
        Some(schedule) => schedule,
        None => {
            let schedule = schedule::Schedule::new(schedule::now()?, schedule::random_second()?);
            store.save_schedule(schedule)?;
            schedule
        }
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| UpdateError::Unavailable)?;
    let client = transport::client()?;
    let mut staged: Option<install::Staged> = None;
    loop {
        let mut state = lock(shared);
        if state.snapshot.next_check_unix_seconds != Some(schedule.next) {
            state.snapshot.next_check_unix_seconds = Some(schedule.next);
            advance(&mut state);
        }
        if shared.stop.load(Ordering::Acquire) {
            break;
        }
        let now = schedule::now()?;
        let automatic = !state.reserved && !state.poisoned && schedule.due(now);
        if state.install_requested {
            state.install_requested = false;
            let generation = state.snapshot.generation;
            let channel = state.snapshot.channel;
            let cancel = state.cancel.clone();
            drop(state);
            let (result, previous_usable) = match staged.take() {
                Some(stage) if stage.generation == generation => {
                    let result = runtime.block_on(install::apply(
                        &store, &stage, current, channel, verifier, &cancel,
                    ));
                    let previous_usable = result.is_err()
                        && result != Err(UpdateError::InstallUncertain)
                        && stage.target.previous_usable(current);
                    (result, previous_usable)
                }
                _ => (Err(UpdateError::Stale), false),
            };
            let mut state = lock(shared);
            state.busy = false;
            state.reserved = false;
            state.previous_usable = previous_usable;
            state.poisoned = true; // no automatic retry or post-install update loop
            state.snapshot.state = match result {
                Ok(release) => UpdateState::Installed(release),
                Err(error) => UpdateState::Failed(error),
            };
            advance(&mut state);
            continue;
        }
        if state.check_requested || automatic {
            if automatic {
                // Persist while holding the reservation lock: a click cannot race
                // a daily claim into replacing the selected archive.
                let claimed = schedule.claim(now, schedule::random_second()?);
                store.save_schedule(claimed)?;
                schedule = claimed;
                state.snapshot.next_check_unix_seconds = Some(schedule.next);
            }
            state.check_requested = false;
            state.busy = true;
            invalidate(shared, &mut state);
            let generation = state.snapshot.generation;
            let channel = state.snapshot.channel;
            let cancel = state.cancel.clone();
            state.snapshot.state = UpdateState::Checking;
            advance(&mut state);
            drop(state);
            staged = None;
            let result = runtime.block_on(check_and_stage(
                shared, &store, &client, current, channel, generation, verifier, &cancel,
            ));
            let mut state = lock(shared);
            state.busy = false;
            if state.snapshot.generation == generation && !shared.stop.load(Ordering::Acquire) {
                state.snapshot.state = match result {
                    Ok(Some(stage)) => {
                        let info = stage.selection.info.clone();
                        state.relaunch = Some(stage.target.executable.clone());
                        staged = Some(stage);
                        UpdateState::Ready(info)
                    }
                    Ok(None) => UpdateState::Current,
                    Err(error) => {
                        state.poisoned = matches!(
                            error,
                            UpdateError::Integrity
                                | UpdateError::UnsafeArchive
                                | UpdateError::Storage
                                | UpdateError::TooLarge
                        );
                        UpdateState::Failed(error)
                    }
                };
                advance(&mut state);
            }
            if staged.is_none() {
                store.remove_payload()?;
            }
            continue;
        }
        // No idle polling. A timer is armed only for an unreserved, usable state;
        // manual/channel/shutdown intents wake this same bounded worker.
        state = if state.reserved || state.poisoned {
            shared.wake.wait(state).unwrap_or_else(|e| e.into_inner())
        } else {
            let wait = Duration::from_secs(schedule.next.saturating_sub(now).max(1));
            shared
                .wake
                .wait_timeout(state, wait)
                .unwrap_or_else(|e| e.into_inner())
                .0
        };
        drop(state);
    }
    runtime.shutdown_timeout(Duration::from_secs(1));
    Ok(())
}
#[allow(clippy::too_many_arguments)]
async fn check_and_stage(
    shared: &Shared,
    store: &store::Store,
    client: &reqwest::Client,
    current: &Version,
    channel: UpdateChannel,
    generation: u64,
    verifier: Option<&dyn ArtifactVerifier>,
    cancel: &Cancellation,
) -> Result<Option<install::Staged>, UpdateError> {
    let target = install::Target::detect(current, cancel)?;
    let result = interruptible(
        shared,
        generation,
        transport::check(client, current, channel, target.kind),
    )
    .await;
    {
        let mut state = lock(shared);
        if state.snapshot.generation == generation {
            state.snapshot.last_check = Some(UpdateCheckResult {
                at_unix_seconds: schedule::now()?,
                result: result
                    .as_ref()
                    .map(|value| value.as_ref().map(|selection| selection.info.clone()))
                    .map_err(|e| *e),
            });
            advance(&mut state);
        }
    }
    let Some(selection) = result? else {
        return Ok(None);
    };
    interruptible(shared, generation, target.preflight()).await?;
    let mut file = store.create_payload(target.kind)?;
    publish(
        shared,
        generation,
        UpdateState::Downloading {
            release: selection.info.clone(),
            received: 0,
            total: selection.info.bytes,
        },
    );
    interruptible(
        shared,
        generation,
        transport::download(client, &selection, &mut file, |received| {
            publish(
                shared,
                generation,
                UpdateState::Downloading {
                    release: selection.info.clone(),
                    received,
                    total: selection.info.bytes,
                },
            );
        }),
    )
    .await?;
    drop(file);
    publish(
        shared,
        generation,
        UpdateState::Verifying(selection.info.clone()),
    );
    let stage = install::Staged {
        generation,
        selection,
        target,
    };
    stage.verify(store, verifier, cancel).await?;
    Ok(Some(stage))
}

#[cfg(test)]
mod tests;
