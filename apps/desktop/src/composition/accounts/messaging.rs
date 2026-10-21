//! Finite polling and canonical message reconciliation on the admitted account connection.
use super::*;
use talk_core::protocol::messaging as wire;
use talk_core::ui::presentation::{DeliveryState, MessageView};
use talk_core::ui::{conversation::RemoteState, messaging as view};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Work {
    Thread,
    Capabilities,
    History,
    Latest,
    Older,
    Poll,
    Action,
    Files,
    Pins,
    PinChange,
    Share,
    Send,
    Read(u128),
    Unread,
    Typing,
    Context(u128),
}
impl Work {
    pub(super) fn diagnostic_operation(self) -> Option<DiagnosticOperation> {
        Some(match self {
            Self::History => DiagnosticOperation::History,
            Self::Older => DiagnosticOperation::Older,
            Self::Context(_) => DiagnosticOperation::Context,
            Self::Thread => DiagnosticOperation::Thread,
            Self::Latest | Self::Poll => DiagnosticOperation::Poll,
            Self::Send => DiagnosticOperation::Send,
            _ => return None,
        })
    }
    fn backward(self) -> bool {
        matches!(
            self,
            Self::History | Self::Latest | Self::Older | Self::Context(_)
        )
    }
    pub(super) fn background(self) -> bool {
        matches!(self, Self::Latest | Self::Poll)
    }
}
impl Accounts {
    pub(super) fn restore_messages(&mut self, context: ConversationRef) {
        self.history.open(context);
        self.history.expire(history::now_unix());
        self.snapshot.message_panel = Some(view::MessagePanel::new(context));
        self.project_history(context);
    }
    pub(super) fn start_messages(&mut self, context: ConversationRef) -> Result<(), SubmitError> {
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
        let _ = self.start_message(
            RequestId(0),
            context,
            wire::MessagingCommand::Capabilities,
            Work::Capabilities,
        );
        let _ = self.room_action(
            RequestId(0),
            context,
            talk_core::ui::conversation::ConversationAction::Read,
        );
        self.message_poll_at = Instant::now() + Duration::from_secs(1);
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
        let (command, work) = match self.history.get(context).and_then(|h| h.latest) {
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
            let messages_changed = self.snapshot.messages != history.messages;
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
                self.snapshot.messages = history.messages.clone();
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
    }
    pub(super) fn invalidate_history(&mut self, context: ConversationRef, state: ContentState) {
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
        if self.pending.iter().any(|p| {
            matches!(p.purpose, Purpose::Message(_, c, _, w)
                if c == context && (w == work || w.backward() && work.backward()
                    || work == Work::Poll && w == Work::History
                        && !self.history.get(context).is_some_and(|h| h.loaded)))
        }) {
            return Err(SubmitError::Busy);
        }
        let operation = self.operation(context)?;
        if self.history.get(context).is_none() {
            self.history.open(context);
        }
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
        if let Some(panel) = self.snapshot.message_panel.as_mut() {
            match work {
                Work::History | Work::Older => panel.history = RemoteState::Loading(request),
                Work::Action | Work::PinChange | Work::Share | Work::Send => {
                    panel.state = RemoteState::Saving(request)
                }
                _ => (),
            }
        }
        if work == Work::Send {
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
        let Ok((command, work)) = self.live_message_request(context) else {
            return;
        };
        self.message_poll_request = self.message_poll_request.saturating_add(1);
        let _ = self.start_message(RequestId(self.message_poll_request), context, command, work);
    }
    pub(super) fn message_action(
        &mut self,
        request: RequestId,
        context: ConversationRef,
        action: view::MessageAction,
    ) -> Result<(), SubmitError> {
        use view::MessageAction as V;
        use wire::MessagingCommand as W;
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
            V::Pins => Work::Pins,
            V::ShareUpload { .. } => Work::Share,
            V::Thread(_) => Work::Thread,
            V::Older => Work::Older,
            V::Refresh => Work::History,
            V::Send { .. } => Work::Send,
            V::MarkRead(id) => Work::Read(*id),
            V::MarkUnread => Work::Unread,
            V::Typing(_) => Work::Typing,
            V::Pin(_) | V::Unpin(_) => Work::PinChange,
            V::Context(id) => Work::Context(*id),
            _ => Work::Action,
        };
        if work == Work::Send
            && (self.snapshot.submission.is_some()
                || self.unresolved.len() >= 8
                || self.unresolved.iter().any(|u| u.context == context))
        {
            return Err(SubmitError::Busy);
        }
        if matches!(
            work,
            Work::Action | Work::PinChange | Work::Share | Work::Send
        ) && self
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
            panel.thread_root = Some(root);
            panel.thread_messages = Page::empty();
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
        let network_failure = matches!(
            reply,
            Err(IdentityError::Request(
                ErrorKind::Transport | ErrorKind::Deadline | ErrorKind::ServerUnavailable
            ))
        );
        let missing_room = matches!(
            work,
            Work::History | Work::Latest | Work::Older | Work::Poll
        ) && matches!(reply, Err(IdentityError::Request(ErrorKind::NotFound)));
        let result = match reply {
            Ok(RuntimeReply::Protocol(ProtocolReply::Messaging {
                conversation,
                operation: returned,
                result,
            })) if conversation == context.conversation
                && returned == operation
                && canonical_scope(&result, context) =>
            {
                Ok(result)
            }
            Err(IdentityError::Request(ErrorKind::WrongScope | ErrorKind::Cancelled))
                if work == Work::Send =>
            {
                Err(Completion::Uncertain)
            }
            Err(error) => Err(completion_error(error)),
            _ if work == Work::Send => Err(Completion::Uncertain),
            _ => Err(Completion::Failed(UnavailableReason::PolicyStale)),
        };
        if !self
            .history
            .complete(context, operation, work, request, &result, network_failure)
        {
            if work == Work::Send {
                self.settle_retained_send(request, context, operation, &result);
            }
            return;
        }
        if missing_room
            || matches!(
                work,
                Work::History
                    | Work::Latest
                    | Work::Older
                    | Work::Poll
                    | Work::Thread
                    | Work::Context(_)
            ) && result == Err(Completion::Failed(UnavailableReason::PermissionDenied))
        {
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
        self.project_history(context);
        let changed: Vec<u128> = match &result {
            Ok(wire::MessagingResult::Messages { messages, .. })
                if matches!(
                    work,
                    Work::History | Work::Latest | Work::Older | Work::Poll | Work::Context(_)
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
                        panel.file_limit = capabilities.file_limit;
                        panel.message_limit = capabilities.message_limit.unwrap_or(4096).min(4096);
                        panel.features = page(capabilities.actions.as_slice().iter().filter_map(
                            |(feature, value)| {
                                feature_view(*feature).map(|feature| {
                                    (feature, super::conversations::availability(*value))
                                })
                            },
                        ));
                    }
                    wire::MessagingResult::Messages {
                        messages,
                        cursor: _,
                        common_read: _,
                    } => {
                        if let Work::Context(id) = work {
                            panel.located = Some((request, id));
                        } else if work == Work::Pins {
                            panel.pinned_messages =
                                page(messages.as_slice().iter().map(message_view));
                        } else if work == Work::Files {
                            panel.shared_files = page(messages.as_slice().iter().map(message_view));
                        } else if work == Work::Thread {
                            panel.thread_messages =
                                page(messages.as_slice().iter().map(message_view));
                        }
                    }
                    wire::MessagingResult::Message(_) => (),
                    wire::MessagingResult::Send(receipt) if receipt.operation == operation => {
                        completion = match receipt.state {
                            SendState::ServerAccepted(_) => Completion::ServerAccepted,
                            SendState::Rejected => {
                                Completion::Failed(UnavailableReason::PermissionDenied)
                            }
                            SendState::Uncertain => Completion::Uncertain,
                        };
                        reload = completion == Completion::ServerAccepted;
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
                        panel.reminders = page(reminder.into_iter().map(|r| view::Reminder {
                            message: u128::from(r.message.sequence),
                            timestamp: r.timestamp,
                        }));
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
                        panel.scheduled = page(std::iter::once(view::Scheduled {
                            id: u128::from(schedule.id.sequence),
                            body: text(schedule.text.as_str()),
                            timestamp: schedule.send_at,
                        }));
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
                        reload = !matches!(work, Work::Read(_) | Work::Typing | Work::Unread);
                        if let Work::Read(id) = work {
                            let reached_latest = self
                                .snapshot
                                .messages
                                .as_slice()
                                .last()
                                .is_some_and(|m| m.local_id <= id);
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
            Completion::ServerAccepted => RemoteState::Idle,
            Completion::Failed(reason) => RemoteState::Failed(request, reason),
            Completion::Uncertain => RemoteState::Uncertain(request),
        };
        if matches!(work, Work::History | Work::Older) {
            panel.history = state;
        }
        if matches!(
            work,
            Work::Action | Work::PinChange | Work::Share | Work::Send
        ) {
            panel.state = state;
            panel.result = Some((request, completion));
        }
        if work == Work::Send {
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
        if work == Work::PinChange && completion == Completion::ServerAccepted {
            let _ = self.start_message(request, context, wire::MessagingCommand::Pins, Work::Pins);
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
        } => {
            messages.as_slice().iter().all(message)
                && cursor.is_none_or(|id| id.conversation == context.conversation)
                && common_read.is_none_or(|id| id.conversation == context.conversation)
        }
        wire::MessagingResult::Message(value) => message(value),
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
            expires_at: m.message.expires_at,
            timestamp: m.message.timestamp,
            truncated,
            edited: m.message.edited_at.is_some(),
            deleted,
            system: m.message.kind == protocol::MessageKind::System,
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
// UTC is explicit until native locale-aware timestamp formatting is available.
fn time_label(seconds: u64) -> String {
    format!("{:02}:{:02} UTC", seconds / 3600 % 24, seconds / 60 % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
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
