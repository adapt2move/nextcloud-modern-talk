//! Composition-owned Tokio lifetime. UI uses nonblocking observations/projections only.
use super::{
    browser,
    error::{IdentityError, Result},
    ledger::{self, Ledger, Operation},
    login::{Grant, LoginFlow, Origin, Poll},
    private_fs::PrivateDir,
    provider::{self, Providers, WriteResult},
};
use crate::nextcloud::{AuthenticatedBinding, LoginBinding, NetworkRuntime, NextcloudClient};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};
use talk_core::{
    bounded::{Page, Text},
    identity::{QualifiedItem, StorageChoice},
    ids::{
        AccountId, AccountScope, ClockEpoch, ConversationId, LoginAttemptId, OperationId,
        RuntimeGeneration,
    },
    lifecycle::Cancellation,
    ports::{ErrorKind, NextcloudPort, OperationContext, ProtocolReply, ProtocolRequest},
    preferences::navigation::NavigationAccountKey,
    protocol::{AccountProbe, SendReceipt, SendState},
};
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore, mpsc, oneshot};
use zeroize::Zeroizing;

pub struct RuntimeConfig {
    media: Option<Arc<crate::media::MediaService>>,
    path: Option<PathBuf>,
    fixture: bool,
    browser_fixture: bool,
    #[cfg(test)]
    pub(super) faults: Arc<provider::faults::Faults>,
}
impl RuntimeConfig {
    pub fn platform() -> Self {
        Self {
            media: None,
            path: None,
            fixture: false,
            browser_fixture: false,
            #[cfg(test)]
            faults: Arc::default(),
        }
    }
    pub fn with_media(mut self, media: Arc<crate::media::MediaService>) -> Self {
        self.media = Some(media);
        self
    }
    /// Only isolated synthetic loopback labs. Private path contains app credentials.
    #[cfg(any(test, feature = "backend-test"))]
    pub fn isolated_fixture(path: PathBuf) -> Self {
        Self {
            media: None,
            path: Some(path),
            fixture: true,
            browser_fixture: true,
            #[cfg(test)]
            faults: Arc::default(),
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountPhase {
    Disconnected,
    WaitingForBrowser,
    Authorized,
    Active,
    ReauthenticationRequired,
    StorageRecovery,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoredState {
    Unknown,
    Suppressed,
    Unresolved,
    Eligible(QualifiedItem),
    SessionOnly,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoverySelection {
    pub attempted: QualifiedItem,
    pub previous: Option<QualifiedItem>,
    pub revision: u64,
    pub settlement: talk_core::identity::Settlement,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoredItemStatus {
    pub item: QualifiedItem,
    pub operation: OperationId,
    pub retained: Option<QualifiedItem>,
    pub settlement: talk_core::identity::Settlement,
    pub eligible: bool,
    pub was_activated: bool,
    pub retired: bool,
    pub cleanup_pending: bool,
    pub last_failure: Option<talk_core::identity::StorageFailure>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountSummary {
    pub account: AccountId,
    pub navigation_key: NavigationAccountKey,
    pub server_label: Option<Text<128>>,
    pub account_label: Option<Text<128>>,
    pub attempt: Option<LoginAttemptId>,
    pub scope: Option<AccountScope>,
    pub phase: AccountPhase,
    pub display_name: Text<256>,
    pub storage: StoredState,
    pub cleanup_pending: bool,
    pub suppression_unconfirmed: bool,
    pub connection_degraded: bool,
    pub recovery: Option<RecoverySelection>,
    pub revision: u64,
}
/// Worker/composition vocabulary, never a renderer import. No credentials or wire URL.
pub enum RuntimeCommand {
    Accounts,
    StorageStatus {
        account: AccountId,
    },
    Begin {
        server: Text<2048>,
        replace: Option<AccountId>,
    },
    BeginSave {
        account: AccountId,
    },
    RetireStored {
        account: AccountId,
        item: QualifiedItem,
    },
    RetireAll {
        account: AccountId,
    },
    ForgetAccount {
        account: AccountId,
    },
    Poll {
        account: AccountId,
        attempt: LoginAttemptId,
    },
    ReopenBrowser {
        account: AccountId,
        attempt: LoginAttemptId,
    },
    Save {
        account: AccountId,
        attempt: LoginAttemptId,
        choice: StorageChoice,
    },
    Restore {
        account: AccountId,
    },
    RetainPrevious {
        account: AccountId,
        attempt: LoginAttemptId,
    },
    ResolvePrevious {
        account: AccountId,
        selection: RecoverySelection,
    },
    Maintain {
        account: AccountId,
    },
    Request {
        scope: AccountScope,
        request: ProtocolRequest,
    },
}
#[derive(Clone, Debug)]
pub enum RuntimeReply {
    Accounts(Page<AccountSummary, 8>),
    StorageStatus {
        account: AccountId,
        revision: u64,
        items: Page<StoredItemStatus, 16>,
    },
    Begun {
        account: AccountId,
        attempt: LoginAttemptId,
    },
    Pending {
        account: AccountId,
        attempt: LoginAttemptId,
    },
    Authorized {
        account: AccountId,
        attempt: LoginAttemptId,
    },
    BrowserOpened {
        account: AccountId,
        attempt: LoginAttemptId,
    },
    Active {
        summary: AccountSummary,
        probe: AccountProbe,
    },
    State(AccountSummary),
    Forgotten(AccountId),
    Protocol(ProtocolReply),
}
struct Slot {
    _permit: OwnedSemaphorePermit,
}
pub struct Observation {
    reply: oneshot::Receiver<Result<RuntimeReply>>,
    account: Arc<Mutex<Option<AccountId>>>,
    cancel: Cancellation,
    wake: Arc<Notify>,
    consumed: bool,
    cancellable: bool,
    _slot: Arc<Slot>,
}
impl Observation {
    /// Signal cancellation while retaining observation of the exact operation.
    /// Maintenance settlement remains owned and deliberately cannot be cancelled.
    pub fn cancel(&self) -> bool {
        if !self.cancellable {
            return false;
        }
        self.cancel.cancel();
        self.wake.notify_one();
        true
    }
    /// Correlates a reserved account even when the initial browser-flow request fails.
    /// None means no account was reserved (or this is an account-list operation).
    pub fn account(&self) -> Option<AccountId> {
        self.account.lock().ok().and_then(|value| *value)
    }
    pub fn try_take(&mut self) -> Option<Result<RuntimeReply>> {
        match self.reply.try_recv() {
            Ok(r) => {
                self.consumed = true;
                Some(r)
            }
            Err(oneshot::error::TryRecvError::Empty) => None,
            Err(oneshot::error::TryRecvError::Closed) => {
                self.consumed = true;
                Some(Err(IdentityError::WorkerUnavailable))
            }
        }
    }
    pub async fn wait(mut self) -> Result<RuntimeReply> {
        let r = (&mut self.reply)
            .await
            .unwrap_or(Err(IdentityError::WorkerUnavailable));
        self.consumed = true;
        r
    }
}
impl Drop for Observation {
    fn drop(&mut self) {
        if !self.consumed && self.cancellable {
            self.cancel.cancel();
            self.wake.notify_one();
        }
    }
}
struct Job {
    command: RuntimeCommand,
    account: Arc<Mutex<Option<AccountId>>>,
    reply: oneshot::Sender<Result<RuntimeReply>>,
    cancel: Cancellation,
    epoch: u64,
    owner: Arc<()>,
    _slot: Arc<Slot>,
}
struct Pending {
    attempt: LoginAttemptId,
    epoch: u64,
    creator: Arc<()>,
    cancel: Cancellation,
    handoff: Option<Cancellation>,
    operation: Option<OperationId>,
    expires: std::time::Instant,
}
impl Pending {
    fn cancelled(&self) -> bool {
        self.cancel.is_cancelled()
            || self
                .handoff
                .as_ref()
                .is_some_and(Cancellation::is_cancelled)
    }

    fn owned_by(&self, epoch: u64, attempt: Option<LoginAttemptId>, creator: &Arc<()>) -> bool {
        self.epoch == epoch
            && (attempt == Some(self.attempt) || Arc::ptr_eq(&self.creator, creator))
    }
}

// The account worker has one executing command. An attempt cancellation may
// cancel this observer only when both its attempt and account epoch match.
// A provider writer already dispatched by that command has a separate owner.
struct InFlight {
    epoch: u64,
    attempt: Option<LoginAttemptId>,
    cancel: Cancellation,
}
impl InFlight {
    fn cancel_attempt(&self, pending: &Pending) {
        if self.epoch == pending.epoch && self.attempt == Some(pending.attempt) {
            self.cancel.cancel();
        }
    }
}
#[derive(Clone, Copy)]
struct Suppression {
    session: bool,
    attempted: Option<OperationId>,
    cleanup_all: bool,
}
struct Control {
    account: AccountId,
    slots: Arc<Semaphore>,
    epoch: AtomicU64,
    close: AtomicBool,
    rejected: AtomicBool,
    removed: AtomicBool,
    changed: Notify,
    inflight: Mutex<Option<InFlight>>,
    runtime: Mutex<Option<(AccountScope, Cancellation)>>,
    pending: Mutex<Option<Pending>>,
    // A failed restrictive commit is an owned admission obligation, not UI state.
    suppression: Mutex<Option<Suppression>>,
    #[cfg(test)]
    faults: Arc<provider::faults::Faults>,
}
impl Control {
    fn rejection_handler(
        self: &Arc<Self>,
        wake: Arc<Notify>,
    ) -> crate::nextcloud::AccountRejection {
        let owner = Arc::downgrade(self);
        Arc::new(move |scope| {
            let Some(control) = owner.upgrade() else {
                return;
            };
            // Serialize with activation/replacement. An obsolete call cannot
            // reject a newly authorized runtime of the same account.
            let Ok(runtime) = control.runtime.lock() else {
                return;
            };
            let Some((_, cancel)) = runtime.as_ref().filter(|(current, _)| *current == scope)
            else {
                return;
            };
            control.rejected.store(true, Ordering::Release);
            cancel.cancel();
            drop(runtime);
            wake.notify_one();
        })
    }
    fn suppress(&self, ledger: &Mutex<Ledger>, decision: Suppression) -> Result<()> {
        let mut pending = lock(&self.suppression)?;
        *pending = Some(match *pending {
            Some(old) => Suppression {
                session: old.session && decision.session,
                attempted: decision.attempted.or(old.attempted),
                cleanup_all: old.cleanup_all
                    || decision.cleanup_all
                    || (old.attempted.is_some()
                        && decision.attempted.is_some()
                        && old.attempted != decision.attempted),
            },
            None => decision,
        });
        self.commit_suppression(ledger, &mut pending)
    }
    fn commit_suppression(
        &self,
        ledger: &Mutex<Ledger>,
        pending: &mut Option<Suppression>,
    ) -> Result<()> {
        if let Some(d) = *pending {
            #[cfg(test)]
            let _delayed_commit = self.faults.before_suppression();
            lock(ledger)?
                .suppress(self.account, d.session, d.attempted, d.cleanup_all)
                .map_err(|_| IdentityError::SuppressionUnconfirmed)?;
            // This exact decision remained owned across the synchronous commit.
            // A concurrent new close still has its separate priority flag/epoch.
            *pending = None;
        }
        Ok(())
    }
    fn retry_suppression(&self, ledger: &Mutex<Ledger>) -> Result<()> {
        self.commit_suppression(ledger, &mut *lock(&self.suppression)?)
    }
    fn suppression_pending(&self) -> Result<bool> {
        Ok(self.close.load(Ordering::Acquire) || lock(&self.suppression)?.is_some())
    }
    fn abandon_cancelled_pending(&self, ledger: &Mutex<Ledger>) -> Result<bool> {
        let closed = {
            let mut pending = lock(&self.pending)?;
            if pending.as_ref().is_some_and(Pending::cancelled) {
                pending.take()
            } else {
                None
            }
        };
        let Some(attempt) = closed else {
            return Ok(false);
        };
        // A Save validates with a separate command token. Retiring its unread
        // handoff also cancels that exact in-flight command, never another epoch.
        if let Some(inflight) = lock(&self.inflight)?.as_ref() {
            inflight.cancel_attempt(&attempt);
        }
        if attempt.operation.is_some() {
            // Both owners use the same current-thread executor, with no await
            // between taking Pending and installing this restrictive decision.
            // Release the UI-facing pending lock before any durable I/O.
            let _ = self.suppress(
                ledger,
                Suppression {
                    session: false,
                    attempted: attempt.operation,
                    cleanup_all: false,
                },
            );
        }
        // An unread grant without a storage intent has no N to suppress and
        // must leave an independently eligible/live P untouched.
        Ok(true)
    }
    fn close(&self) {
        self.changed.notify_one();
        self.epoch.fetch_add(1, Ordering::AcqRel);
        self.close.store(true, Ordering::Release);
        if let Ok(c) = self.inflight.lock()
            && let Some(inflight) = c.as_ref()
        {
            inflight.cancel.cancel();
        }
        if let Ok(c) = self.runtime.lock()
            && let Some((_, c)) = c.as_ref()
        {
            c.cancel();
        }
        if let Ok(c) = self.pending.lock()
            && let Some(c) = c.as_ref()
        {
            c.cancel.cancel();
        }
    }
}
struct Shared {
    summaries: Mutex<Page<AccountSummary, 8>>,
    controls: Mutex<Vec<Arc<Control>>>,
    failure: Mutex<Option<IdentityError>>,
    closing: AtomicBool,
    wake: Arc<Notify>,
}
pub struct IdentityRuntime {
    send: mpsc::Sender<Job>,
    shared: Arc<Shared>,
    root_slots: Arc<Semaphore>,
    done: std::sync::mpsc::Receiver<()>,
    thread: Option<thread::JoinHandle<()>>,
}
impl IdentityRuntime {
    /// Starts one owned background thread. All directory/SQLite/network work runs there.
    pub fn start(config: RuntimeConfig) -> Result<Self> {
        let (send, receive) = mpsc::channel(64);
        let (done_tx, done) = std::sync::mpsc::sync_channel(1);
        let shared = Arc::new(Shared {
            summaries: Mutex::new(Page::empty()),
            controls: Mutex::new(Vec::new()),
            failure: Mutex::new(None),
            closing: AtomicBool::new(false),
            wake: Arc::new(Notify::new()),
        });
        let state = shared.clone();
        let thread = thread::Builder::new()
            .name("talk-identity".into())
            .spawn(move || {
                let result = (|| {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .max_blocking_threads(2)
                        .build()
                        .map_err(|_| IdentityError::WorkerUnavailable)?;
                    let result = runtime.block_on(run(config, state.clone(), receive));
                    // Blocking native operations cannot be killed or treated as settled.
                    runtime.shutdown_timeout(Duration::from_secs(3));
                    result
                })();
                if let Err(e) = result
                    && let Ok(mut error) = state.failure.lock()
                {
                    *error = Some(e);
                }
                let _ = done_tx.send(());
            })
            .map_err(|_| IdentityError::WorkerUnavailable)?;
        Ok(Self {
            send,
            shared,
            root_slots: Arc::new(Semaphore::new(7)),
            done,
            thread: Some(thread),
        })
    }
    pub fn failure(&self) -> Option<IdentityError> {
        self.shared.failure.lock().ok().and_then(|e| *e)
    }
    pub fn snapshot(&self) -> Page<AccountSummary, 8> {
        self.shared
            .summaries
            .lock()
            .map(|s| s.clone())
            .unwrap_or_else(|_| Page::empty())
    }
    pub fn try_submit(&self, command: RuntimeCommand) -> Result<Observation> {
        if self.shared.closing.load(Ordering::Acquire) {
            return Err(ErrorKind::Cancelled.into());
        }
        if let Some(e) = self.failure() {
            return Err(e);
        }
        let account = command_account(&command);
        let mut epoch = 0;
        let slots = if let Some(id) = account {
            let controls = self
                .shared
                .controls
                .lock()
                .map_err(|_| IdentityError::WorkerUnavailable)?;
            controls
                .iter()
                .find(|c| c.account == id && !c.removed.load(Ordering::Acquire))
                .map(|c| {
                    if matches!(command, RuntimeCommand::RetireAll { .. }) {
                        c.close();
                        self.shared.wake.notify_one();
                    }
                    epoch = c.epoch.load(Ordering::Acquire);
                    c.slots.clone()
                })
                .ok_or(ErrorKind::NotFound)?
        } else {
            self.root_slots.clone()
        };
        let permit = slots.try_acquire_owned().map_err(|_| ErrorKind::Busy)?;
        let slot = Arc::new(Slot { _permit: permit });
        let cancel = Cancellation::default();
        let (reply, rx) = oneshot::channel();
        let account = Arc::new(Mutex::new(account));
        let cancellable = !matches!(command, RuntimeCommand::Maintain { .. });
        let observation = Observation {
            account: account.clone(),
            cancellable,
            reply: rx,
            cancel: cancel.clone(),
            wake: self.shared.wake.clone(),
            consumed: false,
            _slot: slot.clone(),
        };
        self.send
            .try_send(Job {
                account,
                command,
                reply,
                cancel,
                epoch,
                owner: Arc::new(()),
                _slot: slot,
            })
            .map_err(|_| ErrorKind::Busy)?;
        Ok(observation)
    }
    /// Priority local closure is immediate and has no ordinary mailbox dependency.
    /// The snapshot later reports durable suppression and separately pending cleanup.
    pub fn logout(&self, account: AccountId) -> Result<()> {
        let controls = self
            .shared
            .controls
            .lock()
            .map_err(|_| IdentityError::WorkerUnavailable)?;
        let control = controls
            .iter()
            .find(|c| c.account == account)
            .ok_or(ErrorKind::NotFound)?;
        control.close();
        self.shared.wake.notify_one();
        Ok(())
    }
    /// Closes only this pending attempt. Suppression has a reserved supervisor path.
    pub fn cancel_attempt(&self, account: AccountId, attempt: LoginAttemptId) -> Result<()> {
        let controls = lock(&self.shared.controls)?;
        let control = controls
            .iter()
            .find(|c| c.account == account)
            .ok_or(ErrorKind::NotFound)?;
        let pending = lock(&control.pending)?;
        let pending = pending
            .as_ref()
            .filter(|p| p.attempt == attempt)
            .ok_or(ErrorKind::WrongScope)?;
        pending.cancel.cancel();
        if let Some(inflight) = lock(&control.inflight)?.as_ref() {
            inflight.cancel_attempt(pending);
        }
        self.shared.wake.notify_one();
        Ok(())
    }
    /// Composition/media owner receives only this exact admitted account's closure token.
    pub fn account_cancellation(&self, scope: AccountScope) -> Result<Cancellation> {
        if self.shared.closing.load(Ordering::Acquire) {
            return Err(ErrorKind::Cancelled.into());
        }
        let controls = lock(&self.shared.controls)?;
        let control = controls
            .iter()
            .find(|c| c.account == scope.account)
            .ok_or(ErrorKind::NotFound)?;
        let current = lock(&control.runtime)?;
        match current.as_ref() {
            Some((bound, cancel)) if *bound == scope && !cancel.is_cancelled() => {
                Ok(cancel.clone())
            }
            _ => Err(ErrorKind::WrongScope.into()),
        }
    }
    #[cfg(test)]
    pub(super) fn expire_pending(&self, account: AccountId) {
        let controls = self.shared.controls.lock().unwrap();
        let control = controls.iter().find(|c| c.account == account).unwrap();
        if let Some(p) = control.pending.lock().unwrap().as_mut() {
            p.expires = std::time::Instant::now();
        }
        control.changed.notify_one();
    }
    pub fn shutdown(&self) {
        self.shared.closing.store(true, Ordering::Release);
        self.shared.wake.notify_one();
    }
    /// Nonblocking worker termination observation. A stopped supervisor does not
    /// prove that a native provider settled; durable unresolved markers survive.
    pub fn is_stopped(&self) -> bool {
        self.thread
            .as_ref()
            .is_none_or(thread::JoinHandle::is_finished)
    }
    /// Composition background owner may join; never invoke this blocking method on GPUI.
    pub fn join(mut self) -> bool {
        self.shutdown();
        let ended = self.done.recv_timeout(Duration::from_secs(7)).is_ok();
        if ended && let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        ended
    }
}
impl Drop for IdentityRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}
fn command_account(command: &RuntimeCommand) -> Option<AccountId> {
    match command {
        RuntimeCommand::Accounts => None,
        RuntimeCommand::Begin { replace, .. } => *replace,
        RuntimeCommand::StorageStatus { account }
        | RuntimeCommand::BeginSave { account }
        | RuntimeCommand::RetireStored { account, .. }
        | RuntimeCommand::RetireAll { account }
        | RuntimeCommand::ForgetAccount { account }
        | RuntimeCommand::Poll { account, .. }
        | RuntimeCommand::ReopenBrowser { account, .. }
        | RuntimeCommand::Save { account, .. }
        | RuntimeCommand::Restore { account }
        | RuntimeCommand::RetainPrevious { account, .. }
        | RuntimeCommand::ResolvePrevious { account, .. }
        | RuntimeCommand::Maintain { account } => Some(*account),
        RuntimeCommand::Request { scope, .. } => Some(scope.account),
    }
}
fn lock<T>(m: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>> {
    m.lock().map_err(|_| IdentityError::WorkerUnavailable)
}
fn path(config: &RuntimeConfig) -> Result<PathBuf> {
    if let Some(p) = &config.path {
        return Ok(p.clone());
    }
    let base = dirs::data_local_dir().ok_or(IdentityError::RecoveryRequired)?;
    let home = dirs::home_dir().ok_or(IdentityError::RecoveryRequired)?;
    if !base.is_absolute()
        || !base.starts_with(home)
        || base
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(IdentityError::RecoveryRequired);
    }
    if base.components().any(|c| {
        matches!(
            c.as_os_str().to_str(),
            Some("tmp" | "Temp" | "Dropbox" | "OneDrive" | "CloudStorage" | "Mobile Documents")
        )
    }) {
        return Err(IdentityError::RecoveryRequired);
    }
    Ok(base.join("org.adapt2move.nextcloud-modern-talk"))
}
struct ProviderPermit {
    _global: OwnedSemaphorePermit,
    _account: OwnedSemaphorePermit,
}
struct ProbePlan {
    due: tokio::time::Instant,
    retries: u8,
}
struct SendRecord {
    operation: OperationId,
    conversation: ConversationId,
    state: SendState,
}
struct Worker {
    media: Option<Arc<crate::media::MediaService>>,
    account: AccountId,
    epoch: u64,
    control: Arc<Control>,
    ledger: Arc<Mutex<Ledger>>,
    shared: Arc<Shared>,
    network: NetworkRuntime,
    providers: Arc<Providers>,
    capacity: Arc<Semaphore>,
    provider_slot: Arc<Semaphore>,
    quarantine: Arc<Mutex<Vec<ProviderPermit>>>,
    fixture: bool,
    browser_fixture: bool,
    flow: Option<(LoginAttemptId, LoginFlow)>,
    grant: Option<(LoginAttemptId, Grant)>,
    operation: Option<Operation>,
    client: Option<NextcloudClient>,
    probe: Option<AccountProbe>,
    active_item: Option<QualifiedItem>,
    active_grant: Option<Grant>,
    health_probe: Option<ProbePlan>,
    connection_degraded: bool,
    sends: Vec<SendRecord>,
    create_uncertain: bool,
    tag_create_uncertain: bool,
}
async fn run(
    config: RuntimeConfig,
    shared: Arc<Shared>,
    mut jobs: mpsc::Receiver<Job>,
) -> Result<()> {
    let directory = Arc::new(PrivateDir::open(&path(&config)?, config.fixture)?);
    let ledger = Ledger::open(directory.clone())?;
    let providers = Providers::new(directory, ledger.lock.clone());
    #[cfg(test)]
    let providers = Providers {
        faults: config.faults.clone(),
        ..providers
    };
    let providers = Arc::new(providers);
    let network = NetworkRuntime::new(ClockEpoch::from_local(ledger::random()?));
    let accounts = ledger.accounts()?;
    let ledger = Arc::new(Mutex::new(ledger));
    let capacity = Arc::new(Semaphore::new(2));
    let quarantine = Arc::new(Mutex::new(Vec::new()));
    let mut workers: Vec<(AccountId, mpsc::Sender<Job>)> = Vec::new();
    let mut tasks = tokio::task::JoinSet::new();
    let add =
        |account: AccountId, tasks: &mut tokio::task::JoinSet<()>| -> Result<mpsc::Sender<Job>> {
            let control = Arc::new(Control {
                account,
                slots: Arc::new(Semaphore::new(7)),
                epoch: AtomicU64::new(0),
                close: AtomicBool::new(false),
                rejected: AtomicBool::new(false),
                removed: AtomicBool::new(false),
                changed: Notify::new(),
                inflight: Mutex::new(None),
                runtime: Mutex::new(None),
                pending: Mutex::new(None),
                suppression: Mutex::new(None),
                #[cfg(test)]
                faults: providers.faults.clone(),
            });
            lock(&shared.controls)?.push(control.clone());
            let (send, receive) = mpsc::channel(7);
            let provider_slot = Arc::new(Semaphore::new(1));
            let uncertain = lock(&ledger)?
                .operations(account)?
                .iter()
                .filter(|o| o.settlement == 0)
                .count();
            if uncertain > 1 {
                return Err(IdentityError::RecoveryRequired);
            }
            if uncertain == 1 {
                lock(&quarantine)?.push(ProviderPermit {
                    _global: capacity
                        .clone()
                        .try_acquire_owned()
                        .map_err(|_| IdentityError::RecoveryRequired)?,
                    _account: provider_slot
                        .clone()
                        .try_acquire_owned()
                        .map_err(|_| IdentityError::RecoveryRequired)?,
                });
            }
            let worker = Worker {
                media: config.media.clone(),
                account,
                epoch: 0,
                control,
                ledger: ledger.clone(),
                shared: shared.clone(),
                network: network.clone(),
                providers: providers.clone(),
                capacity: capacity.clone(),
                provider_slot,
                quarantine: quarantine.clone(),
                fixture: config.fixture,
                browser_fixture: config.browser_fixture,
                flow: None,
                grant: None,
                operation: None,
                client: None,
                probe: None,
                active_item: None,
                active_grant: None,
                health_probe: None,
                connection_degraded: false,
                sends: Vec::new(),
                create_uncertain: false,
                tag_create_uncertain: false,
            };
            worker.publish(AccountPhase::Disconnected)?;
            tasks.spawn(worker.run(receive));
            Ok(send)
        };
    for account in &accounts {
        workers.push((account.id, add(account.id, &mut tasks)?));
    }
    loop {
        while tasks.try_join_next().is_some() {}
        priority(&shared, &ledger)?;
        if shared.closing.load(Ordering::Acquire) {
            break;
        }
        tokio::select! {
            _=shared.wake.notified()=>{},
            _ = tasks.join_next(), if !tasks.is_empty() => {},
            next=jobs.recv()=>{
                let Some(mut job)=next else{break;};
                priority(&shared,&ledger)?;
                if job.cancel.is_cancelled(){
                    if let RuntimeCommand::Poll{account,attempt}|RuntimeCommand::ReopenBrowser{account,attempt}|RuntimeCommand::Save{account,attempt,..}|RuntimeCommand::RetainPrevious{account,attempt}=&job.command {
                        let controls=lock(&shared.controls)?;
                        if let Some(control)=controls.iter().find(|c|c.account==*account)
                            && let Some(pending)=lock(&control.pending)?.as_ref().filter(|p|p.attempt==*attempt && p.epoch==job.epoch && control.epoch.load(Ordering::Acquire)==job.epoch) {
                            pending.cancel.cancel();
                        }
                    }
                    priority(&shared,&ledger)?;
                    let _=job.reply.send(Err(ErrorKind::Cancelled.into()));continue;
                }
                if matches!(job.command,RuntimeCommand::Accounts){let _=job.reply.send(Ok(RuntimeReply::Accounts(lock(&shared.summaries)?.clone())));continue;}
                if let RuntimeCommand::Begin{server,replace:None}=&job.command {
                    if tasks.len()>=8 {let _=job.reply.send(Err(ErrorKind::Busy.into()));continue;}
                    let origin=match Origin::parse(server.as_str(),config.fixture){Ok(v)=>v,Err(e)=>{let _=job.reply.send(Err(e));continue;}};
                    let account=match lock(&ledger)?.reserve(origin.text){Ok(a)=>a,Err(e)=>{let _=job.reply.send(Err(e));continue;}};
                    *lock(&job.account)?=Some(account.id);
                    let send=add(account.id,&mut tasks)?;workers.push((account.id,send));
                    job.command=RuntimeCommand::Begin{server:server.clone(),replace:Some(account.id)};
                }
                let id=command_account(&job.command).ok_or(IdentityError::WorkerUnavailable)?;
                if matches!(job.command,RuntimeCommand::ForgetAccount{..}) {
                    let controls=lock(&shared.controls)?.clone();
                    let control=controls.iter().find(|c|c.account==id);
                    let result=if let Some(control)=control {
                        // No pending attempt, active runtime or unresolved cleanup may disappear.
                        if lock(&control.runtime)?.is_some()||lock(&control.pending)?.is_some(){Err(ErrorKind::Conflict.into())}
                        else{lock(&ledger)?.forget(id).map(|()|RuntimeReply::Forgotten(id))}
                    }else{Err(ErrorKind::NotFound.into())};
                    if result.is_ok(){
                        if let Some(control)=control{control.removed.store(true,Ordering::Release);control.close();}
                        workers.retain(|(account,_)|*account!=id);
                        lock(&shared.controls)?.retain(|c|c.account!=id);
                        let old=lock(&shared.summaries)?.clone();let mut remaining=Page::empty();
                        for value in old.as_slice().iter().filter(|v|v.account!=id){remaining.try_push(value.clone()).map_err(|_|ErrorKind::Busy)?;}
                        *lock(&shared.summaries)?=remaining;
                    }
                    let _=job.reply.send(result);continue;
                }
                if let Some((_, send)) = workers.iter().find(|(a,_)|*a==id) {
                    if let Err(error) = send.try_send(job) {
                        let job = error.into_inner();
                        let _ = job.reply.send(Err(ErrorKind::Busy.into()));
                    }
                } else {
                    let _ = job.reply.send(Err(ErrorKind::NotFound.into()));
                }
            },
        }
    }
    for control in lock(&shared.controls)?.iter() {
        if let Some((_, c)) = lock(&control.runtime)?.as_ref() {
            c.cancel();
        }
        if let Some(c) = lock(&control.pending)?.as_ref() {
            c.cancel.cancel();
        }
    }
    // Account tasks are owned here; aborting observation cannot settle provider I/O.
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    Ok(())
}
fn priority(shared: &Shared, ledger: &Mutex<Ledger>) -> Result<()> {
    let controls = lock(&shared.controls)?.clone();
    for control in &controls {
        // The final priority pass runs before shutdown cancels live runtimes.
        // A token already cancelled here still owns explicit suppression,
        // including when Quit followed cancellation before the worker woke.
        let runtime_cancelled = lock(&control.runtime)?
            .as_ref()
            .is_some_and(|(_, c)| c.is_cancelled());
        let explicitly_closed = control.close.swap(false, Ordering::AcqRel);
        let rejected = control.rejected.swap(false, Ordering::AcqRel);
        let close_account = explicitly_closed || runtime_cancelled || rejected;
        let mut changed = close_account;
        if close_account {
            if !explicitly_closed {
                control.epoch.fetch_add(1, Ordering::AcqRel);
            }
            // Install ownership before attempting the durable write. Failure retains it.
            let _ = control.suppress(
                ledger,
                Suppression {
                    session: false,
                    attempted: None,
                    cleanup_all: true,
                },
            );
            *lock(&control.runtime)? = None;
            *lock(&control.pending)? = None;
        } else if !shared.closing.load(Ordering::Acquire) {
            changed = control.abandon_cancelled_pending(ledger)?;
        }
        if !changed {
            continue;
        }
        control.changed.notify_one();
        let unconfirmed = control.suppression_pending()?;
        let l = lock(ledger)?;
        let a = l.account(control.account)?;
        let ops = l.operations(control.account)?;
        let eligible = a.eligible.and_then(|id| ops.iter().find(|o| o.id == id));
        let mut values = lock(&shared.summaries)?;
        let mut replacement = Page::empty();
        for old in values.as_slice() {
            let mut v = old.clone();
            if v.account == control.account {
                v.attempt = None;
                if close_account {
                    v.scope = None;
                }
                if rejected {
                    v.phase = AccountPhase::ReauthenticationRequired;
                } else if v.phase != AccountPhase::ReauthenticationRequired {
                    v.phase = if v.scope.is_some() {
                        AccountPhase::Active
                    } else {
                        AccountPhase::Disconnected
                    };
                }
                v.storage = if unconfirmed {
                    StoredState::Unknown
                } else {
                    match (a.disposition, a.mode, eligible) {
                        (2, _, Some(op)) => StoredState::Eligible(op.item),
                        (1, _, _) => StoredState::Unresolved,
                        (0, 0, _) if v.scope.is_some() => StoredState::SessionOnly,
                        _ => StoredState::Suppressed,
                    }
                };
                v.suppression_unconfirmed = unconfirmed;
                v.cleanup_pending = unconfirmed
                    || ops
                        .iter()
                        .any(|o| o.settlement == 0 || (o.abandoned && !o.cleanup));
                v.revision = a.revision;
            }
            replacement.try_push(v).map_err(|_| ErrorKind::Busy)?;
        }
        *values = replacement;
    }
    Ok(())
}
impl Worker {
    async fn run(mut self, mut receive: mpsc::Receiver<Job>) {
        loop {
            self.synchronize();
            if self.control.removed.load(Ordering::Acquire) {
                break;
            }
            let due = self.health_probe.as_ref().map(|p| p.due);
            let expiry = lock(&self.control.pending).ok().and_then(|p| {
                p.as_ref()
                    .filter(|p| !p.cancelled())
                    .map(|p| tokio::time::Instant::from_std(p.expires))
            });
            let job = tokio::select! {
                _ = self.control.changed.notified()=>continue,
                _ = async {match expiry{Some(time)=>tokio::time::sleep_until(time).await,None=>std::future::pending().await}}=>{if let Ok(pending)=lock(&self.control.pending)&& let Some(p)=pending.as_ref(){p.cancel.cancel();}self.shared.wake.notify_one();continue;},
                _ = async {match due{Some(time)=>tokio::time::sleep_until(time).await,None=>std::future::pending().await}}=>{self.health_probe().await;continue;},
                next=receive.recv()=>{let Some(job)=next else{break;};job}
            };
            self.synchronize();
            let attempt = match &job.command {
                RuntimeCommand::Poll { attempt, .. }
                | RuntimeCommand::ReopenBrowser { attempt, .. }
                | RuntimeCommand::Save { attempt, .. }
                | RuntimeCommand::RetainPrevious { attempt, .. } => Some(*attempt),
                _ => None,
            };
            if let Ok(mut inflight) = lock(&self.control.inflight) {
                *inflight = Some(InFlight {
                    epoch: job.epoch,
                    attempt,
                    cancel: job.cancel.clone(),
                });
            }
            let result = if job.epoch != self.control.epoch.load(Ordering::Acquire)
                || job.epoch != self.epoch
            {
                Err(ErrorKind::Cancelled.into())
            } else {
                self.execute(&job.command, &job.cancel, &job.owner).await
            };
            if let Ok(mut inflight) = lock(&self.control.inflight) {
                *inflight = None;
            }
            if job.cancel.is_cancelled() {
                let owned = lock(&self.control.pending).ok().and_then(|pending| {
                    pending
                        .as_ref()
                        .filter(|p| {
                            self.epoch == job.epoch && p.owned_by(job.epoch, attempt, &job.owner)
                        })
                        .map(|p| {
                            p.cancel.cancel();
                            p.attempt
                        })
                });
                if let Some(id) = owned {
                    // A forwarded cancelled stale command owns neither a newer
                    // waiting flow nor its consumed grant. A creator is tracked
                    // explicitly, including BeginSave; command kind is not proof.
                    if self.flow.as_ref().is_some_and(|(a, _)| *a == id) {
                        self.flow = None;
                    }
                    if self.grant.as_ref().is_some_and(|(a, _)| *a == id) {
                        self.grant = None;
                    }
                    let _ = self.control.abandon_cancelled_pending(&self.ledger);
                    self.shared.wake.notify_one();
                }
                let _ = job.reply.send(match result {
                    Err(error) => Err(error),
                    Ok(_) => Err(ErrorKind::Cancelled.into()),
                });
            } else if job.reply.send(result).is_err() {
                self.shared.wake.notify_one();
            }
            #[cfg(test)]
            if matches!(job.command, RuntimeCommand::Poll { .. }) {
                self.providers.faults.after_reply().await;
            }
        }
    }
    fn synchronize(&mut self) {
        let current = self.control.epoch.load(Ordering::Acquire);
        if self.epoch != current {
            self.flow = None;
            self.grant = None;
            self.operation = None;
            self.client = None;
            self.probe = None;
            self.active_item = None;
            self.active_grant = None;
            self.health_probe = None;
            self.connection_degraded = false;
            self.epoch = current;
        }
        if lock(&self.control.pending)
            .ok()
            .is_some_and(|p| p.is_none())
        {
            self.flow = None;
            self.grant = None;
        }
    }
    fn state(&self, phase: AccountPhase) -> Result<AccountSummary> {
        let unconfirmed = self.control.suppression_pending()?;
        let ledger = lock(&self.ledger)?;
        let a = ledger.account(self.account)?;
        let ops = ledger.operations(self.account)?;
        let eligible = a
            .eligible
            .map(|id| ledger.operation(self.account, id))
            .transpose()?;
        let recovery = if a.disposition == 1 {
            ops.last()
                .filter(|o| !o.abandoned && !o.activated)
                .map(|op| RecoverySelection {
                    attempted: op.item,
                    previous: op
                        .retained
                        .and_then(|id| ops.iter().find(|p| p.id == id).map(|p| p.item)),
                    revision: a.revision,
                    settlement: match op.settlement {
                        1 => talk_core::identity::Settlement::SettledPresent,
                        2 => talk_core::identity::Settlement::SettledAbsent,
                        _ => talk_core::identity::Settlement::Unknown,
                    },
                })
        } else {
            None
        };
        let scope = self
            .probe
            .as_ref()
            .filter(|_| {
                self.client
                    .as_ref()
                    .is_some_and(|c| !c.cancellation().is_cancelled())
            })
            .map(|p| p.scope);
        Ok(AccountSummary {
            account: self.account,
            navigation_key: ledger.account_key(self.account)?,
            server_label: Origin::parse(&a.origin, self.fixture)?.label(),
            account_label: ledger.account_label(self.account)?,
            attempt: lock(&self.control.pending)?
                .as_ref()
                .filter(|p| !p.cancelled() && p.epoch == self.epoch)
                .map(|p| p.attempt),
            scope,
            phase,
            display_name: self
                .probe
                .as_ref()
                .map(|p| p.display_name.clone())
                .unwrap_or(Text::new("").map_err(|_| ErrorKind::Malformed)?),
            storage: if unconfirmed {
                StoredState::Unknown
            } else {
                match (a.disposition, a.mode, eligible) {
                    (2, _, Some(o)) => StoredState::Eligible(o.item),
                    (1, _, _) => StoredState::Unresolved,
                    (0, 0, _) if scope.is_some() => StoredState::SessionOnly,
                    _ => StoredState::Suppressed,
                }
            },
            cleanup_pending: unconfirmed
                || ops
                    .iter()
                    .any(|o| o.settlement == 0 || (o.abandoned && !o.cleanup)),
            suppression_unconfirmed: unconfirmed,
            connection_degraded: self.connection_degraded,
            recovery,
            revision: a.revision,
        })
    }
    fn publish(&self, phase: AccountPhase) -> Result<AccountSummary> {
        let value = self.state(phase)?;
        let mut values = lock(&self.shared.summaries)?;
        let mut new = Page::empty();
        let mut found = false;
        for old in values.as_slice() {
            let next = if old.account == self.account {
                found = true;
                value.clone()
            } else {
                old.clone()
            };
            new.try_push(next).map_err(|_| ErrorKind::Busy)?;
        }
        if !found {
            new.try_push(value.clone()).map_err(|_| ErrorKind::Busy)?;
        }
        *values = new;
        Ok(value)
    }
    fn context_admitted(&self, cancel: &Cancellation) -> Result<()> {
        if self.control.epoch.load(Ordering::Acquire) != self.epoch
            || cancel.is_cancelled()
            || self.control.close.load(Ordering::Acquire)
            || self.shared.closing.load(Ordering::Acquire)
            || self.control.removed.load(Ordering::Acquire)
            || lock(&self.control.runtime)?
                .as_ref()
                .is_some_and(|(_, active)| active.is_cancelled())
        {
            Err(ErrorKind::Cancelled.into())
        } else {
            Ok(())
        }
    }
    fn admitted(&self, cancel: &Cancellation) -> Result<()> {
        self.context_admitted(cancel)?;
        if self.control.suppression_pending()? {
            return Err(IdentityError::SuppressionUnconfirmed);
        }
        Ok(())
    }
    fn attempt_admitted(&self, attempt: LoginAttemptId, cancel: &Cancellation) -> Result<()> {
        self.admitted(cancel)?;
        let pending = lock(&self.control.pending)?;
        if pending.as_ref().is_none_or(|p| {
            p.attempt != attempt
                || p.epoch != self.epoch
                || p.cancelled()
                || std::time::Instant::now() >= p.expires
        }) {
            return Err(ErrorKind::Cancelled.into());
        }
        Ok(())
    }
    fn provider_permit(&self) -> Result<ProviderPermit> {
        let account = self
            .provider_slot
            .clone()
            .try_acquire_owned()
            .map_err(|_| ErrorKind::Busy)?;
        let global = self
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| ErrorKind::Busy)?;
        Ok(ProviderPermit {
            _account: account,
            _global: global,
        })
    }
    fn context(&self, scope: AccountScope, cancel: Cancellation) -> OperationContext {
        let mut deadline = self.network.now();
        deadline.millis = deadline.millis.saturating_add(3000);
        OperationContext {
            scope,
            deadline,
            cancellation: cancel,
        }
    }
    async fn validate(&self, grant: &mut Grant, cancel: &Cancellation) -> Result<AccountProbe> {
        self.admitted(cancel)?;
        // A private probe-only reservation cannot collide with/replace the live account.
        let scope = AccountScope {
            account: AccountId::from_local(ledger::random()?),
            runtime: RuntimeGeneration::from_local(ledger::random()?),
        };
        let a = lock(&self.ledger)?.account(self.account)?;
        if a.origin != grant.origin.text
            || (!a.user.is_empty() && !grant.user.is_empty() && a.user != grant.user.as_str())
        {
            return Err(ErrorKind::WrongScope.into());
        }
        let expected_user = if !a.user.is_empty() {
            Some(Zeroizing::new(a.user))
        } else if !grant.user.is_empty() {
            Some(grant.user.clone())
        } else {
            None
        };
        let (user, probe) = NextcloudClient::validate_login(
            LoginBinding {
                scope,
                host: a.host,
                origin: grant.origin.transport()?,
                login_name: grant.login_name.clone(),
                password: grant.password.clone(),
                expected_user,
                cancellation: Cancellation::default(),
            },
            self.network.clone(),
            self.context(scope, cancel.clone()),
        )
        .await
        .map_err(|e| IdentityError::Request(e.kind))?;
        self.admitted(cancel)?;
        if !probe.talk_available || probe.family == talk_core::protocol::ServerFamily::Unknown {
            return Err(ErrorKind::Unsupported.into());
        }
        grant.user = user;
        Ok(probe)
    }
    async fn execute(
        &mut self,
        command: &RuntimeCommand,
        cancel: &Cancellation,
        owner: &Arc<()>,
    ) -> Result<RuntimeReply> {
        self.context_admitted(cancel)?;
        if !matches!(
            command,
            RuntimeCommand::Maintain { .. }
                | RuntimeCommand::StorageStatus { .. }
                | RuntimeCommand::RetireAll { .. }
        ) {
            self.admitted(cancel)?;
        }
        match command {
            RuntimeCommand::Accounts | RuntimeCommand::ForgetAccount { .. } => unreachable!(),
            RuntimeCommand::StorageStatus { .. } => {
                let unconfirmed = self.control.suppression_pending()?;
                let ledger = lock(&self.ledger)?;
                let account = ledger.account(self.account)?;
                let operations = ledger.operations(self.account)?;
                let mut items = Page::empty();
                for op in &operations {
                    items
                        .try_push(StoredItemStatus {
                            item: op.item,
                            operation: op.id,
                            retained: op.retained.and_then(|id| {
                                operations.iter().find(|p| p.id == id).map(|p| p.item)
                            }),
                            settlement: match op.settlement {
                                1 => talk_core::identity::Settlement::SettledPresent,
                                2 => talk_core::identity::Settlement::SettledAbsent,
                                _ => talk_core::identity::Settlement::Unknown,
                            },
                            eligible: !unconfirmed && account.eligible == Some(op.id),
                            was_activated: op.activated,
                            retired: op.retired,
                            cleanup_pending: op.settlement == 0 || (op.abandoned && !op.cleanup),
                            last_failure: op.last_failure,
                        })
                        .map_err(|_| ErrorKind::Busy)?;
                }
                Ok(RuntimeReply::StorageStatus {
                    account: self.account,
                    revision: account.revision,
                    items,
                })
            }
            RuntimeCommand::RetireAll { .. } => {
                self.control.retry_suppression(&self.ledger)?;
                // try_submit already closed local admission with priority; this is
                // explicit removal authority, separate from ordinary suppression.
                self.flow = None;
                self.grant = None;
                self.active_grant = None;
                self.client = None;
                self.probe = None;
                self.active_item = None;
                *lock(&self.control.runtime)? = None;
                *lock(&self.control.pending)? = None;
                lock(&self.ledger)?.retire_all(self.account)?;
                Ok(RuntimeReply::State(
                    self.publish(AccountPhase::Disconnected)?,
                ))
            }
            RuntimeCommand::RetireStored { item, .. } => {
                if item.account != self.account {
                    return Err(ErrorKind::WrongScope.into());
                }
                lock(&self.ledger)?.retire(*item)?;
                Ok(RuntimeReply::State(self.publish(
                    if self.client.is_some() {
                        AccountPhase::Active
                    } else {
                        AccountPhase::Disconnected
                    },
                )?))
            }
            RuntimeCommand::BeginSave { .. } => {
                if self.flow.is_some() || self.grant.is_some() {
                    return Err(ErrorKind::Busy.into());
                }
                let active = self
                    .active_grant
                    .as_ref()
                    .ok_or(ErrorKind::ReauthenticationRequired)?;
                let mut grant = Grant {
                    origin: Origin::parse(&active.origin.text, self.fixture)?,
                    user: active.user.clone(),
                    login_name: active.login_name.clone(),
                    password: active.password.clone(),
                };
                self.validate(&mut grant, cancel).await?;
                self.admitted(cancel)?;
                let attempt = LoginAttemptId::from_local(ledger::random()?);
                *lock(&self.control.pending)? = Some(Pending {
                    attempt,
                    epoch: self.epoch,
                    creator: owner.clone(),
                    cancel: cancel.clone(),
                    handoff: None,
                    operation: None,
                    expires: std::time::Instant::now() + Duration::from_secs(1200),
                });
                self.grant = Some((attempt, grant));
                self.operation = None;
                self.publish(AccountPhase::Authorized)?;
                Ok(RuntimeReply::Authorized {
                    account: self.account,
                    attempt,
                })
            }
            RuntimeCommand::Begin { server, .. } => {
                if self.flow.is_some() || self.grant.is_some() {
                    return Err(ErrorKind::Busy.into());
                }
                let origin = Origin::parse(server.as_str(), self.fixture)?;
                if origin.text != lock(&self.ledger)?.account(self.account)?.origin {
                    return Err(ErrorKind::WrongScope.into());
                }
                let attempt = LoginAttemptId::from_local(ledger::random()?);
                let flow = LoginFlow::start(origin, cancel.clone()).await?;
                self.admitted(cancel)?;
                if self.browser_fixture {
                    #[cfg(any(test, feature = "backend-test"))]
                    {
                        self.providers.directory.remove("browser.login")?;
                        self.providers
                            .directory
                            .create("browser.login", flow.browser().as_bytes())?;
                    }
                } else {
                    browser::open(flow.browser(), cancel).await?;
                }
                *lock(&self.control.pending)? = Some(Pending {
                    attempt,
                    epoch: self.epoch,
                    creator: owner.clone(),
                    cancel: cancel.clone(),
                    handoff: None,
                    operation: None,
                    expires: flow.expires(),
                });
                self.flow = Some((attempt, flow));
                self.operation = None;
                self.publish(AccountPhase::WaitingForBrowser)?;
                Ok(RuntimeReply::Begun {
                    account: self.account,
                    attempt,
                })
            }
            RuntimeCommand::Poll { attempt, .. } => {
                self.attempt_admitted(*attempt, cancel)?;
                let (id, flow) = self.flow.as_mut().ok_or(ErrorKind::NotFound)?;
                if id != attempt {
                    return Err(ErrorKind::WrongScope.into());
                }
                let outcome = flow.poll().await;
                let outcome = match outcome {
                    Ok(v) => v,
                    Err(e) => {
                        self.flow = None;
                        self.grant = None;
                        if let Some(p) = lock(&self.control.pending)?.take() {
                            p.cancel.cancel();
                        }
                        self.publish(if self.client.is_some() {
                            AccountPhase::Active
                        } else {
                            AccountPhase::Disconnected
                        })?;
                        return Err(e);
                    }
                };
                match outcome {
                    Poll::Pending => Ok(RuntimeReply::Pending {
                        account: self.account,
                        attempt: *attempt,
                    }),
                    Poll::Granted(mut grant) => {
                        self.flow = None;
                        let probe = match self.validate(&mut grant, cancel).await {
                            Ok(probe) => probe,
                            Err(e) => {
                                if let Some(p) = lock(&self.control.pending)?.take() {
                                    p.cancel.cancel();
                                }
                                return Err(e);
                            }
                        };
                        self.attempt_admitted(*attempt, cancel)?;
                        {
                            let mut ledger = lock(&self.ledger)?;
                            // A fresh login can authorize an identity already bound to
                            // another local account. Detect that before SQLite's unique
                            // constraint turns it into a misleading storage-recovery
                            // failure. Do not move the grant or alter the existing account.
                            if ledger.accounts()?.iter().any(|account| {
                                account.id != self.account
                                    && account.origin == grant.origin.text
                                    && account.user == grant.user.as_str()
                            }) {
                                return Err(ErrorKind::Conflict.into());
                            }
                            ledger.bind_user(self.account, &grant.user)?;
                        }
                        lock(&self.ledger)?.set_account_label(self.account, &probe.display_name)?;
                        self.flow = None;
                        self.grant = Some((*attempt, grant));
                        if let Some(p) = lock(&self.control.pending)?.as_mut() {
                            p.handoff = Some(cancel.clone());
                        }
                        self.publish(AccountPhase::Authorized)?;
                        Ok(RuntimeReply::Authorized {
                            account: self.account,
                            attempt: *attempt,
                        })
                    }
                }
            }
            RuntimeCommand::ReopenBrowser { attempt, .. } => {
                self.attempt_admitted(*attempt, cancel)?;
                let (id, flow) = self.flow.as_ref().ok_or(ErrorKind::NotFound)?;
                if id != attempt {
                    return Err(ErrorKind::WrongScope.into());
                }
                if !self.browser_fixture {
                    browser::open(flow.browser(), cancel).await?;
                }
                self.attempt_admitted(*attempt, cancel)?;
                Ok(RuntimeReply::BrowserOpened {
                    account: self.account,
                    attempt: *attempt,
                })
            }
            RuntimeCommand::Save {
                attempt, choice, ..
            } => self.save(*attempt, *choice, cancel).await,
            RuntimeCommand::Restore { .. } => self.restore(cancel).await,
            RuntimeCommand::RetainPrevious { attempt, .. } => self.retain(*attempt, cancel).await,
            RuntimeCommand::ResolvePrevious { selection, .. } => {
                let a = lock(&self.ledger)?.account(self.account)?;
                if selection.attempted.account != self.account
                    || a.revision != selection.revision
                    || a.disposition != 1
                {
                    return Err(ErrorKind::WrongScope.into());
                }
                let op = lock(&self.ledger)?
                    .operations(self.account)?
                    .into_iter()
                    .find(|o| o.item == selection.attempted)
                    .ok_or(ErrorKind::WrongScope)?;
                let previous = lock(&self.ledger)?.operation(
                    self.account,
                    op.retained.ok_or(IdentityError::RecoveryRequired)?,
                )?;
                let settlement = match op.settlement {
                    1 => talk_core::identity::Settlement::SettledPresent,
                    2 => talk_core::identity::Settlement::SettledAbsent,
                    _ => talk_core::identity::Settlement::Unknown,
                };
                if Some(previous.item) != selection.previous || selection.settlement != settlement {
                    return Err(ErrorKind::WrongScope.into());
                }
                self.retain_exact(op, previous, a.revision, Some(op.settlement), cancel)
                    .await
            }
            RuntimeCommand::Maintain { .. } => {
                if let Err(error) = self.control.retry_suppression(&self.ledger) {
                    self.publish_current()?;
                    return Err(error);
                }
                self.maintain().await?;
                Ok(RuntimeReply::State(self.publish(
                    if self.client.is_some() {
                        AccountPhase::Active
                    } else {
                        AccountPhase::Disconnected
                    },
                )?))
            }
            RuntimeCommand::Request { scope, request } => {
                let mut context = self.context(*scope, cancel.clone());
                // A bounded, streamed file transfer needs longer than a small JSON
                // request. Keep account cancellation and the same admitted scope;
                // this does not extend login/probe/provider deadlines.
                if matches!(
                    request,
                    ProtocolRequest::Messaging {
                        command: talk_core::protocol::messaging::MessagingCommand::UploadFile { .. }
                            | talk_core::protocol::messaging::MessagingCommand::DownloadFile { .. },
                        ..
                    }
                ) {
                    context.deadline.millis = self.network.now().millis.saturating_add(60_000);
                } else if matches!(
                    request,
                    ProtocolRequest::Content {
                        command: talk_core::protocol::content::ContentCommand::Asset(_),
                        ..
                    }
                ) {
                    // Server document thumbnails may require a cold bounded render.
                    // This keeps the admitted account and caller cancellation unchanged.
                    context.deadline.millis = self.network.now().millis.saturating_add(15_000);
                }
                if self.probe.as_ref().is_none_or(|p| p.scope != *scope) {
                    return Err(ErrorKind::WrongScope.into());
                }
                if let ProtocolRequest::SendStatus(operation) = request
                    && let Some(old) = self.sends.iter().find(|s| s.operation == *operation)
                {
                    return Ok(RuntimeReply::Protocol(ProtocolReply::Send(SendReceipt {
                        operation: *operation,
                        state: old.state,
                    })));
                }
                if let ProtocolRequest::Send(send) = request {
                    if let Some(old) = self.sends.iter().find(|s| s.operation == send.operation) {
                        if old.conversation != send.conversation {
                            return Err(ErrorKind::WrongScope.into());
                        }
                        return Ok(RuntimeReply::Protocol(ProtocolReply::Send(SendReceipt {
                            operation: old.operation,
                            state: old.state,
                        })));
                    }
                    if self.sends.len() >= 100 {
                        return Err(ErrorKind::Busy.into());
                    }
                    self.sends.push(SendRecord {
                        operation: send.operation,
                        conversation: send.conversation,
                        state: SendState::Uncertain,
                    });
                }
                if matches!(request, ProtocolRequest::CreateGroup { .. }) {
                    if self.create_uncertain {
                        return Err(ErrorKind::Uncertain.into());
                    }
                    self.create_uncertain = true;
                }
                if matches!(request, ProtocolRequest::CreateTag { .. }) {
                    if self.tag_create_uncertain {
                        return Err(ErrorKind::Uncertain.into());
                    }
                    self.tag_create_uncertain = true;
                }
                let result = self
                    .client
                    .as_mut()
                    .ok_or(ErrorKind::ReauthenticationRequired)?
                    .request(context, request.clone())
                    .await;
                match (&result, request) {
                    (Ok(ProtocolReply::Tags(_)), ProtocolRequest::ReadTags)
                    | (Ok(ProtocolReply::TagCreated(_)), ProtocolRequest::CreateTag { .. }) => {
                        self.tag_create_uncertain = false;
                    }
                    (Err(e), ProtocolRequest::CreateTag { .. })
                        if e.kind != ErrorKind::Uncertain =>
                    {
                        self.tag_create_uncertain = false;
                    }
                    (Ok(ProtocolReply::Send(receipt)), _) => {
                        if let Some(s) = self
                            .sends
                            .iter_mut()
                            .find(|s| s.operation == receipt.operation)
                        {
                            s.state = receipt.state;
                        }
                    }
                    (Err(e), ProtocolRequest::Send(send)) if e.kind != ErrorKind::Uncertain => {
                        if let Some(s) = self
                            .sends
                            .iter_mut()
                            .find(|s| s.operation == send.operation)
                        {
                            s.state = SendState::Rejected;
                        }
                    }
                    (Ok(ProtocolReply::History(page)), _) => {
                        for message in page.messages.as_slice() {
                            if let Some(operation) = message.own_operation
                                && let Some(s) =
                                    self.sends.iter_mut().find(|s| s.operation == operation)
                            {
                                s.state = SendState::ServerAccepted(message.id);
                            }
                        }
                    }
                    (Ok(ProtocolReply::Conversation(_)), ProtocolRequest::CreateGroup { .. }) => {
                        self.create_uncertain = false;
                    }
                    (Err(e), ProtocolRequest::CreateGroup { .. })
                        if e.kind != ErrorKind::Uncertain =>
                    {
                        self.create_uncertain = false;
                    }
                    _ => {}
                }
                match result {
                    Ok(mut reply) => {
                        // Layout keys are non-authoritative, account-qualified and
                        // persisted only from a server room identity, never its title.
                        self.project_navigation(&mut reply);
                        if matches!(
                            request,
                            ProtocolRequest::ProbeAccount | ProtocolRequest::InspectAccount
                        ) {
                            self.health_probe = None;
                            self.connection_degraded = false;
                            self.publish_current()?;
                        }
                        Ok(RuntimeReply::Protocol(reply))
                    }
                    Err(e) => {
                        if e.kind == ErrorKind::ReauthenticationRequired
                            || (e.kind == ErrorKind::WrongScope
                                && (matches!(
                                    request,
                                    ProtocolRequest::ProbeAccount | ProtocolRequest::InspectAccount
                                ) || self
                                    .client
                                    .as_ref()
                                    .is_some_and(|c| c.cancellation().is_cancelled())))
                        {
                            if let Some(c) = self.client.as_ref() {
                                c.cancellation().cancel();
                            }
                            self.control.close();
                            self.publish(AccountPhase::ReauthenticationRequired)?;
                            self.shared.wake.notify_one();
                        } else if ambiguous(e.kind) {
                            self.schedule_probe(e.retry);
                            self.publish_current()?;
                        }
                        Err(IdentityError::Request(e.kind))
                    }
                }
            }
        }
    }
    fn project_navigation(&self, reply: &mut ProtocolReply) {
        // Optional layout persistence must not turn an acknowledged server write
        // into a retryable error. An unavailable mapping leaves its key unknown.
        let Some(client) = self.client.as_ref() else {
            return;
        };
        let Ok(mut ledger) = lock(&self.ledger) else {
            return;
        };
        let mut project = |room: &mut talk_core::protocol::Conversation| {
            room.navigation_key = client
                .room_identity(room.id)
                .ok()
                .flatten()
                .and_then(|server_id| ledger.room_key(self.account, server_id).ok().flatten());
        };
        match reply {
            ProtocolReply::Conversation(room) => project(room),
            ProtocolReply::Conversations(list) => {
                let mut rooms = Page::empty();
                for mut room in list.rooms.as_slice().iter().cloned() {
                    project(&mut room);
                    rooms.try_push(room).expect("same conversation bound");
                }
                list.rooms = rooms;
            }
            _ => {}
        }
    }
    fn publish_current(&self) -> Result<()> {
        let old = lock(&self.shared.summaries)?
            .as_slice()
            .iter()
            .find(|a| a.account == self.account)
            .cloned()
            .ok_or(ErrorKind::NotFound)?;
        self.publish(old.phase)?;
        Ok(())
    }
    fn schedule_probe(&mut self, retry: talk_core::ports::Retry) {
        self.connection_degraded = true;
        let guidance = match retry {
            talk_core::ports::Retry::AfterDelay { millis } => {
                Duration::from_millis(u64::from(millis))
            }
            _ => Duration::ZERO,
        };
        let due = tokio::time::Instant::now() + guidance;
        match self.health_probe.as_mut() {
            Some(p) => {
                if guidance > Duration::ZERO {
                    p.due = p.due.max(due);
                }
            }
            None => self.health_probe = Some(ProbePlan { due, retries: 0 }),
        }
    }
    async fn health_probe(&mut self) {
        let Some(mut plan) = self.health_probe.take() else {
            return;
        };
        let Some(scope) = self.probe.as_ref().map(|p| p.scope) else {
            return;
        };
        if self
            .client
            .as_ref()
            .is_none_or(|c| c.cancellation().is_cancelled())
            || self.control.epoch.load(Ordering::Acquire) != self.epoch
        {
            return;
        }
        let Ok(_slot) = self.control.slots.clone().try_acquire_owned() else {
            plan.due = tokio::time::Instant::now() + Duration::from_secs(2);
            self.health_probe = Some(plan);
            return;
        };
        let cancel = Cancellation::default();
        if let Ok(mut inflight) = lock(&self.control.inflight) {
            *inflight = Some(InFlight {
                epoch: self.epoch,
                attempt: None,
                cancel: cancel.clone(),
            });
        }
        let context = self.context(scope, cancel.clone());
        if self.admitted(&cancel).is_err() {
            if let Ok(mut inflight) = lock(&self.control.inflight) {
                *inflight = None;
            }
            return;
        }
        let result = self
            .client
            .as_mut()
            .expect("client checked")
            .request(context, ProtocolRequest::ProbeAccount)
            .await;
        if let Ok(mut inflight) = lock(&self.control.inflight) {
            *inflight = None;
        }
        match result {
            Ok(ProtocolReply::AccountValid(valid))
                if valid == scope && self.admitted(&cancel).is_ok() =>
            {
                self.connection_degraded = false;
                let _ = self.publish_current();
            }
            Err(error)
                if matches!(
                    error.kind,
                    ErrorKind::ReauthenticationRequired | ErrorKind::WrongScope
                ) =>
            {
                self.control.close();
                let _ = self.publish(AccountPhase::ReauthenticationRequired);
                self.shared.wake.notify_one();
            }
            Err(error) if ambiguous(error.kind) && self.admitted(&cancel).is_ok() => {
                let base = [2000u64, 4000, 8000, 16000, 30000][usize::from(plan.retries.min(4))];
                let jitter = ledger::random()
                    .map(|n| (n % ((base / 5 + 1) as u128)) as u64)
                    .unwrap_or(0);
                let delay = (base + jitter).min(30000).max(match error.retry {
                    talk_core::ports::Retry::AfterDelay { millis } => u64::from(millis),
                    _ => 0,
                });
                plan.retries = plan.retries.saturating_add(1);
                plan.due = tokio::time::Instant::now() + Duration::from_millis(delay);
                self.health_probe = Some(plan);
            }
            _ => {}
        }
    }
    async fn provider_read(&self, item: QualifiedItem) -> Result<Zeroizing<String>> {
        let permit = self.provider_permit()?;
        let providers = self.providers.clone();
        let job = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            providers.read(item)
        });
        tokio::time::timeout(Duration::from_secs(3), job)
            .await
            .map_err(|_| IdentityError::Request(ErrorKind::Deadline))?
            .map_err(|_| IdentityError::WorkerUnavailable)?
    }
    async fn save(
        &mut self,
        attempt: LoginAttemptId,
        choice: StorageChoice,
        cancel: &Cancellation,
    ) -> Result<RuntimeReply> {
        self.attempt_admitted(attempt, cancel)?;
        let (id, grant) = self.grant.as_ref().ok_or(ErrorKind::NotFound)?;
        if *id != attempt {
            return Err(ErrorKind::WrongScope.into());
        }
        let mut credential = Grant {
            origin: Origin::parse(&grant.origin.text, self.fixture)?,
            user: grant.user.clone(),
            login_name: grant.login_name.clone(),
            password: grant.password.clone(),
        };
        let probe = self.validate(&mut credential, cancel).await?;
        // Validation yields to priority retirement. Neither its copied grant
        // nor its distinct command token can authorize an already retired intent.
        self.attempt_admitted(attempt, cancel)?;
        if choice == StorageChoice::SessionOnly {
            let committed = self.control.suppress(
                &self.ledger,
                Suppression {
                    session: true,
                    attempted: self.operation.map(|o| o.id),
                    cleanup_all: false,
                },
            );
            if committed.is_err() {
                self.publish(AccountPhase::StorageRecovery)?;
                return Err(IdentityError::SuppressionUnconfirmed);
            }
            self.attempt_admitted(attempt, cancel)?;
            return self.activate(credential, probe, cancel, Some(attempt));
        }
        let backend = if choice == StorageChoice::PrivateFile {
            talk_core::identity::StorageBackend::PrivateFile
        } else {
            provider::native_backend()
        };
        let existing = self
            .operation
            .map(|o| lock(&self.ledger).and_then(|l| l.operation(self.account, o.id)))
            .transpose()?;
        let op = if let Some(op) = existing
            .filter(|o| o.settlement == 1 && !o.abandoned && o.item.provider.backend == backend)
        {
            op
        } else {
            let permit = self.provider_permit()?;
            self.attempt_admitted(attempt, cancel)?;
            let op = lock(&self.ledger)?.stage(
                self.account,
                attempt,
                backend,
                &credential.login_name,
            )?;
            self.operation = Some(op);
            // Stage and exact registration have no await: the shared background
            // executor cannot retire Pending between them. External cancellation
            // only marks tokens; it never takes Pending or holds this lock for I/O.
            let registered = {
                let mut pending = lock(&self.control.pending)?;
                pending
                    .as_mut()
                    .filter(|p| p.attempt == attempt && p.epoch == self.epoch)
                    .map(|p| {
                        p.operation = Some(op.id);
                    })
                    .is_some()
            };
            if !registered {
                // Fail closed if the serialized invariant ever changes. No writer
                // was dispatched; retain an exact cleanup obligation for this N.
                if lock(&self.ledger)?.settle_result(op, false, None).is_err() {
                    lock(&self.quarantine)?.push(permit);
                }
                let _ = self.control.suppress(
                    &self.ledger,
                    Suppression {
                        session: false,
                        attempted: Some(op.id),
                        cleanup_all: false,
                    },
                );
                return Err(ErrorKind::Cancelled.into());
            }
            let providers = self.providers.clone();
            let password = credential.password.clone();
            let ledger = self.ledger.clone();
            let quarantine = self.quarantine.clone();
            #[cfg(test)]
            let faults = self.providers.faults.clone();
            let (complete, result) = oneshot::channel();
            // This settlement owner survives the observing command and NEVER activates.
            tokio::spawn(async move {
                let returned = tokio::task::spawn_blocking(move || {
                    let result = providers.write(op.item, &password);
                    (result, permit)
                })
                .await;
                let outcome = match returned {
                    Ok((WriteResult::Present, permit)) => {
                        let settled =
                            lock(&ledger).and_then(|mut l| l.settle_result(op, true, None));
                        if settled.is_err()
                            && let Ok(mut q) = lock(&quarantine)
                        {
                            q.push(permit);
                        }
                        settled
                    }
                    Ok((WriteResult::NotWritten(e), permit)) => {
                        let settled =
                            lock(&ledger).and_then(|mut l| l.settle_result(op, false, Some(e)));
                        if settled.is_err()
                            && let Ok(mut q) = lock(&quarantine)
                        {
                            q.push(permit);
                        }
                        Err(e)
                    }
                    Ok((WriteResult::Unknown(e), permit)) => {
                        let _ = lock(&ledger).and_then(|mut l| l.record_failure(op, e));
                        if let Ok(mut q) = lock(&quarantine) {
                            q.push(permit);
                        }
                        Err(e)
                    }
                    Err(_) => Err(IdentityError::WorkerUnavailable),
                };
                #[cfg(test)]
                faults.settled.fetch_add(1, Ordering::Release);
                let _ = complete.send(outcome);
            });
            match tokio::time::timeout(Duration::from_secs(3), result).await {
                Ok(Ok(Ok(()))) => {}
                Ok(Ok(Err(e))) => {
                    self.publish(AccountPhase::StorageRecovery)?;
                    return Err(e);
                }
                _ => {
                    self.publish(AccountPhase::StorageRecovery)?;
                    return Err(ErrorKind::Uncertain.into());
                }
            }
            op
        };
        self.admitted(cancel)?;
        // Validate both actual readable N and account again before durable eligibility.
        let read = self.provider_read(op.item).await?;
        if read.as_str() != credential.password.as_str() {
            return Err(IdentityError::RecoveryRequired);
        }
        self.validate(&mut credential, cancel).await?;
        self.attempt_admitted(attempt, cancel)?;
        let revision = lock(&self.ledger)?.account(self.account)?.revision;
        lock(&self.ledger)?.activate(op, revision)?;
        self.activate(credential, probe, cancel, Some(attempt))
    }
    fn activate(
        &mut self,
        grant: Grant,
        mut probe: AccountProbe,
        cancel: &Cancellation,
        attempt: Option<LoginAttemptId>,
    ) -> Result<RuntimeReply> {
        self.admitted(cancel)?;
        lock(&self.ledger)?.set_account_label(self.account, &probe.display_name)?;
        // The bounded account journal below survives transport/runtime replacement.
        // It never replays an old operation or clears an uncertain creation barrier.
        // Remove the retiring local cancellation owner before cancelling it. Its
        // deliberate replacement must not be observed as rejection of the new grant.
        *lock(&self.control.runtime)? = None;
        if let Some(old) = self.client.take() {
            old.cancellation().cancel();
            drop(old);
        }
        let scope = AccountScope {
            account: self.account,
            runtime: RuntimeGeneration::from_local(ledger::random()?),
        };
        let a = lock(&self.ledger)?.account(self.account)?;
        let mut client = NextcloudClient::from_identity(
            AuthenticatedBinding {
                scope,
                host: a.host,
                origin: grant.origin.transport()?,
                user: grant.user.clone(),
                login_name: grant.login_name.clone(),
                password: grant.password.clone(),
                cancellation: cancel.clone(),
            },
            self.network.clone(),
        )
        .map_err(|e| IdentityError::Request(e.kind))?;
        client.set_media_service(self.media.clone());
        client.set_account_rejection_handler(
            self.control.rejection_handler(self.shared.wake.clone()),
        );
        let active_item = a
            .eligible
            .map(|id| {
                lock(&self.ledger)?
                    .operation(self.account, id)
                    .map(|o| o.item)
            })
            .transpose()?;
        // Serialize pending cancellation with the activation handoff, without holding
        // a UI-visible lock across ledger, provider or network I/O.
        {
            let mut pending = lock(&self.control.pending)?;
            if let Some(attempt) = attempt
                && !pending.as_ref().is_some_and(|p| {
                    p.attempt == attempt
                        && p.epoch == self.epoch
                        && !p.cancelled()
                        && std::time::Instant::now() < p.expires
                })
            {
                client.cancellation().cancel();
                return Err(ErrorKind::Cancelled.into());
            }
            self.admitted(cancel)?;
            probe.scope = scope;
            self.client = Some(client);
            self.probe = Some(probe.clone());
            self.grant = None;
            self.flow = None;
            *lock(&self.control.runtime)? = Some((scope, cancel.clone()));
            *pending = None;
        }
        if let Err(e) = self.admitted(cancel) {
            if let Some(client) = self.client.as_ref() {
                client.cancellation().cancel();
            }
            return Err(e);
        }
        self.active_item = active_item;
        self.active_grant = Some(grant);
        self.health_probe = None;
        self.connection_degraded = false;
        self.operation = None;
        let summary = self.publish(AccountPhase::Active)?;
        Ok(RuntimeReply::Active { summary, probe })
    }
    async fn restore(&mut self, cancel: &Cancellation) -> Result<RuntimeReply> {
        if self
            .client
            .as_ref()
            .is_some_and(|c| !c.cancellation().is_cancelled())
        {
            return Err(ErrorKind::Conflict.into());
        }
        let a = lock(&self.ledger)?.account(self.account)?;
        if a.disposition != 2 {
            return Err(IdentityError::RecoveryRequired);
        }
        let op = lock(&self.ledger)?.operation(
            self.account,
            a.eligible.ok_or(IdentityError::RecoveryRequired)?,
        )?;
        if op.abandoned || op.settlement != 1 || op.cleanup {
            return Err(IdentityError::RecoveryRequired);
        }
        let password = self.provider_read(op.item).await?;
        self.admitted(cancel)?;
        let mut grant = Grant {
            origin: Origin::parse(&a.origin, self.fixture)?,
            user: Zeroizing::new(a.user.clone()),
            login_name: lock(&self.ledger)?.login_name(op)?,
            password,
        };
        let probe = match self.validate(&mut grant, cancel).await {
            Ok(p) => p,
            Err(e) => {
                if matches!(
                    e,
                    IdentityError::Request(
                        ErrorKind::ReauthenticationRequired | ErrorKind::WrongScope
                    )
                ) {
                    self.control.close();
                    self.publish(AccountPhase::ReauthenticationRequired)?;
                    self.shared.wake.notify_one();
                }
                return Err(e);
            }
        };
        if lock(&self.ledger)?.account(self.account)?.revision != a.revision {
            return Err(IdentityError::RecoveryRequired);
        }
        self.activate(grant, probe, cancel, None)
    }
    async fn retain(
        &mut self,
        attempt: LoginAttemptId,
        cancel: &Cancellation,
    ) -> Result<RuntimeReply> {
        self.attempt_admitted(attempt, cancel)?;
        let op = self.operation.ok_or(ErrorKind::NotFound)?;
        if op.attempt != attempt {
            return Err(ErrorKind::WrongScope.into());
        }
        let a = lock(&self.ledger)?.account(self.account)?;
        if a.disposition != 1 {
            return Err(IdentityError::RecoveryRequired);
        }
        let previous = lock(&self.ledger)?.operation(
            self.account,
            op.retained.ok_or(IdentityError::RecoveryRequired)?,
        )?;
        self.retain_exact(op, previous, a.revision, None, cancel)
            .await
    }
    async fn retain_exact(
        &mut self,
        op: Operation,
        previous: Operation,
        revision: u64,
        selected_settlement: Option<u8>,
        cancel: &Cancellation,
    ) -> Result<RuntimeReply> {
        if op.activated
            || op.abandoned
            || previous.retired
            || previous.abandoned
            || previous.cleanup
            || previous.settlement != 1
        {
            return Err(IdentityError::RecoveryRequired);
        }
        let a = lock(&self.ledger)?.account(self.account)?;
        if a.revision != revision || a.disposition != 1 {
            return Err(ErrorKind::WrongScope.into());
        }
        {
            let mut pending = lock(&self.control.pending)?;
            if pending
                .as_ref()
                .is_some_and(|p| p.attempt != op.attempt || p.operation != Some(op.id))
            {
                return Err(ErrorKind::Conflict.into());
            }
            if let Some(p) = pending.take() {
                p.cancel.cancel();
            }
        }
        self.flow = None;
        self.grant = None;
        if self.active_item == Some(previous.item)
            && self
                .client
                .as_ref()
                .is_some_and(|c| !c.cancellation().is_cancelled())
        {
            let scope = self.probe.as_ref().ok_or(ErrorKind::WrongScope)?.scope;
            let context = self.context(scope, cancel.clone());
            let outcome = self
                .client
                .as_mut()
                .ok_or(ErrorKind::ReauthenticationRequired)?
                .request(context, ProtocolRequest::ProbeAccount)
                .await;
            if let Err(e) = outcome {
                if matches!(
                    e.kind,
                    ErrorKind::ReauthenticationRequired | ErrorKind::WrongScope
                ) {
                    self.control.close();
                    self.publish(AccountPhase::ReauthenticationRequired)?;
                    self.shared.wake.notify_one();
                }
                return Err(e.kind.into());
            }
        } else {
            let password = self.provider_read(previous.item).await?;
            let mut grant = Grant {
                origin: Origin::parse(&a.origin, self.fixture)?,
                user: Zeroizing::new(a.user.clone()),
                login_name: lock(&self.ledger)?.login_name(previous)?,
                password,
            };
            self.validate(&mut grant, cancel).await?;
        }
        self.admitted(cancel)?;
        {
            let mut ledger = lock(&self.ledger)?;
            if selected_settlement.is_some_and(|expected| {
                ledger
                    .operation(self.account, op.id)
                    .map(|current| current.settlement != expected)
                    .unwrap_or(true)
            }) {
                return Err(ErrorKind::WrongScope.into());
            }
            ledger.retain(op, previous, revision)?;
        }
        self.operation = None;
        Ok(RuntimeReply::State(self.publish(
            if self.client.is_some() {
                AccountPhase::Active
            } else {
                AccountPhase::Disconnected
            },
        )?))
    }
    async fn maintain(&mut self) -> Result<()> {
        let until = tokio::time::Instant::now() + Duration::from_secs(3);
        let operations = lock(&self.ledger)?.operations(self.account)?;
        for op in operations.into_iter().filter(|o| {
            o.abandoned && (!o.activated || o.retired) && !o.cleanup && o.settlement != 0
        }) {
            if tokio::time::Instant::now() >= until {
                return Err(ErrorKind::Deadline.into());
            }
            if op.settlement == 2 {
                lock(&self.ledger)?.cleaned(op)?;
                continue;
            }
            let permit = self.provider_permit()?;
            let providers = self.providers.clone();
            let ledger = self.ledger.clone();
            let (send, receive) = oneshot::channel();
            // Exact settled abandoned N only. Completion survives observer/timeout.
            tokio::spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    let result = providers.delete(op.item);
                    (result, permit)
                })
                .await;
                let result = match result {
                    Ok((Ok(()), _permit)) => lock(&ledger).and_then(|mut l| l.cleaned(op)),
                    Ok((Err(e), _permit)) => {
                        let _ = lock(&ledger).and_then(|mut l| l.record_failure(op, e));
                        Err(e)
                    }
                    Err(_) => Err(IdentityError::WorkerUnavailable),
                };
                let _ = send.send(result);
            });
            tokio::time::timeout_at(until, receive)
                .await
                .map_err(|_| IdentityError::Request(ErrorKind::Deadline))?
                .map_err(|_| IdentityError::WorkerUnavailable)??;
        }
        Ok(())
    }
}

fn ambiguous(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::Transport
            | ErrorKind::Deadline
            | ErrorKind::Malformed
            | ErrorKind::ServerUnavailable
            | ErrorKind::RateLimited
            | ErrorKind::RedirectRejected
            | ErrorKind::TooLarge
            | ErrorKind::Uncertain
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closing_priority(cancelled: bool, rejected: bool) {
        let path = std::env::temp_dir().join(format!(
            "nmt-closing-priority-test-{:032x}",
            ledger::random().unwrap()
        ));
        let directory = Arc::new(PrivateDir::open(&path, true).unwrap());
        let mut ledger = Ledger::open(directory.clone()).unwrap();
        let account = ledger.reserve("https://example.invalid/".into()).unwrap();
        ledger.bind_user(account.id, "synthetic-user").unwrap();
        let op = ledger
            .stage(
                account.id,
                LoginAttemptId::from_local(ledger::random().unwrap()),
                talk_core::identity::StorageBackend::PrivateFile,
                "synthetic-user",
            )
            .unwrap();
        ledger.settle(op, true).unwrap();
        let revision = ledger.account(account.id).unwrap().revision;
        ledger.activate(op, revision).unwrap();
        let unrelated_account = ledger
            .reserve("https://other.example.invalid/".into())
            .unwrap();
        ledger
            .bind_user(unrelated_account.id, "other-synthetic-user")
            .unwrap();
        let unrelated_op = ledger
            .stage(
                unrelated_account.id,
                LoginAttemptId::from_local(ledger::random().unwrap()),
                talk_core::identity::StorageBackend::PrivateFile,
                "other-synthetic-user",
            )
            .unwrap();
        ledger.settle(unrelated_op, true).unwrap();
        let unrelated_revision = ledger.account(unrelated_account.id).unwrap().revision;
        ledger.activate(unrelated_op, unrelated_revision).unwrap();
        let cancel = Cancellation::default();
        if cancelled {
            cancel.cancel();
        }
        let scope = AccountScope {
            account: account.id,
            runtime: RuntimeGeneration::from_local(1),
        };
        let control = Arc::new(Control {
            account: account.id,
            slots: Arc::new(Semaphore::new(7)),
            epoch: AtomicU64::new(0),
            close: AtomicBool::new(false),
            rejected: AtomicBool::new(false),
            removed: AtomicBool::new(false),
            changed: Notify::new(),
            inflight: Mutex::new(None),
            runtime: Mutex::new(Some((scope, cancel.clone()))),
            pending: Mutex::new(None),
            suppression: Mutex::new(None),
            faults: Arc::default(),
        });
        let unrelated_cancel = Cancellation::default();
        let unrelated_scope = AccountScope {
            account: unrelated_account.id,
            runtime: RuntimeGeneration::from_local(1),
        };
        let unrelated = Arc::new(Control {
            account: unrelated_account.id,
            slots: Arc::new(Semaphore::new(7)),
            epoch: AtomicU64::new(0),
            close: AtomicBool::new(false),
            rejected: AtomicBool::new(false),
            removed: AtomicBool::new(false),
            changed: Notify::new(),
            inflight: Mutex::new(None),
            runtime: Mutex::new(Some((unrelated_scope, unrelated_cancel.clone()))),
            pending: Mutex::new(None),
            suppression: Mutex::new(None),
            faults: Arc::default(),
        });
        let mut summaries = Page::empty();
        summaries
            .try_push(AccountSummary {
                account: account.id,
                navigation_key: ledger.account_key(account.id).unwrap(),
                server_label: None,
                account_label: None,
                attempt: None,
                scope: Some(scope),
                phase: AccountPhase::Active,
                display_name: Text::new("Synthetic").unwrap(),
                storage: StoredState::Eligible(op.item),
                cleanup_pending: false,
                suppression_unconfirmed: false,
                connection_degraded: false,
                recovery: None,
                revision: ledger.account(account.id).unwrap().revision,
            })
            .unwrap();
        let mut unrelated_summary = summaries.as_slice()[0].clone();
        unrelated_summary.account = unrelated_account.id;
        unrelated_summary.navigation_key = ledger.account_key(unrelated_account.id).unwrap();
        unrelated_summary.scope = Some(unrelated_scope);
        unrelated_summary.storage = StoredState::Eligible(unrelated_op.item);
        unrelated_summary.revision = ledger.account(unrelated_account.id).unwrap().revision;
        summaries.try_push(unrelated_summary).unwrap();
        let shared = Shared {
            summaries: Mutex::new(summaries),
            controls: Mutex::new(vec![control.clone(), unrelated.clone()]),
            failure: Mutex::new(None),
            closing: AtomicBool::new(true),
            wake: Arc::new(Notify::new()),
        };
        if rejected {
            let handler = control.rejection_handler(shared.wake.clone());
            handler(AccountScope {
                runtime: RuntimeGeneration::from_local(2),
                ..scope
            });
            assert!(
                !cancel.is_cancelled(),
                "obsolete call cannot reject the current runtime"
            );
            handler(scope);
            assert!(
                cancel.is_cancelled(),
                "local shutdown precedes durable suppression"
            );
            assert!(!unrelated_cancel.is_cancelled());
        }
        let ledger = Mutex::new(ledger);
        priority(&shared, &ledger).unwrap();
        let closed = cancelled || rejected;
        assert_eq!(lock(&control.runtime).unwrap().is_none(), closed);
        assert!(!control.suppression_pending().unwrap());
        assert!(!unrelated_cancel.is_cancelled());
        assert!(lock(&unrelated.runtime).unwrap().is_some());
        let unrelated_summary = lock(&shared.summaries).unwrap().as_slice()[1].clone();
        assert_eq!(unrelated_summary.scope, Some(unrelated_scope));
        assert_eq!(unrelated_summary.phase, AccountPhase::Active);
        if rejected {
            let summaries = lock(&shared.summaries).unwrap();
            assert_eq!(
                summaries.as_slice()[0].phase,
                AccountPhase::ReauthenticationRequired
            );
            assert_eq!(summaries.as_slice()[0].scope, None);
            assert_eq!(summaries.as_slice()[0].storage, StoredState::Suppressed);
        }
        drop(ledger);
        let reopened = Ledger::open(directory).unwrap();
        let persisted = reopened.account(account.id).unwrap();
        assert_eq!(persisted.disposition, if closed { 0 } else { 2 });
        assert_eq!(persisted.eligible, if closed { None } else { Some(op.id) });
        let unrelated_persisted = reopened.account(unrelated_account.id).unwrap();
        assert_eq!(unrelated_persisted.disposition, 2);
        assert_eq!(unrelated_persisted.eligible, Some(unrelated_op.id));
        drop(reopened);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn cancel_then_quit_durably_suppresses_before_worker_shutdown() {
        closing_priority(true, false);
    }

    #[test]
    fn ordinary_quit_preserves_accepted_restore_eligibility() {
        closing_priority(false, false);
    }

    #[test]
    fn autonomous_rejection_closes_only_current_runtime_and_durably_suppresses() {
        closing_priority(false, true);
    }
}
