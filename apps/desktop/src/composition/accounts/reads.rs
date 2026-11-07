//! One deliberate selected-room GET awaiting synchronous Busy admission.
//! No admitted operation or write enters this slot. It retains bounded target
//! metadata for ten seconds, supersedes older navigation and dies with selection.
//! One terminal failure projection lives only until the next read or selection.
//! Admitted thread pages separately retain one scoped, time-bounded retry plan.
use super::messaging::Work;
use super::*;
use talk_core::{
    protocol::messaging as wire,
    ui::{conversation::RemoteState, messaging as view},
};

#[derive(Clone, Debug, Eq, PartialEq)]
enum Read {
    Thread(u128),
    ThreadOlder {
        root: u128,
        before: u128,
    },
    Context(u128),
    UnreadAfter(u128),
    Older(u128),
    Refresh,
    Pins,
    Files,
    Search {
        query: Text<256>,
        cursor: Option<Text<1024>>,
    },
    Reminders,
    Scheduled,
}
impl Read {
    fn thread_root(&self) -> Option<u128> {
        match self {
            Self::Thread(root) | Self::ThreadOlder { root, .. } => Some(*root),
            _ => None,
        }
    }

    fn capture(
        model: &Accounts,
        context: ConversationRef,
        action: &view::MessageAction,
    ) -> Option<Result<Self, SubmitError>> {
        use view::MessageAction as A;
        Some(Ok(match action {
            A::Thread(root) => Self::Thread(*root),
            A::ThreadOlder(root) => {
                let before = match model
                    .snapshot
                    .message_panel
                    .as_ref()
                    .filter(|panel| panel.context == context && panel.thread_root == Some(*root))
                    .and_then(|panel| panel.thread_older)
                {
                    Some(before) => before,
                    None => return Some(Err(SubmitError::StaleScope)),
                };
                Self::ThreadOlder {
                    root: *root,
                    before,
                }
            }
            A::Context(target) => Self::Context(*target),
            A::LocateUnreadAfter(marker) => Self::UnreadAfter(*marker),
            A::Older => Self::Older(
                match model
                    .snapshot
                    .message_panel
                    .as_ref()
                    .filter(|p| p.context == context)
                    .and_then(|p| p.older)
                {
                    Some(before) => before,
                    None => return Some(Err(SubmitError::StaleScope)),
                },
            ),
            A::Refresh => Self::Refresh,
            A::Pins => Self::Pins,
            A::Files => Self::Files,
            A::Search(query) => Self::Search {
                query: query.clone(),
                cursor: None,
            },
            A::SearchNext => match model
                .message_search
                .as_ref()
                .filter(|(c, _, cursor)| *c == context && cursor.is_some())
            {
                Some((_, query, cursor)) => Self::Search {
                    query: query.clone(),
                    cursor: cursor.clone(),
                },
                None => return Some(Err(SubmitError::StaleScope)),
            },
            A::Reminders => Self::Reminders,
            A::Scheduled => Self::Scheduled,
            _ => return None,
        }))
    }
    fn work(&self) -> Work {
        match self {
            Self::Thread(_) => Work::Thread,
            Self::ThreadOlder { root, .. } => Work::ThreadOlder(*root),
            Self::Context(id) => Work::Context(*id),
            Self::UnreadAfter(marker) => Work::UnreadContext(*marker),
            Self::Older(_) => Work::Older,
            Self::Refresh => Work::History,
            Self::Pins => Work::Pins,
            Self::Files => Work::Files,
            Self::Search { .. } => Work::Search,
            Self::Reminders => Work::Reminders,
            Self::Scheduled => Work::Scheduled,
        }
    }
    fn shared(&self) -> bool {
        matches!(
            self,
            Self::Files | Self::Search { .. } | Self::Reminders | Self::Scheduled
        )
    }
    fn command(&self, context: ConversationRef) -> Result<wire::MessagingCommand, SubmitError> {
        let id = |id: u128| -> Result<protocol::MessageId, SubmitError> {
            Ok(protocol::MessageId {
                conversation: context.conversation,
                sequence: u64::try_from(id)
                    .ok()
                    .filter(|id| *id > 0)
                    .ok_or(SubmitError::StaleScope)?,
            })
        };
        Ok(match self {
            Self::Thread(root) => wire::MessagingCommand::History {
                before: None,
                thread: Some(id(*root)?),
            },
            Self::ThreadOlder { root, before } => wire::MessagingCommand::History {
                before: Some(id(*before)?),
                thread: Some(id(*root)?),
            },
            Self::Context(target) => wire::MessagingCommand::Context(id(*target)?),
            Self::UnreadAfter(marker) => {
                wire::MessagingCommand::UnreadAfter((*marker > 0).then(|| id(*marker)).transpose()?)
            }
            Self::Older(before) => wire::MessagingCommand::History {
                before: Some(id(*before)?),
                thread: None,
            },
            Self::Refresh => wire::MessagingCommand::History {
                before: None,
                thread: None,
            },
            Self::Pins => wire::MessagingCommand::Pins,
            Self::Files => wire::MessagingCommand::SharedFiles { before: None },
            Self::Search { query, cursor } => wire::MessagingCommand::Search {
                query: query.clone(),
                cursor: cursor.clone(),
            },
            Self::Reminders => wire::MessagingCommand::Reminders,
            Self::Scheduled => wire::MessagingCommand::Scheduled,
        })
    }
}
#[derive(Clone)]
pub(super) struct QueuedRead {
    request: RequestId,
    context: ConversationRef,
    read: Read,
    deadline: Instant,
}

struct ThreadRetry {
    job: QueuedRead,
    attempts: u8,
    due: Option<Instant>,
    expires: Instant,
    failure: UnavailableReason,
}

impl ThreadRetry {
    fn schedule(&mut self, reason: UnavailableReason, now: Instant) -> bool {
        const DELAYS: [u64; 4] = [2, 4, 8, 16];
        let Some(delay) = DELAYS.get(usize::from(self.attempts)) else {
            return false;
        };
        if now >= self.expires {
            return false;
        }
        self.failure = reason;
        self.due = Some(now + Duration::from_secs(*delay));
        true
    }
}

fn transient_thread_error(error: IdentityError) -> Option<UnavailableReason> {
    let IdentityError::Request(kind) = error else {
        return None;
    };
    match kind {
        ErrorKind::Transport
        | ErrorKind::Deadline
        | ErrorKind::ServerUnavailable
        | ErrorKind::RateLimited => connection_failure(kind).map(UnavailableReason::Connection),
        ErrorKind::Busy => Some(UnavailableReason::Busy),
        _ => None,
    }
}
impl QueuedRead {
    fn attempt(
        &self,
        selected: Option<ConversationRef>,
        now: Instant,
        dispatch: impl FnOnce(&Self) -> Result<(), SubmitError>,
    ) -> Result<bool, SubmitError> {
        if selected != Some(self.context) {
            return Err(SubmitError::StaleScope);
        }
        if now >= self.deadline {
            return Err(SubmitError::Unavailable(UnavailableReason::Busy));
        }
        match dispatch(self) {
            Ok(()) => Ok(true),
            Err(SubmitError::Busy) => Ok(false),
            Err(error) => Err(error),
        }
    }
}
#[derive(Default)]
pub(super) struct Reads {
    queued: Option<QueuedRead>,
    failed: Option<(QueuedRead, UnavailableReason)>,
    thread_retry: Option<ThreadRetry>,
    // Shared list panels have one state slot; preserve a simultaneous write's
    // Saving lease while retaining the exact current read's result identity.
    shared: Option<(ConversationRef, RequestId, Work)>,
}
impl Reads {
    pub(super) fn current_shared(
        &self,
        context: ConversationRef,
        request: RequestId,
        work: Work,
    ) -> Option<bool> {
        self.shared.map(|lease| lease == (context, request, work))
    }
    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }
}
fn reason(error: SubmitError) -> UnavailableReason {
    match error {
        SubmitError::Unavailable(reason) => reason,
        SubmitError::Busy => UnavailableReason::Busy,
        SubmitError::StaleScope => UnavailableReason::PolicyStale,
    }
}
fn project(panel: &mut view::MessagePanel, job: &QueuedRead, state: RemoteState, initial: bool) {
    if panel.context != job.context {
        return;
    }
    match &job.read {
        Read::Thread(root) => {
            if initial {
                if panel.thread_root != Some(*root) {
                    super::thread_change::cancel(panel);
                    panel.thread_messages = Page::empty();
                    panel.thread_older = None;
                    panel.thread_has_newer = false;
                    panel.thread_root_message = None;
                    panel.thread_root_state = RemoteState::Idle;
                    panel.thread_details = None;
                    panel.thread_details_request = None;
                }
                panel.thread_root = Some(*root);
            }
            if panel.thread_root == Some(*root) {
                panel.thread_state = state;
            }
        }
        Read::ThreadOlder { root, .. } => {
            if panel.thread_root == Some(*root) {
                panel.thread_state = state;
            }
        }
        Read::Context(target) | Read::UnreadAfter(target) => {
            if initial {
                panel.context_load = Some(view::ContextLoad {
                    target: *target,
                    state,
                });
                panel.located = None;
            } else if let Some(load) = panel
                .context_load
                .as_mut()
                .filter(|load| load.target == *target)
            {
                load.state = state;
            }
        }
        Read::Pins => panel.pins_state = state,
        Read::Older(_) | Read::Refresh => panel.history = state,
        _ => {
            if !matches!(
                panel.state,
                RemoteState::Saving(_) | RemoteState::Uncertain(_)
            ) {
                panel.state = state;
            }
        }
    }
}
impl Accounts {
    pub(super) fn deliberate_read(
        &mut self,
        request: RequestId,
        context: ConversationRef,
        action: &view::MessageAction,
    ) -> Option<Result<(), SubmitError>> {
        let read = Read::capture(self, context, action)?;
        Some((|| {
            let read = read?;
            if self.snapshot.selected != Some(context)
                || !context.is_consistent()
                || !self
                    .snapshot
                    .message_panel
                    .as_ref()
                    .is_some_and(|p| p.context == context)
            {
                return Err(SubmitError::StaleScope);
            }
            let _ = read.command(context)?;
            if let Some(previous) = self.queued_read.queued.take()
                && let Some(panel) = self.snapshot.message_panel.as_mut()
            {
                project(
                    panel,
                    &previous,
                    RemoteState::Failed(previous.request, UnavailableReason::PolicyStale),
                    false,
                );
            }
            self.queued_read.failed = None;
            if read.shared() {
                self.queued_read.shared = Some((context, request, read.work()));
            }
            if let Read::Search { query, cursor } = &read {
                self.message_search = Some((context, query.clone(), cursor.clone()));
            }
            let job = QueuedRead {
                request,
                context,
                read,
                deadline: Instant::now() + Duration::from_secs(10),
            };
            if job.read.thread_root().is_some() {
                self.queued_read.thread_retry = None;
            }
            self.history.expire(history::now_unix());
            let cached_thread = match &job.read {
                Read::Thread(root) => self.history.get(context).and_then(|history| {
                    history
                        .thread(*root)
                        .map(|rows| (*root, rows.clone(), history.thread_root(*root).cloned()))
                }),
                _ => None,
            };
            if let Some(panel) = self.snapshot.message_panel.as_mut() {
                project(panel, &job, RemoteState::Loading(request), true);
                if let Some((root, rows, root_message)) = cached_thread
                    && panel.thread_root == Some(root)
                {
                    panel.thread_messages = rows;
                    panel.thread_root_message = root_message;
                }
            }
            // Local acceptance always has a correlated loading/failure result.
            // No operation ticket is created until start_message admits it.
            self.attempt_deliberate_read(job, Instant::now());
            self.touch();
            Ok(())
        })())
    }
    fn attempt_deliberate_read(&mut self, job: QueuedRead, now: Instant) {
        match job.attempt(self.snapshot.selected, now, |job| {
            if self.snapshot.content == ContentState::Denied {
                return Err(SubmitError::Unavailable(
                    UnavailableReason::PermissionDenied,
                ));
            }
            self.start_message(
                job.request,
                job.context,
                job.read.command(job.context)?,
                job.read.work(),
            )
        }) {
            Ok(true) => {
                if job.read.thread_root().is_some() {
                    self.queued_read.thread_retry = Some(ThreadRetry {
                        job,
                        attempts: 0,
                        due: None,
                        expires: now + Duration::from_secs(120),
                        failure: UnavailableReason::Busy,
                    });
                }
            }
            Ok(false) => self.queued_read.queued = Some(job),
            Err(error) => {
                let failed = reason(error);
                if let Some(panel) = self
                    .snapshot
                    .message_panel
                    .as_mut()
                    .filter(|p| p.context == job.context)
                {
                    project(panel, &job, RemoteState::Failed(job.request, failed), false);
                }
                self.queued_read.failed = Some((job, failed));
                self.touch();
            }
        }
    }
    pub(super) fn tick_queued_read(&mut self, now: Instant) {
        if let Some(job) = self.queued_read.queued.take() {
            if self.snapshot.selected != Some(job.context) {
                self.queued_read.clear();
                return;
            }
            self.attempt_deliberate_read(job, now);
        }
        self.tick_thread_read_retry(now);
    }

    /// The same request identity and captured page cursor survive a transient GET
    /// failure. A different room, root, or request retires this retry immediately.
    pub(super) fn settle_thread_read(
        &mut self,
        context: ConversationRef,
        request: RequestId,
        work: Work,
        error: Option<IdentityError>,
        now: Instant,
    ) {
        let Some(mut retry) = self.queued_read.thread_retry.take() else {
            return;
        };
        if retry.job.context != context
            || retry.job.request != request
            || retry.job.read.work() != work
        {
            self.queued_read.thread_retry = Some(retry);
            return;
        }
        let Some(reason) = error.and_then(transient_thread_error) else {
            return;
        };
        let current = self.snapshot.selected == Some(context)
            && self.snapshot.message_panel.as_ref().is_some_and(|panel| {
                panel.context == context
                    && panel.thread_root == retry.job.read.thread_root()
                    && matches!(panel.thread_state, RemoteState::Failed(id, _) if id == request)
            });
        if current && retry.schedule(reason, now) {
            if let Some(panel) = self.snapshot.message_panel.as_mut() {
                panel.thread_state = RemoteState::Loading(request);
            }
            self.queued_read.thread_retry = Some(retry);
            self.touch();
        }
    }

    fn tick_thread_read_retry(&mut self, now: Instant) {
        let Some(mut retry) = self.queued_read.thread_retry.take() else {
            return;
        };
        let Some(due) = retry.due else {
            self.queued_read.thread_retry = Some(retry);
            return;
        };
        let job = &retry.job;
        let current = self.snapshot.selected == Some(job.context)
            && self.snapshot.content != ContentState::Denied
            && self.snapshot.message_panel.as_ref().is_some_and(|panel| {
                panel.context == job.context
                    && panel.thread_root == job.read.thread_root()
                    && panel.thread_state == RemoteState::Loading(job.request)
            });
        if !current {
            return;
        }
        if now >= retry.expires {
            if let Some(panel) = self.snapshot.message_panel.as_mut() {
                panel.thread_state = RemoteState::Failed(job.request, retry.failure);
            }
            self.touch();
            return;
        }
        if now < due {
            self.queued_read.thread_retry = Some(retry);
            return;
        }
        if self.queued_read.queued.is_some() {
            retry.due = Some(now + Duration::from_secs(2));
            self.queued_read.thread_retry = Some(retry);
            return;
        }
        let result = job.read.command(job.context).and_then(|command| {
            self.start_message(job.request, job.context, command, job.read.work())
        });
        match result {
            Ok(()) => {
                retry.attempts += 1;
                retry.due = None;
                self.queued_read.thread_retry = Some(retry);
            }
            Err(SubmitError::Busy) => {
                retry.due = Some(now + Duration::from_secs(2));
                self.queued_read.thread_retry = Some(retry);
            }
            Err(error) => {
                if let Some(panel) = self.snapshot.message_panel.as_mut() {
                    panel.thread_state = RemoteState::Failed(job.request, reason(error));
                }
                self.touch();
            }
        }
    }
    pub(super) fn project_queued_read(&mut self, context: ConversationRef) {
        if self.snapshot.selected != Some(context) {
            return;
        }
        let projection = self
            .queued_read
            .queued
            .as_ref()
            .map(|job| (job, RemoteState::Loading(job.request)))
            .or_else(|| {
                self.queued_read
                    .failed
                    .as_ref()
                    .map(|(job, reason)| (job, RemoteState::Failed(job.request, *reason)))
            });
        if let Some((job, state)) = projection
            && let Some(panel) = self
                .snapshot
                .message_panel
                .as_mut()
                .filter(|p| p.context == context)
        {
            project(panel, job, state, false);
        }
    }
    pub(super) fn observed_deliberate_read(
        &mut self,
        context: ConversationRef,
        work: Work,
        success: bool,
    ) {
        if success
            && self
                .queued_read
                .failed
                .as_ref()
                .is_some_and(|(job, _)| job.context == context && job.read.work() == work)
        {
            self.queued_read.failed = None;
        }
    }
}

#[cfg(test)]
mod tests;
