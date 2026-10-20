//! One composition-owned account facade; the renderer receives bounded, scoped data only.
mod background;
mod calls;
pub(super) mod content;
mod conversations;
mod creation;
pub(super) mod files;
mod messaging;
mod notifications;
mod update;
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};
use talk_adapters::identity::{
    AccountConnection, AccountSummary, ConnectionCompletion, ConnectionCursor, ConnectionRequestId,
    ConnectionSnapshot, ConnectionState, IdentityError, RuntimeCommand, RuntimeConfig,
    RuntimeReply, SavedAccountState, StoredState,
};
use talk_core::identity::{StorageBackend, StorageFailure};
use talk_core::ids::{AccountId, AccountScope, LoginAttemptId, OperationId};
use talk_core::ports::{ErrorKind, ProtocolReply, ProtocolRequest};
use talk_core::preferences::{
    RequestId as PreferenceRequest,
    account::{
        AccountPreferenceCommand, AccountPreferenceError, AccountPreferenceSnapshot,
        AccountPreferenceState,
    },
};
use talk_core::protocol::{self, RoomKind, SendMessage, SendState};
use talk_core::ui::presentation::{
    AccountConnectionState, AccountView, Completion, ContentState, ConversationCategory,
    ConversationKind, ConversationListState, ConversationMutationResult, ConversationOrganization,
    ConversationRef, ConversationView, FeatureSupport, LoginFailure, LoginPhase, NativeDesktopPort,
    NativeStoreState, OrganizationResult, Presence, PresentationSnapshot, RequestId,
    SavedAccountPhase, SavedAccountView, StorageChoice, StorageStatus, SubmissionResult, TagEdit,
    TagKind, TagRef, TagView, UiIntent,
};
use talk_core::ui::{Decision, Page, SubmitError, Text, UnavailableReason, UserIntent};

#[derive(Clone, Copy)]
enum Purpose {
    Content(OperationId),
    Create(RequestId, AccountScope, creation::Work),
    Room(RequestId, ConversationRef, OperationId, bool),
    Message(RequestId, ConversationRef, OperationId, messaging::Work),
    IncomingCalls(AccountScope),
    MessageNotifications(AccountScope),
    LiveEvents(AccountScope),
    Call(talk_core::ids::CallScope),
    Tags(AccountScope),
    TagMutation(RequestId, AccountScope),
    Login(RequestId),
    Restore(AccountId),
    Rooms(AccountScope),
    Mutation(RequestId, ConversationRef),
    Send(RequestId, ConversationRef, OperationId),
    Preferences(AccountScope, PreferenceRequest),
}
#[derive(Clone, Copy)]
struct Pending {
    id: ConnectionRequestId,
    purpose: Purpose,
}
struct Login {
    request: RequestId,
    begin: ConnectionRequestId,
    owner: Option<(AccountId, LoginAttemptId)>,
    poll_at: Instant,
}
#[derive(Clone, Copy)]
struct Unresolved {
    request: RequestId,
    context: ConversationRef,
    operation: OperationId,
}

pub struct Accounts {
    content_owner: content::ContentOwner,
    media: std::sync::Arc<talk_adapters::media::MediaService>,
    call_ui: Option<calls::CallUi>,
    call_poll_at: Instant,
    call_events: calls::CallEvents,
    call_actions: std::sync::Arc<calls::CallActions>,
    connection: Result<AccountConnection, IdentityError>,
    cursor: ConnectionCursor,
    next_request: u64,
    pending: Vec<Pending>,
    login: Option<Login>,
    replace: Option<AccountId>,
    snapshot: PresentationSnapshot,
    dirty: bool,
    load_rooms: Option<AccountScope>,
    load_tags: Option<AccountScope>,
    preferences: Vec<AccountPreferenceSnapshot>,
    failures: Vec<(AccountId, LoginFailure)>,
    unresolved: Vec<Unresolved>,
    room_invite: Option<(RequestId, talk_core::ui::conversation::PersonRef)>,
    message_search: Option<(ConversationRef, Text<256>, Option<Text<1024>>)>,
    notices: notifications::Notices,
    background: background::Background,
    picker: Option<files::Picker>,
    picker_open: bool,
    message_poll_at: Instant,
    message_poll_request: u64,
}
impl Accounts {
    pub fn start(media: std::sync::Arc<talk_adapters::media::MediaService>) -> Rc<RefCell<Self>> {
        let mut snapshot = super::fixtures::snapshot(None);
        snapshot.content = ContentState::Loading;
        Rc::new(RefCell::new(Self {
            content_owner: content::ContentOwner::default(),
            connection: AccountConnection::start(
                RuntimeConfig::platform().with_media(media.clone()),
            ),
            media,
            call_ui: None,
            call_poll_at: Instant::now(),
            call_events: calls::CallEvents::new(),
            call_actions: std::sync::Arc::new(calls::CallActions::new()),
            cursor: ConnectionCursor::default(),
            next_request: 0,
            pending: Vec::new(),
            login: None,
            replace: None,
            snapshot,
            dirty: true,
            load_rooms: None,
            load_tags: None,
            preferences: Vec::new(),
            failures: Vec::new(),
            unresolved: Vec::new(),
            room_invite: None,
            message_search: None,
            notices: notifications::Notices::default(),
            background: background::Background::default(),
            picker: None,
            picker_open: false,
            message_poll_at: Instant::now(),
            message_poll_request: 0,
        }))
    }
    pub fn shutdown(&mut self) {
        self.call_actions.clear();
        if let Ok(connection) = &mut self.connection {
            connection.shutdown();
        }
    }
    pub fn stopped(&self) -> bool {
        self.connection
            .as_ref()
            .map_or(true, AccountConnection::is_stopped)
    }
    fn admit(
        &mut self,
        command: RuntimeCommand,
        purpose: Purpose,
    ) -> Result<ConnectionRequestId, IdentityError> {
        if self.call_actions.update_sealed() {
            return Err(ErrorKind::Busy.into());
        }
        if self.pending.len() >= 16 {
            return Err(ErrorKind::Busy.into());
        }
        let next = self
            .next_request
            .checked_add(1)
            .ok_or(IdentityError::Request(ErrorKind::Busy))?;
        let id = ConnectionRequestId(next);
        self.connection
            .as_mut()
            .map_err(|e| *e)?
            .try_submit(id, command)?;
        self.next_request = next;
        self.pending.push(Pending { id, purpose });
        Ok(id)
    }
    fn request(
        &mut self,
        scope: AccountScope,
        request: ProtocolRequest,
        purpose: Purpose,
    ) -> Result<(), SubmitError> {
        self.current(scope)?;
        self.admit(RuntimeCommand::Request { scope, request }, purpose)
            .map(|_| ())
            .map_err(submit_error)
    }
    fn current(&self, scope: AccountScope) -> Result<(), SubmitError> {
        self.connection
            .as_ref()
            .map_err(|e| submit_error(*e))?
            .account_cancellation(scope)
            .map(|_| ())
            .map_err(submit_error)
    }
    fn touch(&mut self) {
        self.dirty = true;
    }
    fn clear_content(&mut self) {
        self.clear_content_assets();
        self.picker = None;
        self.room_invite = None;
        self.message_search = None;
        self.snapshot.selected = None;
        self.snapshot.messages = Page::empty();
        self.snapshot.files = Page::empty();
        self.snapshot.participants = Page::empty();
        self.snapshot.conversation_panel = None;
        self.snapshot.message_panel = None;
        self.snapshot.content = ContentState::Loading;
    }
    fn select(&mut self, scope: Option<AccountScope>) {
        if self.snapshot.active_account == scope {
            return;
        }
        self.snapshot.active_account = scope;
        self.snapshot.create_panel = scope.map(talk_core::ui::conversation::CreatePanel::new);
        self.snapshot.conversations = scope
            .and_then(|s| self.background.account(s))
            .map_or_else(Page::empty, |a| a.rooms.clone());
        self.snapshot.conversation_list = ConversationListState {
            content: if scope.is_some() {
                ContentState::Loading
            } else {
                ContentState::Unconfigured
            },
            complete: false,
        };
        self.snapshot.tags = Page::empty();
        self.snapshot.tag_state = ContentState::Unconfigured;
        self.load_tags = None;
        self.clear_content();
        self.load_rooms = scope;
        self.touch();
    }
    fn sync_accounts(&mut self, state: &ConnectionSnapshot) {
        let accounts = page(
            state
                .accounts
                .as_slice()
                .iter()
                .filter_map(|a| a.summary.scope.map(|scope| account(&a.summary, scope))),
        );
        let saved = page(state.accounts.as_slice().iter().map(|a| {
            SavedAccountView {
                id: a.summary.account,
                label: a.summary.account_label.clone(),
                server_label: a.summary.server_label.clone(),
                phase: match a.state {
                    SavedAccountState::Dormant => SavedAccountPhase::Dormant,
                    SavedAccountState::Restoring => SavedAccountPhase::Restoring,
                    SavedAccountState::WaitingForBrowser => SavedAccountPhase::WaitingForBrowser,
                    SavedAccountState::AwaitingStorageChoice => {
                        SavedAccountPhase::AwaitingStorageChoice
                    }
                    SavedAccountState::Active => SavedAccountPhase::Active,
                    SavedAccountState::ReauthenticationRequired => {
                        SavedAccountPhase::ReauthenticationRequired
                    }
                    SavedAccountState::RecoveryRequired => SavedAccountPhase::RecoveryRequired,
                },
                storage: storage(&a.summary),
                cleanup_pending: a.summary.cleanup_pending,
                suppression_unconfirmed: a.summary.suppression_unconfirmed,
                failure: self
                    .failures
                    .iter()
                    .find(|(id, _)| *id == a.summary.account)
                    .map(|(_, e)| *e),
            }
        }));
        if self.snapshot.accounts != accounts || self.snapshot.saved_accounts != saved {
            self.snapshot.accounts = accounts;
            self.snapshot.saved_accounts = saved;
            self.touch();
        }
        if self.snapshot.active_account.is_some_and(|s| {
            !self
                .snapshot
                .accounts
                .as_slice()
                .iter()
                .any(|a| a.scope == s)
        }) {
            self.select(None);
        }
        self.sync_background();
        let admitted = self.snapshot.accounts.as_slice();
        self.preferences
            .retain(|p| admitted.iter().any(|a| a.scope == p.scope));
        self.unresolved
            .retain(|u| admitted.iter().any(|a| a.scope == u.context.account));
        if self.snapshot.active_account.is_none()
            && let Some(account) = admitted.first()
        {
            self.select(Some(account.scope));
        }
        if let ConnectionState::Failed(error) = state.state {
            self.snapshot.login.phase = LoginPhase::Failed(login_error(error));
            self.snapshot.content = ContentState::Failed;
            self.touch();
        }
    }
    fn tick(&mut self) {
        let update = match &mut self.connection {
            Ok(c) => c.poll(self.cursor),
            Err(error) => Err(*error),
        };
        match update {
            Ok(update) => {
                self.sync_accounts(&update.snapshot);
                for done in update.completions.as_slice() {
                    if let Some(index) = self.pending.iter().position(|p| p.id == done.request) {
                        let pending = self.pending.remove(index);
                        self.complete(pending.purpose, done);
                    }
                }
                self.cursor = update.cursor;
            }
            Err(error) => {
                let phase = LoginPhase::Failed(login_error(error));
                if self.snapshot.login.phase != phase {
                    self.snapshot.login.phase = phase;
                    self.touch();
                }
            }
        }
        self.tick_content();
        self.tick_background();
        self.tick_calls();
        self.poll_messages();
        if let Some(scope) = self.load_tags
            && self
                .request(scope, ProtocolRequest::ReadTags, Purpose::Tags(scope))
                .is_ok()
        {
            self.load_tags = None;
        }
        let poll = self.login.as_ref().and_then(|l| {
            (self.snapshot.login.phase == LoginPhase::BrowserWaiting
                && Instant::now() >= l.poll_at
                && !self
                    .pending
                    .iter()
                    .any(|p| matches!(p.purpose, Purpose::Login(r) if r == l.request)))
            .then_some((l.request, l.owner))
            .and_then(|(r, o)| o.map(|(a, t)| (r, a, t)))
        });
        if let Some((request, account, attempt)) = poll {
            if let Some(login) = &mut self.login {
                login.poll_at = Instant::now() + Duration::from_secs(1);
            }
            if let Err(error) = self.admit(
                RuntimeCommand::Poll { account, attempt },
                Purpose::Login(request),
            ) && error != IdentityError::Request(ErrorKind::Busy)
            {
                self.login_failed(error);
            }
        }
    }
    fn login_failed(&mut self, error: IdentityError) {
        match error {
            IdentityError::Storage { failure, .. } => {
                self.snapshot.login.native_store = match failure {
                    StorageFailure::Locked => NativeStoreState::Locked,
                    StorageFailure::AccessDenied => NativeStoreState::Denied,
                    StorageFailure::Missing
                    | StorageFailure::Unsupported
                    | StorageFailure::Unavailable => NativeStoreState::Missing,
                    _ => NativeStoreState::Unknown,
                };
                self.snapshot.login.phase = LoginPhase::ChooseStorage;
                self.snapshot.login.storage = StorageStatus::NotSaved;
            }
            IdentityError::RecoveryRequired | IdentityError::SuppressionUnconfirmed => {
                self.snapshot.login.storage = StorageStatus::RecoveryRequired;
                self.snapshot.login.phase = LoginPhase::Failed(LoginFailure::Storage);
            }
            _ => self.snapshot.login.phase = LoginPhase::Failed(login_error(error)),
        }
        self.touch();
    }
    fn complete(&mut self, purpose: Purpose, done: &ConnectionCompletion) {
        // A stale/cancelled reply can settle retained work but cannot repopulate a new runtime.
        let result =
            if done.cancel_requested || (done.captured_scope.is_some() && !done.scope_is_current) {
                Err(IdentityError::Request(ErrorKind::WrongScope))
            } else {
                done.result.clone()
            };
        match purpose {
            Purpose::Content(operation) => self.complete_content(operation, result),
            Purpose::Create(request,account,work)=>self.complete_creation(request,account,work,result),
            Purpose::Room(request, context, operation, read) => self.complete_room(request, context, operation, read, result),
            Purpose::Message(request, context, operation, work) => self.complete_message(request, context, operation, work, result),
            Purpose::Call(scope) => self.complete_call(scope, result),
            Purpose::IncomingCalls(scope) => self.complete_incoming(scope, result),
            Purpose::MessageNotifications(scope) => self.complete_message_notifications(scope, result),
            Purpose::LiveEvents(scope) => {
                if done.captured_scope != Some(scope) || done.cancel_requested || !done.scope_is_current {
                    return;
                }
                // A mailbox drain changes scheduling only, not the visible snapshot.
                return self.complete_live_events(scope, result);
            }
            Purpose::Login(request) => {
                if self.login.as_ref().is_none_or(|l| l.request != request) { return; }
                match result {
                    Ok(RuntimeReply::Begun { account, attempt } | RuntimeReply::Pending { account, attempt }) => {
                        if let Some(login) = &mut self.login { login.owner = Some((account, attempt)); login.poll_at = Instant::now() + Duration::from_secs(1); }
                        self.snapshot.login.phase = LoginPhase::BrowserWaiting;
                    }
                    Ok(RuntimeReply::Authorized { account, attempt }) => { if let Some(l) = &mut self.login { l.owner = Some((account, attempt)); } self.snapshot.login.phase = LoginPhase::ChooseStorage; }
                    Ok(RuntimeReply::Active { summary, .. }) => {
                        self.snapshot.login.phase = LoginPhase::Ready; self.snapshot.login.storage = storage(&summary);
                        self.snapshot.login.account_label = summary.account_label.clone().unwrap_or_else(|| text(""));
                        self.snapshot.login.server_label = summary.server_label.clone().unwrap_or_else(|| text(""));
                        if let Some(scope) = summary.scope { self.select(Some(scope)); }
                    }
                    Ok(RuntimeReply::BrowserOpened { .. }) => (),
                    Err(error) => self.login_failed(error),
                    _ => self.login_failed(IdentityError::Request(ErrorKind::Malformed)),
                }
            }
            Purpose::Restore(id) => {
                match result {
                    Ok(RuntimeReply::Active { summary, .. }) => { if let Some(scope) = summary.scope { self.select(Some(scope)); } }
                    Err(error) => { self.failures.retain(|(a, _)| *a != id); if self.failures.len() < 8 { self.failures.push((id, login_error(error))); } }
                    _ => (),
                }
            }
            Purpose::Rooms(scope) => self.complete_rooms(scope, result),
            Purpose::Tags(scope) | Purpose::TagMutation(_, scope) => {
                let completion = match result {
                    Ok(RuntimeReply::Protocol(ProtocolReply::Tags(tags))) if tags.scope == scope => {
                        if self.snapshot.active_account == Some(scope) { self.snapshot.tags = page(tags.tags.as_slice().iter().map(tag_view)); self.snapshot.tag_state = ContentState::Ready; }
                        Completion::ServerAccepted
                    }
                    Ok(RuntimeReply::Protocol(ProtocolReply::TagCreated(tag))) if tag.id.scope == scope => { self.load_tags = Some(scope); Completion::ServerAccepted }
                    Ok(RuntimeReply::Protocol(ProtocolReply::Conversation(row))) if row.id.account == scope.account => {
                        if self.snapshot.active_account == Some(scope) { self.snapshot.conversations = page(self.snapshot.conversations.as_slice().iter().map(|r| if r.context.conversation == row.id { conversation(&row, scope) } else { r.clone() })); }
                        Completion::ServerAccepted
                    }
                    Err(error) => { if self.snapshot.active_account == Some(scope) { self.snapshot.tag_state = content_error(error); } completion_error(error) }
                    _ => Completion::Failed(UnavailableReason::Unsupported),
                };
                if let Purpose::TagMutation(request, _) = purpose {
                    self.snapshot.organization_result = Some(OrganizationResult { request, account: scope, result: completion });
                    // Catalog changes can change room membership. Reconcile, never infer deletion.
                    if self.snapshot.active_account == Some(scope) && completion == Completion::ServerAccepted { self.load_rooms = Some(scope); }
                }
            }
            Purpose::Mutation(request, context) => {
                let completion = match result {
                    Ok(RuntimeReply::Protocol(ProtocolReply::Conversation(row))) if row.id == context.conversation => {
                        if self.snapshot.active_account == Some(context.account) { self.snapshot.conversations = page(self.snapshot.conversations.as_slice().iter().map(|r| if r.context == context { conversation(&row, context.account) } else { r.clone() })); }
                        Completion::ServerAccepted
                    }
                    Err(error) => completion_error(error),
                    _ => Completion::Failed(UnavailableReason::Unsupported),
                };
                self.snapshot.mutation = Some(ConversationMutationResult { request, context, result: completion });
            }
            Purpose::Send(request, context, operation) => {
                let completion = match result {
                    Ok(RuntimeReply::Protocol(ProtocolReply::Send(receipt))) if receipt.operation == operation => match receipt.state { SendState::ServerAccepted(_) => Completion::ServerAccepted, SendState::Rejected => Completion::Failed(UnavailableReason::PermissionDenied), SendState::Uncertain => Completion::Uncertain },
                    Err(error) => completion_error(error),
                    _ => Completion::Uncertain,
                };
                if completion != Completion::Uncertain { self.unresolved.retain(|u| u.operation != operation); }
                self.snapshot.submission = Some(SubmissionResult { request, context, result: completion });
            }
            Purpose::Preferences(scope, request) => {
                if let Some(cache) = self.preferences.iter_mut().find(|p| p.scope == scope && matches!(p.state, AccountPreferenceState::Loading { request: r } | AccountPreferenceState::Saving { request: r } if r == request)) {
                    match result {
                        Ok(RuntimeReply::Protocol(ProtocolReply::Preferences(reply))) if reply.request == request && reply.values.scope == scope => { cache.values = Some(reply.values); cache.state = AccountPreferenceState::Ready; }
                        Err(error) => cache.state = AccountPreferenceState::Failed { request, error: preference_error(error) },
                        _ => cache.state = AccountPreferenceState::Failed { request, error: AccountPreferenceError::Unavailable },
                    }
                }
            }
        }
        self.touch();
    }
    pub fn account_snapshot(&self, scope: AccountScope) -> AccountPreferenceSnapshot {
        self.preferences
            .iter()
            .find(|p| p.scope == scope)
            .cloned()
            .unwrap_or(AccountPreferenceSnapshot {
                scope,
                values: None,
                state: AccountPreferenceState::Unavailable,
            })
    }
    pub fn submit_account(
        &mut self,
        command: AccountPreferenceCommand,
    ) -> Result<(), AccountPreferenceError> {
        let (scope, request, protocol, state) = match command {
            AccountPreferenceCommand::Load { scope, request } => (
                scope,
                request,
                ProtocolRequest::ReadPreferences { request },
                AccountPreferenceState::Loading { request },
            ),
            AccountPreferenceCommand::Set {
                scope,
                request,
                change,
            } => (
                scope,
                request,
                ProtocolRequest::SetPreference { request, change },
                AccountPreferenceState::Saving { request },
            ),
        };
        self.current(scope)
            .map_err(|_| AccountPreferenceError::Stale)?;
        if self.preferences.iter().any(|p| {
            p.scope == scope
                && matches!(
                    p.state,
                    AccountPreferenceState::Loading { .. } | AccountPreferenceState::Saving { .. }
                )
        }) {
            return Err(AccountPreferenceError::Busy);
        }
        if self.preferences.len() >= 8 && !self.preferences.iter().any(|p| p.scope == scope) {
            return Err(AccountPreferenceError::Busy);
        }
        self.admit(
            RuntimeCommand::Request {
                scope,
                request: protocol,
            },
            Purpose::Preferences(scope, request),
        )
        .map_err(preference_error)?;
        if let Some(cache) = self.preferences.iter_mut().find(|p| p.scope == scope) {
            cache.state = state;
        } else {
            self.preferences.push(AccountPreferenceSnapshot {
                scope,
                values: None,
                state,
            });
        }
        Ok(())
    }
    fn cancel_login(&mut self) {
        if let Some(login) = self.login.take()
            && let Ok(connection) = &mut self.connection
        {
            if let Some((account, attempt)) = login.owner {
                let _ = connection.cancel_attempt(account, attempt);
            }
            let _ = connection.cancel_request(login.begin);
        }
        self.snapshot.login = super::fixtures::snapshot(None).login;
        self.touch();
    }
    fn open(&mut self, context: ConversationRef) -> Result<(), SubmitError> {
        self.current(context.account)?;
        let row = self
            .snapshot
            .conversations
            .as_slice()
            .iter()
            .find(|r| r.context == context)
            .ok_or(SubmitError::StaleScope)?;
        if self.snapshot.active_account != Some(context.account) || row.send == Decision::Stale {
            return Err(SubmitError::StaleScope);
        }
        self.clear_content();
        self.snapshot.selected = Some(context);
        self.start_messages(context)?;
        self.snapshot.content_panel.context = Some(context);
        let _ = self.start_poll(RequestId(0), protocol::content::PollCommand::Capabilities);
        self.touch();
        Ok(())
    }
    fn submit(&mut self, intent: UiIntent) -> Result<(), SubmitError> {
        if let Some(result) = self.submit_call(&intent) {
            return result;
        }
        match intent {
            UiIntent::Content {
                request,
                account,
                action,
            } => self.content_action(request, account, action)?,
            UiIntent::Create {
                account,
                request,
                action,
            } => self.create_action(request, account, action)?,
            UiIntent::AcknowledgeCreation { account, request } => {
                if let Some(panel) = self
                    .snapshot
                    .create_panel
                    .as_mut()
                    .filter(|p| p.account == account)
                    && panel.result.is_some_and(|r| r.request == request)
                {
                    panel.result = None;
                    self.touch();
                }
            }
            UiIntent::Conversation {
                request,
                context,
                action,
            } => self.room_action(request, context, action)?,
            UiIntent::Message {
                request,
                context,
                action,
            } => self.message_action(request, context, action)?,
            UiIntent::AcknowledgeRoom { request, context } => {
                if let Some(panel) = self
                    .snapshot
                    .conversation_panel
                    .as_mut()
                    .filter(|p| p.context == context)
                    && panel.result.is_some_and(|r| r.request == request)
                {
                    panel.result = None;
                    self.touch();
                }
            }
            UiIntent::AcknowledgeMessage { request, context } => {
                if let Some(panel) = self
                    .snapshot
                    .message_panel
                    .as_mut()
                    .filter(|p| p.context == context)
                    && panel.result.is_some_and(|(r, _)| r == request)
                {
                    panel.result = None;
                }
            }
            UiIntent::EditTags {
                request,
                account,
                edit,
            } => {
                if self.snapshot.organization_result.is_some()
                    || self
                        .pending
                        .iter()
                        .any(|p| matches!(p.purpose, Purpose::TagMutation(..)))
                {
                    return Err(SubmitError::Busy);
                }
                let request_protocol = match edit {
                    TagEdit::Create(name) => ProtocolRequest::CreateTag { name },
                    TagEdit::Rename { tag, name } => ProtocolRequest::RenameTag {
                        tag: tag_id(tag),
                        name,
                    },
                    TagEdit::Delete(tag) => ProtocolRequest::DeleteTag(tag_id(tag)),
                    TagEdit::Collapse { tag, collapsed } => ProtocolRequest::SetTagCollapsed {
                        tag: tag_id(tag),
                        collapsed,
                    },
                    TagEdit::Reorder(tags) => ProtocolRequest::ReorderTags {
                        tags: page(tags.as_slice().iter().copied().map(tag_id)),
                    },
                    TagEdit::Assign { context, tags } => {
                        if context.account != account {
                            return Err(SubmitError::StaleScope);
                        }
                        ProtocolRequest::SetConversationTags {
                            conversation: context.conversation,
                            tags: page(tags.as_slice().iter().copied().map(tag_id)),
                        }
                    }
                };
                self.request(
                    account,
                    request_protocol,
                    Purpose::TagMutation(request, account),
                )?;
            }
            UiIntent::AcknowledgeOrganization { request, account } => {
                if self
                    .snapshot
                    .organization_result
                    .is_some_and(|r| r.request == request && r.account == account)
                {
                    self.snapshot.organization_result = None;
                }
            }
            UiIntent::ReloadConversations(scope) => {
                self.current(scope)?;
                if self.snapshot.active_account != Some(scope) {
                    return Err(SubmitError::StaleScope);
                }
                self.load_rooms = Some(scope);
            }
            UiIntent::NewLogin => {
                self.cancel_login();
                self.replace = None;
            }
            UiIntent::Reauthenticate(account) => {
                if !self
                    .snapshot
                    .saved_accounts
                    .as_slice()
                    .iter()
                    .any(|a| a.id == account)
                {
                    return Err(SubmitError::StaleScope);
                }
                self.cancel_login();
                self.replace = Some(account);
            }
            UiIntent::BeginLogin { request, server } => {
                if self.login.is_some() {
                    self.cancel_login();
                }
                let id = self
                    .admit(
                        RuntimeCommand::Begin {
                            server: text(server.as_str()),
                            replace: self.replace,
                        },
                        Purpose::Login(request),
                    )
                    .map_err(submit_error)?;
                self.login = Some(Login {
                    request,
                    begin: id,
                    owner: None,
                    poll_at: Instant::now(),
                });
                self.snapshot.login.phase = LoginPhase::Validating;
                self.snapshot.login.attempt = Some(request);
            }
            UiIntent::CancelLogin(request) => {
                if self.login.as_ref().is_some_and(|l| l.request == request) {
                    self.cancel_login();
                } else {
                    return Err(SubmitError::StaleScope);
                }
            }
            UiIntent::ReopenBrowser(request) => {
                let (account, attempt) = self
                    .login
                    .as_ref()
                    .filter(|l| l.request == request)
                    .and_then(|l| l.owner)
                    .ok_or(SubmitError::StaleScope)?;
                self.admit(
                    RuntimeCommand::ReopenBrowser { account, attempt },
                    Purpose::Login(request),
                )
                .map_err(submit_error)?;
            }
            UiIntent::ChooseStorage {
                attempt: request,
                choice,
            } => {
                let (account, attempt) = self
                    .login
                    .as_ref()
                    .filter(|l| l.request == request)
                    .and_then(|l| l.owner)
                    .ok_or(SubmitError::StaleScope)?;
                let choice = match choice {
                    StorageChoice::NativeStore => talk_core::identity::StorageChoice::Native,
                    StorageChoice::PrivateFile => talk_core::identity::StorageChoice::PrivateFile,
                    StorageChoice::SessionOnly => talk_core::identity::StorageChoice::SessionOnly,
                };
                self.admit(
                    RuntimeCommand::Save {
                        account,
                        attempt,
                        choice,
                    },
                    Purpose::Login(request),
                )
                .map_err(submit_error)?;
                self.snapshot.login.storage = StorageStatus::Saving;
            }
            UiIntent::RestoreAccount(account) => {
                self.admit(
                    RuntimeCommand::Restore { account },
                    Purpose::Restore(account),
                )
                .map_err(submit_error)?;
                self.failures.retain(|(a, _)| *a != account);
            }
            UiIntent::SelectAccount(scope) => {
                self.current(scope)?;
                self.select(Some(scope));
            }
            UiIntent::OpenConversation(context) => self.open(context)?,
            UiIntent::Refresh(context) => {
                if self.snapshot.submission.is_none()
                    && let Some(u) = self
                        .unresolved
                        .iter()
                        .find(|u| u.context == context)
                        .copied()
                {
                    self.request(
                        context.account,
                        ProtocolRequest::SendStatus(u.operation),
                        Purpose::Send(u.request, context, u.operation),
                    )?;
                }
                self.open(context)?;
            }
            UiIntent::SetArchived {
                request,
                context,
                archived,
            } => self.mutate(
                request,
                context,
                ProtocolRequest::SetArchived {
                    conversation: context.conversation,
                    archived,
                },
            )?,
            UiIntent::SetFavorite {
                request,
                context,
                favorite,
            } => self.mutate(
                request,
                context,
                ProtocolRequest::SetFavorite {
                    conversation: context.conversation,
                    favorite,
                },
            )?,
            UiIntent::AcknowledgeMutation { request, context } => {
                if self
                    .snapshot
                    .mutation
                    .is_some_and(|r| r.request == request && r.context == context)
                {
                    self.snapshot.mutation = None;
                }
            }
            UiIntent::SendDraft {
                request,
                context,
                body,
            } => {
                if self.snapshot.submission.is_some()
                    || self
                        .pending
                        .iter()
                        .any(|p| matches!(p.purpose, Purpose::Send(..)))
                    || self.unresolved.len() >= 8
                    || self.unresolved.iter().any(|u| u.context == context)
                {
                    return Err(SubmitError::Busy);
                }
                self.current(context.account)?;
                let operation = self
                    .connection
                    .as_ref()
                    .map_err(|e| submit_error(*e))?
                    .new_operation_id(context.account)
                    .map_err(submit_error)?;
                self.request(
                    context.account,
                    ProtocolRequest::Send(SendMessage {
                        conversation: context.conversation,
                        operation,
                        text: text(body.as_str()),
                    }),
                    Purpose::Send(request, context, operation),
                )?;
                self.unresolved.push(Unresolved {
                    request,
                    context,
                    operation,
                });
            }
            UiIntent::AcknowledgeSubmission { request, context } => {
                if self
                    .snapshot
                    .submission
                    .is_some_and(|r| r.request == request && r.context == context)
                {
                    self.snapshot.submission = None;
                }
            }
            UiIntent::Legacy(UserIntent::Logout(account)) => {
                self.connection
                    .as_ref()
                    .map_err(|e| submit_error(*e))?
                    .logout(account)
                    .map_err(submit_error)?;
                self.preferences.retain(|p| p.scope.account != account);
                if self
                    .snapshot
                    .active_account
                    .is_some_and(|s| s.account == account)
                {
                    self.select(None);
                }
            }
            _ => return Err(SubmitError::Unavailable(UnavailableReason::Unsupported)),
        }
        self.touch();
        Ok(())
    }
    fn mutate(
        &mut self,
        request: RequestId,
        context: ConversationRef,
        protocol: ProtocolRequest,
    ) -> Result<(), SubmitError> {
        if self.snapshot.mutation.is_some()
            || self
                .pending
                .iter()
                .any(|p| matches!(p.purpose, Purpose::Mutation(..)))
        {
            return Err(SubmitError::Busy);
        }
        self.request(
            context.account,
            protocol,
            Purpose::Mutation(request, context),
        )
    }
}
pub struct Port(pub Rc<RefCell<Accounts>>);
impl NativeDesktopPort for Port {
    fn try_submit(&mut self, intent: UiIntent) -> Result<(), SubmitError> {
        self.0.borrow_mut().submit(intent)
    }
    fn take_snapshot(&mut self) -> Option<PresentationSnapshot> {
        let mut model = self.0.borrow_mut();
        model.tick();
        if !model.dirty {
            return None;
        }
        model.dirty = false;
        model.snapshot.revision = model.snapshot.revision.saturating_add(1);
        Some(model.snapshot.clone())
    }
}
fn page<T, const N: usize>(items: impl IntoIterator<Item = T>) -> Page<T, N> {
    let mut result = Page::empty();
    for item in items {
        assert!(
            result.try_push(item).is_ok(),
            "projection preserves producer bound"
        );
    }
    result
}
fn text<const N: usize>(value: &str) -> Text<N> {
    if let Ok(text) = Text::new(value) {
        return text;
    }
    let mut end = N.saturating_sub('…'.len_utf8()).min(value.len());
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    Text::new(&format!("{}…", &value[..end])).expect("bounded UTF-8 projection")
}
fn initials(value: &str) -> Text<16> {
    if value.trim().is_empty() {
        return text("?");
    }
    text(
        &value
            .split(|c: char| c.is_whitespace() || matches!(c, '_' | '-' | '.'))
            .filter(|part| !part.is_empty())
            .take(2)
            .filter_map(|v| v.chars().next())
            .flat_map(char::to_uppercase)
            .collect::<String>(),
    )
}
fn storage(a: &AccountSummary) -> StorageStatus {
    if a.cleanup_pending {
        return StorageStatus::CleanupPending;
    }
    if a.suppression_unconfirmed {
        return StorageStatus::RecoveryRequired;
    }
    match a.storage {
        StoredState::Unknown | StoredState::Suppressed => StorageStatus::NotSaved,
        StoredState::Unresolved => StorageStatus::RecoveryRequired,
        StoredState::SessionOnly => StorageStatus::SessionOnly,
        StoredState::Eligible(item) => {
            if item.provider.backend == StorageBackend::PrivateFile {
                StorageStatus::PrivateFileStored
            } else {
                StorageStatus::NativeStored
            }
        }
    }
}
fn account(a: &AccountSummary, scope: AccountScope) -> AccountView {
    let label = a.account_label.clone().unwrap_or_else(|| text(""));
    AccountView {
        scope,
        navigation_key: Some(a.navigation_key.clone()),
        connection: if a.connection_degraded {
            AccountConnectionState::Disconnected
        } else {
            AccountConnectionState::Connected
        },
        initials: initials(label.as_str()),
        self_name: label.clone(),
        label,
        server_label: a.server_label.clone().unwrap_or_else(|| text("")),
        presence: Presence::Unknown,
        storage: storage(a),
    }
}
fn support(s: protocol::FeatureSupport) -> FeatureSupport {
    match s {
        protocol::FeatureSupport::Unknown => FeatureSupport::Unknown,
        protocol::FeatureSupport::Unsupported => FeatureSupport::Unsupported,
        protocol::FeatureSupport::Supported => FeatureSupport::Supported,
    }
}
fn conversation(r: &protocol::Conversation, scope: AccountScope) -> ConversationView {
    ConversationView {
        context: ConversationRef {
            account: scope,
            conversation: r.id,
        },
        navigation_key: r.navigation_key.clone(),
        title: r.name.clone(),
        initials: initials(r.name.as_str()),
        subtitle: text(""),
        updated_label: text(""),
        kind: match r.kind {
            RoomKind::Direct | RoomKind::FormerDirect => ConversationKind::Direct,
            RoomKind::Group => ConversationKind::Group,
            RoomKind::Public => ConversationKind::GuestAccessible,
            RoomKind::NoteToSelf => ConversationKind::NoteToSelf,
            _ => ConversationKind::Unknown,
        },
        unread: r.unread_messages.unwrap_or(0),
        favorite: r.favorite,
        tags: r.tag_ids.as_ref().map(|tags| {
            page(tags.as_slice().iter().map(|id| TagRef {
                account: id.scope,
                local: id.local,
            }))
        }),
        organization: ConversationOrganization {
            category: if r.organization.channel == Some(true) {
                ConversationCategory::Channel
            } else {
                ConversationCategory::Unknown
            },
            classified: r.organization.classified,
            favorites: support(r.organization.favorites),
            favorite_change: r.organization.favorite_change,
            tags: support(r.organization.tags),
            tag_change: r.organization.tag_change,
        },
        archived: r.archived,
        archive: r.archive,
        presence: Presence::Unknown,
        participants: None,
        send: r.actions.send_message,
        attach: Decision::Unknown,
        draft_clipboard: if r.sensitive || r.classified || r.organization.classified == Some(true) {
            Decision::Denied
        } else if !r.unknown_restrictions && r.organization.classified == Some(false) {
            Decision::Allowed
        } else {
            Decision::Unknown
        },
        call: Decision::Unknown,
        call_scope: r.call_scope,
    }
}
fn submit_error(e: IdentityError) -> SubmitError {
    match e {
        IdentityError::Request(ErrorKind::Unsupported) => {
            SubmitError::Unavailable(UnavailableReason::Unsupported)
        }
        IdentityError::Request(ErrorKind::Busy | ErrorKind::RateLimited) => SubmitError::Busy,
        IdentityError::Request(ErrorKind::WrongScope | ErrorKind::Cancelled) => {
            SubmitError::StaleScope
        }
        IdentityError::Request(ErrorKind::PermissionDenied) => {
            SubmitError::Unavailable(UnavailableReason::PermissionDenied)
        }
        IdentityError::Request(ErrorKind::ReauthenticationRequired) => {
            SubmitError::Unavailable(UnavailableReason::ReauthenticationRequired)
        }
        _ => SubmitError::Unavailable(UnavailableReason::Unconfigured),
    }
}
fn completion_error(e: IdentityError) -> Completion {
    if e == IdentityError::Request(ErrorKind::Uncertain) {
        Completion::Uncertain
    } else {
        Completion::Failed(match submit_error(e) {
            SubmitError::Unavailable(reason) => reason,
            _ => UnavailableReason::PolicyStale,
        })
    }
}
fn content_error(e: IdentityError) -> ContentState {
    match e {
        IdentityError::Request(
            ErrorKind::PermissionDenied
            | ErrorKind::ReauthenticationRequired
            | ErrorKind::WrongScope
            | ErrorKind::Cancelled,
        ) => ContentState::Denied,
        IdentityError::Request(
            ErrorKind::Transport | ErrorKind::Deadline | ErrorKind::ServerUnavailable,
        ) => ContentState::Offline,
        _ => ContentState::Failed,
    }
}
fn login_error(e: IdentityError) -> LoginFailure {
    match e {
        IdentityError::Storage { .. }
        | IdentityError::RecoveryRequired
        | IdentityError::SuppressionUnconfirmed => LoginFailure::Storage,
        IdentityError::Request(
            ErrorKind::PermissionDenied | ErrorKind::ReauthenticationRequired,
        ) => LoginFailure::Denied,
        IdentityError::Request(ErrorKind::Deadline) => LoginFailure::Expired,
        IdentityError::Request(ErrorKind::Transport | ErrorKind::ServerUnavailable) => {
            LoginFailure::Network
        }
        _ => LoginFailure::Unconfigured,
    }
}
fn preference_error(e: IdentityError) -> AccountPreferenceError {
    match e {
        IdentityError::Request(ErrorKind::Busy) => AccountPreferenceError::Busy,
        IdentityError::Request(ErrorKind::WrongScope | ErrorKind::Cancelled) => {
            AccountPreferenceError::Stale
        }
        IdentityError::Request(ErrorKind::PermissionDenied) => {
            AccountPreferenceError::PermissionDenied
        }
        IdentityError::Request(ErrorKind::ReauthenticationRequired) => {
            AccountPreferenceError::AuthenticationRequired
        }
        IdentityError::Request(
            ErrorKind::Transport | ErrorKind::Deadline | ErrorKind::ServerUnavailable,
        ) => AccountPreferenceError::Network,
        IdentityError::Request(ErrorKind::Uncertain) => AccountPreferenceError::Uncertain,
        _ => AccountPreferenceError::Unavailable,
    }
}

fn tag_id(tag: TagRef) -> protocol::ConversationTagId {
    protocol::ConversationTagId {
        scope: tag.account,
        local: tag.local,
    }
}
fn tag_view(tag: &protocol::ConversationTag) -> TagView {
    TagView {
        id: TagRef {
            account: tag.id.scope,
            local: tag.id.local,
        },
        name: tag.name.clone(),
        order: tag.order,
        collapsed: tag.collapsed,
        kind: match tag.kind {
            protocol::TagKind::Custom => TagKind::Custom,
            protocol::TagKind::Favorites => TagKind::Favorites,
            protocol::TagKind::Other => TagKind::Other,
            protocol::TagKind::Unknown => TagKind::Unknown,
        },
    }
}

#[cfg(test)]
mod tests;
