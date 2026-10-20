//! Application-owned local settings. GPUI calls only memory-only admission/polling.
//! One thread owns storage and startup effects, with one replaceable desired slot.
//! No per-click threads, unbounded channels, credentials or account restoration.
mod interaction;
mod native_process;
pub mod notifications;
pub mod permissions;
mod platform;
mod storage;

use platform::StartupEffect;
use std::sync::{
    Arc, Condvar, Mutex, MutexGuard,
    atomic::{AtomicU64, Ordering},
};
use storage::PreferenceStore;
use talk_core::preferences::{
    EffectState, LoadState, LocalPreferences, PreferenceCommand, PreferenceError,
    PreferenceSnapshot, RequestId, Revision, RuntimeState, SaveState, StartupSnapshot,
};

static GENERATION: AtomicU64 = AtomicU64::new(1);
struct Work {
    request: RequestId,
    revision: Revision,
    values: LocalPreferences,
    write: bool,
}
struct RetryDraft {
    revision: Revision,
    values: LocalPreferences,
}
struct State {
    snapshot: PreferenceSnapshot,
    change: u64,
    last_request: RequestId,
    pending: Option<Work>,
    retry: Option<RetryDraft>,
    confirmed_startup: StartupSnapshot,
    interaction: interaction::State,
}
enum WorkerAction {
    Save(Work),
    RefreshInteraction,
    Emoji(u64, std::time::Instant, interaction::Picker),
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
impl State {
    fn changed(&mut self) {
        self.change = self.change.saturating_add(1);
    }
}

pub struct SettingsRuntime {
    shared: Arc<Shared>,
}
impl SettingsRuntime {
    /// Returns before path discovery, load, migration or OS I/O. Poll until Loading
    /// ends before choosing the initial window visibility. Errors stay in snapshots.
    pub fn start() -> Result<Self, PreferenceError> {
        Self::spawn(|| {
            let (path, registration) = platform::current_paths()?;
            Ok((Box::new(storage::FileStore(path)), Box::new(registration)))
        })
    }
    fn spawn<F>(factory: F) -> Result<Self, PreferenceError>
    where
        F: FnOnce() -> Result<(Box<dyn PreferenceStore>, Box<dyn StartupEffect>), PreferenceError>
            + Send
            + 'static,
    {
        let generation = GENERATION
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1))
            .map_err(|_| PreferenceError::RevisionExhausted)?;
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                snapshot: PreferenceSnapshot {
                    generation,
                    revision: Revision(0),
                    durable_revision: Revision(0),
                    active: LocalPreferences::default(),
                    confirmed: LocalPreferences::default(),
                    observed_system_locale: talk_core::preferences::Locale::English,
                    load: LoadState::Loading,
                    save: SaveState::Idle,
                    startup: StartupSnapshot {
                        registered: None,
                        state: EffectState::Unknown,
                    },
                    runtime: RuntimeState::Running,
                },
                change: 1,
                last_request: RequestId(0),
                pending: None,
                retry: None,
                confirmed_startup: StartupSnapshot {
                    registered: None,
                    state: EffectState::Unknown,
                },
                interaction: interaction::State::default(),
            }),
            wake: Condvar::new(),
        });
        let worker = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("talk-preferences".into())
            .spawn(move || {
                let outcome =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match factory() {
                        Ok((mut store, mut effect)) => {
                            run(&worker, store.as_mut(), effect.as_mut())
                        }
                        Err(error) => fail_worker(&worker, error),
                    }));
                if outcome.is_err() {
                    fail_worker(&worker, PreferenceError::WorkerUnavailable);
                }
            })
            .map_err(|_| PreferenceError::WorkerUnavailable)?;
        Ok(Self { shared })
    }
    pub fn snapshot(&self) -> PreferenceSnapshot {
        self.shared.lock().snapshot.clone()
    }
    /// Poll e.g. every 100ms from composition's timer. Save completion changes this
    /// cursor even though the user revision is unchanged. No callback runs on worker.
    pub fn poll(&self, last_change: u64) -> Option<(u64, PreferenceSnapshot)> {
        let state = self.shared.lock();
        (state.change != last_change).then(|| (state.change, state.snapshot.clone()))
    }
    pub fn reduced_motion(&self) -> bool {
        self.shared.lock().interaction.profile.reduced_motion
    }
    pub fn emoji_picker_available(&self) -> bool {
        let state = self.shared.lock();
        state.snapshot.runtime == RuntimeState::Running
            && state.interaction.profile.picker != interaction::Picker::Unavailable
    }
    pub fn show_emoji_picker(&self) -> Result<(), PreferenceError> {
        let mut state = self.shared.lock();
        if state.snapshot.runtime != RuntimeState::Running {
            return Err(PreferenceError::Closed);
        }
        state.interaction.admit(std::time::Instant::now())?;
        self.shared.wake.notify_one();
        Ok(())
    }
    pub fn take_emoji_picker_error(&self) -> Option<PreferenceError> {
        self.shared.lock().interaction.error.take()
    }
    pub fn cancel_emoji_picker(&self) {
        self.shared.lock().interaction.cancel();
        self.shared.wake.notify_one();
    }
    /// Refresh only on app activation. OS preference reads never occur in render.
    pub fn refresh_native_interaction(&self) {
        let mut state = self.shared.lock();
        if state.snapshot.runtime == RuntimeState::Running {
            state.interaction.refresh = true;
            self.shared.wake.notify_one();
        }
    }
    pub fn submit(&self, command: PreferenceCommand) -> Result<Revision, PreferenceError> {
        let mut state = self.shared.lock();
        if state.snapshot.runtime != RuntimeState::Running {
            return Err(PreferenceError::Closed);
        }
        let (generation, request, expected, values, write) = match command {
            PreferenceCommand::Set {
                generation,
                request,
                expected_revision,
                values,
            } => (generation, request, expected_revision, values, true),
            PreferenceCommand::Retry {
                generation,
                request,
                failed_revision,
            } => {
                let draft = state
                    .retry
                    .as_ref()
                    .filter(|draft| draft.revision == failed_revision)
                    .ok_or(PreferenceError::Stale)?;
                (
                    generation,
                    request,
                    failed_revision,
                    draft.values.clone(),
                    true,
                )
            }
            PreferenceCommand::RetryEffects {
                generation,
                request,
                expected_revision,
            } => {
                if matches!(state.snapshot.save, SaveState::Pending { .. })
                    || !matches!(state.snapshot.startup.state, EffectState::Failed(_))
                {
                    return Err(PreferenceError::Busy);
                }
                (
                    generation,
                    request,
                    expected_revision,
                    state.snapshot.confirmed.clone(),
                    false,
                )
            }
        };
        if generation != state.snapshot.generation
            || expected != state.snapshot.revision
            || request <= state.last_request
        {
            return Err(PreferenceError::Stale);
        }
        let revision = if write {
            Revision(
                state
                    .snapshot
                    .revision
                    .0
                    .checked_add(1)
                    .ok_or(PreferenceError::RevisionExhausted)?,
            )
        } else {
            state.snapshot.revision
        };
        state.last_request = request;
        state.retry = None;
        if write {
            state.snapshot.active = values.clone();
            state.snapshot.revision = revision;
            state.snapshot.save = SaveState::Pending { request, revision };
        }
        state.snapshot.startup.state = EffectState::Pending;
        state.pending = Some(Work {
            request,
            revision,
            values,
            write,
        });
        state.changed();
        self.shared.wake.notify_one();
        Ok(revision)
    }
    /// Nonblocking. Reject further edits, finish the running operation and newest
    /// desired slot, then publish Stopped. Composition keeps polling until Stopped
    /// before process exit (or offers an explicit exit-with-unsaved-changes choice).
    /// Drop requests this same drain but never joins a thread on GPUI. Process death
    /// cannot guarantee pending writes; next launch reads only the atomic valid file.
    pub fn request_shutdown(&self) {
        let mut state = self.shared.lock();
        if state.snapshot.runtime == RuntimeState::Running {
            state.snapshot.runtime = RuntimeState::Draining;
            state.interaction.cancel();
            state.changed();
            self.shared.wake.notify_one();
        }
    }
}
impl Drop for SettingsRuntime {
    fn drop(&mut self) {
        self.request_shutdown();
    }
}

fn fail_worker(shared: &Shared, error: PreferenceError) {
    let mut state = shared.lock();
    if state.snapshot.load == LoadState::Loading {
        state.snapshot.load = LoadState::Failed(error);
    }
    if let SaveState::Pending { request, revision } = state.snapshot.save {
        state.snapshot.save = SaveState::Failed {
            request,
            revision,
            error,
            retry_available: false,
        };
    }
    state.snapshot.active = state.snapshot.confirmed.clone();
    state.snapshot.startup.state = EffectState::Failed(error);
    state.snapshot.runtime = RuntimeState::Stopped;
    state.pending = None;
    state.retry = None;
    state.changed();
}
fn run(shared: &Shared, store: &mut dyn PreferenceStore, effect: &mut dyn StartupEffect) {
    let observed_system_locale = platform::system_locale();
    let loaded = store.load();
    let initial_effect = {
        let mut state = shared.lock();
        state.snapshot.observed_system_locale = observed_system_locale;
        let initial = match loaded {
            Ok((values, load)) => {
                state.snapshot.load = load;
                state.snapshot.confirmed = values.clone();
                if state.snapshot.revision == Revision(0) {
                    state.snapshot.active = values.clone();
                }
                Some(values)
            }
            Err(error) => {
                state.snapshot.load = LoadState::Failed(error);
                None
            }
        };
        let initial = initial.filter(|_| state.pending.is_none());
        if initial.is_some() {
            state.snapshot.startup.state = EffectState::Pending;
        }
        state.changed();
        initial
    };
    if let Some(values) = initial_effect {
        let applied = effect.apply(&values);
        let mut state = shared.lock();
        state.snapshot.startup.registered = applied.registered;
        state.confirmed_startup = applied;
        if state.snapshot.revision == Revision(0) {
            state.snapshot.startup.state = applied.state;
        }
        state.changed();
    }
    loop {
        let action = {
            let mut state = shared.lock();
            loop {
                if let Some(work) = state.pending.take() {
                    break WorkerAction::Save(work);
                }
                if state.snapshot.runtime != RuntimeState::Running {
                    state.snapshot.runtime = RuntimeState::Stopped;
                    state.changed();
                    return;
                }
                if let Some((generation, deadline)) = state.interaction.pending.take() {
                    state.interaction.in_flight = true;
                    break WorkerAction::Emoji(
                        generation,
                        deadline,
                        state.interaction.profile.picker,
                    );
                }
                if state.interaction.refresh {
                    state.interaction.refresh = false;
                    break WorkerAction::RefreshInteraction;
                }
                state = shared.wake.wait(state).unwrap_or_else(|e| e.into_inner());
            }
        };
        let work = match action {
            WorkerAction::Save(work) => work,
            WorkerAction::RefreshInteraction => {
                let profile = interaction::observe(|| {
                    shared.lock().snapshot.runtime != RuntimeState::Running
                });
                let mut state = shared.lock();
                state.interaction.profile = profile;
                state.changed();
                continue;
            }
            WorkerAction::Emoji(generation, deadline, picker) => {
                let result = if std::time::Instant::now() >= deadline {
                    Err(PreferenceError::ApplyFailed)
                } else {
                    interaction::open(picker, || {
                        let state = shared.lock();
                        state.snapshot.runtime != RuntimeState::Running
                            || state.interaction.generation != generation
                    })
                };
                let mut state = shared.lock();
                state.interaction.finish(generation, result);
                state.changed();
                continue;
            }
        };
        let saved = if work.write {
            store.save(&work.values)
        } else {
            Ok(())
        };
        // No mutex is held across file/OS work. Effects are separate from durability:
        // a denied registration must not turn a committed preference into "unsaved".
        let applied = if saved.is_ok() {
            Some(effect.apply(&work.values))
        } else {
            None
        };
        let mut state = shared.lock();
        if work.write {
            match saved {
                Ok(()) => {
                    state.snapshot.confirmed = work.values.clone();
                    state.snapshot.durable_revision = work.revision;
                    if state.snapshot.revision == work.revision {
                        state.snapshot.active = work.values;
                        state.snapshot.save = SaveState::Idle;
                        state.retry = None;
                    }
                }
                Err(error) if state.snapshot.revision == work.revision => {
                    state.snapshot.active = state.snapshot.confirmed.clone();
                    state.snapshot.save = SaveState::Failed {
                        request: work.request,
                        revision: work.revision,
                        error,
                        retry_available: true,
                    };
                    state.retry = Some(RetryDraft {
                        revision: work.revision,
                        values: work.values,
                    });
                    // Prior registration observation remains valid; no OS effect ran.
                    state.snapshot.startup = state.confirmed_startup;
                }
                Err(_) => {} // An obsolete failure cannot replace a newer pending edit.
            }
        }
        if let Some(applied) = applied {
            state.confirmed_startup = applied;
            state.snapshot.startup.registered = applied.registered;
            if state.snapshot.revision == work.revision && state.pending.is_none() {
                state.snapshot.startup.state = applied.state;
            }
        }
        state.changed();
    }
}

#[cfg(test)]
mod tests;
