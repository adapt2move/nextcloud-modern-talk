//! Desktop composition's single identity owner. No I/O or credentials escape this API.
//!
//! Sixteen retained jobs (including completed, unacknowledged replies) bound memory
//! and work. The runtime also charges its seven per-account/root admission slots
//! until acknowledgement. A stalled consumer gets Busy, never silent result loss.
//! Cancellation/logout/shutdown do not need an ordinary slot. This is a single
//! consumer interface; only `poll` advances it, and only a returned cursor may
//! acknowledge delivered completions. Snapshots and reply scopes confer no authority.

use super::error::Result;
use super::{
    AccountPhase, AccountSummary, IdentityError, IdentityRuntime, Observation, RuntimeCommand,
    RuntimeConfig, RuntimeReply, StoredState,
};
use talk_core::{
    bounded::Page,
    ids::{AccountId, AccountScope, LoginAttemptId, OperationId},
    lifecycle::Cancellation,
    ports::ErrorKind,
};

pub const CONNECTION_CAPACITY: usize = 16;
pub const CONNECTION_PAGE: usize = 8;

/// Monotonically increasing within one AccountConnection lifetime, starting at one.
/// UI request IDs may be mapped to these IDs by composition; neither is authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConnectionRequestId(pub u64);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
pub struct ConnectionCursor(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionState {
    Starting,
    Ready,
    Failed(IdentityError),
    Closing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SavedAccountState {
    Dormant,
    Restoring,
    WaitingForBrowser,
    AwaitingStorageChoice,
    Active,
    ReauthenticationRequired,
    RecoveryRequired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConnectionPending {
    pub request: ConnectionRequestId,
    pub account: Option<AccountId>,
    pub attempt: Option<LoginAttemptId>,
    pub captured_scope: Option<AccountScope>,
    pub cancel_requested: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectionAccount {
    /// Durable local account identity exists even while scope is None.
    pub summary: AccountSummary,
    pub state: SavedAccountState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectionSnapshot {
    pub state: ConnectionState,
    pub accounts: Page<ConnectionAccount, 8>,
    pub pending: Page<ConnectionPending, CONNECTION_CAPACITY>,
    /// Pending and completed-but-unacknowledged jobs both consume capacity.
    pub retained: usize,
}

#[derive(Clone, Debug)]
pub struct ConnectionCompletion {
    pub cursor: ConnectionCursor,
    pub request: ConnectionRequestId,
    pub account: Option<AccountId>,
    pub attempt: Option<LoginAttemptId>,
    /// Scope captured on dispatch, never replaced by whichever account is selected.
    pub captured_scope: Option<AccountScope>,
    pub cancel_requested: bool,
    /// Recomputed on delivery. An old successful result remains observable but
    /// must not repopulate a closed/replaced account or grant a new scope.
    pub scope_is_current: bool,
    pub result: Result<RuntimeReply>,
}

#[derive(Clone, Debug)]
pub struct ConnectionUpdate {
    pub snapshot: ConnectionSnapshot,
    pub completions: Page<ConnectionCompletion, CONNECTION_PAGE>,
    /// Pass back only after consuming this page. Repeating the prior cursor
    /// redelivers that page; unacknowledged observations remain owned here.
    pub cursor: ConnectionCursor,
}

struct Retained {
    request: ConnectionRequestId,
    observation: Observation,
    attempt: Option<LoginAttemptId>,
    scope: Option<AccountScope>,
    restoring: bool,
    completion: Option<(ConnectionCursor, Result<RuntimeReply>)>,
    cancel_requested: bool,
}

pub struct AccountConnection {
    runtime: IdentityRuntime,
    startup: Option<Observation>,
    state: ConnectionState,
    jobs: Vec<Retained>,
    last_request: u64,
    next_completion: u64,
    acknowledged: ConnectionCursor,
    delivered: ConnectionCursor,
}

impl AccountConnection {
    /// Starts one IdentityRuntime. Production passes RuntimeConfig::platform().
    /// Startup readiness is separate from an initially empty account snapshot.
    pub fn start(config: RuntimeConfig) -> Result<Self> {
        let runtime = IdentityRuntime::start(config)?;
        let startup = runtime.try_submit(RuntimeCommand::Accounts)?;
        Ok(Self {
            runtime,
            startup: Some(startup),
            state: ConnectionState::Starting,
            jobs: Vec::with_capacity(CONNECTION_CAPACITY),
            last_request: 0,
            next_completion: 0,
            acknowledged: ConnectionCursor::default(),
            delivered: ConnectionCursor::default(),
        })
    }

    pub fn try_submit(
        &mut self,
        request: ConnectionRequestId,
        command: RuntimeCommand,
    ) -> Result<()> {
        if self.state != ConnectionState::Ready {
            return Err(match self.state {
                ConnectionState::Failed(error) => error,
                ConnectionState::Closing => ErrorKind::Cancelled.into(),
                _ => ErrorKind::Busy.into(),
            });
        }
        if request.0 <= self.last_request {
            return Err(ErrorKind::Conflict.into());
        }
        if self.jobs.len() >= CONNECTION_CAPACITY
            || self.next_completion > u64::MAX - CONNECTION_CAPACITY as u64
        {
            return Err(ErrorKind::Busy.into());
        }
        let attempt = match &command {
            RuntimeCommand::Poll { attempt, .. }
            | RuntimeCommand::Save { attempt, .. }
            | RuntimeCommand::RetainPrevious { attempt, .. }
            | RuntimeCommand::ReopenBrowser { attempt, .. } => Some(*attempt),
            _ => None,
        };
        let scope = match &command {
            RuntimeCommand::Request { scope, .. } => {
                self.runtime.account_cancellation(*scope)?;
                Some(*scope)
            }
            _ => None,
        };
        let restoring = matches!(
            command,
            RuntimeCommand::Restore { .. } | RuntimeCommand::RestoreExact { .. }
        );
        let observation = self.runtime.try_submit(command)?;
        self.last_request = request.0;
        self.jobs.push(Retained {
            request,
            observation,
            attempt,
            scope,
            restoring,
            completion: None,
            cancel_requested: false,
        });
        Ok(())
    }

    pub fn snapshot(&self) -> ConnectionSnapshot {
        let mut accounts = Page::empty();
        for mut summary in self.runtime.snapshot().as_slice().iter().cloned() {
            if summary
                .scope
                .is_some_and(|scope| self.account_cancellation(scope).is_err())
            {
                summary.scope = None;
            }
            let restoring = self.jobs.iter().any(|job| {
                job.restoring
                    && job.completion.is_none()
                    && !job.cancel_requested
                    && job.observation.account() == Some(summary.account)
            });
            let state = if summary.suppression_unconfirmed
                || summary.storage == StoredState::Unresolved
                || summary.phase == AccountPhase::StorageRecovery
            {
                SavedAccountState::RecoveryRequired
            } else if restoring {
                SavedAccountState::Restoring
            } else {
                match summary.phase {
                    AccountPhase::WaitingForBrowser => SavedAccountState::WaitingForBrowser,
                    AccountPhase::Authorized => SavedAccountState::AwaitingStorageChoice,
                    AccountPhase::ReauthenticationRequired => {
                        SavedAccountState::ReauthenticationRequired
                    }
                    _ if summary.scope.is_some() => SavedAccountState::Active,
                    _ => SavedAccountState::Dormant,
                }
            };
            // The input page has the same bound; no truncation is possible here.
            accounts
                .try_push(ConnectionAccount { summary, state })
                .expect("same account bound");
        }
        let mut pending = Page::empty();
        for job in self.jobs.iter().filter(|job| job.completion.is_none()) {
            pending
                .try_push(ConnectionPending {
                    request: job.request,
                    account: job.observation.account(),
                    attempt: job.attempt,
                    captured_scope: job.scope,
                    cancel_requested: job.cancel_requested,
                })
                .expect("retained job bound");
        }
        ConnectionSnapshot {
            state: self
                .runtime
                .failure()
                .map(ConnectionState::Failed)
                .unwrap_or(self.state),
            accounts,
            pending,
            retained: self.jobs.len(),
        }
    }

    pub fn poll(&mut self, cursor: ConnectionCursor) -> Result<ConnectionUpdate> {
        if cursor < self.acknowledged || cursor > self.delivered {
            return Err(ErrorKind::Conflict.into());
        }
        self.acknowledged = cursor;
        self.jobs
            .retain(|job| job.completion.as_ref().is_none_or(|(seq, _)| *seq > cursor));
        if let Some(startup) = &mut self.startup
            && let Some(result) = startup.try_take()
        {
            self.startup = None;
            if self.state != ConnectionState::Closing {
                self.state = match result {
                    Ok(RuntimeReply::Accounts(_)) => ConnectionState::Ready,
                    Err(error) => ConnectionState::Failed(error),
                    _ => ConnectionState::Failed(IdentityError::WorkerUnavailable),
                };
            }
        }
        for job in &mut self.jobs {
            if job.completion.is_some() {
                continue;
            }
            if let Some(result) = job.observation.try_take() {
                if let Ok(
                    RuntimeReply::Begun { attempt, .. }
                    | RuntimeReply::Authorized { attempt, .. }
                    | RuntimeReply::Pending { attempt, .. },
                ) = &result
                {
                    job.attempt = Some(*attempt);
                }
                self.next_completion += 1; // admitted with space for every retained job
                job.completion = Some((ConnectionCursor(self.next_completion), result));
            }
        }
        let mut completed: Vec<_> = self
            .jobs
            .iter()
            .filter(|job| job.completion.is_some())
            .collect();
        completed.sort_by_key(|job| job.completion.as_ref().map(|(seq, _)| *seq));
        let mut completions = Page::empty();
        let mut next = cursor;
        for job in completed.into_iter().take(CONNECTION_PAGE) {
            let (sequence, result) = job.completion.as_ref().expect("completed job");
            let returned_scope = match result {
                Ok(RuntimeReply::Active { summary, .. }) => summary.scope,
                _ => job.scope,
            };
            completions
                .try_push(ConnectionCompletion {
                    cursor: *sequence,
                    request: job.request,
                    account: job.observation.account(),
                    attempt: job.attempt,
                    captured_scope: job.scope,
                    cancel_requested: job.cancel_requested,
                    scope_is_current: returned_scope
                        .is_some_and(|scope| self.account_cancellation(scope).is_ok()),
                    result: result.clone(),
                })
                .expect("completion page bound");
            next = *sequence;
        }
        self.delivered = self.delivered.max(next);
        Ok(ConnectionUpdate {
            snapshot: self.snapshot(),
            completions,
            cursor: next,
        })
    }

    /// Retains the job and its eventual result. This also cancels an unread successful
    /// activation; a consumed/acknowledged activation is closed by logout instead.
    pub fn cancel_request(&mut self, request: ConnectionRequestId) -> Result<()> {
        let job = self
            .jobs
            .iter_mut()
            .find(|job| job.request == request)
            .ok_or(ErrorKind::NotFound)?;
        if !job.observation.cancel() {
            return Err(ErrorKind::Conflict.into());
        }
        job.cancel_requested = true;
        Ok(())
    }

    pub fn cancel_attempt(&self, account: AccountId, attempt: LoginAttemptId) -> Result<()> {
        self.runtime.cancel_attempt(account, attempt)
    }

    pub fn logout(&self, account: AccountId) -> Result<()> {
        self.runtime.logout(account)
    }

    pub fn account_cancellation(&self, scope: AccountScope) -> Result<Cancellation> {
        if self.state == ConnectionState::Closing {
            return Err(ErrorKind::Cancelled.into());
        }
        self.runtime.account_cancellation(scope)
    }

    /// Allocate once for one user send and retain it through uncertainty/status
    /// reconciliation. This random correlation ID grants no account authority.
    pub fn new_operation_id(&self, scope: AccountScope) -> Result<OperationId> {
        self.account_cancellation(scope)?;
        Ok(OperationId::from_local(super::ledger::random()?))
    }

    /// Correlation for a saved-account local intent. Dispatch must independently
    /// obtain the current live scope; a queue identifier is never authorization.
    pub fn new_local_operation_id(&self, account: AccountId) -> Result<OperationId> {
        if self.state != ConnectionState::Ready
            || !self.runtime.snapshot().as_slice().iter().any(|s| {
                s.account == account
                    && matches!(s.storage, super::runtime::StoredState::Eligible(_))
            })
        {
            return Err(ErrorKind::WrongScope.into());
        }
        Ok(OperationId::from_local(super::ledger::random()?))
    }

    pub fn shutdown(&mut self) {
        self.state = ConnectionState::Closing;
        self.runtime.shutdown();
    }

    /// Nonblocking supervisor termination observation, not a provider settlement
    /// claim. After shutdown, composition may poll this before dropping the owner.
    pub fn is_stopped(&self) -> bool {
        self.runtime.is_stopped()
    }

    /// Blocking composition teardown only, never a renderer call. Ordinary drop
    /// requests shutdown; it does not report that providers have settled.
    pub fn join(self) -> bool {
        self.runtime.join()
    }
}
