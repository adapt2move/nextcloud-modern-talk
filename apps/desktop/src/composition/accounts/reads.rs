//! One deliberate selected-room GET awaiting synchronous Busy admission.
//! No admitted operation or write enters this slot. It retains bounded target
//! metadata for ten seconds, supersedes older navigation and dies with selection.
//! One terminal failure projection lives only until the next read or selection;
//! it never occupies an admission slot or silently retries after that deadline.
use super::messaging::Work;
use super::*;
use talk_core::{
    protocol::messaging as wire,
    ui::{conversation::RemoteState, messaging as view},
};

#[derive(Clone, Debug, Eq, PartialEq)]
enum Read {
    Thread(u128),
    Context(u128),
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
    fn capture(
        model: &Accounts,
        context: ConversationRef,
        action: &view::MessageAction,
    ) -> Option<Result<Self, SubmitError>> {
        use view::MessageAction as A;
        Some(Ok(match action {
            A::Thread(root) => Self::Thread(*root),
            A::Context(target) => Self::Context(*target),
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
            Self::Context(id) => Work::Context(*id),
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
            Self::Context(target) => wire::MessagingCommand::Context(id(*target)?),
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
                    panel.thread_messages = Page::empty();
                }
                panel.thread_root = Some(*root);
            }
            if panel.thread_root == Some(*root) {
                panel.thread_state = state;
            }
        }
        Read::Context(target) => {
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
            if let Some(panel) = self.snapshot.message_panel.as_mut() {
                project(panel, &job, RemoteState::Loading(request), true);
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
            Ok(true) => (),
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
        let Some(job) = self.queued_read.queued.take() else {
            return;
        };
        if self.snapshot.selected != Some(job.context) {
            self.queued_read.clear();
            return;
        }
        self.attempt_deliberate_read(job, now);
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
