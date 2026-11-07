//! Account/room-keyed optimistic intents. Canonical observations stay in History;
//! overlays are replayed over the latest observation, never rolled back from an
//! old copy. A durable dispatch marker precedes every possible network effect.
#[cfg(test)]
mod tests;
use super::*;
use talk_adapters::identity::{
    DataCommand, DataResult, OutboxEntry, OutboxIntent as Intent, OutboxOptions, OutboxPhase,
    OutboxPrivateReply, OutboxRecord,
};
use talk_core::preferences::navigation::NavigationRoomKey;
use talk_core::protocol::messaging as wire;
use talk_core::ui::{
    messaging as view,
    presentation::{DeliveryState, MessageView},
};

const CAPACITY: usize = 64;
const BYTES: usize = 512 * 1024;
#[derive(Clone, Copy)]
pub(super) enum Work {
    Save(AccountId, OperationId),
    Mark(AccountId, OperationId),
    Remove(AccountId, OperationId),
    Fail(AccountId, OperationId),
}
#[derive(Clone, Copy, Eq, PartialEq)]
enum State {
    Saving,
    Queued,
    Marking,
    Ready,
    Running,
    Uncertain,
    ParkedUncertain,
    Confirming,
    Removing,
    Failed,
}
struct Entry {
    account: AccountId,
    record: OutboxRecord,
    intent: Intent,
    state: State,
    durable: bool,
    request: Option<(RequestId, ConversationRef)>,
    context: Option<ConversationRef>,
    created: u64,
    uncertain_notified: bool,
    failed_saved: bool,
    replaces: Option<OperationId>,
    retry: Instant,
    ordinal: u64,
}
impl Entry {
    fn delivery(&self) -> DeliveryState {
        if self.record.phase == OutboxPhase::Uncertain
            || matches!(
                self.state,
                State::Uncertain | State::ParkedUncertain | State::Confirming
            )
        {
            DeliveryState::Uncertain
        } else if matches!(self.state, State::Queued | State::Marking | State::Ready) {
            DeliveryState::Queued
        } else {
            DeliveryState::Pending
        }
    }
}
#[derive(Default)]
pub(super) struct Outbox {
    entries: Vec<Entry>,
    loaded: Vec<AccountId>,
    errors: Vec<(AccountId, NavigationRoomKey, u64, view::MessageActionError)>,
    next_error: u64,
}
impl Outbox {
    fn next_index(&self, account: AccountId, now: Instant) -> Option<usize> {
        if self.entries.iter().any(|entry| {
            entry.account == account && matches!(entry.state, State::Running | State::Marking)
        }) {
            return None;
        }
        self.entries
            .iter()
            .enumerate()
            .find(|(index, entry)| {
                entry.account == account
                    && now >= entry.retry
                    && !(entry.state == State::Failed
                        && retain_failure(&entry.intent)
                        && entry.failed_saved)
                    && !self.entries[..*index].iter().any(|earlier| {
                        earlier.account == account
                            && !matches!(earlier.state, State::Removing | State::Failed)
                            && conflicts(earlier, entry)
                    })
            })
            .map(|(index, _)| index)
    }
    pub(super) fn retire_account(&mut self, account: AccountId) {
        self.entries.retain(|e| e.account != account);
        self.loaded.retain(|a| *a != account);
        self.errors.retain(|(a, _, _, _)| *a != account);
    }
    pub(super) fn lost_scope(&mut self, operation: OperationId) {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|e| e.record.operation == operation)
        {
            entry.state = State::Uncertain;
            entry.record.phase = OutboxPhase::Uncertain;
            entry.context = None;
            entry.retry = Instant::now() + Duration::from_secs(1);
        }
    }
    pub(super) fn contains(&self, operation: OperationId) -> bool {
        self.entries.iter().any(|e| e.record.operation == operation)
    }
    pub(super) fn load(&mut self, account: AccountId, entries: &Page<OutboxEntry, 64>) {
        if self.loaded.contains(&account) {
            return;
        }
        if self.loaded.len() >= 8 {
            return;
        }
        self.loaded.push(account);
        for value in entries.as_slice() {
            if self.contains(value.record.operation)
                || self.entries.iter().filter(|e| e.account == account).count() >= CAPACITY
            {
                continue;
            }
            let Ok(intent) = value.record.intent() else {
                self.error(
                    account,
                    value.record.room.clone(),
                    UnavailableReason::PolicyStale,
                );
                continue;
            };
            let mut record = value.record.clone();
            if !matches!(record.phase, OutboxPhase::Queued | OutboxPhase::Failed) {
                record.phase = OutboxPhase::Uncertain;
            }
            self.entries.push(Entry {
                account,
                record,
                intent,
                state: match value.record.phase {
                    OutboxPhase::Queued => State::Queued,
                    OutboxPhase::Failed => State::Failed,
                    _ => State::Uncertain,
                },
                durable: true,
                request: None,
                context: None,
                created: value.saved_at,
                uncertain_notified: false,
                failed_saved: true,
                replaces: None,
                retry: Instant::now(),
                ordinal: value.ordinal,
            });
        }
        self.entries
            .sort_by_key(|e| (e.account.as_local(), e.ordinal));
    }
    fn error(&mut self, account: AccountId, room: NavigationRoomKey, reason: UnavailableReason) {
        self.notice(account, room, view::MessageActionError::Failed(reason));
    }
    fn notice(
        &mut self,
        account: AccountId,
        room: NavigationRoomKey,
        reason: view::MessageActionError,
    ) {
        self.next_error = self.next_error.saturating_add(1);
        self.errors
            .retain(|(a, r, _, _)| *a != account || *r != room);
        if self.errors.len() >= 8 {
            self.errors.remove(0);
        }
        self.errors.push((account, room, self.next_error, reason));
    }
    pub(super) fn project(
        &self,
        context: ConversationRef,
        key: &NavigationRoomKey,
        author: &str,
        messages: &mut Page<MessageView, 100>,
        panel: &mut view::MessagePanel,
    ) {
        panel.queued_actions = 0;
        panel.unread_queued = false;
        panel.thread_following_intent = None;
        panel.failed_drafts = Page::empty();
        for entry in self
            .entries
            .iter()
            .filter(|e| e.account == context.account.account && &e.record.room == key)
        {
            if (entry.state == State::Failed || entry.replaces.is_some())
                && let Some(draft) = failed_draft(entry)
            {
                let _ = panel.failed_drafts.try_push(draft);
            }
        }
        panel.action_error = self
            .errors
            .iter()
            .find(|(a, r, _, _)| *a == context.account.account && r == key)
            .map(|(_, _, id, e)| (*id, *e));
        for entry in self.entries.iter().filter(|e| {
            e.account == context.account.account
                && (&e.record.room == key
                    || matches!(&e.intent,Intent::Forward{destination,..}if destination==key))
                && !matches!(e.state, State::Failed | State::Removing)
        }) {
            panel.queued_actions = panel.queued_actions.saturating_add(1);
            if matches!(entry.intent, Intent::MarkUnread) {
                panel.unread_queued = true;
            }
            if entry.state == State::ParkedUncertain {
                continue;
            }
            if let Intent::Forward {
                destination,
                author: source_author,
                body,
                ..
            } = &entry.intent
            {
                if destination == key {
                    let body = text(&format!(
                        "Forwarded message from {}:\n{}",
                        source_author.as_str(),
                        body.as_str()
                    ));
                    append_pending(entry, author, body, messages);
                }
            } else {
                overlay(entry, author, messages, panel);
            }
        }
    }
}
impl Accounts {
    pub(super) fn queue_room_unread(
        &mut self,
        request: RequestId,
        context: ConversationRef,
    ) -> Result<(), SubmitError> {
        if self.snapshot.active_account != Some(context.account)
            || !self
                .snapshot
                .conversations
                .as_slice()
                .iter()
                .any(|row| row.context == context)
        {
            return Err(SubmitError::StaleScope);
        }
        self.queue_message_action(request, context, &view::MessageAction::MarkUnread)
            .unwrap_or(Err(SubmitError::Unavailable(
                UnavailableReason::PolicyStale,
            )))
    }

    /// Overlay queued unread state over the newest room observation, not a
    /// stale copy captured before another device changed the room.
    pub(super) fn project_room_unread(&self, snapshot: &mut PresentationSnapshot) {
        snapshot.conversations = page(snapshot.conversations.as_slice().iter().cloned().map(
            |mut room| {
                if let Some(key) = room.navigation_key.as_ref()
                    && self.outbox.entries.iter().any(|entry| {
                        entry.account == room.context.account.account
                            && &entry.record.room == key
                            && matches!(entry.intent, Intent::MarkUnread)
                            && !matches!(entry.state, State::Failed | State::Removing)
                    })
                {
                    room.unread = room.unread.max(1);
                }
                room
            },
        ));
    }

    /// Apply pending settings to the published account page, leaving its server
    /// observation intact so a rejection can reveal it again.
    pub(super) fn project_thread_list(&self, snapshot: &mut PresentationSnapshot) {
        let Some(scope) = snapshot
            .threads
            .account
            .filter(|scope| Some(*scope) == snapshot.active_account)
        else {
            return;
        };
        let live = |entry: &&Entry| {
            entry.account == scope.account
                && !matches!(
                    entry.state,
                    State::Failed | State::Removing | State::ParkedUncertain
                )
        };
        let mut entries = Vec::new();
        for row in snapshot.threads.entries.as_slice() {
            let key = self.cache.stable_room(row.context).or_else(|| {
                snapshot
                    .conversations
                    .as_slice()
                    .iter()
                    .find(|room| room.context == row.context)
                    .and_then(|room| room.navigation_key.clone())
            });
            let mut row = row.clone();
            let mut followed = true;
            if let Some(key) = key {
                for entry in self
                    .outbox
                    .entries
                    .iter()
                    .filter(live)
                    .filter(|entry| entry.record.room == key)
                {
                    match &entry.intent {
                        Intent::RenameThread { root, title } if u128::from(*root) == row.root => {
                            row.title = title.clone()
                        }
                        Intent::ThreadNotifications { root, level }
                            if u128::from(*root) == row.root =>
                        {
                            followed = *level != 3
                        }
                        _ => (),
                    }
                }
            }
            if followed {
                entries.push(row);
            }
        }
        // A just-followed thread may not have appeared in the observed page.
        // Add it only from this open, scoped thread's bounded observation.
        if snapshot.threads.offset == 0
            && let Some(panel) = snapshot.message_panel.as_ref()
            && Some(panel.context) == snapshot.selected
            && panel.context.account == scope
            && panel.thread_following_intent == Some(true)
            && let Some(details) = panel.thread_details.as_ref()
            && panel.thread_root == Some(details.root)
            && !entries
                .iter()
                .any(|row| row.context == panel.context && row.root == details.root)
            && entries.len() < 50
        {
            let root = panel.thread_root_message.as_ref().or_else(|| {
                snapshot
                    .messages
                    .as_slice()
                    .iter()
                    .find(|row| row.local_id == details.root)
            });
            entries.insert(
                0,
                talk_core::ui::presentation::ThreadView {
                    context: panel.context,
                    root: details.root,
                    title: details.title.clone(),
                    replies: root.map_or(0, |row| u32::from(row.reply_count)),
                    last_message: panel
                        .thread_messages
                        .as_slice()
                        .last()
                        .map(|row| row.local_id),
                },
            );
        }
        snapshot.threads.entries = page(entries);
    }

    pub(super) fn outbox_available(&self, context: ConversationRef) -> bool {
        self.cache.persistable(context)
            || self.current(context.account).is_ok()
                && self
                    .snapshot
                    .conversations
                    .as_slice()
                    .iter()
                    .any(|row| row.context == context && row.navigation_key.is_some())
    }

    fn recovery_index(
        &self,
        context: ConversationRef,
        operation: u128,
    ) -> Result<usize, SubmitError> {
        if self.snapshot.selected != Some(context) {
            return Err(SubmitError::StaleScope);
        }
        let key = self
            .cache
            .stable_room(context)
            .or_else(|| {
                self.snapshot
                    .conversations
                    .as_slice()
                    .iter()
                    .find(|r| r.context == context)
                    .and_then(|r| r.navigation_key.clone())
            })
            .ok_or(SubmitError::StaleScope)?;
        self.outbox
            .entries
            .iter()
            .position(|e| {
                e.account == context.account.account
                    && e.record.room == key
                    && e.record.operation.as_local() == operation
                    && e.state == State::Failed
                    && retain_failure(&e.intent)
            })
            .ok_or(SubmitError::StaleScope)
    }
    fn retry_outbox(
        &mut self,
        context: ConversationRef,
        operation: u128,
    ) -> Result<(), SubmitError> {
        let index = self.recovery_index(context, operation)?;
        let entry = &self.outbox.entries[index];
        if !entry.failed_saved {
            return Err(SubmitError::Busy);
        }
        let previous = entry.record.operation;
        let operation = if entry.durable {
            self.connection
                .as_ref()
                .map_err(|e| submit_error(*e))?
                .new_local_operation_id(entry.account)
        } else {
            self.connection
                .as_ref()
                .map_err(|e| submit_error(*e))?
                .new_operation_id(context.account)
        }
        .map_err(submit_error)?;
        let mut record = entry.record.clone();
        record.operation = operation;
        record.phase = OutboxPhase::Queued;
        let durable = entry.durable;
        if durable {
            match self.admit(
                RuntimeCommand::AccountData {
                    account: context.account.account,
                    command: DataCommand::Replace {
                        previous,
                        record: record.clone(),
                    },
                },
                Purpose::Outbox(Work::Save(context.account.account, operation)),
            ) {
                Ok(_) | Err(IdentityError::Request(ErrorKind::Busy)) => (),
                Err(error) => return Err(submit_error(error)),
            }
        }
        let entry = &mut self.outbox.entries[index];
        entry.record = record;
        entry.state = if durable {
            State::Saving
        } else {
            State::Queued
        };
        entry.replaces = durable.then_some(previous);
        entry.context = Some(context);
        entry.created = history::now_unix();
        entry.retry = Instant::now();
        entry.uncertain_notified = false;
        entry.request = None;
        self.project_history(context);
        self.touch();
        Ok(())
    }
    fn discard_outbox(
        &mut self,
        context: ConversationRef,
        operation: u128,
    ) -> Result<(), SubmitError> {
        let index = self.recovery_index(context, operation)?;
        if !self.outbox.entries[index].failed_saved {
            return Err(SubmitError::Busy);
        }
        self.outbox.entries[index].state = State::Removing;
        self.outbox.entries[index].retry = Instant::now();
        self.project_history(context);
        self.touch();
        Ok(())
    }
    pub(super) fn load_outbox(&mut self) {
        for account in self.snapshot.saved_accounts.as_slice() {
            if let Some(snapshot) = self.cache.loaded(account.id) {
                self.outbox.load(account.id, &snapshot.outbox);
            }
        }
    }
    pub(super) fn queue_message_action(
        &mut self,
        request: RequestId,
        context: ConversationRef,
        action: &view::MessageAction,
    ) -> Option<Result<(), SubmitError>> {
        match action {
            view::MessageAction::RetryQueued(operation) => {
                return Some(self.retry_outbox(context, *operation));
            }
            view::MessageAction::DiscardQueued(operation) => {
                return Some(self.discard_outbox(context, *operation));
            }
            _ => (),
        }
        let stable = self.cache.stable_room(context).or_else(|| {
            self.snapshot
                .conversations
                .as_slice()
                .iter()
                .find(|r| r.context == context)
                .and_then(|r| r.navigation_key.clone())
        })?;
        let converted = self.outbox_intent(context, action)?;
        Some((|| {
            if self.snapshot.selected != Some(context)
                && (!matches!(action, view::MessageAction::MarkUnread)
                    || self.snapshot.active_account != Some(context.account)
                    || !self
                        .snapshot
                        .conversations
                        .as_slice()
                        .iter()
                        .any(|row| row.context == context))
            {
                return Err(SubmitError::StaleScope);
            }
            let intent = converted?;
            if let Intent::RenameThread { root, .. } | Intent::ThreadNotifications { root, .. } =
                &intent
            {
                let panel = self
                    .snapshot
                    .message_panel
                    .as_ref()
                    .filter(|panel| {
                        panel.context == context && panel.thread_root == Some(u128::from(*root))
                    })
                    .ok_or(SubmitError::StaleScope)?;
                let details = panel
                    .thread_details
                    .as_ref()
                    .filter(|details| details.root == u128::from(*root))
                    .ok_or(SubmitError::StaleScope)?;
                let availability = panel.availability(view::MessageFeature::Threads);
                // A live thread observation proves this room exposed threads.
                // Local admission can outlive that observation's permission
                // TTL; the protocol owner revalidates before dispatch.
                if availability.server == talk_core::ui::presentation::FeatureSupport::Unsupported
                    || availability.allowed == Decision::Denied
                    || matches!(&intent, Intent::RenameThread { .. })
                        && details.rename == Decision::Denied
                    || matches!(&intent, Intent::ThreadNotifications { .. })
                        && details.notification_level.is_none()
                {
                    return Err(SubmitError::Unavailable(UnavailableReason::PolicyStale));
                }
            }
            let room = stable;
            // A repeated deliberate activation may replace an uncertain mark,
            // but must not create overlapping read-marker writes. Reapplying
            // after uncertainty is a new user intent, never an automatic retry.
            let replaced_unread = if matches!(intent, Intent::MarkUnread) {
                match self.outbox.entries.iter().position(|entry| {
                    entry.account == context.account.account
                        && entry.record.room == room
                        && matches!(entry.intent, Intent::MarkUnread)
                        && !matches!(entry.state, State::Failed | State::Removing)
                }) {
                    Some(index)
                        if matches!(
                            self.outbox.entries[index].state,
                            State::Uncertain | State::ParkedUncertain
                        ) =>
                    {
                        Some(index)
                    }
                    Some(_) => return Err(SubmitError::Busy),
                    None => None,
                }
            } else {
                None
            };
            let durable = self.cache.persistable(context)
                && match &intent {
                    Intent::Send { options, .. } => {
                        options.private_reply.as_ref().is_none_or(|p| {
                            self.outbox_context(context.account.account, &p.source)
                                .is_some_and(|source| self.cache.persistable(source))
                        })
                    }
                    _ => true,
                };
            if !durable {
                self.current(context.account)?;
            }
            let count = self
                .outbox
                .entries
                .iter()
                .filter(|e| e.account == context.account.account)
                .count();
            if count >= CAPACITY {
                return Err(SubmitError::Busy);
            }
            let operation = if durable {
                self.connection
                    .as_ref()
                    .map_err(|e| submit_error(*e))?
                    .new_local_operation_id(context.account.account)
            } else {
                self.connection
                    .as_ref()
                    .map_err(|e| submit_error(*e))?
                    .new_operation_id(context.account)
            }
            .map_err(submit_error)?;
            let record = OutboxRecord::from_intent(operation, room, &intent)
                .map_err(|_| SubmitError::Busy)?;
            let bytes = self
                .outbox
                .entries
                .iter()
                .filter(|e| e.account == context.account.account)
                .map(|e| e.record.payload.as_str().len())
                .sum::<usize>();
            if bytes.saturating_add(record.payload.as_str().len()) > BYTES {
                return Err(SubmitError::Busy);
            }
            if durable {
                match self.admit(
                    RuntimeCommand::AccountData {
                        account: context.account.account,
                        command: DataCommand::Enqueue(record.clone()),
                    },
                    Purpose::Outbox(Work::Save(context.account.account, operation)),
                ) {
                    Ok(_) | Err(IdentityError::Request(ErrorKind::Busy)) => (),
                    Err(error) => return Err(submit_error(error)),
                }
            }
            let created = history::now_unix();
            if let Some(index) = replaced_unread {
                self.outbox.entries[index].state = State::Removing;
            }
            self.outbox.entries.push(Entry {
                account: context.account.account,
                record,
                intent,
                state: if durable {
                    State::Saving
                } else {
                    State::Queued
                },
                durable,
                request: Some((request, context)),
                context: Some(context),
                created,
                uncertain_notified: false,
                failed_saved: false,
                replaces: None,
                retry: Instant::now(),
                ordinal: created,
            });
            if !durable {
                self.outbox_admitted(operation);
            }
            if matches!(
                action,
                view::MessageAction::RenameThread { .. }
                    | view::MessageAction::ThreadNotifications { .. }
            ) && let Some(panel) = self
                .snapshot
                .message_panel
                .as_mut()
                .filter(|panel| panel.context == context)
            {
                panel.thread_details_request = None;
            }
            self.project_history(context);
            self.touch();
            Ok(())
        })())
    }
    fn outbox_intent(
        &self,
        context: ConversationRef,
        action: &view::MessageAction,
    ) -> Option<Result<Intent, SubmitError>> {
        use view::MessageAction as A;
        let id = |id: u128| {
            u64::try_from(id)
                .ok()
                .filter(|n| *n > 0)
                .ok_or(SubmitError::StaleScope)
        };
        let options = |o: &view::ComposeTarget, body: &str| {
            if o.new_thread
                && (o.reply_to.is_some() || o.private_reply.is_some() || o.thread.is_some())
            {
                return Err(SubmitError::StaleScope);
            }
            Ok(OutboxOptions {
                reply_to: o.reply_to.map(id).transpose()?,
                private_reply: o
                    .private_reply
                    .as_ref()
                    .map(|private| {
                        let source = self
                            .outbox_context(context.account.account, &private.source)
                            .ok_or(SubmitError::StaleScope)?;
                        if source.account != context.account || source == context {
                            return Err(SubmitError::StaleScope);
                        }
                        Ok(OutboxPrivateReply {
                            source: private.source.clone(),
                            message: id(private.message)?,
                            author: private.author.clone(),
                            body: private.body.clone(),
                        })
                    })
                    .transpose()?,
                thread: o.thread.map(id).transpose()?,
                thread_title: o
                    .new_thread
                    .then(|| messaging::thread_title(body))
                    .transpose()?,
                silent: o.silent,
            })
        };
        Some(match action {
            A::Send { body, target } => {
                options(target, body.as_str()).map(|options| Intent::Send {
                    body: body.clone(),
                    options,
                })
            }
            A::Edit { message, body } => id(*message).map(|message| Intent::Edit {
                message,
                body: body.clone(),
            }),
            A::Delete(message) => id(*message).map(|message| Intent::Delete { message }),
            A::React {
                message,
                emoji,
                add,
            } => id(*message).map(|message| Intent::Reaction {
                message,
                emoji: emoji.clone(),
                add: *add,
            }),
            A::Pin(message) | A::Unpin(message) => id(*message).map(|message| Intent::Pin {
                message,
                pinned: matches!(action, A::Pin(_)),
            }),
            A::Reminder { message, timestamp } => id(*message).map(|message| Intent::Reminder {
                message,
                timestamp: *timestamp,
            }),
            A::Schedule {
                body,
                timestamp,
                target,
            } => {
                if target.new_thread {
                    Err(SubmitError::Unavailable(UnavailableReason::Unsupported))
                } else {
                    options(target, body.as_str()).map(|options| Intent::Schedule {
                        body: body.clone(),
                        timestamp: *timestamp,
                        options,
                    })
                }
            }
            A::CancelScheduled(message) => {
                id(*message).map(|message| Intent::CancelScheduled { message })
            }
            A::MarkUnread => Ok(Intent::MarkUnread),
            A::RenameThread { root, title } => id(*root).map(|root| Intent::RenameThread {
                root,
                title: title.clone(),
            }),
            A::ThreadNotifications { root, level } => {
                id(*root).map(|root| Intent::ThreadNotifications {
                    root,
                    level: *level,
                })
            }
            A::Forward {
                message,
                destination,
            } => (|| {
                if destination.account != context.account || *destination == context {
                    return Err(SubmitError::StaleScope);
                }
                let destination = self
                    .cache
                    .stable_room(*destination)
                    .or_else(|| {
                        self.snapshot
                            .conversations
                            .as_slice()
                            .iter()
                            .find(|r| r.context == *destination)
                            .and_then(|r| r.navigation_key.clone())
                    })
                    .ok_or(SubmitError::StaleScope)?;
                Ok(Intent::Forward {
                    message: id(*message)?,
                    destination,
                    author: self
                        .snapshot
                        .messages
                        .as_slice()
                        .iter()
                        .find(|m| m.local_id == *message)
                        .map(|m| text(m.author.as_str()))
                        .ok_or(SubmitError::StaleScope)?,
                    body: self
                        .snapshot
                        .messages
                        .as_slice()
                        .iter()
                        .find(|m| m.local_id == *message)
                        .map(|m| m.body.clone())
                        .ok_or(SubmitError::StaleScope)?,
                })
            })(),
            _ => return None,
        })
    }
    fn outbox_admitted(&mut self, operation: OperationId) {
        let Some(entry) = self
            .outbox
            .entries
            .iter_mut()
            .find(|e| e.record.operation == operation)
        else {
            return;
        };
        let Some((request, context)) = entry.request.take() else {
            return;
        };
        if matches!(entry.intent, Intent::Send { .. }) {
            // This acknowledgement releases the editor only after the durable
            // transaction (or explicit session-memory admission) has completed.
            self.snapshot.submission = Some(SubmissionResult {
                request,
                context,
                result: Completion::Queued,
            });
        } else if let Some(panel) = self.snapshot.message_panel.as_mut().filter(|p| {
            p.context == context
                || p.context.account.account == entry.account
                    && self.cache.stable_room(p.context).as_ref() == Some(&entry.record.room)
        }) {
            panel.result = Some((request, Completion::Queued));
            panel.state = talk_core::ui::conversation::RemoteState::Idle;
        }
        self.touch();
    }
    pub(super) fn complete_outbox_data(
        &mut self,
        work: Work,
        result: Result<RuntimeReply, IdentityError>,
    ) {
        let (account, operation) = match work {
            Work::Save(a, o) | Work::Mark(a, o) | Work::Remove(a, o) | Work::Fail(a, o) => (a, o),
        };
        let result = match result {
            Ok(RuntimeReply::AccountData { account: a, result }) if a == account => result,
            _ => Err(talk_adapters::identity::DataError::Unavailable),
        };
        let Some(index) = self
            .outbox
            .entries
            .iter()
            .position(|e| e.account == account && e.record.operation == operation)
        else {
            return;
        };
        match (work, result) {
            (Work::Save(..), Ok(DataResult::Queued(saved)))
                if saved.record == self.outbox.entries[index].record =>
            {
                self.outbox.entries[index].state = State::Queued;
                self.outbox.entries[index].ordinal = saved.ordinal;
                self.outbox.entries[index].replaces = None;
                self.outbox.entries[index].failed_saved = false;
                self.outbox_admitted(operation);
            }
            (Work::Save(..), Err(talk_adapters::identity::DataError::InvalidInput))
                if history::now_unix().saturating_sub(self.outbox.entries[index].created) < 8
                    && self
                        .cache
                        .metadata_pending(account, &self.outbox.entries[index].record.room) =>
            {
                self.outbox.entries[index].retry = Instant::now() + Duration::from_millis(200);
            }
            (
                Work::Save(..),
                Err(
                    talk_adapters::identity::DataError::Unavailable
                    | talk_adapters::identity::DataError::Corrupt,
                ),
            ) => {
                // A lost storage acknowledgement is not proof of non-admission.
                // Retrying the same complete record is idempotent; retain the
                // editor and observation until the owner can settle that record.
                self.outbox.entries[index].retry = Instant::now() + Duration::from_secs(5);
            }
            (Work::Save(..), _) => {
                if let Some(previous) = self.outbox.entries[index].replaces.take() {
                    self.outbox.entries[index].record.operation = previous;
                    self.outbox.entries[index].record.phase = OutboxPhase::Failed;
                    self.outbox.entries[index].state = State::Failed;
                    self.outbox.entries[index].failed_saved = true;
                    let room = self.outbox.entries[index].record.room.clone();
                    self.outbox.error(account, room, UnavailableReason::Busy);
                    if let Some(context) = self.snapshot.selected {
                        self.project_history(context)
                    }
                    self.touch();
                    return;
                }
                let entry = self.outbox.entries.remove(index);
                if let Some((request, context)) = entry.request {
                    if matches!(entry.intent, Intent::Send { .. }) {
                        self.snapshot.submission = Some(SubmissionResult {
                            request,
                            context,
                            result: Completion::Failed(UnavailableReason::Busy),
                        });
                    } else if let Some(panel) = self
                        .snapshot
                        .message_panel
                        .as_mut()
                        .filter(|p| p.context == context)
                    {
                        panel.result = Some((request, Completion::Failed(UnavailableReason::Busy)));
                    }
                }
                self.outbox
                    .error(account, entry.record.room, UnavailableReason::Busy);
            }
            (Work::Mark(..), Ok(DataResult::Saved)) => {
                self.outbox.entries[index].state = State::Ready
            }
            (Work::Mark(..), _) => {
                self.outbox.entries[index].state = State::Queued;
                self.outbox.entries[index].retry = Instant::now() + Duration::from_secs(5);
            }
            (Work::Remove(..), Ok(DataResult::Saved)) => {
                self.outbox.entries.remove(index);
            }
            (Work::Remove(..), _) => {
                self.outbox.entries[index].retry = Instant::now() + Duration::from_secs(5);
            }
            (Work::Fail(..), Ok(DataResult::Saved)) => {
                self.outbox.entries[index].failed_saved = true
            }
            (Work::Fail(..), _) => {
                self.outbox.entries[index].retry = Instant::now() + Duration::from_secs(5)
            }
        }
        if let Some(context) = self.snapshot.selected {
            self.project_history(context)
        }
        self.touch();
    }
    pub(super) fn tick_outbox(&mut self) {
        let now = Instant::now();
        // Memory-only scopes retire with their account. Durable intents survive
        // network outages but are discarded by the credential owner's logout.
        self.outbox.entries.retain(|e| {
            if e.durable {
                self.snapshot.saved_accounts.as_slice().iter().any(|a| {
                    a.id == e.account
                        && !matches!(a.phase, SavedAccountPhase::ReauthenticationRequired)
                })
            } else {
                e.context.is_some_and(|c| {
                    self.snapshot
                        .accounts
                        .as_slice()
                        .iter()
                        .any(|a| a.scope == c.account)
                })
            }
        });
        self.outbox.loaded.retain(|id| {
            self.snapshot
                .saved_accounts
                .as_slice()
                .iter()
                .any(|a| a.id == *id)
        });
        self.outbox.errors.retain(|(id, _, _, _)| {
            self.snapshot
                .saved_accounts
                .as_slice()
                .iter()
                .any(|a| a.id == *id)
                || self
                    .snapshot
                    .accounts
                    .as_slice()
                    .iter()
                    .any(|a| a.scope.account == *id)
        });
        let accounts: Vec<AccountId> = self
            .snapshot
            .saved_accounts
            .as_slice()
            .iter()
            .map(|a| a.id)
            .chain(
                self.snapshot
                    .accounts
                    .as_slice()
                    .iter()
                    .map(|a| a.scope.account),
            )
            .collect();
        let revoked: Vec<_> = self
            .outbox
            .entries
            .iter()
            .filter(|e| {
                self.cache.revoked(e.account, &e.record.room)
                    && !matches!(e.state, State::Failed | State::Removing | State::Running)
            })
            .map(|e| e.record.operation)
            .collect();
        for operation in revoked {
            self.fail_outbox(operation, UnavailableReason::PermissionDenied);
        }
        for account in accounts {
            if self.pending.iter().any(|p|matches!(p.purpose,Purpose::Outbox(Work::Save(a,_)|Work::Mark(a,_)|Work::Remove(a,_)|Work::Fail(a,_))if a==account)) {continue}
            let Some(index) = self.outbox.next_index(account, now) else {
                continue;
            };
            let entry = &self.outbox.entries[index];
            if now < entry.retry
                || matches!(entry.state, State::Marking | State::Running)
                || self.snapshot.submission.is_some()
            {
                continue;
            }
            let operation = entry.record.operation;
            if entry.state == State::Saving {
                let command = match entry.replaces {
                    Some(previous) => DataCommand::Replace {
                        previous,
                        record: entry.record.clone(),
                    },
                    None => DataCommand::Enqueue(entry.record.clone()),
                };
                let _ = self.admit(
                    RuntimeCommand::AccountData { account, command },
                    Purpose::Outbox(Work::Save(account, operation)),
                );
                continue;
            }
            if matches!(entry.state, State::Removing | State::Failed) {
                if entry.state == State::Failed && retain_failure(&entry.intent) {
                    if !entry.durable {
                        self.outbox.entries[index].failed_saved = true;
                        continue;
                    }
                    if self
                        .admit(
                            RuntimeCommand::AccountData {
                                account,
                                command: DataCommand::SetPhase {
                                    operation,
                                    phase: OutboxPhase::Failed,
                                },
                            },
                            Purpose::Outbox(Work::Fail(account, operation)),
                        )
                        .is_ok()
                    {
                        self.outbox.entries[index].retry = now + Duration::from_secs(5);
                    }
                    continue;
                }
                if !entry.durable {
                    self.outbox.entries.remove(index);
                    continue;
                }
                if self
                    .admit(
                        RuntimeCommand::AccountData {
                            account,
                            command: DataCommand::Acknowledge(operation),
                        },
                        Purpose::Outbox(Work::Remove(account, operation)),
                    )
                    .is_ok()
                {
                    self.outbox.entries[index].retry = now + Duration::from_secs(5);
                }
                continue;
            }
            let key = entry.record.room.clone();
            let Some(mut context) = self.outbox_context(account, &key) else {
                self.outbox.entries[index].retry = now + Duration::from_secs(2);
                continue;
            };
            if self
                .snapshot
                .accounts
                .as_slice()
                .iter()
                .find(|a| a.scope == context.account)
                .is_none_or(|a| {
                    matches!(
                        a.connection_failure,
                        Some(
                            ConnectionFailure::Unreachable
                                | ConnectionFailure::TimedOut
                                | ConnectionFailure::ServerUnavailable
                        )
                    )
                })
            {
                continue;
            }
            if entry.state == State::Queued {
                if entry.durable {
                    if self
                        .admit(
                            RuntimeCommand::AccountData {
                                account,
                                command: DataCommand::SetPhase {
                                    operation,
                                    phase: OutboxPhase::Dispatched,
                                },
                            },
                            Purpose::Outbox(Work::Mark(account, operation)),
                        )
                        .is_ok()
                    {
                        self.outbox.entries[index].state = State::Marking;
                    }
                    continue;
                }
                self.outbox.entries[index].state = State::Ready;
            }
            let entry = &self.outbox.entries[index];
            let uncertain = matches!(
                entry.state,
                State::Uncertain | State::ParkedUncertain | State::Confirming
            );
            if uncertain && let Intent::Forward { destination, .. } = &entry.intent {
                let Some(destination) = self.outbox_context(account, destination) else {
                    self.outbox.entries[index].retry = now + Duration::from_secs(2);
                    continue;
                };
                context = destination;
            }
            let built = if uncertain {
                reconcile_command(context, &entry.intent)
            } else {
                command(context, &entry.intent, |key| {
                    self.outbox_context(account, key)
                })
            };
            let Some((command, work)) = built else {
                self.outbox.entries[index].retry = now + Duration::from_secs(10);
                continue;
            };
            match self.start_outbox_message(context, operation, command, work) {
                Ok(()) => {
                    self.outbox.entries[index].state = State::Running;
                    self.outbox.entries[index].context = Some(context);
                    self.outbox.entries[index].record.phase = if uncertain {
                        OutboxPhase::Uncertain
                    } else {
                        OutboxPhase::Dispatched
                    };
                    self.project_history(context);
                }
                Err(SubmitError::Busy) => {
                    self.outbox.entries[index].retry = now + Duration::from_millis(100);
                }
                Err(SubmitError::StaleScope) => {
                    self.outbox.entries[index].retry = now + Duration::from_secs(2)
                }
                Err(SubmitError::Unavailable(reason)) => self.fail_outbox(operation, reason),
            }
        }
    }
    fn outbox_context(
        &self,
        account: AccountId,
        key: &NavigationRoomKey,
    ) -> Option<ConversationRef> {
        self.outbox_context_with(account, key, |scope| self.current(scope).is_ok())
    }
    // Pure context-resolution seam: credential currency is independently owned.
    fn outbox_context_with(
        &self,
        account: AccountId,
        key: &NavigationRoomKey,
        is_current: impl Fn(AccountScope) -> bool,
    ) -> Option<ConversationRef> {
        // A restored account can already be current while its retained room
        // rows still have presentation-only IDs. Resolve a protocol-owned room
        // before the caller may durably mark dispatch or create any request.
        let live = |context: ConversationRef| {
            context.is_consistent()
                && context.account.account == account
                && context.account.runtime.as_local() != 0
                && context.conversation.host.as_local() != 0
                && is_current(context.account)
        };
        self.cache
            .context(account, key)
            .filter(|c| live(*c))
            .or_else(|| {
                self.snapshot
                    .accounts
                    .as_slice()
                    .iter()
                    .filter(|a| a.scope.account == account)
                    .filter_map(|a| self.background.account(a.scope))
                    .flat_map(|a| a.rooms.as_slice())
                    .find(|r| r.navigation_key.as_ref() == Some(key) && live(r.context))
                    .map(|r| r.context)
            })
    }
    fn fail_outbox(&mut self, operation: OperationId, reason: UnavailableReason) {
        let Some(entry) = self
            .outbox
            .entries
            .iter_mut()
            .find(|e| e.record.operation == operation)
        else {
            return;
        };
        entry.state = State::Failed;
        entry.failed_saved = !entry.durable;
        if retain_failure(&entry.intent) {
            entry.record.phase = OutboxPhase::Failed;
        }
        let account = entry.account;
        let room = entry.record.room.clone();
        self.outbox.error(account, room, reason);
        self.touch();
    }
    pub(super) fn outbox_result(
        &mut self,
        operation: OperationId,
        reply: &Result<wire::MessagingResult, Completion>,
    ) -> bool {
        let Some(index) = self
            .outbox
            .entries
            .iter()
            .position(|e| e.record.operation == operation)
        else {
            return false;
        };
        let result = reply.as_ref().ok();
        let is_reconcile = self.outbox.entries[index].record.phase == OutboxPhase::Uncertain;
        let confirmed_thread = match (
            &self.outbox.entries[index].intent,
            result,
            self.outbox.entries[index].context,
        ) {
            (
                intent @ (Intent::RenameThread { .. } | Intent::ThreadNotifications { .. }),
                Some(wire::MessagingResult::Thread(thread)),
                Some(context),
            ) if reconciled(intent, &wire::MessagingResult::Thread(thread.clone())) => {
                Some((context, intent.clone(), thread.as_ref().clone()))
            }
            _ => None,
        };
        if !is_reconcile
            && matches!(self.outbox.entries[index].intent, Intent::MarkUnread)
            && matches!(result, Some(wire::MessagingResult::Changed))
            && let Some(context) = self.outbox.entries[index].context
        {
            self.remember_room_unread(context);
        }
        match result {
            Some(wire::MessagingResult::Send(receipt)) if receipt.state == SendState::Rejected => {
                self.fail_outbox(operation, UnavailableReason::PermissionDenied)
            }
            Some(wire::MessagingResult::Send(receipt))
                if matches!(receipt.state, SendState::ServerAccepted(_))
                    && !is_reconcile
                    && matches!(self.outbox.entries[index].intent, Intent::Forward { .. }) =>
            {
                self.outbox.entries[index].state = State::Confirming;
                self.outbox.entries[index].record.phase = OutboxPhase::Uncertain;
            }
            Some(wire::MessagingResult::Send(receipt)) if receipt.state == SendState::Uncertain => {
                self.outbox.entries[index].state = State::Uncertain;
                self.outbox.entries[index].record.phase = OutboxPhase::Uncertain;
                self.outbox.entries[index].retry = Instant::now() + Duration::from_secs(10);
            }
            Some(result) if is_reconcile => {
                let intent = &self.outbox.entries[index].intent;
                if reconciled(intent, result) {
                    self.outbox.entries[index].state = State::Removing;
                } else if matches!(
                    intent,
                    Intent::Send { .. } | Intent::Forward { .. } | Intent::Schedule { .. }
                ) {
                    self.outbox.entries[index].state = State::Uncertain;
                    self.outbox.entries[index].retry = Instant::now() + Duration::from_secs(30);
                } else {
                    self.outbox.entries[index].state = State::ParkedUncertain;
                    self.outbox.entries[index].retry = Instant::now() + Duration::from_secs(30);
                }
            }
            Some(result)
                if matches!(
                    self.outbox.entries[index].intent,
                    Intent::RenameThread { .. } | Intent::ThreadNotifications { .. }
                ) && !reconciled(&self.outbox.entries[index].intent, result) =>
            {
                self.outbox.entries[index].state = State::Uncertain;
                self.outbox.entries[index].record.phase = OutboxPhase::Uncertain;
                self.outbox.entries[index].retry = Instant::now() + Duration::from_secs(3);
            }
            Some(_) => self.outbox.entries[index].state = State::Removing,
            None => match reply {
                Err(Completion::Failed(
                    UnavailableReason::Busy
                    | UnavailableReason::Connection(
                        ConnectionFailure::Unreachable
                        | ConnectionFailure::TimedOut
                        | ConnectionFailure::ServerUnavailable,
                    ),
                )) => {
                    // Adapter returns these only before a mutation could happen;
                    // effect uncertainty is explicitly ErrorKind::Uncertain.
                    self.outbox.entries[index].state = if is_reconcile {
                        State::Uncertain
                    } else {
                        State::Ready
                    };
                    self.outbox.entries[index].retry = Instant::now() + Duration::from_secs(3);
                }
                Err(Completion::Uncertain) => {
                    self.outbox.entries[index].state = State::Uncertain;
                    self.outbox.entries[index].record.phase = OutboxPhase::Uncertain;
                    self.outbox.entries[index].retry = Instant::now() + Duration::from_secs(3);
                }
                Err(Completion::Failed(_)) if is_reconcile => {
                    self.outbox.entries[index].state = State::ParkedUncertain;
                    self.outbox.entries[index].retry = Instant::now() + Duration::from_secs(30);
                }
                Err(Completion::Failed(reason)) => self.fail_outbox(operation, *reason),
                _ => {
                    self.outbox.entries[index].state = State::Uncertain;
                    self.outbox.entries[index].record.phase = OutboxPhase::Uncertain;
                }
            },
        }
        if self.outbox.entries[index].state == State::Removing
            && let Some((context, intent, thread)) = confirmed_thread
        {
            self.confirm_thread_list(context, &intent, &thread);
        }
        if matches!(
            self.outbox.entries[index].state,
            State::Uncertain | State::ParkedUncertain
        ) && !self.outbox.entries[index].uncertain_notified
        {
            self.outbox.entries[index].uncertain_notified = true;
            let entry = &self.outbox.entries[index];
            self.outbox.notice(
                entry.account,
                entry.record.room.clone(),
                view::MessageActionError::Uncertain,
            );
        }
        true
    }

    fn confirm_thread_list(
        &mut self,
        context: ConversationRef,
        intent: &Intent,
        thread: &wire::Thread,
    ) {
        use talk_core::ui::presentation::{FeatureSupport, ThreadView};
        if self.snapshot.threads.account != Some(context.account)
            || self.snapshot.threads.support != FeatureSupport::Supported
        {
            return;
        }
        let root = u128::from(thread.root.sequence);
        let mut rows = self.snapshot.threads.entries.as_slice().to_vec();
        let index = rows
            .iter()
            .position(|row| row.context == context && row.root == root);
        if matches!(intent, Intent::ThreadNotifications { level: 3, .. }) {
            if let Some(index) = index {
                rows.remove(index);
            }
        } else {
            let observed = ThreadView {
                context,
                root,
                title: thread.title.clone(),
                replies: thread.replies,
                last_message: thread.last_message.map(|id| u128::from(id.sequence)),
            };
            if let Some(index) = index {
                rows[index] = observed;
            } else if matches!(intent, Intent::ThreadNotifications { .. })
                && self.snapshot.threads.offset == 0
            {
                if rows.len() == 50 {
                    rows.pop();
                    self.snapshot.threads.has_next = true;
                }
                rows.insert(0, observed);
            }
        }
        self.snapshot.threads.entries = page(rows);
    }
    pub(super) fn project_outbox(&mut self, context: ConversationRef) {
        let Some(key) = self.cache.stable_room(context).or_else(|| {
            self.snapshot
                .conversations
                .as_slice()
                .iter()
                .find(|r| r.context == context)
                .and_then(|r| r.navigation_key.clone())
        }) else {
            return;
        };
        let author = self
            .snapshot
            .accounts
            .as_slice()
            .iter()
            .find(|a| a.scope == context.account)
            .map_or("", |a| a.self_name.as_str());
        if let Some(panel) = self
            .snapshot
            .message_panel
            .as_mut()
            .filter(|p| p.context == context)
        {
            self.outbox
                .project(context, &key, author, &mut self.snapshot.messages, panel);
        }
    }
}
fn retain_failure(intent: &Intent) -> bool {
    matches!(
        intent,
        Intent::Send { .. } | Intent::Edit { .. } | Intent::Schedule { .. }
    )
}
fn failed_draft(entry: &Entry) -> Option<view::FailedDraft> {
    let target = |o: &OutboxOptions| view::ComposeTarget {
        reply_to: o.reply_to.map(u128::from),
        private_reply: o
            .private_reply
            .as_ref()
            .map(|private| view::PrivateReplyTarget {
                source: private.source.clone(),
                message: u128::from(private.message),
                author: private.author.clone(),
                body: private.body.clone(),
            }),
        thread: o.thread.map(u128::from),
        new_thread: o.thread_title.is_some(),
        silent: o.silent,
    };
    let (body, target, kind) = match &entry.intent {
        Intent::Send { body, options } => {
            (body.clone(), target(options), view::FailedDraftKind::Send)
        }
        Intent::Edit { message, body } => (
            body.clone(),
            Default::default(),
            view::FailedDraftKind::Edit(u128::from(*message)),
        ),
        Intent::Schedule {
            body,
            timestamp,
            options,
        } => (
            body.clone(),
            target(options),
            view::FailedDraftKind::Schedule(*timestamp),
        ),
        _ => return None,
    };
    Some(view::FailedDraft {
        operation: entry.replaces.unwrap_or(entry.record.operation).as_local(),
        body,
        target,
        kind,
        retrying: entry.replaces.is_some() || !entry.failed_saved,
    })
}
fn append_pending(
    entry: &Entry,
    author: &str,
    body: Text<4096>,
    messages: &mut Page<MessageView, 100>,
) {
    let id = (1u128 << 127) | (entry.record.operation.as_local() & ((1u128 << 127) - 1));
    if messages.as_slice().iter().any(|m| m.local_id == id) {
        return;
    }
    if messages.as_slice().len() == 100 {
        *messages = page(messages.as_slice().iter().skip(1).cloned());
    }
    let mut row = MessageView {
        local_id: id,
        author: text(author),
        initials: initials(author),
        time_label: text(&messaging::time_label(entry.created)),
        body,
        own: true,
        delivery: entry.delivery(),
        attachments: Page::empty(),
        reply_count: 0,
        metadata: Default::default(),
    };
    row.metadata.timestamp = entry.created;
    row.metadata.local_operation = Some(entry.record.operation.as_local());
    row.metadata.local_order = entry.request.map(|(request, _)| request.0);
    let _ = messages.try_push(row);
}
fn conflicts(earlier: &Entry, later: &Entry) -> bool {
    fn outgoing(e: &Entry) -> Option<&NavigationRoomKey> {
        match &e.intent {
            Intent::Send { .. } => Some(&e.record.room),
            Intent::Forward { destination, .. } => Some(destination),
            _ => None,
        }
    }
    if let (Some(a), Some(b)) = (outgoing(earlier), outgoing(later)) {
        return a == b;
    }
    if earlier.record.room != later.record.room {
        return false;
    }
    let message = |intent: &Intent| match intent {
        Intent::Edit { message, .. }
        | Intent::Delete { message }
        | Intent::Reaction { message, .. }
        | Intent::Pin { message, .. }
        | Intent::Reminder { message, .. } => Some(*message),
        Intent::RenameThread { root, .. } | Intent::ThreadNotifications { root, .. } => Some(*root),
        _ => None,
    };
    if let (Some(a), Some(b)) = (message(&earlier.intent), message(&later.intent)) {
        return a == b;
    }
    match (&earlier.intent, &later.intent) {
        (Intent::Schedule { timestamp: a, .. }, Intent::Schedule { timestamp: b, .. }) => a == b,
        (Intent::CancelScheduled { message: a }, Intent::CancelScheduled { message: b }) => a == b,
        _ => false,
    }
}
fn command(
    context: ConversationRef,
    intent: &Intent,
    resolve: impl Fn(&NavigationRoomKey) -> Option<ConversationRef>,
) -> Option<(wire::MessagingCommand, messaging::Work)> {
    use messaging::Work as W;
    use wire::MessagingCommand as C;
    let mid = |sequence| protocol::MessageId {
        conversation: context.conversation,
        sequence,
    };
    let opts = |o: &OutboxOptions| {
        Some(wire::ComposeOptions {
            reply_to: o.reply_to.map(mid),
            private_reply: match &o.private_reply {
                Some(private) => {
                    let source = resolve(&private.source)?;
                    if source.account != context.account || source == context {
                        return None;
                    }
                    Some(Box::new(protocol::MessageId {
                        conversation: source.conversation,
                        sequence: private.message,
                    }))
                }
                None => None,
            },
            thread: o.thread.map(mid),
            thread_title: o.thread_title.clone(),
            silent: o.silent,
        })
    };
    Some(match intent {
        Intent::Send { body, options } => (
            C::Send {
                text: text(body.as_str()),
                options: opts(options)?,
            },
            W::Send,
        ),
        Intent::Edit { message, body } => (
            C::Edit {
                message: mid(*message),
                text: text(body.as_str()),
            },
            W::Action,
        ),
        Intent::Delete { message } => (C::Delete(mid(*message)), W::Action),
        Intent::Reaction {
            message,
            emoji,
            add,
        } => (
            C::React {
                message: mid(*message),
                emoji: emoji.clone(),
                add: *add,
            },
            W::Reaction(u128::from(*message)),
        ),
        Intent::Pin { message, pinned } => (
            if *pinned {
                C::Pin {
                    message: mid(*message),
                    until: 0,
                }
            } else {
                C::Unpin(mid(*message))
            },
            W::PinChange {
                message: u128::from(*message),
                pinned: *pinned,
            },
        ),
        Intent::Reminder { message, timestamp } => (
            C::Reminder {
                message: mid(*message),
                timestamp: *timestamp,
            },
            W::Reminder(u128::from(*message)),
        ),
        Intent::Schedule {
            body,
            timestamp,
            options,
        } => (
            C::Schedule {
                text: text(body.as_str()),
                send_at: *timestamp,
                options: opts(options)?,
            },
            W::Action,
        ),
        Intent::CancelScheduled { message } => (
            C::CancelScheduled(mid(*message)),
            W::CancelScheduled(u128::from(*message)),
        ),
        Intent::MarkUnread => (C::MarkUnread, W::Unread),
        Intent::RenameThread { root, title } => (
            C::RenameThread {
                root: mid(*root),
                title: title.clone(),
            },
            W::ThreadQueued(u128::from(*root)),
        ),
        Intent::ThreadNotifications { root, level } => (
            C::ThreadNotifications {
                root: mid(*root),
                level: *level,
            },
            W::ThreadQueued(u128::from(*root)),
        ),
        Intent::Forward {
            message,
            destination,
            author,
            body,
        } => {
            let dest = resolve(destination)?;
            (
                C::ForwardApproved {
                    source: mid(*message),
                    destination: dest.conversation,
                    author: author.clone(),
                    body: body.clone(),
                },
                W::Forward(dest),
            )
        }
    })
}
fn reconcile_command(
    context: ConversationRef,
    intent: &Intent,
) -> Option<(wire::MessagingCommand, messaging::Work)> {
    use messaging::Work as W;
    use wire::MessagingCommand as C;
    let mid = |sequence| protocol::MessageId {
        conversation: context.conversation,
        sequence,
    };
    Some(match intent {
        // Repeating DELETE can move the read marker past messages that arrived
        // since the first attempt. An uncertain mutation needs user attention.
        Intent::MarkUnread => return None,
        Intent::RenameThread { root, .. } | Intent::ThreadNotifications { root, .. } => {
            (C::Thread(mid(*root)), W::ThreadQueued(u128::from(*root)))
        }
        Intent::Send { .. } => (C::ReconcileSend, W::Send),
        Intent::Forward { .. } => (C::ReconcileSend, W::Send),
        Intent::Edit { message, .. }
        | Intent::Delete { message }
        | Intent::Reaction { message, .. } => (C::Context(mid(*message)), W::Recovery),
        Intent::Pin { .. } => (C::Pins, W::Pins),
        Intent::Reminder { message, .. } => (
            C::ReadReminder(mid(*message)),
            W::Reminder(u128::from(*message)),
        ),
        Intent::Schedule { .. } | Intent::CancelScheduled { .. } => (C::Scheduled, W::Action),
    })
}
fn reconciled(intent: &Intent, result: &wire::MessagingResult) -> bool {
    let messages: Vec<&wire::MessageDetails> = match result {
        wire::MessagingResult::Message(m) => vec![m],
        wire::MessagingResult::Messages { messages, .. } => messages.as_slice().iter().collect(),
        _ => Vec::new(),
    };
    match intent {
        Intent::Send { .. } => matches!(
            result,
            wire::MessagingResult::Message(_)
                | wire::MessagingResult::Send(protocol::SendReceipt {
                    state: SendState::ServerAccepted(_),
                    ..
                })
        ),
        Intent::Edit { message, body } => messages
            .iter()
            .any(|m| m.message.id.sequence == *message && m.message.text.as_str() == body.as_str()),
        Intent::Delete { message } => messages.iter().any(|m| {
            m.message.id.sequence == *message && m.message.kind == protocol::MessageKind::Deleted
        }),
        Intent::Reaction {
            message,
            emoji,
            add,
        } => messages
            .iter()
            .find(|m| m.message.id.sequence == *message)
            .is_some_and(|m| {
                m.reactions
                    .as_slice()
                    .iter()
                    .any(|r| r.emoji == *emoji && r.own)
                    == *add
            }),
        Intent::Pin { message, pinned } => {
            matches!(result, wire::MessagingResult::Messages { .. })
                && messages.iter().any(|m| m.message.id.sequence == *message) == *pinned
        }
        Intent::Reminder { timestamp, .. } => {
            matches!(result,wire::MessagingResult::Reminder(value)if value.as_ref().map(|r|r.timestamp)==*timestamp)
        }
        Intent::CancelScheduled { message } => {
            matches!(result,wire::MessagingResult::Scheduled(values)if !values.as_slice().iter().any(|v|v.id.sequence==*message))
        }
        Intent::Forward { .. } => matches!(
            result,
            wire::MessagingResult::Message(_)
                | wire::MessagingResult::Send(protocol::SendReceipt {
                    state: SendState::ServerAccepted(_),
                    ..
                })
        ),
        Intent::Schedule { .. } => false,
        Intent::MarkUnread => false,
        Intent::RenameThread { root, title } => {
            matches!(result, wire::MessagingResult::Thread(thread)
            if thread.root.sequence == *root && thread.title == *title)
        }
        Intent::ThreadNotifications { root, level } => {
            matches!(result, wire::MessagingResult::Thread(thread)
            if thread.root.sequence == *root && thread.notification_level == Some(*level))
        }
    }
}
fn overlay(
    entry: &Entry,
    author: &str,
    messages: &mut Page<MessageView, 100>,
    panel: &mut view::MessagePanel,
) {
    let pending_id = (1u128 << 127) | (entry.record.operation.as_local() & ((1u128 << 127) - 1));
    let mutate = |row: &mut MessageView| {
        match &entry.intent {
            Intent::Edit { message, body } if row.local_id == u128::from(*message) => {
                row.body = body.clone();
                row.metadata.edited = true;
            }
            Intent::Delete { message } if row.local_id == u128::from(*message) => {
                row.body = text("");
                row.metadata.deleted = true;
                row.metadata.reactions = Page::empty();
                row.metadata.rich = Page::empty();
                row.metadata.parent = None;
            }
            Intent::Reaction {
                message,
                emoji,
                add,
            } if row.local_id == u128::from(*message) && !row.metadata.deleted => {
                let mut reactions = row.metadata.reactions.clone();
                if let Some(existing) = reactions
                    .as_slice()
                    .iter()
                    .find(|r| r.emoji == *emoji)
                    .cloned()
                {
                    if existing.own != *add {
                        reactions = page(reactions.as_slice().iter().filter_map(|r| {
                            let mut r = r.clone();
                            if r.emoji == *emoji {
                                r.own = *add;
                                r.count = if *add {
                                    r.count.saturating_add(1)
                                } else {
                                    r.count.saturating_sub(1)
                                };
                            }
                            (r.count > 0).then_some(r)
                        }));
                    }
                } else if *add {
                    let _ = reactions.try_push(view::Reaction {
                        emoji: emoji.clone(),
                        count: 1,
                        own: true,
                    });
                }
                row.metadata.reactions = reactions;
            }
            _ => (),
        }
        if let Intent::Delete { message } = &entry.intent
            && let Some(parent) = row
                .metadata
                .parent
                .as_mut()
                .filter(|p| !p.external && p.id == u128::from(*message))
        {
            parent.unavailable = true;
            parent.author = text("");
            parent.text = text("");
        }
    };
    *messages = page(messages.as_slice().iter().cloned().map(|mut row| {
        mutate(&mut row);
        row
    }));
    if let Some(row) = panel.thread_root_message.as_mut() {
        mutate(row);
    }
    panel.thread_messages = page(panel.thread_messages.as_slice().iter().cloned().map(
        |mut row| {
            mutate(&mut row);
            row
        },
    ));
    panel.pinned_messages = page(
        panel
            .pinned_messages
            .as_slice()
            .iter()
            .cloned()
            .map(|mut row| {
                mutate(&mut row);
                row
            })
            .filter(|r| !r.metadata.deleted),
    );
    panel.shared_files = page(panel.shared_files.as_slice().iter().cloned().filter_map(
        |mut row| {
            mutate(&mut row);
            (!row.metadata.deleted).then_some(row)
        },
    ));
    if let Intent::Delete { message } = &entry.intent {
        panel.search = page(
            panel
                .search
                .as_slice()
                .iter()
                .filter(|hit| hit.id != u128::from(*message))
                .cloned(),
        );
    }
    match &entry.intent {
        Intent::RenameThread { root, title } if panel.thread_root == Some(u128::from(*root)) => {
            if let Some(details) = panel
                .thread_details
                .as_mut()
                .filter(|details| details.root == u128::from(*root))
            {
                details.title = title.clone();
            }
        }
        Intent::ThreadNotifications { root, level }
            if panel.thread_root == Some(u128::from(*root)) =>
        {
            if let Some(details) = panel
                .thread_details
                .as_mut()
                .filter(|details| details.root == u128::from(*root))
            {
                details.notification_level = Some(*level);
                panel.thread_following_intent = Some(*level != 3);
            }
        }
        Intent::Send { body, options } => {
            let mut row = MessageView {
                local_id: pending_id,
                author: text(author),
                initials: initials(author),
                time_label: text(&messaging::time_label(entry.created)),
                body: body.clone(),
                own: true,
                delivery: entry.delivery(),
                attachments: Page::empty(),
                reply_count: 0,
                metadata: Default::default(),
            };
            row.metadata.timestamp = entry.created;
            row.metadata.local_operation = Some(entry.record.operation.as_local());
            row.metadata.thread = options.thread.map(u128::from);
            row.metadata.parent = options
                .private_reply
                .as_ref()
                .map(|private| view::Reply {
                    id: u128::from(private.message),
                    external: true,
                    author: text(private.author.as_str()),
                    text: text(private.body.as_str()),
                    unavailable: false,
                })
                .or_else(|| {
                    options
                        .reply_to
                        .and_then(|id| {
                            messages
                                .as_slice()
                                .iter()
                                .chain(panel.thread_root_message.iter())
                                .chain(panel.thread_messages.as_slice())
                                .find(|m| m.local_id == u128::from(id))
                        })
                        .map(|m| view::Reply {
                            id: m.local_id,
                            external: false,
                            author: text(m.author.as_str()),
                            text: m.body.clone(),
                            unavailable: m.metadata.deleted,
                        })
                });
            if options.thread.is_some() && panel.thread_root == options.thread.map(u128::from) {
                if panel.thread_messages.as_slice().len() == 50 {
                    panel.thread_messages =
                        page(panel.thread_messages.as_slice().iter().skip(1).cloned());
                }
                let _ = panel.thread_messages.try_push(row);
            } else {
                if messages.as_slice().len() == 100 {
                    *messages = page(messages.as_slice().iter().skip(1).cloned());
                }
                let _ = messages.try_push(row);
            }
        }
        Intent::Pin { message, pinned } => {
            panel.pinned_messages = page(
                panel
                    .pinned_messages
                    .as_slice()
                    .iter()
                    .filter(|m| m.local_id != u128::from(*message))
                    .cloned(),
            );
            if *pinned
                && let Some(row) = messages
                    .as_slice()
                    .iter()
                    .find(|m| m.local_id == u128::from(*message) && !m.metadata.deleted)
            {
                let _ = panel.pinned_messages.try_push(row.clone());
            }
        }
        Intent::Reminder { message, timestamp } => {
            panel.reminders = page(
                panel
                    .reminders
                    .as_slice()
                    .iter()
                    .filter(|r| r.message != u128::from(*message))
                    .cloned(),
            );
            if let Some(timestamp) = timestamp {
                let _ = panel.reminders.try_push(view::Reminder {
                    message: u128::from(*message),
                    timestamp: *timestamp,
                });
            }
        }
        Intent::Schedule {
            body, timestamp, ..
        } => {
            panel.scheduled = page(
                panel
                    .scheduled
                    .as_slice()
                    .iter()
                    .filter(|s| s.id != pending_id)
                    .cloned(),
            );
            let _ = panel.scheduled.try_push(view::Scheduled {
                id: pending_id,
                body: body.clone(),
                timestamp: *timestamp,
            });
        }
        Intent::CancelScheduled { message } => {
            panel.scheduled = page(
                panel
                    .scheduled
                    .as_slice()
                    .iter()
                    .filter(|r| r.id != u128::from(*message))
                    .cloned(),
            )
        }
        _ => (),
    }
}
