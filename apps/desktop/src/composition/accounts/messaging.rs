//! Finite polling and canonical message reconciliation on the admitted account connection.
use super::*;
use talk_core::protocol::messaging as wire;
use talk_core::ui::presentation::{DeliveryState, MessageView};
use talk_core::ui::{conversation::RemoteState, messaging as view};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Work {
    Thread,
    History,
    Latest,
    Prefetch,
    Older,
    Poll,
    Metadata,
    Action,
    Forward(ConversationRef),
    Reminder(u128),
    CancelScheduled(u128),
    Reaction(u128),
    Files,
    Search,
    Reminders,
    Scheduled,
    Pins,
    PinChange { message: u128, pinned: bool },
    Share,
    Send,
    Read(u128),
    Unread,
    Typing,
    Context(u128),
    Recovery,
}
impl Work {
    fn foreground_result(self) -> bool {
        matches!(
            self,
            Self::Action
                | Self::Forward(_)
                | Self::Reminder(_)
                | Self::CancelScheduled(_)
                | Self::Reaction(_)
                | Self::PinChange { .. }
                | Self::Share
                | Self::Send
        )
    }
    pub(super) fn diagnostic_operation(self) -> Option<DiagnosticOperation> {
        Some(match self {
            Self::History => DiagnosticOperation::History,
            Self::Older => DiagnosticOperation::Older,
            Self::Context(_) => DiagnosticOperation::Context,
            Self::Thread => DiagnosticOperation::Thread,
            Self::Latest | Self::Prefetch | Self::Poll | Self::Metadata => {
                DiagnosticOperation::Poll
            }
            Self::Recovery => DiagnosticOperation::Context,
            Self::Send => DiagnosticOperation::Send,
            _ => return None,
        })
    }
    fn backward(self) -> bool {
        matches!(
            self,
            Self::History
                | Self::Latest
                | Self::Prefetch
                | Self::Older
                | Self::Context(_)
                | Self::Recovery
        )
    }
    pub(super) fn background(self) -> bool {
        matches!(
            self,
            Self::Latest | Self::Prefetch | Self::Poll | Self::Metadata | Self::Recovery
        )
    }
}

/// One application-wide pre-dispatch send, at most 4 KiB plus compose metadata.
/// Only synchronous Busy non-admissions are retried; admitted writes immediately
/// leave this slot and retain their existing operation/uncertainty lifecycle.
pub(super) struct QueuedSend {
    request: RequestId,
    context: ConversationRef,
    body: Text<4096>,
    target: view::ComposeTarget,
    deadline: Instant,
}
impl QueuedSend {
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
impl Accounts {
    pub(super) fn tick_queued_send(&mut self, now: Instant) {
        // Never overwrite a retained completion. Its acknowledgement is local
        // and does not require another transport slot. This also bounds a slow
        // UI consumer to one queued body and one retained result.
        if self.snapshot.submission.is_some() {
            return;
        }
        let Some(queued) = self.queued_send.take() else {
            return;
        };
        match queued.attempt(self.snapshot.selected, now, |queued| {
            if let Some(failure) = self
                .snapshot
                .accounts
                .as_slice()
                .iter()
                .find(|account| account.scope == queued.context.account)
                .and_then(|account| account.connection_failure)
            {
                return Err(SubmitError::Unavailable(UnavailableReason::Connection(
                    failure,
                )));
            }
            if self.snapshot.content != ContentState::Ready {
                return Err(SubmitError::Unavailable(UnavailableReason::PolicyStale));
            }
            self.dispatch_message_action(
                queued.request,
                queued.context,
                view::MessageAction::Send {
                    body: queued.body.clone(),
                    target: queued.target.clone(),
                },
            )
        }) {
            Ok(true) => (),
            Ok(false) => self.queued_send = Some(queued),
            Err(error) => {
                let reason = match error {
                    SubmitError::Unavailable(reason) => reason,
                    SubmitError::Busy => UnavailableReason::Busy,
                    SubmitError::StaleScope => UnavailableReason::PolicyStale,
                };
                self.snapshot.submission = Some(SubmissionResult {
                    request: queued.request,
                    context: queued.context,
                    result: Completion::Failed(reason),
                });
                self.touch();
            }
        }
    }
    pub(super) fn restore_messages(&mut self, context: ConversationRef) {
        self.history.open(context);
        self.history.expire(history::now_unix());
        self.snapshot.message_panel = Some(view::MessagePanel::new(context));
        self.project_history(context);
    }
    pub(super) fn start_messages(&mut self, context: ConversationRef) -> Result<(), SubmitError> {
        // Interactive navigation takes priority over speculative reads in the
        // serialized account lane. Cancelling a GET never marks a room read.
        if let Ok(connection) = &mut self.connection {
            for pending in &self.pending {
                if matches!(pending.purpose, Purpose::Message(_, c, _, Work::Prefetch) if c != context)
                {
                    let _ = connection.cancel_request(pending.id);
                }
            }
        }
        self.restore_messages(context);
        let (command, work) = self.live_message_request(context)?;
        match self.start_message(RequestId(0), context, command, work) {
            Ok(()) | Err(SubmitError::Busy) => (),
            Err(error) => {
                let reason = match error {
                    SubmitError::Unavailable(reason) => reason,
                    _ => UnavailableReason::PolicyStale,
                };
                self.history.admission_failed(context, RequestId(0), reason);
                self.project_history(context);
                if !self.history.get(context).is_some_and(|h| h.loaded) {
                    self.snapshot.content = ContentState::Failed;
                }
            }
        }
        self.revalidate_room_metadata(context);
        self.message_poll_at = Instant::now() + Duration::from_secs(1);
        Ok(())
    }
    /// Retry selected history without replacing its panel or retained projection.
    /// A denied room has no cache; a deliberate retry creates a new read ticket,
    /// but never restores data discarded after access loss.
    pub(super) fn refresh_messages(&mut self, context: ConversationRef) -> Result<(), SubmitError> {
        if self.snapshot.selected != Some(context) {
            return Err(SubmitError::StaleScope);
        }
        if self.history.get(context).is_none() {
            self.restore_messages(context);
            self.snapshot.content = ContentState::Loading;
        }
        let result = self.start_message(
            RequestId(0),
            context,
            wire::MessagingCommand::History {
                before: None,
                thread: None,
            },
            Work::History,
        );
        if let Err(error) = result {
            if error != SubmitError::Busy {
                let reason = match error {
                    SubmitError::Unavailable(reason) => reason,
                    _ => UnavailableReason::PolicyStale,
                };
                self.history.admission_failed(context, RequestId(0), reason);
                self.project_history(context);
                if !self.history.get(context).is_some_and(|h| h.loaded) {
                    self.snapshot.content = ContentState::Failed;
                }
                self.touch();
            }
            return Err(error);
        }
        if let Some(account) = self.background.account_mut(context.account) {
            account.metadata.invalidate(context);
        }
        self.revalidate_room_metadata(context);
        Ok(())
    }
    /// Background reconciliation never starts an explicit history baseline.
    /// A loaded empty room has no event cursor yet, but remains ready while its
    /// next newest-page read is outstanding. Existing event cursors are retained
    /// after a send so a long gap is not skipped by fetching only the newest page.
    fn live_message_request(
        &self,
        context: ConversationRef,
    ) -> Result<(wire::MessagingCommand, Work), SubmitError> {
        let (command, work) = match self
            .history
            .get(context)
            .and_then(|h| h.poll_cursor(Instant::now()))
        {
            Some(id) => (
                wire::MessagingCommand::Poll {
                    after: message_id(context, id)?,
                    thread: None,
                },
                Work::Poll,
            ),
            None => (
                wire::MessagingCommand::History {
                    before: None,
                    thread: None,
                },
                if self.history.get(context).is_some_and(|h| h.loaded) {
                    Work::Latest
                } else {
                    Work::History
                },
            ),
        };
        Ok((command, work))
    }
    pub(super) fn project_history(&mut self, context: ConversationRef) {
        if self.snapshot.selected != Some(context) {
            return;
        }
        if let Some(history) = self.history.get(context) {
            // Pin activity remains canonical for cursors/read state, while its
            // already visible banner and marker replace redundant chat prose.
            let messages = history.visible_messages();
            let messages_changed = self.snapshot.messages != messages;
            let content = if history.offline {
                ContentState::Offline
            } else {
                ContentState::Ready
            };
            let changed = messages_changed
                || (history.loaded || history.offline) && self.snapshot.content != content;
            let now = history::now_unix();
            let expired: Vec<_> = self
                .snapshot
                .messages
                .as_slice()
                .iter()
                .filter(|m| m.metadata.expires_at.is_some_and(|at| at > 0 && at <= now))
                .map(|m| m.local_id)
                .collect();
            if messages_changed {
                self.snapshot.messages = messages;
            }
            if let Some(panel) = self.snapshot.message_panel.as_mut() {
                history.project(panel);
                expire_related(panel, &expired, now);
            }
            if history.loaded || history.offline {
                self.snapshot.content = content;
            }
            if changed {
                self.touch();
            }
        }
        self.project_cache(context);
        self.project_queued_read(context);
        self.project_outbox(context);
    }
    pub(super) fn invalidate_history(&mut self, context: ConversationRef, state: ContentState) {
        if self.snapshot.selected == Some(context) {
            self.queued_read.clear();
        }
        if state == ContentState::Denied {
            self.revoke_room_metadata(context, UnavailableReason::PermissionDenied);
            self.invalidate_cached_room(context);
        }
        self.history.remove(context);
        self.notices.remove_room(context);
        if self.snapshot.selected == Some(context) {
            self.clear_content_assets();
            self.snapshot.messages = Page::empty();
            self.snapshot.files = Page::empty();
            self.snapshot.participants = Page::empty();
            self.snapshot.message_panel = Some(view::MessagePanel::new(context));
            self.snapshot.content = state;
            self.touch();
        }
    }
    pub(super) fn start_message(
        &mut self,
        request: RequestId,
        context: ConversationRef,
        command: wire::MessagingCommand,
        work: Work,
    ) -> Result<(), SubmitError> {
        self.start_message_inner(request, context, command, work, None)
    }
    pub(super) fn start_outbox_message(
        &mut self,
        context: ConversationRef,
        operation: OperationId,
        command: wire::MessagingCommand,
        work: Work,
    ) -> Result<(), SubmitError> {
        self.start_message_inner(RequestId(0), context, command, work, Some(operation))
    }
    fn start_message_inner(
        &mut self,
        request: RequestId,
        context: ConversationRef,
        command: wire::MessagingCommand,
        work: Work,
        queued: Option<OperationId>,
    ) -> Result<(), SubmitError> {
        // One optional metadata repair across room switches leaves four
        // admission slots for required work. Rejection owns no probe or timer.
        if work == Work::Metadata
            && (self.pending.len() >= 12
                || self.pending.iter().any(|pending| {
                    matches!(pending.purpose, Purpose::Message(_, _, _, Work::Metadata))
                }))
        {
            return Err(SubmitError::Busy);
        }
        if self.pending.iter().any(|p| {
            matches!(p.purpose, Purpose::Message(_, c, _, w)
                if c == context && (w == work || w.backward() && work.backward()
                    || matches!((w, work), (Work::Reaction(_), Work::Reaction(_)))
                    || work == Work::Poll && w == Work::History
                        && !self.history.get(context).is_some_and(|h| h.loaded)))
        }) {
            return Err(SubmitError::Busy);
        }
        let operation = if let Some(operation) = queued {
            self.current(context.account)?;
            operation
        } else if work == Work::Prefetch {
            // This exception to selected-room admission is deliberately limited
            // to a newest-page GET. It cannot authorize any receipt or mutation.
            if !matches!(
                &command,
                wire::MessagingCommand::History {
                    before: None,
                    thread: None
                }
            ) {
                return Err(SubmitError::StaleScope);
            }
            self.current(context.account)?;
            if !self
                .background
                .account(context.account)
                .is_some_and(|a| a.rooms.as_slice().iter().any(|r| r.context == context))
            {
                return Err(SubmitError::StaleScope);
            }
            self.connection
                .as_ref()
                .map_err(|e| submit_error(*e))?
                .new_operation_id(context.account)
                .map_err(submit_error)?
        } else {
            self.operation(context)?
        };
        if self.history.get(context).is_none() {
            if work == Work::Prefetch {
                if !self.history.prefetch(context) {
                    return Err(SubmitError::Busy);
                }
            } else {
                self.history.open(context);
            }
        }
        if let Work::Reaction(message) = work
            && !self.history.can_react(context, message)
        {
            return Err(SubmitError::Busy);
        }
        let poll_after = match &command {
            wire::MessagingCommand::Poll { after, .. } => Some(u128::from(after.sequence)),
            _ => None,
        };
        self.request(
            context.account,
            ProtocolRequest::Messaging {
                conversation: context.conversation,
                operation,
                command,
            },
            Purpose::Message(request, context, operation, work),
        )?;
        self.history
            .admitted(context, operation, work, request, self.next_request);
        if let Some(after) = poll_after {
            self.history.poll_admitted(context, operation, after);
        }
        if queued.is_none()
            && let Some(panel) = self
                .snapshot
                .message_panel
                .as_mut()
                .filter(|p| p.context == context)
        {
            match work {
                Work::Reaction(message) => {
                    panel.reaction_pending = Some(message);
                    panel.state = RemoteState::Saving(request);
                }
                Work::History | Work::Older => panel.history = RemoteState::Loading(request),
                Work::Thread => panel.thread_state = RemoteState::Loading(request),
                Work::Context(target) => {
                    panel.context_load = Some(view::ContextLoad {
                        target,
                        state: RemoteState::Loading(request),
                    });
                    panel.located = None;
                }
                Work::Pins => panel.pins_state = RemoteState::Loading(request),
                Work::Files | Work::Search | Work::Reminders | Work::Scheduled => {
                    if !matches!(
                        panel.state,
                        RemoteState::Saving(_) | RemoteState::Uncertain(_)
                    ) {
                        panel.state = RemoteState::Loading(request);
                    }
                }
                work if work.foreground_result() => panel.state = RemoteState::Saving(request),
                _ => (),
            }
        }
        if work == Work::Send && queued.is_none() {
            self.unresolved.push(Unresolved {
                request,
                context,
                operation,
            });
        }
        if !work.background() {
            self.touch();
        }
        Ok(())
    }
    pub(super) fn poll_messages(&mut self) {
        if Instant::now() < self.message_poll_at {
            return;
        }
        self.message_poll_at = Instant::now() + Duration::from_secs(2);
        self.history.expire(history::now_unix());
        let Some(context) = self.snapshot.selected else {
            return;
        };
        self.project_history(context);
        if self.snapshot.content == ContentState::Denied {
            return;
        }
        if self.history.get(context).is_some_and(|h| h.needs_pins()) {
            let _ = self.start_message(
                RequestId(0),
                context,
                wire::MessagingCommand::Pins,
                Work::Pins,
            );
        }
        let Ok((command, work)) = self.live_message_request(context) else {
            return;
        };
        self.message_poll_request = self.message_poll_request.saturating_add(1);
        let _ = self.start_message(RequestId(self.message_poll_request), context, command, work);
        // Ordinary polling keeps its event cursor. This independent bounded GET
        // repairs retained legacy classifications without establishing a baseline.
        if let Some(before) = self
            .history
            .get(context)
            .and_then(|h| h.metadata_before(Instant::now()))
            .and_then(|id| message_id(context, id).ok())
        {
            let _ = self.start_message(
                RequestId(0),
                context,
                wire::MessagingCommand::History {
                    before: Some(before),
                    thread: None,
                },
                Work::Metadata,
            );
        }
    }
    pub(super) fn message_action(
        &mut self,
        request: RequestId,
        context: ConversationRef,
        action: view::MessageAction,
    ) -> Result<(), SubmitError> {
        if let Some(result) = self.deliberate_read(request, context, &action) {
            return result;
        }
        if let Some(result) = self.queue_message_action(request, context, &action) {
            return result;
        }
        let view::MessageAction::Send { body, target } = &action else {
            return self.dispatch_message_action(request, context, action);
        };
        self.current(context.account)?;
        if self.snapshot.selected != Some(context) {
            return Err(SubmitError::StaleScope);
        }
        // An unresolved write must reconcile; it cannot become a new queued
        // send. The single local slot also refuses replacement by another click.
        if self.queued_send.is_some() || self.unresolved.iter().any(|u| u.context == context) {
            return Err(SubmitError::Busy);
        }
        let queued = QueuedSend {
            request,
            context,
            body: body.clone(),
            target: target.clone(),
            deadline: Instant::now() + Duration::from_secs(10),
        };
        match self.dispatch_message_action(request, context, action) {
            Err(SubmitError::Busy) => {
                self.queued_send = Some(queued);
                self.touch();
                Ok(())
            }
            result => result,
        }
    }
    fn dispatch_message_action(
        &mut self,
        request: RequestId,
        context: ConversationRef,
        action: view::MessageAction,
    ) -> Result<(), SubmitError> {
        use view::MessageAction as V;
        use wire::MessagingCommand as W;
        let action = match action {
            V::MarkRead(id) => V::MarkRead(
                self.history
                    .get(context)
                    .map_or(id, |history| history.read_through_pin_notices(id)),
            ),
            other => other,
        };
        match &action {
            V::PickUpload => {
                return self.queue_picker(request, context, super::files::Kind::Upload);
            }
            V::Download {
                message,
                placeholder,
                name,
            } => {
                return self.queue_picker(
                    request,
                    context,
                    super::files::Kind::Download {
                        message: *message,
                        placeholder: placeholder.clone(),
                        name: name.clone(),
                    },
                );
            }
            V::DismissUpload => {
                if let Some(panel) = self.snapshot.message_panel.as_mut() {
                    panel.uploaded = None;
                    self.touch();
                }
                return Ok(());
            }
            _ => (),
        }
        let work = match &action {
            V::Files => Work::Files,
            V::Search(_) | V::SearchNext => Work::Search,
            V::Reminders => Work::Reminders,
            V::Scheduled => Work::Scheduled,
            V::Pins => Work::Pins,
            V::ShareUpload { .. } => Work::Share,
            V::Thread(_) => Work::Thread,
            V::Older => Work::Older,
            V::Refresh => Work::History,
            V::Send { .. } => Work::Send,
            V::MarkRead(id) => Work::Read(*id),
            V::MarkUnread => Work::Unread,
            V::Typing(_) => Work::Typing,
            V::Pin(message) => Work::PinChange {
                message: *message,
                pinned: true,
            },
            V::Unpin(message) => Work::PinChange {
                message: *message,
                pinned: false,
            },
            V::React { message, .. } => Work::Reaction(*message),
            V::Context(id) => Work::Context(*id),
            V::Forward { destination, .. } => Work::Forward(*destination),
            V::Reminder { message, .. } => Work::Reminder(*message),
            V::CancelScheduled(id) => Work::CancelScheduled(*id),
            _ => Work::Action,
        };
        if work == Work::Send
            && (self.snapshot.submission.is_some()
                || self.unresolved.len() >= 8
                || self.unresolved.iter().any(|u| u.context == context)
                || self.pending.iter().any(|pending| {
                    matches!(
                        pending.purpose,
                        Purpose::Send(..) | Purpose::Message(_, _, _, Work::Send)
                    )
                }))
        {
            return Err(SubmitError::Busy);
        }
        if work.foreground_result()
            && self
                .snapshot
                .message_panel
                .as_ref()
                .is_some_and(|p| p.context == context && p.state.pending())
        {
            return Err(SubmitError::Busy);
        }
        let thread_root = if let V::Thread(root) = &action {
            Some(*root)
        } else {
            None
        };
        let command = match action {
            V::RetryQueued(_) | V::DiscardQueued(_) => return Err(SubmitError::StaleScope),
            V::PickUpload | V::Download { .. } | V::DismissUpload => {
                return Err(SubmitError::StaleScope);
            }
            V::Files => W::SharedFiles { before: None },
            V::ShareUpload { caption } => W::ShareFile {
                path: self
                    .snapshot
                    .message_panel
                    .as_ref()
                    .and_then(|p| p.uploaded.as_ref())
                    .ok_or(SubmitError::StaleScope)?
                    .path
                    .clone(),
                caption,
            },
            V::Thread(root) => W::History {
                before: None,
                thread: Some(message_id(context, root)?),
            },
            V::Older => W::History {
                before: Some(
                    self.snapshot
                        .message_panel
                        .as_ref()
                        .filter(|p| p.context == context)
                        .and_then(|p| p.older)
                        .ok_or(SubmitError::StaleScope)
                        .and_then(|id| message_id(context, id))?,
                ),
                thread: None,
            },
            V::Refresh => W::History {
                before: None,
                thread: None,
            },
            V::Context(id) => W::Context(message_id(context, id)?),
            V::Send { body, target } => W::Send {
                text: text(body.as_str()),
                options: options(context, target)?,
            },
            V::Edit { message, body } => W::Edit {
                message: message_id(context, message)?,
                text: text(body.as_str()),
            },
            V::Delete(id) => W::Delete(message_id(context, id)?),
            V::React {
                message,
                emoji,
                add,
            } => W::React {
                message: message_id(context, message)?,
                emoji,
                add,
            },
            V::MarkRead(id) => W::MarkRead(message_id(context, id)?),
            V::MarkUnread => W::MarkUnread,
            V::Typing(v) => W::Typing(v),
            V::Mentions(v) => W::Mentions(v),
            V::Search(query) => {
                self.message_search = Some((context, query.clone(), None));
                W::Search {
                    query,
                    cursor: None,
                }
            }
            V::SearchNext => {
                let (_, query, cursor) = self
                    .message_search
                    .as_ref()
                    .filter(|(c, _, cursor)| *c == context && cursor.is_some())
                    .ok_or(SubmitError::StaleScope)?;
                W::Search {
                    query: query.clone(),
                    cursor: cursor.clone(),
                }
            }
            V::Pin(id) => W::Pin {
                message: message_id(context, id)?,
                until: 0,
            },
            V::Unpin(id) => W::Unpin(message_id(context, id)?),
            V::Pins => W::Pins,
            V::Reminder { message, timestamp } => W::Reminder {
                message: message_id(context, message)?,
                timestamp,
            },
            V::Reminders => W::Reminders,
            V::Schedule {
                body,
                timestamp,
                target,
            } => W::Schedule {
                text: text(body.as_str()),
                send_at: timestamp,
                options: options(context, target)?,
            },
            V::Scheduled => W::Scheduled,
            V::CancelScheduled(id) => W::CancelScheduled(message_id(context, id)?),
            V::Forward {
                message,
                destination,
            } => {
                if destination.account != context.account
                    || !self
                        .snapshot
                        .conversations
                        .as_slice()
                        .iter()
                        .any(|r| r.context == destination && r.send == Decision::Allowed)
                {
                    return Err(SubmitError::StaleScope);
                }
                W::Forward {
                    source: message_id(context, message)?,
                    destination: destination.conversation,
                }
            }
            V::ShareFile { path, caption } => W::ShareFile { path, caption },
        };
        self.start_message(request, context, command, work)?;
        if let Some(root) = thread_root
            && let Some(panel) = self.snapshot.message_panel.as_mut()
        {
            if panel.thread_root != Some(root) {
                panel.thread_messages = Page::empty();
            }
            panel.thread_root = Some(root);
        }
        Ok(())
    }
    pub(super) fn complete_message(
        &mut self,
        (request, diagnostic_request): (RequestId, u64),
        context: ConversationRef,
        operation: OperationId,
        work: Work,
        reply: Result<RuntimeReply, IdentityError>,
    ) {
        let event = diagnostic_result(diagnostic_request, context, operation, work, &reply);
        if self.current(context.account).is_err() {
            self.outbox.lost_scope(operation);
            if let Some(mut event) = event {
                event.outcome = DiagnosticOutcome::Ignored;
                event.error = Some(ErrorKind::WrongScope);
                diagnostics::record(event);
            }
            self.history.remove(context);
            return;
        }
        let idle_history = work == Work::History
            && self
                .history
                .get(context)
                .is_some_and(|h| h.loaded && h.latest.is_none());
        let before = self.history.get(context).map(|h| h.offline);
        self.apply_message(request, context, operation, work, reply);
        let after = self.history.get(context).map(|h| h.offline);
        if let Some(event) = event {
            // Empty successful polls and repeated offline failures are silent.
            let repeat_offline = before == Some(true)
                && after == Some(true)
                && matches!(
                    event.error,
                    Some(ErrorKind::Transport | ErrorKind::Deadline | ErrorKind::ServerUnavailable)
                );
            if !work.background() && !idle_history
                || event.count > 0
                || event.error.is_some() && !repeat_offline
            {
                diagnostics::record(event);
            }
            if let Some(offline) = after
                && (offline && before != Some(true) || !offline && before == Some(true))
            {
                diagnostics::record(Diagnostic {
                    outcome: if offline {
                        DiagnosticOutcome::Offline
                    } else {
                        DiagnosticOutcome::Recovered
                    },
                    ..event
                });
            }
        }
    }
    // Pure completion seam: production validates the account runtime immediately
    // above; deterministic tests can exercise switching and response order here.
    pub(super) fn apply_message(
        &mut self,
        request: RequestId,
        context: ConversationRef,
        operation: OperationId,
        work: Work,
        reply: Result<RuntimeReply, IdentityError>,
    ) {
        // Navigation belongs to the panel that admitted its exact request and
        // target. A reopened panel or newer target cannot inherit its result.
        // Retire obsolete cache tickets as no-effect cancellations below, so
        // neither a stale success nor failure changes rows or current feedback.
        let current_target = match work {
            Work::Thread => {
                self.snapshot.selected == Some(context)
                    && self.snapshot.message_panel.as_ref().is_some_and(|panel| {
                        panel.context == context
                            && panel.thread_state == RemoteState::Loading(request)
                    })
            }
            Work::Context(target) => {
                self.snapshot.selected == Some(context)
                    && self.snapshot.message_panel.as_ref().is_some_and(|panel| {
                        panel.context == context
                            && panel.context_load.as_ref().is_some_and(|load| {
                                load.target == target && load.state == RemoteState::Loading(request)
                            })
                    })
            }
            Work::Files | Work::Search | Work::Reminders | Work::Scheduled => {
                self.snapshot.selected == Some(context)
                    && self
                        .queued_read
                        .current_shared(context, request, work)
                        .unwrap_or_else(|| {
                            self.snapshot.message_panel.as_ref().is_some_and(|p| {
                                p.context == context && p.state == RemoteState::Loading(request)
                            })
                        })
            }
            _ => true,
        };
        if !current_target {
            let _ = self.history.complete(
                context,
                operation,
                work,
                request,
                &Err(Completion::Failed(UnavailableReason::PolicyStale)),
                false,
            );
            return;
        }
        let network_failure = matches!(
            reply,
            Err(IdentityError::Request(
                ErrorKind::Transport
                    | ErrorKind::Deadline
                    | ErrorKind::ServerUnavailable
                    | ErrorKind::ServerAccessDenied
            ))
        );
        let room_read = matches!(
            work,
            Work::History
                | Work::Latest
                | Work::Prefetch
                | Work::Older
                | Work::Poll
                | Work::Metadata
        );
        // A denial is authoritative only for the current room observation.
        // Context/thread endpoints can deny one target while the chat remains
        // readable; an older read cannot erase newer successful history.
        let access_lost = room_read
            && matches!(
                reply,
                Err(IdentityError::Request(
                    ErrorKind::NotFound | ErrorKind::PermissionDenied
                ))
            )
            && self
                .history
                .read_failure_is_current(context, operation, work);
        let result = match reply {
            Ok(RuntimeReply::Protocol(ProtocolReply::Messaging {
                conversation,
                operation: returned,
                result,
            })) if conversation == context.conversation
                && returned == operation
                && match (work, &result) {
                    (Work::Forward(destination), wire::MessagingResult::Send(receipt)) => {
                        receipt.operation == operation && canonical_scope(&result, destination)
                    }
                    (Work::Forward(_), _) => false,
                    _ => canonical_scope(&result, context),
                } =>
            {
                Ok(result)
            }
            Err(IdentityError::Request(ErrorKind::WrongScope | ErrorKind::Cancelled))
                if work.foreground_result() =>
            {
                Err(Completion::Uncertain)
            }
            Err(error) => Err(completion_error(error)),
            _ if work.foreground_result() => Err(Completion::Uncertain),
            _ => Err(Completion::Failed(UnavailableReason::PolicyStale)),
        };
        let mut result = if matches!(work, Work::Thread | Work::Context(_))
            && !matches!(&result, Ok(wire::MessagingResult::Messages { .. }) | Err(_))
        {
            Err(Completion::Failed(UnavailableReason::PolicyStale))
        } else {
            result
        };
        if result.is_ok()
            && !match (work, &result) {
                (Work::Search, Ok(wire::MessagingResult::Search { .. }))
                | (Work::Reminders, Ok(wire::MessagingResult::Reminders(_)))
                | (Work::Scheduled, Ok(wire::MessagingResult::Scheduled(_)))
                | (
                    Work::Files | Work::Pins | Work::Metadata,
                    Ok(wire::MessagingResult::Messages { .. }),
                ) => true,
                (
                    Work::Search
                    | Work::Reminders
                    | Work::Scheduled
                    | Work::Files
                    | Work::Pins
                    | Work::Metadata,
                    _,
                ) => false,
                _ => true,
            }
        {
            result = Err(Completion::Failed(UnavailableReason::PolicyStale));
        }
        if let Work::Reaction(target) = work
            && !matches!(&result, Ok(wire::MessagingResult::Reactions { message, .. })
                if u128::from(message.sequence) == target)
            && result.is_ok()
        {
            result = Err(Completion::Uncertain);
        }
        if matches!(work, Work::PinChange { .. })
            && !matches!(
                &result,
                Ok(wire::MessagingResult::Message(_) | wire::MessagingResult::Changed) | Err(_)
            )
        {
            result = Err(Completion::Uncertain);
        }
        if work == Work::Send
            && matches!(&result, Ok(wire::MessagingResult::Message(message))
                if !message.own || message.message.own_operation != Some(operation))
        {
            result = Err(Completion::Uncertain);
        }
        self.history
            .reconcile_reactions(context, operation, work, &mut result);
        let queued = self.outbox_result(operation, &result);
        if !self.history.complete(
            context,
            operation,
            work,
            request,
            &result,
            network_failure && room_read,
        ) {
            if work == Work::Send {
                self.settle_retained_send(request, context, operation, &result);
            }
            return;
        }
        if access_lost {
            self.invalidate_history(context, ContentState::Denied);
            return;
        }
        // A switched-away room still receives its admitted canonical updates.
        // Mutation receipts also settle without changing another room's panels.
        if self.snapshot.selected != Some(context) {
            if work == Work::Send {
                self.settle_retained_send(request, context, operation, &result);
            }
            return;
        }
        self.history.expire(history::now_unix());
        self.observed_deliberate_read(context, work, result.is_ok());
        self.project_history(context);
        let changed: Vec<u128> = match &result {
            Ok(wire::MessagingResult::Messages { messages, .. })
                if matches!(
                    work,
                    Work::History
                        | Work::Latest
                        | Work::Prefetch
                        | Work::Older
                        | Work::Poll
                        | Work::Metadata
                        | Work::Context(_)
                ) =>
            {
                messages
                    .as_slice()
                    .iter()
                    .map(|m| u128::from(m.message.id.sequence))
                    .collect()
            }
            Ok(wire::MessagingResult::Message(message)) => {
                vec![u128::from(message.message.id.sequence)]
            }
            Ok(wire::MessagingResult::Reactions { message, .. }) => {
                vec![u128::from(message.sequence)]
            }
            _ => Vec::new(),
        };
        let Some(panel) = self
            .snapshot
            .message_panel
            .as_mut()
            .filter(|p| p.context == context)
        else {
            return;
        };
        let mut reload = false;
        let completion = match result {
            Ok(result) => {
                let mut completion = Completion::ServerAccepted;
                match result {
                    wire::MessagingResult::Capabilities(capabilities) => {
                        project_capabilities(panel, &capabilities);
                    }
                    wire::MessagingResult::Messages { messages, .. } => {
                        if let Work::Context(id) = work {
                            if messages
                                .as_slice()
                                .iter()
                                .any(|m| u128::from(m.message.id.sequence) == id)
                            {
                                panel.located = Some((request, id));
                            } else {
                                completion = Completion::Failed(UnavailableReason::PolicyUnknown);
                            }
                        } else if work == Work::Files {
                            panel.shared_files = page(
                                messages
                                    .as_slice()
                                    .iter()
                                    .map(message_view)
                                    .filter(|m| !m.metadata.pin_notice),
                            );
                        } else if work == Work::Thread {
                            panel.thread_messages = page(
                                messages
                                    .as_slice()
                                    .iter()
                                    .map(message_view)
                                    .filter(|m| !m.metadata.pin_notice),
                            );
                        }
                    }
                    wire::MessagingResult::Message(message) if work == Work::Send => {
                        completion =
                            if message.own && message.message.own_operation == Some(operation) {
                                Completion::ServerAccepted
                            } else {
                                Completion::Uncertain
                            };
                    }
                    wire::MessagingResult::Message(_) => (),
                    wire::MessagingResult::Reactions { message, reactions } => {
                        reconcile_related_reactions(
                            panel,
                            u128::from(message.sequence),
                            &reactions,
                        );
                    }
                    wire::MessagingResult::Send(receipt) if receipt.operation == operation => {
                        completion = match receipt.state {
                            SendState::ServerAccepted(_) => Completion::ServerAccepted,
                            SendState::Rejected => {
                                Completion::Failed(UnavailableReason::PermissionDenied)
                            }
                            SendState::Uncertain => Completion::Uncertain,
                        };
                        reload = completion == Completion::ServerAccepted
                            && !matches!(work, Work::Forward(_));
                    }
                    wire::MessagingResult::Send(_) => completion = Completion::Uncertain,
                    wire::MessagingResult::Search { hits, cursor } => {
                        if let Some((c, _, next)) = self
                            .message_search
                            .as_mut()
                            .filter(|(c, _, _)| *c == context)
                        {
                            let _ = c;
                            *next = cursor.clone();
                        }
                        panel.search = page(hits.as_slice().iter().map(|hit| view::SearchResult {
                            id: u128::from(hit.message.sequence),
                            title: hit.title.clone(),
                            excerpt: hit.excerpt.clone(),
                        }));
                        panel.search_loaded = true;
                        panel.search_more = cursor.is_some();
                    }
                    wire::MessagingResult::Mentions(mentions) => {
                        panel.mentions = page(mentions.as_slice().iter().map(|m| view::Mention {
                            identifier: m.id.clone(),
                            label: m.label.clone(),
                        }));
                    }
                    wire::MessagingResult::Reminders(reminders) => {
                        panel.reminders =
                            page(reminders.as_slice().iter().map(|r| view::Reminder {
                                message: u128::from(r.message.sequence),
                                timestamp: r.timestamp,
                            }));
                    }
                    wire::MessagingResult::Reminder(reminder) => {
                        let target = match work {
                            Work::Reminder(message) => Some(message),
                            _ => reminder.as_ref().map(|r| u128::from(r.message.sequence)),
                        };
                        panel.reminders = page(
                            panel
                                .reminders
                                .as_slice()
                                .iter()
                                .filter(|r| Some(r.message) != target)
                                .take(if reminder.is_some() { 99 } else { 100 })
                                .cloned()
                                .chain(reminder.into_iter().map(|r| view::Reminder {
                                    message: u128::from(r.message.sequence),
                                    timestamp: r.timestamp,
                                })),
                        );
                    }
                    wire::MessagingResult::Scheduled(schedules) => {
                        panel.scheduled =
                            page(schedules.as_slice().iter().map(|s| view::Scheduled {
                                id: u128::from(s.id.sequence),
                                body: text(s.text.as_str()),
                                timestamp: s.send_at,
                            }));
                    }
                    wire::MessagingResult::Schedule(schedule) => {
                        panel.scheduled = page(
                            panel
                                .scheduled
                                .as_slice()
                                .iter()
                                .filter(|s| s.id != u128::from(schedule.id.sequence))
                                .take(99)
                                .cloned()
                                .chain(std::iter::once(view::Scheduled {
                                    id: u128::from(schedule.id.sequence),
                                    body: text(schedule.text.as_str()),
                                    timestamp: schedule.send_at,
                                })),
                        );
                    }
                    wire::MessagingResult::Uploaded { path, bytes } => {
                        let name = panel.upload_name.take().unwrap_or_else(|| text("File"));
                        panel.uploaded = Some(view::UploadedFile {
                            path,
                            name,
                            bytes: u64::from(bytes),
                        });
                    }
                    wire::MessagingResult::FileSaved { bytes } => {
                        panel.file_saved = Some(u64::from(bytes))
                    }
                    wire::MessagingResult::Changed => {
                        if let Work::CancelScheduled(id) = work {
                            panel.scheduled = page(
                                panel
                                    .scheduled
                                    .as_slice()
                                    .iter()
                                    .filter(|s| s.id != id)
                                    .cloned(),
                            );
                        }
                        reload = !matches!(work, Work::Read(_) | Work::Typing | Work::Unread);
                        if let Work::Read(id) = work {
                            let reached_latest = self
                                .history
                                .get(context)
                                .is_some_and(|history| history.read_reaches_latest(id));
                            if reached_latest {
                                self.snapshot.conversations = page(
                                    self.snapshot.conversations.as_slice().iter().map(|row| {
                                        let mut row = row.clone();
                                        if row.context == context {
                                            row.unread = 0;
                                        }
                                        row
                                    }),
                                );
                            }
                        } else if work == Work::Unread {
                            self.snapshot.conversations =
                                page(self.snapshot.conversations.as_slice().iter().map(|row| {
                                    let mut row = row.clone();
                                    if row.context == context {
                                        row.unread = row.unread.max(1);
                                    }
                                    row
                                }));
                        }
                        if work == Work::Share {
                            panel.uploaded = None;
                        }
                    }
                    wire::MessagingResult::ReactionAuthors(_)
                    | wire::MessagingResult::Threads(_)
                    | wire::MessagingResult::Thread(_) => (),
                }
                completion
            }
            Err(error) => error,
        };
        let state = match completion {
            Completion::Queued | Completion::ServerAccepted => RemoteState::Idle,
            Completion::Failed(reason) => RemoteState::Failed(request, reason),
            Completion::Uncertain => RemoteState::Uncertain(request),
        };
        if work == Work::Thread {
            panel.thread_state = state;
            if completion == Completion::Failed(UnavailableReason::PermissionDenied) {
                panel.thread_messages = Page::empty();
            }
        }
        if matches!(work, Work::Context(_))
            && let Some(load) = panel.context_load.as_mut()
        {
            load.state = state;
        }
        let owns_reaction = matches!(work, Work::Reaction(message)
            if panel.reaction_pending == Some(message) && panel.state == RemoteState::Saving(request));
        if matches!(
            work,
            Work::Files | Work::Search | Work::Reminders | Work::Scheduled
        ) && panel.state == RemoteState::Loading(request)
        {
            panel.state = state;
        }
        if work.foreground_result() && panel.state == RemoteState::Saving(request) {
            panel.state = state;
            panel.result = Some((request, completion));
        }
        if owns_reaction {
            panel.reaction_pending = None;
        }
        if work == Work::Send && !queued {
            if completion != Completion::Uncertain {
                self.unresolved.retain(|u| u.operation != operation);
            }
            self.snapshot.submission = Some(SubmissionResult {
                request,
                context,
                result: completion,
            });
        }
        reconcile_related(panel, &self.snapshot.messages, &changed);
        if work == Work::History
            && completion != Completion::ServerAccepted
            && !self.history.get(context).is_some_and(|h| h.loaded)
        {
            self.snapshot.content = if network_failure {
                ContentState::Offline
            } else {
                ContentState::Failed
            };
        }
        if self
            .snapshot
            .content_panel
            .transfer
            .as_ref()
            .is_some_and(|t| t.request == request && t.attachment.context == context)
            && let Some(progress) = self.content_owner.transfer.take()
        {
            let (received, total, _) = progress.snapshot();
            if let Some(transfer) = self.snapshot.content_panel.transfer.as_mut() {
                transfer.received = received;
                transfer.total = total;
                transfer.state =
                    if progress.is_published() || completion == Completion::ServerAccepted {
                        talk_core::ui::content::DownloadState::Saved
                    } else if progress.is_cancelled() {
                        talk_core::ui::content::DownloadState::Cancelled
                    } else {
                        talk_core::ui::content::DownloadState::Failed(match completion {
                            Completion::Failed(reason) => reason,
                            _ => UnavailableReason::PolicyStale,
                        })
                    };
            }
        }
        self.reconcile_content_assets();
        if reload && let Ok((command, work)) = self.live_message_request(context) {
            let _ = self.start_message(request, context, command, work);
        }
        self.touch();
    }
    fn settle_retained_send(
        &mut self,
        request: RequestId,
        context: ConversationRef,
        operation: OperationId,
        result: &Result<wire::MessagingResult, Completion>,
    ) {
        if !self
            .unresolved
            .iter()
            .any(|u| u.context == context && u.operation == operation)
        {
            return;
        }
        let completion = match result {
            Ok(wire::MessagingResult::Message(message))
                if message.own && message.message.own_operation == Some(operation) =>
            {
                Completion::ServerAccepted
            }
            Ok(wire::MessagingResult::Send(receipt)) if receipt.operation == operation => {
                match receipt.state {
                    SendState::ServerAccepted(_) => Completion::ServerAccepted,
                    SendState::Rejected => Completion::Failed(UnavailableReason::PermissionDenied),
                    SendState::Uncertain => Completion::Uncertain,
                }
            }
            Err(error) => *error,
            _ => Completion::Uncertain,
        };
        if completion != Completion::Uncertain {
            self.unresolved.retain(|u| u.operation != operation);
        }
        self.snapshot.submission = Some(SubmissionResult {
            request,
            context,
            result: completion,
        });
        self.touch();
    }
}

#[cfg(test)]
mod send_tests;

// Only scalar facts from a scope-validated reply cross into diagnostics. A
// received history page is not a claim that an obsolete cache ticket was applied.
fn diagnostic_result(
    request: u64,
    context: ConversationRef,
    operation: OperationId,
    work: Work,
    reply: &Result<RuntimeReply, IdentityError>,
) -> Option<Diagnostic> {
    let mut event = Diagnostic::new(work.diagnostic_operation()?, DiagnosticOutcome::Received);
    event.request = request;
    match reply {
        Ok(RuntimeReply::Protocol(ProtocolReply::Messaging {
            conversation,
            operation: returned,
            result,
        })) if *conversation == context.conversation
            && *returned == operation
            && canonical_scope(result, context) =>
        {
            match result {
                wire::MessagingResult::Message(message)
                    if work == Work::Send
                        && message.own
                        && message.message.own_operation == Some(operation) =>
                {
                    event.outcome = DiagnosticOutcome::Accepted;
                    event.count = 1;
                }
                wire::MessagingResult::Messages { messages, .. } if work != Work::Send => {
                    event.count = messages.as_slice().len() as u64
                }
                wire::MessagingResult::Send(receipt)
                    if work == Work::Send && receipt.operation == operation =>
                {
                    event.outcome = match receipt.state {
                        SendState::ServerAccepted(_) => DiagnosticOutcome::Accepted,
                        SendState::Rejected => DiagnosticOutcome::Rejected,
                        SendState::Uncertain => DiagnosticOutcome::Uncertain,
                    };
                }
                _ => {
                    event.outcome = if work == Work::Send {
                        DiagnosticOutcome::Uncertain
                    } else {
                        DiagnosticOutcome::Failed
                    };
                    event.error = Some(ErrorKind::Malformed);
                }
            }
        }
        Err(error) => {
            event.error = Some(diagnostic_error(*error));
            event.outcome = if work == Work::Send
                && matches!(
                    event.error,
                    Some(ErrorKind::Uncertain | ErrorKind::Cancelled | ErrorKind::WrongScope)
                ) {
                DiagnosticOutcome::Uncertain
            } else {
                DiagnosticOutcome::Failed
            };
        }
        _ => {
            event.outcome = if work == Work::Send {
                DiagnosticOutcome::Uncertain
            } else {
                DiagnosticOutcome::Failed
            };
            event.error = Some(ErrorKind::Malformed);
        }
    }
    Some(event)
}

fn canonical_scope(result: &wire::MessagingResult, context: ConversationRef) -> bool {
    let message = |m: &wire::MessageDetails| {
        m.message.id.conversation == context.conversation
            && m.parent
                .as_ref()
                .is_none_or(|p| p.id.conversation == context.conversation)
            && m.thread
                .is_none_or(|id| id.conversation == context.conversation)
    };
    match result {
        wire::MessagingResult::Messages {
            messages,
            cursor,
            common_read,
            ..
        } => {
            messages.as_slice().iter().all(message)
                && cursor.is_none_or(|id| id.conversation == context.conversation)
                && common_read.is_none_or(|id| id.conversation == context.conversation)
        }
        wire::MessagingResult::Message(value) => message(value),
        wire::MessagingResult::Reactions { message, .. } => {
            message.conversation == context.conversation
        }
        wire::MessagingResult::Reminder(reminder) => reminder
            .as_ref()
            .is_none_or(|r| r.message.conversation == context.conversation),
        wire::MessagingResult::Reminders(reminders) => reminders
            .as_slice()
            .iter()
            .all(|r| r.message.conversation == context.conversation),
        wire::MessagingResult::Schedule(schedule) => {
            schedule.id.conversation == context.conversation
        }
        wire::MessagingResult::Scheduled(schedules) => schedules
            .as_slice()
            .iter()
            .all(|s| s.id.conversation == context.conversation),
        wire::MessagingResult::Search { hits, .. } => hits
            .as_slice()
            .iter()
            .all(|hit| hit.message.conversation == context.conversation),
        wire::MessagingResult::Send(protocol::SendReceipt {
            state: SendState::ServerAccepted(id),
            ..
        }) => id.conversation == context.conversation,
        _ => true,
    }
}
fn expire_related(panel: &mut view::MessagePanel, expired: &[u128], now: u64) {
    let mut expired = expired.to_vec();
    for messages in [
        &panel.thread_messages,
        &panel.pinned_messages,
        &panel.shared_files,
    ] {
        expired.extend(
            messages
                .as_slice()
                .iter()
                .filter(|m| m.metadata.expires_at.is_some_and(|at| at > 0 && at <= now))
                .map(|m| m.local_id),
        );
    }
    if expired.is_empty() {
        return;
    }
    let retain = |message: &MessageView| {
        if expired.contains(&message.local_id) {
            return None;
        }
        let mut message = message.clone();
        if let Some(parent) = message.metadata.parent.as_mut()
            && expired.contains(&parent.id)
        {
            parent.unavailable = true;
            parent.text = text("");
        }
        Some(message)
    };
    panel.thread_messages = page(panel.thread_messages.as_slice().iter().filter_map(retain));
    panel.pinned_messages = page(panel.pinned_messages.as_slice().iter().filter_map(retain));
    panel.shared_files = page(panel.shared_files.as_slice().iter().filter_map(retain));
    panel.search = page(
        panel
            .search
            .as_slice()
            .iter()
            .filter(|hit| !expired.contains(&hit.id))
            .cloned(),
    );
}
pub(super) fn reaction_views(reactions: &Page<wire::Reaction, 64>) -> Page<view::Reaction, 64> {
    page(reactions.as_slice().iter().map(|reaction| view::Reaction {
        emoji: reaction.emoji.clone(),
        count: reaction.count,
        own: reaction.own,
    }))
}

fn reconcile_related_reactions(
    panel: &mut view::MessagePanel,
    target: u128,
    reactions: &Page<wire::Reaction, 64>,
) {
    let reconcile = |message: &MessageView| {
        let mut message = message.clone();
        if message.local_id == target && !message.metadata.deleted {
            message.metadata.reactions = reaction_views(reactions);
        }
        message
    };
    panel.thread_messages = page(panel.thread_messages.as_slice().iter().map(reconcile));
    panel.pinned_messages = page(panel.pinned_messages.as_slice().iter().map(reconcile));
    panel.shared_files = page(panel.shared_files.as_slice().iter().map(reconcile));
}

/// Canonical edits/tombstones invalidate every retained excerpt in this room.
fn reconcile_related(
    panel: &mut view::MessagePanel,
    canonical: &Page<MessageView, 100>,
    changed: &[u128],
) {
    if changed.is_empty() {
        return;
    }
    let reconcile = |message: &MessageView| {
        let mut result = canonical
            .as_slice()
            .iter()
            .find(|m| m.local_id == message.local_id && changed.contains(&m.local_id))
            .unwrap_or(message)
            .clone();
        if let Some(parent) = result.metadata.parent.as_mut()
            && canonical
                .as_slice()
                .iter()
                .any(|m| m.local_id == parent.id && m.metadata.deleted)
        {
            parent.unavailable = true;
            parent.text = text("");
        }
        result
    };
    panel.thread_messages = page(panel.thread_messages.as_slice().iter().map(reconcile));
    panel.pinned_messages = page(
        panel
            .pinned_messages
            .as_slice()
            .iter()
            .map(reconcile)
            .filter(|m| !m.metadata.deleted),
    );
    panel.shared_files = page(
        panel
            .shared_files
            .as_slice()
            .iter()
            .map(reconcile)
            .filter(|m| !m.metadata.deleted),
    );
    panel.search = page(panel.search.as_slice().iter().filter_map(|hit| {
        let mut hit = hit.clone();
        if let Some(message) = canonical
            .as_slice()
            .iter()
            .find(|m| m.local_id == hit.id && changed.contains(&m.local_id))
        {
            if message.metadata.deleted {
                return None;
            }
            hit.excerpt = message.body.clone();
        }
        Some(hit)
    }));
}
fn options(
    context: ConversationRef,
    target: view::ComposeTarget,
) -> Result<wire::ComposeOptions, SubmitError> {
    Ok(wire::ComposeOptions {
        reply_to: target
            .reply_to
            .map(|id| message_id(context, id))
            .transpose()?,
        thread: target
            .thread
            .map(|id| message_id(context, id))
            .transpose()?,
        thread_title: None,
        silent: target.silent,
    })
}
fn message_id(context: ConversationRef, id: u128) -> Result<protocol::MessageId, SubmitError> {
    Ok(protocol::MessageId {
        conversation: context.conversation,
        sequence: u64::try_from(id).map_err(|_| SubmitError::StaleScope)?,
    })
}
pub(super) fn project_capabilities(
    panel: &mut view::MessagePanel,
    capabilities: &wire::MessagingCapabilities,
) {
    panel.pin_change = capabilities.pin_change;
    panel.file_limit = capabilities.file_limit;
    panel.message_limit = capabilities.message_limit.unwrap_or(4096).min(4096);
    panel.features = page(
        capabilities
            .actions
            .as_slice()
            .iter()
            .filter_map(|(feature, value)| {
                feature_view(*feature)
                    .map(|feature| (feature, super::conversations::availability(*value)))
            }),
    );
}
pub(super) fn message_view(m: &wire::MessageDetails) -> MessageView {
    let deleted = m.message.kind == protocol::MessageKind::Deleted;
    let (body, truncated) = rich_body(m);
    MessageView {
        local_id: u128::from(m.message.id.sequence),
        author: text(m.message.author.as_str()),
        initials: initials(m.message.author.as_str()),
        time_label: text(&time_label(m.message.timestamp)),
        body: if deleted { text("") } else { body },
        own: m.own,
        delivery: DeliveryState::Received,
        attachments: Page::empty(),
        reply_count: 0,
        metadata: view::MessageMetadata {
            local_operation: m.message.own_operation.map(OperationId::as_local),
            author_key: m.author_key.clone(),
            expires_at: m.message.expires_at,
            timestamp: m.message.timestamp,
            truncated,
            edited: m.message.edited_at.is_some(),
            deleted,
            system: m.message.kind == protocol::MessageKind::System,
            system_event: system_event(m),
            pin_notice: m.message.kind == protocol::MessageKind::System
                && m.system.as_ref().is_some_and(|kind| {
                    matches!(kind.as_str(), "message_pinned" | "message_unpinned")
                }),
            read_through: None,
            replyable: m.replyable == Some(true) && !deleted,
            parent: m.parent.as_ref().map(|p| view::Reply {
                id: u128::from(p.id.sequence),
                author: p.author.clone(),
                text: if p.unavailable {
                    text("")
                } else {
                    p.text.clone()
                },
                unavailable: p.unavailable,
            }),
            thread: m.thread.map(|m| u128::from(m.sequence)),
            reactions: page(m.reactions.as_slice().iter().map(|r| view::Reaction {
                emoji: r.emoji.clone(),
                count: r.count,
                own: r.own,
            })),
            rich: page(m.rich.as_slice().iter().map(|r| view::RichObject {
                id: r.id.clone(),
                mime_type: r.mime_type.clone(),
                size: r.size,
                placeholder: r.placeholder.clone(),
                kind: r.kind.clone(),
                name: r.name.clone(),
                link: r.link.clone(),
            })),
            edit: if truncated { Decision::Denied } else { m.edit },
            delete: m.delete,
        },
    }
}
fn system_event(message: &wire::MessageDetails) -> view::SystemEvent {
    use view::SystemEvent;
    if message.message.kind != protocol::MessageKind::System {
        return SystemEvent::Unknown;
    }
    // Pinned parser sources rewrite a caller's missed call to call_tried:
    // https://github.com/nextcloud/spreed/blob/17afc250cf5fe3f7e016739dd3d7a3a44bb2cb79/lib/Chat/Parser/SystemMessage.php
    // https://github.com/nextcloud/spreed/blob/f35255ab86f31e8ee894641f6970a09ce4b90927/lib/Chat/Parser/SystemMessage.php
    // No failed-call history tag is established by these pins; localized text
    // is never classification evidence.
    match message.system.as_ref().map(Text::as_str) {
        None => SystemEvent::Unknown,
        Some("call_started") if message.own => SystemEvent::CallOutgoing,
        Some("call_started") => SystemEvent::CallIncoming,
        Some("call_joined") => SystemEvent::CallJoined,
        Some("call_left") => SystemEvent::CallLeft,
        Some("call_ended" | "call_ended_everyone") => SystemEvent::CallEnded,
        Some("call_missed") => SystemEvent::CallMissed,
        Some("call_tried") => SystemEvent::CallUnanswered,
        Some(_) => SystemEvent::Other,
    }
}
/// Rich templates are rendered as bounded text; names never become markup or URLs.
fn rich_body(message: &wire::MessageDetails) -> (Text<4096>, bool) {
    let source = message.message.text.as_str();
    let mut output = String::new();
    let mut rest = source;
    while !rest.is_empty() && output.len() < 4096 {
        if let Some(tail) = rest.strip_prefix('{')
            && let Some(end) = tail.find('}').filter(|n| *n <= 128)
            && let Some(object) = message
                .rich
                .as_slice()
                .iter()
                .find(|r| r.placeholder.as_str() == &tail[..end])
        {
            output.push_str(object.name.as_str());
            rest = &tail[end + 1..];
            continue;
        }
        let ch = rest.chars().next().expect("nonempty");
        output.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    let truncated = !rest.is_empty() || output.len() > 4096 || output.lines().count() > 256;
    (text(&output), truncated)
}
/// Stable identity merges updates/deletions and bounds loaded history independently of virtualization.
pub(super) fn merge(
    existing: &Page<MessageView, 100>,
    incoming: impl Iterator<Item = MessageView>,
    older: bool,
) -> Page<MessageView, 100> {
    let mut messages = existing.as_slice().to_vec();
    for message in incoming.take(50) {
        if let Some(current) = messages.iter_mut().find(|m| m.local_id == message.local_id) {
            *current = message;
        } else {
            messages.push(message);
        }
    }
    messages.sort_by_key(|m| m.local_id);
    if messages.len() > 100 {
        if older {
            messages.truncate(100);
        } else {
            messages.drain(..messages.len() - 100);
        }
    }
    let unavailable: Vec<u128> = messages
        .iter()
        .filter(|m| m.metadata.deleted)
        .map(|m| m.local_id)
        .collect();
    for message in &mut messages {
        if let Some(parent) = message.metadata.parent.as_mut()
            && unavailable.contains(&parent.id)
        {
            parent.unavailable = true;
            parent.text = text("");
        }
    }
    page(messages)
}
fn feature_view(f: wire::MessagingFeature) -> Option<view::MessageFeature> {
    use view::MessageFeature as V;
    use wire::MessagingFeature as W;
    Some(match f {
        W::React => V::React,
        W::Reply => V::Reply,
        W::Edit => V::Edit,
        W::Delete => V::Delete,
        W::Threads => V::Threads,
        W::Mentions => V::Mentions,
        W::Silent => V::Silent,
        W::Search => V::Search,
        W::Pins => V::Pins,
        W::Reminders => V::Reminders,
        W::Schedule => V::Schedules,
        W::Forward => V::Forward,
        W::Files => V::Files,
        W::Typing => V::Typing,
        W::ReadReceipts => V::ReadReceipts,
        _ => return None,
    })
}
pub(super) fn time_label(seconds: u64) -> String {
    talk_adapters::system_time::time_label(seconds).unwrap_or_else(|| "–".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_warmup_is_single_flight_across_room_switches() {
        let mut model = super::super::tests::model();
        model.snapshot = super::super::super::fixtures::snapshot(Some("chat"));
        let selected = model.snapshot.selected.unwrap();
        let previous = super::super::super::fixtures::context(2);
        assert_ne!(previous, selected);
        model.history.open(selected);
        model.pending.push(Pending {
            id: ConnectionRequestId(3),
            purpose: Purpose::Message(
                RequestId(0),
                previous,
                OperationId::from_local(3),
                Work::Metadata,
            ),
        });
        assert_eq!(
            model.start_message(
                RequestId(0),
                selected,
                wire::MessagingCommand::History {
                    before: Some(protocol::MessageId {
                        conversation: selected.conversation,
                        sequence: 8,
                    }),
                    thread: None,
                },
                Work::Metadata,
            ),
            Err(SubmitError::Busy)
        );
        assert_eq!(model.pending.len(), 1);
    }

    #[test]
    fn metadata_warmup_keeps_forward_polling_independent_and_yields_to_backpressure() {
        let mut model = super::super::tests::model();
        let context = super::super::super::fixtures::context(1);
        model.snapshot = super::super::super::fixtures::snapshot(Some("chat"));
        model.history.open(context);
        let message = wire::MessageDetails {
            author_key: None,
            message: protocol::Message {
                id: protocol::MessageId {
                    conversation: context.conversation,
                    sequence: 7,
                },
                author: text("Synthetic user"),
                text: text("Localized legacy system text"),
                kind: protocol::MessageKind::System,
                timestamp: 1,
                expires_at: None,
                edited_at: None,
                own_operation: None,
            },
            notify: Decision::Denied,
            own: false,
            replyable: None,
            parent: None,
            thread: None,
            reactions: Page::empty(),
            rich: Page::empty(),
            system: None,
            edit: Decision::Unknown,
            delete: Decision::Unknown,
        };
        for (serial, work, rows, cursor) in [
            (1_u64, Work::History, page([message]), None),
            (
                2,
                Work::Poll,
                Page::empty(),
                Some(protocol::MessageId {
                    conversation: context.conversation,
                    sequence: 31,
                }),
            ),
        ] {
            let operation = OperationId::from_local(u128::from(serial));
            model
                .history
                .admitted(context, operation, work, RequestId(serial), serial);
            assert!(model.history.complete(
                context,
                operation,
                work,
                RequestId(serial),
                &Ok(wire::MessagingResult::Messages {
                    capabilities: None,
                    messages: rows,
                    cursor,
                    common_read: None
                }),
                false
            ));
        }
        let assert_forward = |model: &Accounts| {
            let (command, work) = model.live_message_request(context).unwrap();
            assert_eq!(work, Work::Poll);
            assert!(
                matches!(command, wire::MessagingCommand::Poll { after, thread: None } if after.sequence == 31)
            );
        };
        assert_forward(&model);
        let command = || wire::MessagingCommand::History {
            before: Some(protocol::MessageId {
                conversation: context.conversation,
                sequence: 8,
            }),
            thread: None,
        };
        model.pending.push(Pending {
            id: ConnectionRequestId(3),
            purpose: Purpose::Message(
                RequestId(0),
                context,
                OperationId::from_local(3),
                Work::Metadata,
            ),
        });
        assert_eq!(
            model.start_message(RequestId(0), context, command(), Work::Metadata),
            Err(SubmitError::Busy)
        );
        assert_forward(&model);
        // A pending metadata GET does not hold the backward-read or poll guard.
        let (poll, work) = model.live_message_request(context).unwrap();
        assert_ne!(
            model.start_message(RequestId(0), context, poll, work),
            Err(SubmitError::Busy)
        );
        assert_ne!(
            model.start_message(RequestId(0), context, command(), Work::Older),
            Err(SubmitError::Busy)
        );
        model.pending.clear();
        for id in 1..=12 {
            model.pending.push(Pending {
                id: ConnectionRequestId(id),
                purpose: Purpose::Tags(context.account),
            });
        }
        assert_eq!(
            model.start_message(RequestId(0), context, command(), Work::Metadata),
            Err(SubmitError::Busy)
        );
        assert_eq!(model.pending.len(), 12);
        assert_eq!(
            model
                .history
                .get(context)
                .unwrap()
                .metadata_before(Instant::now()),
            Some(8)
        );
        assert_forward(&model);
        assert!(Work::Metadata.background());
        assert!(!Work::Metadata.backward());
    }

    #[test]
    fn live_reconciliation_uses_initial_load_only_until_the_room_has_loaded() {
        let mut model = super::super::tests::model();
        let context = super::super::super::fixtures::context(1);
        model.history.open(context);
        assert_eq!(
            model.live_message_request(context).unwrap().1,
            Work::History
        );
        let operation = OperationId::from_local(1);
        model
            .history
            .admitted(context, operation, Work::History, RequestId(1), 1);
        assert!(model.history.complete(
            context,
            operation,
            Work::History,
            RequestId(1),
            &Ok(wire::MessagingResult::Messages {
                capabilities: None,
                messages: Page::empty(),
                cursor: None,
                common_read: None
            }),
            false
        ));
        assert_eq!(model.live_message_request(context).unwrap().1, Work::Latest);
        let operation = OperationId::from_local(2);
        model
            .history
            .admitted(context, operation, Work::Poll, RequestId(2), 2);
        assert!(model.history.complete(
            context,
            operation,
            Work::Poll,
            RequestId(2),
            &Ok(wire::MessagingResult::Messages {
                capabilities: None,
                messages: Page::empty(),
                cursor: Some(protocol::MessageId {
                    conversation: context.conversation,
                    sequence: 70
                }),
                common_read: None
            }),
            false
        ));
        let (command, work) = model.live_message_request(context).unwrap();
        assert_eq!(work, Work::Poll);
        assert!(
            matches!(command, wire::MessagingCommand::Poll { after, thread: None } if after.sequence == 70)
        );
    }
    fn row(id: u128, body: &str) -> MessageView {
        MessageView {
            local_id: id,
            author: text("Alex"),
            initials: text("A"),
            time_label: text(""),
            body: text(body),
            own: false,
            delivery: DeliveryState::Received,
            attachments: Page::empty(),
            reply_count: 0,
            metadata: Default::default(),
        }
    }
    #[test]
    fn diagnostic_reply_projection_excludes_content_and_keeps_only_local_correlation() {
        let context = super::super::super::fixtures::context(1);
        let operation = OperationId::from_local(99);
        let message = wire::MessageDetails {
            author_key: Some(text("4:user:fixture")),
            message: protocol::Message {
                id: protocol::MessageId {
                    conversation: context.conversation,
                    sequence: 42,
                },
                author: text("Synthetic Private Name"),
                text: text(
                    "draft secret, person@example.test, https://private.example.test/room-token, Bearer synthetic-secret",
                ),
                kind: protocol::MessageKind::Comment,
                timestamp: 1,
                expires_at: None,
                edited_at: None,
                own_operation: None,
            },
            notify: Decision::Denied,
            own: false,
            replyable: Some(true),
            parent: None,
            thread: None,
            reactions: Page::empty(),
            rich: Page::empty(),
            system: None,
            edit: Decision::Unknown,
            delete: Decision::Unknown,
        };
        let reply = Ok(RuntimeReply::Protocol(ProtocolReply::Messaging {
            conversation: context.conversation,
            operation,
            result: wire::MessagingResult::Messages {
                capabilities: None,
                messages: page([message]),
                cursor: None,
                common_read: None,
            },
        }));
        let event = diagnostic_result(73, context, operation, Work::History, &reply).unwrap();
        assert_eq!(event.request, 73);
        assert_eq!(event.count, 1);
        let line = diagnostics::test_line(event);
        for secret in [
            "Synthetic Private Name",
            "draft secret",
            "person@example.test",
            "https://",
            "room-token",
            "Bearer",
            "synthetic-secret",
        ] {
            assert!(!line.contains(secret));
        }
        assert!(line.contains("\"request\":73,"));
        let error = Err(IdentityError::Request(ErrorKind::Uncertain));
        let event = diagnostic_result(74, context, operation, Work::Send, &error).unwrap();
        assert!(matches!(event.outcome, DiagnosticOutcome::Uncertain));
        assert_eq!(event.error, Some(ErrorKind::Uncertain));
        let stale = diagnostic_result(
            75,
            context,
            OperationId::from_local(100),
            Work::History,
            &reply,
        )
        .unwrap();
        assert!(matches!(stale.outcome, DiagnosticOutcome::Failed));
        assert_eq!(stale.error, Some(ErrorKind::Malformed));
        assert_eq!(stale.count, 0);
    }
    #[test]
    fn canonical_merge_updates_without_duplicate_and_bounds_both_paging_directions() {
        let current = page((1..=100).map(|n| row(n, "old")));
        let next = merge(
            &current,
            [row(50, "edited"), row(101, "new")].into_iter(),
            false,
        );
        assert_eq!(next.as_slice().len(), 100);
        assert_eq!(next.as_slice()[0].local_id, 2);
        assert_eq!(
            next.as_slice()
                .iter()
                .find(|m| m.local_id == 50)
                .unwrap()
                .body
                .as_str(),
            "edited"
        );
        let older = merge(&next, [row(1, "older")].into_iter(), true);
        assert_eq!(older.as_slice()[0].local_id, 1);
        assert_eq!(older.as_slice().last().unwrap().local_id, 100);
    }
    #[test]
    fn deleted_messages_remove_all_retained_excerpts_and_pins() {
        let context = super::super::super::fixtures::context(1);
        let mut panel = view::MessagePanel::new(context);
        panel.pinned_messages = page([row(1, "secret")]);
        panel.shared_files = page([row(1, "secret")]);
        panel.thread_messages = page([row(1, "secret")]);
        panel.search = page([view::SearchResult {
            id: 1,
            title: text("title"),
            excerpt: text("secret"),
        }]);
        let mut deleted = row(1, "");
        deleted.metadata.deleted = true;
        reconcile_related(&mut panel, &page([deleted]), &[1]);
        assert!(panel.pinned_messages.as_slice().is_empty());
        assert!(panel.shared_files.as_slice().is_empty());
        assert!(panel.search.as_slice().is_empty());
        assert!(panel.thread_messages.as_slice()[0].metadata.deleted);
        assert!(panel.thread_messages.as_slice()[0].body.as_str().is_empty());
    }
    #[test]
    fn deletion_erases_reply_excerpt() {
        let mut child = row(2, "reply");
        child.metadata.parent = Some(view::Reply {
            id: 1,
            author: text("A"),
            text: text("secret"),
            unavailable: false,
        });
        let mut deleted = row(1, "");
        deleted.metadata.deleted = true;
        let merged = merge(
            &page([row(1, "secret"), child]),
            std::iter::once(deleted),
            false,
        );
        let parent = merged.as_slice()[1].metadata.parent.as_ref().unwrap();
        assert!(parent.unavailable);
        assert_eq!(parent.text.as_str(), "");
    }
}

#[cfg(test)]
mod immediate_send_tests {
    use super::*;
    #[test]
    fn canonical_send_reply_is_visible_and_settled_without_a_history_reload() {
        let mut model = super::super::tests::model();
        let context = super::super::super::fixtures::context(1);
        model.snapshot.active_account = Some(context.account);
        model.snapshot.selected = Some(context);
        model.restore_messages(context);
        let operation = OperationId::from_local(77);
        let request = RequestId(4);
        model
            .history
            .admitted(context, operation, Work::Send, request, 1);
        model.unresolved.push(Unresolved {
            request,
            context,
            operation,
        });
        let reply = RuntimeReply::Protocol(ProtocolReply::Messaging {
            conversation: context.conversation,
            operation,
            result: wire::MessagingResult::Message(Box::new(wire::MessageDetails {
                author_key: Some(text("5:users:synthetic")),
                message: protocol::Message {
                    id: protocol::MessageId {
                        conversation: context.conversation,
                        sequence: 99,
                    },
                    author: text("Synthetic sender"),
                    text: text("Accepted body"),
                    kind: protocol::MessageKind::Comment,
                    timestamp: 1,
                    expires_at: None,
                    edited_at: None,
                    own_operation: Some(operation),
                },
                own: true,
                notify: Decision::Denied,
                replyable: Some(true),
                parent: None,
                thread: None,
                reactions: Page::empty(),
                rich: Page::empty(),
                system: None,
                edit: Decision::Allowed,
                delete: Decision::Allowed,
            })),
        });
        model.apply_message(request, context, operation, Work::Send, Ok(reply.clone()));
        assert_eq!(model.snapshot.messages.as_slice().len(), 1);
        assert_eq!(model.snapshot.messages.as_slice()[0].local_id, 99);
        assert!(model.snapshot.messages.as_slice()[0].own);
        assert_eq!(
            model.snapshot.submission.unwrap().result,
            Completion::ServerAccepted
        );
        assert!(model.unresolved.is_empty());
        assert!(model.pending.is_empty());

        // A different send's row must neither enter history nor acknowledge this one.
        let other_operation = OperationId::from_local(78);
        let other_request = RequestId(5);
        model
            .history
            .admitted(context, other_operation, Work::Send, other_request, 2);
        model.unresolved.push(Unresolved {
            request: other_request,
            context,
            operation: other_operation,
        });
        let mut wrong = reply;
        if let RuntimeReply::Protocol(ProtocolReply::Messaging {
            operation,
            result: wire::MessagingResult::Message(message),
            ..
        }) = &mut wrong
        {
            *operation = other_operation;
            message.message.id.sequence = 100;
        } else {
            panic!("fixture message reply");
        }
        model.apply_message(
            other_request,
            context,
            other_operation,
            Work::Send,
            Ok(wrong),
        );
        assert_eq!(model.snapshot.messages.as_slice().len(), 1);
        assert_eq!(model.snapshot.messages.as_slice()[0].local_id, 99);
        assert_eq!(
            model.snapshot.submission.unwrap().result,
            Completion::Uncertain
        );
        assert_eq!(model.unresolved.len(), 1);
    }
}
