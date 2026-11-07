//! One composition-owned account facade; the renderer receives bounded, scoped data only.
mod background;
mod cache;
mod calls;
pub(super) mod content;
mod conversations;
mod creation;
mod discover;
mod federation;
pub(super) mod files;
mod history;
#[cfg(debug_assertions)]
mod latency;
pub(super) mod markup;
mod messaging;
mod notifications;
mod outbox;
mod preview;
mod reads;
mod references;
mod reminders;
mod removal;
mod search;
mod startup;
mod status;
mod summary;
mod thread_change;
mod threads;
mod translation;
mod typing;
mod update;
use crate::diagnostics::{
    self, Event as Diagnostic, Operation as DiagnosticOperation, Outcome as DiagnosticOutcome,
};
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};
use talk_adapters::identity::{
    AccountConnection, AccountSummary, ConnectionCompletion, ConnectionCursor, ConnectionRequestId,
    ConnectionSnapshot, ConnectionState, IdentityError, ProfileFailure, RuntimeCommand,
    RuntimeConfig, RuntimeReply, SavedAccountState, StoredState,
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
    AccountConnectionState, AccountLoadFailure, AccountView, Completion, ConnectionFailure,
    ContentState, ConversationCategory, ConversationKind, ConversationListState,
    ConversationMutationResult, ConversationOrganization, ConversationRef, ConversationView,
    FeatureSupport, LoginFailure, LoginPhase, NativeDesktopPort, NativeStoreState,
    OrganizationResult, Presence, PresentationSnapshot, RequestId, SavedAccountPhase,
    SavedAccountView, ServerInput, StorageChoice, StorageSaveFailure, StorageStatus,
    SubmissionResult, TagEdit, TagKind, TagRef, TagView, UiIntent,
};
use talk_core::ui::{Decision, Page, SubmitError, Text, UnavailableReason, UserIntent};

#[derive(Clone, Copy)]
enum Purpose {
    Outbox(outbox::Work),
    QueuedSetting(ConversationRef, OperationId),
    Data(cache::Work),
    Content(OperationId),
    Files(files::Work),
    Create(RequestId, AccountScope, creation::Work),
    Search(RequestId, AccountScope, bool),
    Discover(RequestId, AccountScope, Option<ConversationRef>),
    DiscoverProbe(AccountScope, u64),
    ParticipantIndex(ConversationRef, u64),
    Reference(ConversationRef, u128, u16, u64),
    Room(RequestId, ConversationRef, OperationId, conversations::Work),
    Message(RequestId, ConversationRef, OperationId, messaging::Work),
    IncomingCalls(AccountScope),
    MessageNotifications(AccountScope),
    AccountReminders(AccountScope, u64),
    FederationInvitations(AccountScope, u64),
    FederationChange(AccountScope, OperationId, u128),
    LiveEvents(AccountScope),
    Typing(ConversationRef, typing::Work),
    Call(talk_core::ids::CallScope),
    BreakoutLeave(
        talk_core::ids::CallScope,
        RequestId,
        ConversationRef,
        ConversationRef,
        bool,
    ),
    Tags(AccountScope),
    TagMutation(RequestId, AccountScope),
    Login(RequestId, LoginWork),
    Restore(AccountId),
    RestoreAutomatic(AccountId),
    Removal(RequestId, AccountId, removal::Step),
    Rooms(AccountScope),
    Threads(AccountScope, u32),
    Mutation(RequestId, ConversationRef),
    Send(RequestId, ConversationRef, OperationId),
    Preferences(AccountScope, PreferenceRequest),
}
#[derive(Clone, Copy)]
struct Pending {
    id: ConnectionRequestId,
    purpose: Purpose,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LoginWork {
    Begin,
    Poll,
    Browser,
    Save,
}
struct Login {
    request: RequestId,
    begin: ConnectionRequestId,
    server: ServerInput,
    reservation: Option<AccountId>,
    owner: Option<(AccountId, LoginAttemptId)>,
    poll_at: Instant,
    automatic_save_attempted: bool,
    automatic_save_pending: Option<Instant>,
    automatic_save_choice: talk_core::identity::StorageChoice,
    save_request: Option<ConnectionRequestId>,
}
struct Replacement {
    account: AccountId,
    // None only for an explicit Reauthenticate intent before its first Begin.
    // Retain the full input, not the origin-only display label: installations
    // under different paths on one host must never reuse a retry reservation.
    server: Option<ServerInput>,
    save_choice: talk_core::identity::StorageChoice,
}
#[derive(Clone, Copy)]
struct Unresolved {
    request: RequestId,
    context: ConversationRef,
    operation: OperationId,
}

#[derive(Clone, Copy)]
struct OptimisticRoom {
    request: RequestId,
    context: ConversationRef,
    favorite: Option<bool>,
    archived: Option<bool>,
}

impl OptimisticRoom {
    fn project(self, snapshot: &mut PresentationSnapshot) {
        snapshot.conversations = page(snapshot.conversations.as_slice().iter().cloned().map(
            |mut row| {
                if row.context == self.context {
                    if let Some(value) = self.favorite {
                        row.favorite = Some(value);
                    }
                    if let Some(value) = self.archived {
                        row.archived = Some(value);
                    }
                }
                row
            },
        ));
    }
}

pub struct Accounts {
    content_owner: content::ContentOwner,
    media: std::sync::Arc<talk_adapters::media::MediaService>,
    call_ui: Option<calls::CallUi>,
    breakout_leave: Option<calls::BreakoutLeave>,
    breakout_observation: Option<(ConversationRef, Page<protocol::Conversation, 20>)>,
    call_poll_at: Instant,
    call_events: calls::CallEvents,
    call_actions: std::sync::Arc<calls::CallActions>,
    connection: Result<AccountConnection, IdentityError>,
    cursor: ConnectionCursor,
    diagnostic_connection: Option<ConnectionState>,
    startup: startup::Restoration,
    next_request: u64,
    pending: Vec<Pending>,
    login: Option<Login>,
    login_retry_server: Option<ServerInput>,
    removal: Option<removal::Removal>,
    replace: Option<Replacement>,
    snapshot: PresentationSnapshot,
    dirty: bool,
    load_rooms: Option<AccountScope>,
    load_tags: Option<AccountScope>,
    preferences: Vec<AccountPreferenceSnapshot>,
    status_retry: Vec<status::Retry>,
    // Keep a DND intent until a scoped server observation resolves an uncertain write.
    message_pending: Vec<(AccountScope, PreferenceRequest)>,
    status_pending: Vec<(
        AccountScope,
        PreferenceRequest,
        talk_core::preferences::account::StatusChoice,
    )>,
    failures: Vec<(AccountId, LoginFailure)>,
    unresolved: Vec<Unresolved>,
    queued_send: Option<messaging::QueuedSend>,
    optimistic_room: Option<OptimisticRoom>,
    queued_read: reads::Reads,
    outbox: outbox::Outbox,
    room_invite: Option<(RequestId, talk_core::ui::conversation::PersonRef)>,
    message_search: Option<(ConversationRef, Text<256>, Option<Text<1024>>)>,
    account_search: Option<protocol::search::Query>,
    account_search_retry: Option<search::Retry>,
    discover_probe: Option<discover::Probe>,
    discover_probe_generation: u64,
    discovery_retry: Option<discover::Retry>,
    discovery_join: Option<discover::OptimisticJoin>,
    reference_owner: references::Owner,
    reference_generation: u64,
    notices: notifications::Notices,
    reminders: reminders::Reminders,
    federation: federation::Federation,
    background: background::Background,
    typing: typing::Typing,
    history: history::Cache,
    markup: markup::Owner,
    cache: cache::Cache,
    #[cfg(debug_assertions)]
    latency: latency::Metrics,
    picker: Option<files::Picker>,
    picker_open: bool,
    avatar_picker: Option<(RequestId, ConversationRef)>,
    avatar_dialog: Option<(RequestId, ConversationRef)>,
    csv_file_owner: files::CsvOwner,
    attachment_owner: files::Owner,
    network_revision: u64,
    last_tick: std::time::SystemTime,
    message_poll_at: Instant,
    message_poll_request: u64,
    thread_refresh_due: Option<(AccountScope, u32, Instant)>,
    scheduled_poll: Option<(ConversationRef, Instant)>,
    translation: translation::Translations,
    translation_poll: Option<(ConversationRef, Instant)>,
    summary: summary::Summaries,
}
impl Accounts {
    pub fn start(media: std::sync::Arc<talk_adapters::media::MediaService>) -> Rc<RefCell<Self>> {
        Self::start_with_config(media, RuntimeConfig::platform())
    }
    pub(super) fn start_with_config(
        media: std::sync::Arc<talk_adapters::media::MediaService>,
        config: RuntimeConfig,
    ) -> Rc<RefCell<Self>> {
        let mut snapshot = super::fixtures::snapshot(None);
        snapshot.content = ContentState::Loading;
        Rc::new(RefCell::new(Self {
            content_owner: content::ContentOwner::default(),
            connection: AccountConnection::start(config.with_media(media.clone())),
            media,
            call_ui: None,
            breakout_leave: None,
            breakout_observation: None,
            call_poll_at: Instant::now(),
            call_events: calls::CallEvents::new(),
            call_actions: std::sync::Arc::new(calls::CallActions::new()),
            cursor: ConnectionCursor::default(),
            diagnostic_connection: None,
            startup: startup::Restoration::default(),
            next_request: 0,
            pending: Vec::new(),
            login: None,
            login_retry_server: None,
            removal: None,
            replace: None,
            snapshot,
            dirty: true,
            load_rooms: None,
            load_tags: None,
            preferences: Vec::new(),
            status_retry: Vec::new(),
            status_pending: Vec::new(),
            message_pending: Vec::new(),
            failures: Vec::new(),
            unresolved: Vec::new(),
            queued_send: None,
            optimistic_room: None,
            queued_read: reads::Reads::default(),
            outbox: outbox::Outbox::default(),
            room_invite: None,
            message_search: None,
            account_search: None,
            account_search_retry: None,
            discover_probe: None,
            discover_probe_generation: 0,
            discovery_retry: None,
            discovery_join: None,
            reference_owner: references::Owner::default(),
            reference_generation: 0,
            notices: notifications::Notices::default(),
            reminders: reminders::Reminders::default(),
            federation: federation::Federation::default(),
            background: background::Background::default(),
            typing: typing::Typing::default(),
            history: history::Cache::default(),
            markup: markup::Owner::default(),
            cache: cache::Cache::default(),
            #[cfg(debug_assertions)]
            latency: latency::Metrics::default(),
            picker: None,
            picker_open: false,
            avatar_picker: None,
            avatar_dialog: None,
            csv_file_owner: files::CsvOwner::default(),
            attachment_owner: files::Owner::default(),
            network_revision: 0,
            last_tick: std::time::SystemTime::now(),
            message_poll_at: Instant::now(),
            message_poll_request: 0,
            thread_refresh_due: None,
            scheduled_poll: None,
            translation: translation::Translations::default(),
            summary: summary::Summaries::default(),
            translation_poll: None,
        }))
    }
    pub fn shutdown(&mut self) {
        self.clear_attachment_work();
        self.cancel_csv_file();
        self.avatar_picker = None;
        if self
            .picker
            .as_ref()
            .is_some_and(|picker| matches!(picker.kind, files::Kind::Avatar))
        {
            self.picker = None;
        }
        self.stop_video();
        self.queued_send = None;
        self.queued_read.clear();
        self.history = history::Cache::default();
        self.call_actions.clear();
        if let Ok(connection) = &mut self.connection {
            connection.shutdown();
        }
    }
    pub fn stopped(&self) -> bool {
        if !self.csv_file_stopped() {
            return false;
        }
        if !self.content_owner.video.stopped() {
            return false;
        }
        self.connection
            .as_ref()
            .map_or(true, AccountConnection::is_stopped)
    }
    fn admit(
        &mut self,
        command: RuntimeCommand,
        purpose: Purpose,
    ) -> Result<ConnectionRequestId, IdentityError> {
        // An empty room has no forward cursor, so its periodic poll still uses
        // History. Keep those no-change retries as quiet as ordinary empty polls.
        let idle_history = matches!(purpose, Purpose::Message(_, context, _, messaging::Work::History)
            if self.history.get(context).is_some_and(|h| h.loaded && h.latest.is_none()));
        let priority = match &command {
            RuntimeCommand::Request {
                request:
                    ProtocolRequest::Content {
                        command:
                            protocol::content::ContentCommand::Asset(
                                protocol::content::AssetRequest::Attachment { large: true, .. },
                            ),
                        ..
                    },
                ..
            } => 0,
            RuntimeCommand::Request {
                request:
                    ProtocolRequest::Content {
                        command:
                            protocol::content::ContentCommand::Asset(_)
                            | protocol::content::ContentCommand::RevalidateAvatar(_)
                            | protocol::content::ContentCommand::SelfProfile
                            | protocol::content::ContentCommand::RevalidateProfile,
                        ..
                    }
                    | ProtocolRequest::ResolveReference { .. }
                    | ProtocolRequest::PollTyping(_),
                ..
            } => 2,
            RuntimeCommand::Request {
                request:
                    ProtocolRequest::Content {
                        command:
                            protocol::content::ContentCommand::Video { .. }
                            | protocol::content::ContentCommand::VoiceAudio { .. }
                            | protocol::content::ContentCommand::DownloadCloudFile { .. },
                        ..
                    },
                ..
            } => 0,
            RuntimeCommand::Request {
                request: ProtocolRequest::Content { operation, .. },
                ..
            } => self.content_owner.priority(*operation),
            _ => purpose.priority(),
        };
        if priority == 2
            && let RuntimeCommand::Request { scope, .. } = &command
            && !self.background.background_ready(*scope, Instant::now())
        {
            return Err(ErrorKind::Busy.into());
        }
        let result = (|| {
            if self.call_actions.update_sealed() {
                return Err(ErrorKind::Busy.into());
            }
            // Preserve admission for interactive work even when disposable
            // image/history discovery fills its smaller lane.
            let capacity = match priority {
                2 => 10,
                1 => 14,
                _ => 16,
            };
            if self.pending.len() >= capacity {
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
                .try_submit_priority(
                    id,
                    command,
                    match priority {
                        2 => talk_adapters::identity::RequestPriority::Background,
                        1 => talk_adapters::identity::RequestPriority::Visible,
                        _ => talk_adapters::identity::RequestPriority::User,
                    },
                )?;
            self.next_request = next;
            self.pending.push(Pending { id, purpose });
            Ok(id)
        })();
        if let Some(operation) = purpose.diagnostic_operation()
            && operation != DiagnosticOperation::Poll
            && !idle_history
        {
            let (request, outcome, error) = match result {
                Ok(id) => (id.0, DiagnosticOutcome::Started, None),
                Err(error) => (
                    0,
                    DiagnosticOutcome::Rejected,
                    Some(diagnostic_error(error)),
                ),
            };
            diagnostics::record(Diagnostic {
                profile: None,
                request,
                operation,
                outcome,
                error,
                count: 0,
                storage: result.err().and_then(diagnostic_storage),
            });
        }
        result
    }
    fn request(
        &mut self,
        scope: AccountScope,
        request: ProtocolRequest,
        purpose: Purpose,
    ) -> Result<(), SubmitError> {
        self.current(scope)?;
        let priority = purpose.priority();
        if priority == 2 {
            if !self.background.background_ready(scope, Instant::now())
                || self
                    .pending
                    .iter()
                    .any(|p| p.purpose.account() == Some(scope) && p.purpose.priority() < priority)
                || self.pending.len() >= 8
            {
                return Err(SubmitError::Busy);
            }
        } else if let Ok(connection) = &mut self.connection {
            for pending in &self.pending {
                // Cancel only disposable reads. Durable work, call signaling
                // and mutations retain their owners and uncertainty protocol.
                if pending.purpose.account() == Some(scope)
                    && pending.purpose.priority() > priority
                    && (matches!(
                        pending.purpose,
                        Purpose::Message(_, _, _, messaging::Work::Prefetch)
                    ) || pending.purpose.room() != self.snapshot.selected)
                    && matches!(
                        pending.purpose,
                        Purpose::Message(
                            _,
                            _,
                            _,
                            messaging::Work::Prefetch
                                | messaging::Work::Latest
                                | messaging::Work::Poll
                                | messaging::Work::Metadata
                        )
                    )
                {
                    let _ = connection.cancel_request(pending.id);
                }
            }
        }
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
        self.retire_references();
        self.clear_attachment_work();
        self.cancel_csv_file();
        self.avatar_picker = None;
        self.breakout_leave = None;
        self.breakout_observation = None;
        self.queued_read.clear();
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
        if let Some(context) = self.snapshot.selected
            && let Some(key) = self.cache.stable_room(context).or_else(|| {
                self.snapshot
                    .conversations
                    .as_slice()
                    .iter()
                    .find(|r| r.context == context)
                    .and_then(|r| r.navigation_key.clone())
            })
        {
            self.cache.remember_selection(context.account.account, key);
        }
        self.snapshot.active_account = scope;
        self.thread_refresh_due = None;
        self.snapshot.threads = talk_core::ui::presentation::ThreadList {
            account: scope,
            ..Default::default()
        };
        self.retire_references();
        self.reference_owner = references::Owner::default();
        self.account_search = None;
        self.account_search_retry = None;
        self.snapshot.search_panel = None;
        self.snapshot.discover_panel = scope.map(discover::empty_panel);
        self.discovery_retry = None;
        self.discovery_join = None;
        self.discover_probe_generation = self.discover_probe_generation.saturating_add(1);
        self.discover_probe =
            scope.map(|scope| discover::Probe::new(scope, self.discover_probe_generation));
        self.snapshot.create_panel = scope.map(talk_core::ui::conversation::CreatePanel::new);
        self.snapshot.conversations = scope
            .and_then(|s| self.background.account(s))
            .map_or_else(Page::empty, |a| a.rooms.clone());
        self.snapshot.conversation_list = scope
            .and_then(|s| self.background.account(s))
            .map(|a| a.state)
            .unwrap_or(ConversationListState {
                content: if scope.is_some() {
                    ContentState::Loading
                } else {
                    ContentState::Unconfigured
                },
                complete: false,
            });
        self.snapshot.tags = Page::empty();
        self.snapshot.tag_state = ContentState::Unconfigured;
        self.load_tags = None;
        self.clear_content();
        self.project_account_reminders();
        self.project_federation();
        self.load_rooms = scope;
        self.touch();
    }
    fn diagnostic_connection(&mut self, state: ConnectionState) {
        if self.diagnostic_connection == Some(state) {
            return;
        }
        self.diagnostic_connection = Some(state);
        let failure = if let ConnectionState::Failed(error) = state {
            Some(error)
        } else {
            None
        };
        let (outcome, error) = match state {
            ConnectionState::Starting => (DiagnosticOutcome::Loading, None),
            ConnectionState::Ready => (DiagnosticOutcome::Ready, None),
            ConnectionState::Closing => (DiagnosticOutcome::Stopped, None),
            ConnectionState::Failed(error) => {
                (DiagnosticOutcome::Failed, Some(diagnostic_error(error)))
            }
        };
        diagnostics::record(Diagnostic {
            request: 0,
            operation: DiagnosticOperation::Connection,
            outcome,
            error,
            count: self.snapshot.accounts.as_slice().len() as u64,
            storage: failure.and_then(diagnostic_storage),
            profile: failure.and_then(diagnostic_profile),
        });
    }
    fn sync_accounts(&mut self, state: &ConnectionSnapshot) {
        self.diagnostic_connection(state.state);
        self.sync_cache_accounts(state);
        let accounts = page(state.accounts.as_slice().iter().filter_map(|a| {
            a.summary
                .scope
                .or_else(|| self.cache.local_scope(a.summary.account))
                .map(|scope| {
                    let mut row = account(&a.summary, scope);
                    if self.cache.is_local(scope) {
                        row.connection = if (self.startup.connecting(a) || self.pending.iter().any(|p| {
                            matches!(p.purpose, Purpose::RestoreAutomatic(id) | Purpose::Restore(id) if id == a.summary.account)
                        }))
                            && !self.failures.iter().any(|(id, _)| *id == a.summary.account)
                        {
                            AccountConnectionState::Connecting
                        } else {
                            AccountConnectionState::Disconnected
                        };
                    }
                    row
                })
        }));
        let saved = page(state.accounts.as_slice().iter().map(|a| {
            SavedAccountView {
                connected_once: a.summary.connected_once,
                id: a.summary.account,
                navigation_key: a.summary.navigation_key.clone(),
                label: a.summary.account_label.clone(),
                server_label: a.summary.server_label.clone(),
                phase: match a.state {
                    SavedAccountState::Dormant
                        if !matches!(a.summary.storage, StoredState::Eligible(_)) =>
                    {
                        if a.summary.attempt.is_none()
                            && a.summary.scope.is_none()
                            && !a.summary.cleanup_pending
                            && !a.summary.suppression_unconfirmed
                            && matches!(
                                a.summary.storage,
                                StoredState::Suppressed | StoredState::SessionOnly
                            )
                        {
                            SavedAccountPhase::ReauthenticationRequired
                        } else {
                            SavedAccountPhase::RecoveryRequired
                        }
                    }
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
                    SavedAccountState::RecoveryRequired
                        if a.summary.attempt.is_none()
                            && a.summary.scope.is_none()
                            && !a.summary.suppression_unconfirmed
                            && a.summary.recovery.is_some_and(|recovery| {
                                recovery.previous.is_none()
                                    && recovery.settlement
                                        == talk_core::identity::Settlement::SettledAbsent
                            }) =>
                    {
                        // A legacy failed first save has no credential to Restore.
                        // Explicit reauthorization still uses this exact account;
                        // the runtime must qualify the new grant and durable save.
                        SavedAccountPhase::ReauthenticationRequired
                    }
                    SavedAccountState::RecoveryRequired => SavedAccountPhase::RecoveryRequired,
                },
                storage: storage(&a.summary),
                cleanup_pending: a.summary.cleanup_pending,
                suppression_unconfirmed: a.summary.suppression_unconfirmed,
                failure: a
                    .summary
                    .connection_error
                    .and_then(connection_failure)
                    .map(LoginFailure::Connection)
                    .or_else(|| {
                        self.failures
                            .iter()
                            .find(|(id, _)| *id == a.summary.account)
                            .map(|(_, e)| *e)
                    }),
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
        self.restore_cache_views();
        let admitted = self.snapshot.accounts.as_slice();
        self.preferences
            .retain(|p| admitted.iter().any(|a| a.scope == p.scope));
        self.status_pending
            .retain(|(scope, _, _)| admitted.iter().any(|a| a.scope == *scope));
        self.message_pending
            .retain(|(scope, _)| admitted.iter().any(|a| a.scope == *scope));
        self.unresolved
            .retain(|u| admitted.iter().any(|a| a.scope == u.context.account));
        if self.snapshot.active_account.is_none()
            && let Some(account) = admitted.first()
        {
            self.select(Some(account.scope));
        }
        self.restore_cache_selection();
        if let ConnectionState::Failed(error) = state.state {
            self.account_load_failed(error);
        }
    }
    fn network_resumed(&mut self, notify: bool) {
        if let Ok(connection) = &mut self.connection {
            if notify {
                connection.network_changed();
            }
            self.network_revision = connection.network_revision();
            for pending in &self.pending {
                if matches!(
                    pending.purpose,
                    Purpose::Reference(..)
                        | Purpose::Message(
                            _,
                            _,
                            _,
                            messaging::Work::Poll
                                | messaging::Work::Latest
                                | messaging::Work::Prefetch
                                | messaging::Work::Metadata
                        )
                ) || matches!(pending.purpose,Purpose::Content(id) if self.content_owner.disposable(id))
                {
                    let _ = connection.cancel_request(pending.id);
                }
            }
        }
        if let Some(context) = self.snapshot.selected {
            self.history.request_catchup(context);
        }
        self.message_poll_at = Instant::now();
        self.background.resume(Instant::now());
        self.touch();
    }
    // Prime nonblocking account/cache admission before native window/GPU setup.
    // Workers retain all I/O; the regular presentation tick takes over afterward.
    pub(in crate::composition) fn prepare_startup(&mut self) {
        self.tick();
    }
    fn tick(&mut self) {
        let wall = std::time::SystemTime::now();
        if wall
            .duration_since(self.last_tick)
            .is_ok_and(|gap| gap > Duration::from_secs(5))
        {
            self.network_resumed(true);
        }
        self.last_tick = wall;
        if self
            .connection
            .as_ref()
            .is_ok_and(|connection| connection.network_revision() != self.network_revision)
        {
            self.network_resumed(false);
        }
        self.poll_csv_file();
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
                self.tick_cache();
                self.restore_on_startup(&update.snapshot);
            }
            Err(error) => {
                self.diagnostic_connection(ConnectionState::Failed(error));
                self.account_load_failed(error);
            }
        }
        self.expire_cached_presence();
        if let Some((request, deadline, choice)) = self.login.as_ref().and_then(|login| {
            login
                .automatic_save_pending
                .map(|deadline| (login.request, deadline, login.automatic_save_choice))
        }) {
            match self.save_login(request, choice) {
                Err(IdentityError::Request(ErrorKind::Busy)) if Instant::now() < deadline => {}
                result => {
                    if let Some(login) = &mut self.login {
                        login.automatic_save_pending = None;
                    }
                    if let Err(error) = result {
                        self.login_failed(error);
                    }
                }
            }
        }
        self.tick_removal();
        self.load_outbox();
        self.tick_status();
        self.tick_outbox();
        // A deliberate send gets the next free admission before asset/poll work.
        self.tick_queued_send(Instant::now());
        self.tick_queued_read(Instant::now());
        self.poll_messages();
        self.tick_content();
        self.tick_file_work();
        self.tick_account_search();
        self.tick_discovery_probe();
        self.tick_discovery_retry();
        self.tick_background();
        self.tick_typing();
        self.tick_thread_refresh(Instant::now());
        self.tick_account_reminders();
        self.tick_notification_navigation();
        self.tick_federation();
        self.tick_translations();
        self.tick_summary();
        self.tick_breakout_leave();
        self.tick_calls();
        self.tick_prefetch();
        self.tick_participant_index();
        self.tick_references();
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
                    .any(|p| matches!(p.purpose, Purpose::Login(r, _) if r == l.request)))
            .then_some((l.request, l.owner))
            .and_then(|(r, o)| o.map(|(a, t)| (r, a, t)))
        });
        if let Some((request, account, attempt)) = poll {
            if let Some(login) = &mut self.login {
                login.poll_at = Instant::now() + Duration::from_secs(1);
            }
            if let Err(error) = self.admit(
                RuntimeCommand::Poll { account, attempt },
                Purpose::Login(request, LoginWork::Poll),
            ) && error != IdentityError::Request(ErrorKind::Busy)
            {
                self.login_failed(error);
            }
        }
    }
    fn account_load_failed(&mut self, error: IdentityError) {
        let phase = LoginPhase::Failed(LoginFailure::AccountStorage(account_load_failure(error)));
        if self.snapshot.login.phase != phase || self.snapshot.content != ContentState::Failed {
            self.snapshot.login.phase = phase;
            self.snapshot.content = ContentState::Failed;
            self.touch();
        }
    }
    fn login_failed(&mut self, error: IdentityError) {
        match error {
            IdentityError::Storage { backend, failure } => {
                self.snapshot.login.storage_failure = Some(storage_save_failure(backend, failure));
                if backend != StorageBackend::PrivateFile {
                    self.snapshot.login.native_store = match failure {
                        StorageFailure::Locked => NativeStoreState::Locked,
                        StorageFailure::AccessDenied => NativeStoreState::Denied,
                        StorageFailure::Missing
                        | StorageFailure::Unsupported
                        | StorageFailure::Unavailable => NativeStoreState::Missing,
                        _ => NativeStoreState::Unknown,
                    };
                }
                self.snapshot.login.phase = LoginPhase::ChooseStorage;
                self.snapshot.login.storage = StorageStatus::NotSaved;
            }
            IdentityError::RecoveryRequired
            | IdentityError::SuppressionUnconfirmed
            | IdentityError::Request(ErrorKind::Uncertain) => {
                self.snapshot.login.storage = StorageStatus::RecoveryRequired;
                self.snapshot.login.storage_failure = Some(match error {
                    IdentityError::RecoveryRequired => StorageSaveFailure::RecoveryRequired,
                    IdentityError::SuppressionUnconfirmed => {
                        StorageSaveFailure::SuppressionUnconfirmed
                    }
                    _ => StorageSaveFailure::Uncertain,
                });
                self.snapshot.login.phase = LoginPhase::Failed(LoginFailure::Storage);
            }
            _ => self.snapshot.login.phase = LoginPhase::Failed(login_error(error)),
        }
        self.touch();
    }
    fn save_login(
        &mut self,
        request: RequestId,
        choice: talk_core::identity::StorageChoice,
    ) -> Result<(), IdentityError> {
        let (account, attempt) = self
            .login
            .as_ref()
            .filter(|login| login.request == request)
            .and_then(|login| login.owner)
            .ok_or(ErrorKind::WrongScope)?;
        let id = self.admit(
            RuntimeCommand::Save {
                account,
                attempt,
                choice,
            },
            Purpose::Login(request, LoginWork::Save),
        )?;
        if let Some(login) = &mut self.login {
            login.save_request = Some(id);
            // Only an admitted explicit choice changes a retry's destination.
            login.automatic_save_choice = choice;
        }
        self.snapshot.login.phase = LoginPhase::ChooseStorage;
        self.snapshot.login.storage = StorageStatus::Saving;
        self.snapshot.login.storage_failure = None;
        Ok(())
    }
    fn complete(&mut self, purpose: Purpose, done: &ConnectionCompletion) {
        // A stale/cancelled reply can settle retained work but cannot repopulate a new runtime.
        let result =
            if done.cancel_requested || (done.captured_scope.is_some() && !done.scope_is_current) {
                Err(IdentityError::Request(ErrorKind::WrongScope))
            } else {
                done.result.clone()
            };
        if let Some(scope) = done
            .captured_scope
            .filter(|_| done.scope_is_current && !done.cancel_requested)
        {
            let observed = match &result {
                Err(IdentityError::Request(
                    ErrorKind::Transport
                    | ErrorKind::Deadline
                    | ErrorKind::ServerUnavailable
                    | ErrorKind::RateLimited,
                )) => Some(false),
                Ok(RuntimeReply::Protocol(
                    ProtocolReply::Conversations(_) | ProtocolReply::Messaging { .. },
                )) => Some(true),
                _ => None,
            };
            if let Some(success) = observed {
                self.background
                    .transport_result(scope, done.request.0, success, Instant::now());
            }
        }
        match purpose {
            Purpose::Outbox(work) => self.complete_outbox_data(work, result),
            Purpose::QueuedSetting(context, operation) => {
                if done.cancel_requested || !done.scope_is_current {
                    self.outbox.lost_scope(operation);
                } else {
                    self.complete_queued_setting(context, operation, result);
                }
            }
            Purpose::Data(work) => self.complete_data(work, result),
            Purpose::Content(operation) => self.complete_content(operation, result),
            Purpose::Files(work) => self.complete_file_work(work, result),
            Purpose::Create(request, account, work) => {
                self.complete_creation(request, account, work, result)
            }
            Purpose::Search(request, account, append) => {
                self.complete_account_search(request, account, append, result)
            }
            Purpose::Discover(request, account, joining) => {
                self.complete_discovery(request, account, joining, result)
            }
            Purpose::DiscoverProbe(account, generation) => {
                self.complete_discovery_probe(account, generation, result)
            }
            Purpose::ParticipantIndex(context, generation) => {
                self.complete_participant_index(context, generation, result)
            }
            Purpose::Reference(context, message, ordinal, generation) => {
                self.complete_reference(context, message, ordinal, generation, result)
            }
            Purpose::Room(request, context, operation, work) => {
                self.complete_room(request, context, operation, work, result)
            }
            Purpose::Message(request, context, operation, work) => {
                self.complete_message((request, done.request.0), context, operation, work, result);
                self.cache_history(context);
            }
            Purpose::Call(scope) => self.complete_call(scope, result),
            Purpose::Typing(context, work) => self.complete_typing(context, work, result),
            Purpose::BreakoutLeave(scope, request, context, target, assigned) => {
                self.complete_breakout_leave(scope, request, context, target, assigned, result)
            }
            Purpose::IncomingCalls(scope) => self.complete_incoming(scope, result),
            Purpose::MessageNotifications(scope) => {
                self.complete_message_notifications(scope, result)
            }
            Purpose::AccountReminders(scope, version) => {
                self.complete_account_reminders(scope, version, result)
            }
            Purpose::FederationInvitations(scope, version) => {
                self.complete_federation(scope, version, result)
            }
            Purpose::FederationChange(scope, operation, invitation) => {
                self.complete_federation_change(scope, operation, invitation, result)
            }
            Purpose::LiveEvents(scope) => {
                if done.captured_scope != Some(scope)
                    || done.cancel_requested
                    || !done.scope_is_current
                {
                    return;
                }
                // A mailbox drain changes scheduling only, not the visible snapshot.
                return self.complete_live_events(scope, result);
            }
            Purpose::Login(request, work) => {
                let Some(login) = self.login.as_ref().filter(|l| l.request == request) else {
                    return;
                };
                let current_work = match work {
                    LoginWork::Begin => {
                        done.request == login.begin
                            && self.snapshot.login.phase == LoginPhase::Validating
                    }
                    LoginWork::Poll | LoginWork::Browser => {
                        self.snapshot.login.phase == LoginPhase::BrowserWaiting
                            && !login.automatic_save_attempted
                    }
                    LoginWork::Save => {
                        login.save_request == Some(done.request)
                            && self.snapshot.login.storage == StorageStatus::Saving
                    }
                };
                if !current_work
                    || done.cancel_requested
                    || (done.captured_scope.is_some() && !done.scope_is_current)
                    || login
                        .reservation
                        .is_some_and(|account| done.account != Some(account))
                    || login.owner.is_some_and(|(account, attempt)| {
                        done.account != Some(account) || done.attempt != Some(attempt)
                    })
                {
                    return;
                }
                if work == LoginWork::Begin
                    && let Some(login) = &mut self.login
                {
                    // A failed browser handoff can still have reserved an account.
                    // Keep its correlation so Retry does not consume another slot.
                    login.reservation = done.account;
                }
                if let Some(event) = login_completion_event(work, done) {
                    diagnostics::record(event);
                }
                match result {
                    Ok(
                        RuntimeReply::Begun { account, attempt }
                        | RuntimeReply::Pending { account, attempt },
                    ) => {
                        if self
                            .login
                            .as_ref()
                            .is_some_and(|l| l.automatic_save_attempted)
                        {
                            return;
                        }
                        self.failures.retain(|(id, _)| *id != account);
                        if let Some(login) = &mut self.login {
                            login.owner = Some((account, attempt));
                            login.poll_at = Instant::now() + Duration::from_secs(1);
                        }
                        self.snapshot.login.phase = LoginPhase::BrowserWaiting;
                    }
                    Ok(RuntimeReply::Authorized { account, attempt }) => {
                        let Some(login) = &mut self.login else {
                            return;
                        };
                        if login.automatic_save_attempted || login.owner != Some((account, attempt))
                        {
                            return;
                        }
                        // Exactly one automatic storage attempt. The identity owner still
                        // commits intent, reconciles uncertainty and authorizes activation.
                        login.automatic_save_attempted = true;
                        login.automatic_save_pending =
                            Some(Instant::now() + Duration::from_secs(5));
                        self.snapshot.login.phase = LoginPhase::ChooseStorage;
                        self.snapshot.login.storage = StorageStatus::Saving;
                    }
                    Ok(RuntimeReply::Active { summary, .. }) => {
                        if self
                            .login
                            .as_ref()
                            .and_then(|l| l.owner)
                            .is_none_or(|(account, _)| account != summary.account)
                            || !done.scope_is_current
                        {
                            return;
                        }
                        // The accepted activation now belongs to the account, not this
                        // login screen. Starting another login must not cancel it.
                        if let Some(login) = &mut self.login {
                            login.save_request = None;
                        }
                        self.snapshot.login.phase = LoginPhase::Ready;
                        self.snapshot.login.storage = storage(&summary);
                        self.snapshot.login.storage_failure = None;
                        self.snapshot.login.account_label =
                            summary.account_label.clone().unwrap_or_else(|| text(""));
                        // Keep the normalized destination selected before browser handoff.
                        if let Some(scope) = summary.scope {
                            self.select(Some(scope));
                        }
                    }
                    Ok(RuntimeReply::BrowserOpened { .. }) => (),
                    Err(IdentityError::Request(ErrorKind::Conflict)) if work == LoginWork::Poll => {
                        // Only a validated Poll can identify a duplicate canonical
                        // account. A conflict in another stage is not this diagnosis.
                        self.snapshot.login.phase = LoginPhase::Failed(LoginFailure::AccountExists);
                        self.snapshot.login.storage = StorageStatus::NotSaved;
                        self.snapshot.login.storage_failure = None;
                    }
                    Err(error) => {
                        let expired = work == LoginWork::Poll
                            && error == IdentityError::Request(ErrorKind::Deadline);
                        let failure = if expired {
                            LoginFailure::Expired
                        } else {
                            login_error(error)
                        };
                        if let Some(id) = done.account {
                            self.failures.retain(|(account, _)| *account != id);
                            if self.failures.len() < 8 {
                                self.failures.push((id, failure));
                            }
                        }
                        self.login_failed(error);
                        if expired {
                            self.snapshot.login.phase = LoginPhase::Failed(failure);
                        }
                    }
                    _ => self.login_failed(IdentityError::Request(ErrorKind::Malformed)),
                }
            }
            Purpose::Restore(id) | Purpose::RestoreAutomatic(id) => {
                let error = result.as_ref().err().copied();
                diagnostics::record(Diagnostic {
                    profile: None,
                    request: done.request.0,
                    operation: DiagnosticOperation::AccountRestore,
                    outcome: if matches!(&result, Ok(RuntimeReply::Active { .. })) {
                        DiagnosticOutcome::Accepted
                    } else {
                        DiagnosticOutcome::Failed
                    },
                    error: error.map(diagnostic_error),
                    storage: error.and_then(diagnostic_storage),
                    count: 0,
                });
                match result {
                    Ok(RuntimeReply::Active { summary, .. }) => {
                        if let Some(scope) = summary.scope
                            && done.scope_is_current
                            && self.current(scope).is_ok()
                            && (matches!(purpose, Purpose::Restore(_))
                                || self.snapshot.active_account.is_none())
                        {
                            self.select(Some(scope));
                        }
                    }
                    Err(error) => {
                        self.failures.retain(|(a, _)| *a != id);
                        if self.failures.len() < 8 {
                            self.failures.push((id, login_error(error)));
                        }
                    }
                    _ => (),
                }
            }
            Purpose::Removal(request, account, step) => {
                self.complete_removal(request, account, step, result)
            }
            Purpose::Rooms(scope) => self.complete_rooms(scope, result),
            Purpose::Threads(scope, offset) => self.complete_threads(scope, offset, result),
            Purpose::Tags(scope) | Purpose::TagMutation(_, scope) => {
                let completion = match result {
                    Ok(RuntimeReply::Protocol(ProtocolReply::Tags(tags)))
                        if tags.scope == scope =>
                    {
                        if self.snapshot.active_account == Some(scope) {
                            self.snapshot.tags = page(tags.tags.as_slice().iter().map(tag_view));
                            self.snapshot.tag_state = ContentState::Ready;
                        }
                        Completion::ServerAccepted
                    }
                    Ok(RuntimeReply::Protocol(ProtocolReply::TagCreated(tag)))
                        if tag.id.scope == scope =>
                    {
                        self.load_tags = Some(scope);
                        Completion::ServerAccepted
                    }
                    Ok(RuntimeReply::Protocol(ProtocolReply::Conversation(row)))
                        if row.id.account == scope.account =>
                    {
                        if self.snapshot.active_account == Some(scope) {
                            self.snapshot.conversations =
                                page(self.snapshot.conversations.as_slice().iter().map(|r| {
                                    if r.context.conversation == row.id {
                                        conversation_with_existing(&row, scope, r)
                                    } else {
                                        r.clone()
                                    }
                                }));
                        }
                        Completion::ServerAccepted
                    }
                    Err(error) => {
                        if self.snapshot.active_account == Some(scope) {
                            self.snapshot.tag_state = content_error(error);
                        }
                        completion_error(error)
                    }
                    _ => Completion::Failed(UnavailableReason::Unsupported),
                };
                if let Purpose::TagMutation(request, _) = purpose {
                    self.snapshot.organization_result = Some(OrganizationResult {
                        request,
                        account: scope,
                        result: completion,
                    });
                    // Catalog changes can change room membership. Reconcile, never infer deletion.
                    if self.snapshot.active_account == Some(scope)
                        && completion == Completion::ServerAccepted
                    {
                        self.load_rooms = Some(scope);
                    }
                }
            }
            Purpose::Mutation(request, context) => {
                if self
                    .optimistic_room
                    .is_some_and(|p| p.request == request && p.context == context)
                {
                    self.optimistic_room = None;
                }
                let completion = match result {
                    Ok(RuntimeReply::Protocol(ProtocolReply::Conversation(row)))
                        if row.id == context.conversation =>
                    {
                        if self.snapshot.active_account == Some(context.account) {
                            self.snapshot.conversations =
                                page(self.snapshot.conversations.as_slice().iter().map(|r| {
                                    if r.context == context {
                                        conversation_with_existing(&row, context.account, r)
                                    } else {
                                        r.clone()
                                    }
                                }));
                        }
                        Completion::ServerAccepted
                    }
                    Err(error) => completion_error(error),
                    _ => Completion::Failed(UnavailableReason::Unsupported),
                };
                self.snapshot.mutation = Some(ConversationMutationResult {
                    request,
                    context,
                    result: completion,
                });
            }
            Purpose::Send(request, context, operation) => {
                let error = result.as_ref().err().copied().map(diagnostic_error);
                let completion = match result {
                    Ok(RuntimeReply::Protocol(ProtocolReply::Send(receipt)))
                        if receipt.operation == operation =>
                    {
                        match receipt.state {
                            SendState::ServerAccepted(_) => Completion::ServerAccepted,
                            SendState::Rejected => {
                                Completion::Failed(UnavailableReason::PermissionDenied)
                            }
                            SendState::Uncertain => Completion::Uncertain,
                        }
                    }
                    Err(error) => completion_error(error),
                    _ => Completion::Uncertain,
                };
                diagnostics::record(Diagnostic {
                    profile: None,
                    request: done.request.0,
                    operation: DiagnosticOperation::Send,
                    outcome: diagnostic_completion(completion),
                    count: 0,
                    error,
                    storage: None,
                });
                if completion != Completion::Uncertain {
                    self.unresolved.retain(|u| u.operation != operation);
                }
                self.snapshot.submission = Some(SubmissionResult {
                    request,
                    context,
                    result: completion,
                });
            }
            Purpose::Preferences(scope, request) => {
                if self.complete_status(scope, request, &result) {
                    return;
                }
                if let Some(cache) = self.preferences.iter_mut().find(|p| p.scope == scope && matches!(p.state, AccountPreferenceState::Loading { request: r } | AccountPreferenceState::Saving { request: r } if r == request)) {
                    match result {
                        Ok(RuntimeReply::Protocol(ProtocolReply::Preferences(reply))) if reply.request == request && reply.values.scope == scope => {
                            let status_resolved = reply.values.user_status_observed || reply.values.user_status_enabled != Some(true);
                            let awaiting_status = self.status_pending.iter().any(|(pending_scope, _, _)| *pending_scope == scope)
                                || self.message_pending.iter().any(|(pending_scope, _)| *pending_scope == scope);
                            if status_resolved {
                                self.status_pending.retain(|(pending_scope, _, _)| *pending_scope != scope);
                                self.message_pending.retain(|(pending_scope, _)| *pending_scope != scope);
                            }
                            let mut values = reply.values;
                            if values.user_status_enabled == Some(true) && !values.user_status_observed
                                && let Some(previous) = cache.values.as_ref().filter(|v| v.scope == scope && v.user_status_enabled == Some(true) && v.user_status_observed)
                            {
                                values.user_presence = previous.user_presence;
                                values.status_message = previous.status_message.clone();
                                values.user_status_observed = true;
                            }
                            if values.user_status_enabled == Some(true) && values.predefined_statuses.is_none()
                                && let Some(previous) = cache.values.as_ref().filter(|v| v.scope == scope && v.user_status_enabled == Some(true))
                            {
                                values.predefined_statuses = previous.predefined_statuses.clone();
                            }
                            cache.values = Some(values);
                            cache.state = if awaiting_status && !status_resolved {
                                AccountPreferenceState::Failed { request, error: AccountPreferenceError::Uncertain }
                            } else { AccountPreferenceState::Ready };
                        }
                        Err(error) => {
                            let error = preference_error(error);
                            if !matches!(error, AccountPreferenceError::Network | AccountPreferenceError::Uncertain) {
                                self.status_pending.retain(|(pending_scope, pending_request, _)| *pending_scope != scope || *pending_request != request);
                                self.message_pending.retain(|(pending_scope, pending_request)| *pending_scope != scope || *pending_request != request);
                            }
                            cache.state = AccountPreferenceState::Failed { request, error };
                        },
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
        let status_change = match &command {
            AccountPreferenceCommand::Set {
                change:
                    change @ (talk_core::preferences::account::AccountPreferenceChange::UserPresence(_)
                    | talk_core::preferences::account::AccountPreferenceChange::StatusMessage(_)),
                ..
            } => Some(change.clone()),
            _ => None,
        };
        let message_change = matches!(
            &command,
            AccountPreferenceCommand::Set {
                change: talk_core::preferences::account::AccountPreferenceChange::StatusMessage(_),
                ..
            }
        );
        let status_choice = match &command {
            AccountPreferenceCommand::Set {
                change:
                    talk_core::preferences::account::AccountPreferenceChange::UserPresence(choice),
                ..
            } => Some(*choice),
            _ => None,
        };
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
                && (matches!(p.state, AccountPreferenceState::Saving { .. })
                    || matches!(p.state, AccountPreferenceState::Loading { .. })
                        && status_choice.is_none()
                        && !message_change)
        }) {
            return Err(AccountPreferenceError::Busy);
        }
        if self.preferences.len() >= 8 && !self.preferences.iter().any(|p| p.scope == scope) {
            return Err(AccountPreferenceError::Busy);
        }
        let submitted = self.admit(
            RuntimeCommand::Request {
                scope,
                request: protocol,
            },
            Purpose::Preferences(scope, request),
        );
        match submitted {
            Ok(_) => (),
            Err(IdentityError::Request(ErrorKind::Busy)) if status_change.is_some() => (),
            Err(error) => return Err(preference_error(error)),
        }
        if let Some(change) = status_change {
            self.status_retry.retain(|r| r.scope != scope);
            let mut retry = status::Retry::new(scope, request, change);
            if submitted.is_err() {
                retry.queued();
            }
            self.status_retry.push(retry);
        }
        if message_change {
            self.message_pending
                .retain(|(pending_scope, _)| *pending_scope != scope);
            self.message_pending.push((scope, request));
        }
        if let Some(choice) = status_choice {
            self.status_pending
                .retain(|(pending_scope, _, _)| *pending_scope != scope);
            self.status_pending.push((scope, request, choice));
        }
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
    fn suppress_account_notifications(&self, scope: AccountScope) -> bool {
        use talk_core::preferences::account::StatusChoice;
        self.status_pending
            .iter()
            .any(|(pending_scope, _, choice)| {
                *pending_scope == scope && *choice == StatusChoice::DoNotDisturb
            })
            || self.preferences.iter().any(|snapshot| {
                snapshot.scope == scope
                    && snapshot.values.as_ref().is_some_and(|values| {
                        values.user_status_enabled == Some(true)
                            && values.user_status_observed
                            && values.user_presence == Some(StatusChoice::DoNotDisturb)
                    })
            })
    }

    fn cancel_login(&mut self) {
        self.login_retry_server = None;
        if let Some(login) = self.login.take()
            && let Ok(connection) = &mut self.connection
        {
            // Save can activate between UI polls and retire the login attempt.
            // Its retained observation still owns that unaccepted activation;
            // cancelling only the attempt/Begin would leave the account active.
            if let Some(save) = login.save_request {
                let _ = connection.cancel_request(save);
            }
            if let Some((account, attempt)) = login.owner {
                let _ = connection.cancel_attempt(account, attempt);
            }
            let _ = connection.cancel_request(login.begin);
        }
        self.snapshot.login = super::fixtures::snapshot(None).login;
        self.touch();
    }
    fn prepare_login(&mut self, server: &ServerInput) {
        let target = if let Some(login) = &self.login {
            (login.server == *server)
                .then_some(login.reservation)
                .flatten()
                .map(|account| (account, login.automatic_save_choice))
        } else {
            self.replace.as_ref().and_then(|replacement| {
                replacement
                    .server
                    .as_ref()
                    .is_none_or(|previous| previous == server)
                    .then_some((replacement.account, replacement.save_choice))
            })
        };
        self.cancel_login();
        // Retain the target even if Begin admission is busy or unavailable. A
        // retry must not allocate B after Poll has already bound the user to A.
        self.replace = target.map(|(account, save_choice)| Replacement {
            account,
            server: Some(server.clone()),
            save_choice,
        });
    }
    fn open(&mut self, context: ConversationRef) -> Result<(), SubmitError> {
        if self
            .discovery_join
            .as_ref()
            .is_some_and(|join| join.context == context)
        {
            if self.snapshot.selected != Some(context) {
                self.clear_content();
                self.snapshot.selected = Some(context);
                self.snapshot.content = ContentState::Loading;
                self.touch();
            }
            return Ok(());
        }
        if self.cache.is_presentation(context) {
            self.select_conversation(context)?;
            return Ok(());
        }
        self.current(context.account)?;
        if !self.select_conversation(context)? {
            return Ok(());
        }
        self.start_messages(context)?;
        self.snapshot.content_panel.context = Some(context);
        let _ = self.start_poll(RequestId(0), protocol::content::PollCommand::Capabilities);
        self.touch();
        Ok(())
    }
    /// Navigation restores a previously authorized read projection. Send policy
    /// is not a read gate; the protocol owner revalidates every mutation.
    fn select_conversation(&mut self, context: ConversationRef) -> Result<bool, SubmitError> {
        self.snapshot
            .conversations
            .as_slice()
            .iter()
            .find(|r| r.context == context)
            .ok_or(SubmitError::StaleScope)?;
        if self.snapshot.active_account != Some(context.account) {
            return Err(SubmitError::StaleScope);
        }
        if self.snapshot.selected == Some(context) {
            return Ok(false);
        }
        self.remember_poll_room();
        self.clear_content();
        self.snapshot.selected = Some(context);
        self.restore_room_metadata(context);
        self.restore_messages(context);
        self.snapshot.content_panel.context = Some(context);
        self.restore_poll_room(context);
        self.touch();
        Ok(true)
    }
    fn submit(&mut self, intent: UiIntent) -> Result<(), SubmitError> {
        if let Some(result) = self.submit_call(&intent) {
            return result;
        }
        match intent {
            UiIntent::Typing { context, active } => self.typing_activity(context, active)?,
            UiIntent::RefreshThreads(scope) => self.refresh_threads(scope)?,
            UiIntent::PageThreads { account, offset } => self.page_threads(account, offset)?,
            UiIntent::MarkRoomUnread { request, context } => {
                self.queue_room_unread(request, context)?;
            }
            UiIntent::Discover {
                account,
                request,
                action,
            } => self.submit_discovery(account, request, action)?,
            UiIntent::AccountSearch {
                account,
                request,
                action,
            } => self.submit_account_search(account, request, action)?,
            UiIntent::FinishFileExport { request, context } => {
                self.finish_file_export(request, context)
            }
            UiIntent::OpenMessageLink {
                context,
                message,
                source,
                ordinal,
            } => {
                self.open_message_link(context, message, source, ordinal)?;
            }
            UiIntent::OpenLocation {
                context,
                message,
                placeholder,
                id,
            } => {
                self.open_location(context, message, placeholder, id)?;
            }
            UiIntent::RemoveAccount {
                request,
                account,
                scope,
            } => self.remove_account(request, account, scope)?,
            UiIntent::AcknowledgeAccountRemoval { request, account } => {
                if self
                    .snapshot
                    .account_removal
                    .is_some_and(|result| result.request == request && result.account == account)
                {
                    self.snapshot.account_removal = None;
                }
            }
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
            UiIntent::RetryAccountLoad | UiIntent::RepairAccountBinding => {
                if !matches!(
                    self.snapshot.login.phase,
                    LoginPhase::Failed(LoginFailure::AccountStorage(_))
                ) {
                    return Err(SubmitError::StaleScope);
                }
                let result = match &mut self.connection {
                    Ok(connection) if matches!(intent, UiIntent::RepairAccountBinding) => {
                        connection.repair_startup_binding()
                    }
                    Ok(connection) => connection.retry_startup(),
                    Err(_) => {
                        self.connection = AccountConnection::start(
                            RuntimeConfig::platform().with_media(self.media.clone()),
                        );
                        self.connection.as_ref().map(|_| ()).map_err(|error| *error)
                    }
                };
                match result {
                    Ok(()) => {
                        self.cursor = ConnectionCursor::default();
                        self.next_request = 0;
                        self.startup = startup::Restoration::default();
                        self.snapshot.login.phase = LoginPhase::EnterServer;
                        self.snapshot.content = ContentState::Loading;
                        self.diagnostic_connection(ConnectionState::Starting);
                        self.touch();
                    }
                    // A startup thread may still be releasing its lock. Keep
                    // the existing actionable failure; never retry in a loop.
                    Err(IdentityError::Request(ErrorKind::Busy)) => (),
                    Err(error) => self.account_load_failed(error),
                }
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
                self.replace = Some(Replacement {
                    account,
                    server: None,
                    save_choice: if self.snapshot.saved_accounts.as_slice().iter().any(|saved| {
                        saved.id == account && saved.storage == StorageStatus::PrivateFileStored
                    }) {
                        talk_core::identity::StorageChoice::PrivateFile
                    } else {
                        talk_core::identity::StorageChoice::Native
                    },
                });
            }
            UiIntent::BeginLogin { request, server } => {
                let selected_origin =
                    talk_adapters::identity::selected_server_origin(server.as_str())
                        .map_err(submit_error)?;
                self.prepare_login(&server);
                // Keep the full installation path even when worker admission fails.
                // The renderer only receives an origin label and cannot reconstruct it.
                self.login_retry_server = Some(server.clone());
                let replacement = self.replace.as_ref().map(|target| target.account);
                self.snapshot.login.server_label = selected_origin;
                let id = match self.admit(
                    RuntimeCommand::Begin {
                        server: text(server.as_str()),
                        replace: replacement,
                    },
                    Purpose::Login(request, LoginWork::Begin),
                ) {
                    Ok(id) => id,
                    Err(error) => {
                        self.login_failed(error);
                        return Err(submit_error(error));
                    }
                };
                self.login = Some(Login {
                    request,
                    begin: id,
                    server,
                    reservation: replacement,
                    owner: None,
                    poll_at: Instant::now(),
                    automatic_save_attempted: false,
                    automatic_save_pending: None,
                    save_request: None,
                    // Reauthorization preserves a deliberately committed file choice.
                    // Native-store recovery alone must not silently migrate it.
                    automatic_save_choice: self
                        .replace
                        .as_ref()
                        .map_or(talk_core::identity::StorageChoice::Native, |target| {
                            target.save_choice
                        }),
                });
                self.snapshot.login.phase = LoginPhase::Validating;
                self.snapshot.login.attempt = Some(request);
                self.snapshot.login.storage_failure = None;
            }
            UiIntent::RetryLogin { request } => {
                if !matches!(self.snapshot.login.phase, LoginPhase::Failed(_)) {
                    return Err(SubmitError::StaleScope);
                }
                let server = self
                    .login
                    .as_ref()
                    .map(|login| &login.server)
                    .or(self.login_retry_server.as_ref())
                    .ok_or(SubmitError::StaleScope)?
                    .clone();
                // Use the same admission/replacement path as an explicit address.
                // Never retry from the display label, which omits subpaths.
                self.submit(UiIntent::BeginLogin { request, server })?;
            }
            UiIntent::CancelLogin(request) => {
                if self.login.as_ref().is_some_and(|l| l.request == request) {
                    self.cancel_login();
                    self.replace = None;
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
                    Purpose::Login(request, LoginWork::Browser),
                )
                .map_err(submit_error)?;
            }
            UiIntent::ChooseStorage {
                attempt: request,
                choice,
            } => {
                if self.snapshot.login.phase != LoginPhase::ChooseStorage
                    || matches!(
                        self.snapshot.login.storage,
                        StorageStatus::Saving
                            | StorageStatus::RecoveryRequired
                            | StorageStatus::CleanupPending
                    )
                {
                    return Err(SubmitError::Busy);
                }
                let choice = match choice {
                    StorageChoice::NativeStore => talk_core::identity::StorageChoice::Native,
                    StorageChoice::PrivateFile => talk_core::identity::StorageChoice::PrivateFile,
                    StorageChoice::SessionOnly => {
                        return Err(SubmitError::Unavailable(UnavailableReason::Unsupported));
                    }
                };
                self.save_login(request, choice).map_err(submit_error)?;
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
                self.reminders.cancel_navigation();
                if !self.cache.is_local(scope) {
                    self.current(scope)?;
                }
                self.select(Some(scope));
            }
            UiIntent::OpenConversation(context) => {
                self.reminders.cancel_navigation();
                self.open(context)?;
            }
            UiIntent::RefreshAccountReminders(scope) => {
                if self.snapshot.active_account != Some(scope) {
                    return Err(SubmitError::StaleScope);
                }
                self.refresh_account_reminders(scope);
            }
            UiIntent::RefreshFederationInvitations(scope) => {
                if self.snapshot.active_account != Some(scope) {
                    return Err(SubmitError::StaleScope);
                }
                self.refresh_federation(scope);
            }
            UiIntent::ChangeFederationInvitation {
                scope,
                invitation,
                accept,
            } => {
                self.change_federation(scope, invitation, accept)?;
            }
            UiIntent::OpenReminder {
                request,
                context,
                message,
            } => {
                self.reminders.cancel_navigation();
                self.open_reminder(request, context, message)?;
            }
            UiIntent::OpenNotificationMessage {
                request,
                context,
                message,
            } => {
                self.open_notification_message(request, context, message)?;
            }
            UiIntent::Refresh(context) => {
                self.current(context.account)?;
                let already_selected = self.snapshot.selected == Some(context);
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
                if already_selected {
                    self.refresh_messages(context)?;
                }
            }
            UiIntent::SetArchived {
                request,
                context,
                archived,
            } => match self.queue_message_action(
                request,
                context,
                &talk_core::ui::messaging::MessageAction::SetArchived(archived),
            ) {
                Some(result) => result?,
                None => self.mutate(
                    request,
                    context,
                    ProtocolRequest::SetArchived {
                        conversation: context.conversation,
                        archived,
                    },
                )?,
            },
            UiIntent::SetFavorite {
                request,
                context,
                favorite,
            } => match self.queue_message_action(
                request,
                context,
                &talk_core::ui::messaging::MessageAction::SetFavorite(favorite),
            ) {
                Some(result) => result?,
                None => self.mutate(
                    request,
                    context,
                    ProtocolRequest::SetFavorite {
                        conversation: context.conversation,
                        favorite,
                    },
                )?,
            },
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
                if self.queued_send.is_some()
                    || self.snapshot.submission.is_some()
                    || self.pending.iter().any(|p| {
                        matches!(
                            p.purpose,
                            Purpose::Send(..) | Purpose::Message(_, _, _, messaging::Work::Send)
                        )
                    })
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
                self.status_pending
                    .retain(|(scope, _, _)| scope.account != account);
                self.message_pending
                    .retain(|(scope, _)| scope.account != account);
                self.history.remove_account(account);
                self.outbox.retire_account(account);
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
        let optimistic = match &protocol {
            ProtocolRequest::SetFavorite { favorite, .. } => Some(OptimisticRoom {
                request,
                context,
                favorite: Some(*favorite),
                archived: None,
            }),
            ProtocolRequest::SetArchived { archived, .. } => Some(OptimisticRoom {
                request,
                context,
                favorite: None,
                archived: Some(*archived),
            }),
            _ => None,
        };
        self.request(
            context.account,
            protocol,
            Purpose::Mutation(request, context),
        )?;
        self.optimistic_room = optimistic;
        Ok(())
    }
}
pub struct Port(pub Rc<RefCell<Accounts>>);
impl NativeDesktopPort for Port {
    fn composer_drafts(
        &self,
        context: ConversationRef,
    ) -> Page<talk_core::ui::messaging::ComposerDraft, 32> {
        self.0.borrow().composer_drafts(context)
    }
    fn save_composer_draft(
        &mut self,
        context: ConversationRef,
        draft: talk_core::ui::messaging::ComposerDraft,
    ) -> Result<(), SubmitError> {
        self.0.borrow_mut().save_composer_draft(context, draft)
    }

    #[cfg(debug_assertions)]
    fn enable_latency_probe(&mut self) {
        self.0.borrow_mut().latency.enable();
    }
    #[cfg(debug_assertions)]
    fn observe_rendered(&mut self, snapshot: &PresentationSnapshot) {
        self.0.borrow_mut().latency.rendered(snapshot);
    }
    fn try_submit(&mut self, intent: UiIntent) -> Result<(), SubmitError> {
        let mut model = self.0.borrow_mut();
        #[cfg(debug_assertions)]
        let start = model.latency.intent(&intent, &model.snapshot);
        model.submit(intent)?;
        #[cfg(debug_assertions)]
        model.latency.begin(start);
        Ok(())
    }
    fn take_snapshot(&mut self) -> Option<PresentationSnapshot> {
        let mut model = self.0.borrow_mut();
        model.tick();
        if !model.dirty {
            return None;
        }
        model.dirty = false;
        model.snapshot.revision = model.snapshot.revision.saturating_add(1);
        let mut snapshot = model.snapshot.clone();
        model.project_room_unread(&mut snapshot);
        model.project_thread_list(&mut snapshot);
        model.project_queued_settings(&mut snapshot);
        if let Some(pending) = model.optimistic_room {
            pending.project(&mut snapshot);
        }
        model.markup.project(&mut snapshot);
        Some(snapshot)
    }
}
impl Accounts {
    pub(super) fn voice_allowed(&self, context: ConversationRef) -> bool {
        self.snapshot.selected == Some(context)
            && self.snapshot.active_account == Some(context.account)
            && self.snapshot.content == ContentState::Ready
            && self.snapshot.message_panel.as_ref().is_some_and(|panel| {
                panel.context == context
                    && panel
                        .availability(talk_core::ui::messaging::MessageFeature::Voice)
                        .enabled()
                    && panel
                        .availability(talk_core::ui::messaging::MessageFeature::Files)
                        .enabled()
            })
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
        connection_failure: a.connection_error.and_then(connection_failure),
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
        action_error: None,
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
        unread_direct_mention: r.unread_direct_mention,
        last_read_message: r.last_read_message.map(u128::from),
        unread_mention: r.unread_mention,
        notification_muted: r.notification_muted,
        last_activity: r.last_activity,
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
        presence: match r.user_status.as_ref().map(|status| status.presence) {
            Some(protocol::UserPresence::Online) => Presence::Available,
            Some(protocol::UserPresence::Away) => Presence::Away,
            Some(protocol::UserPresence::DoNotDisturb) => Presence::DoNotDisturb,
            Some(protocol::UserPresence::Offline | protocol::UserPresence::Invisible) => {
                Presence::Offline
            }
            Some(protocol::UserPresence::Unknown) | None => Presence::Unknown,
        },
        status_icon: r
            .user_status
            .as_ref()
            .and_then(|status| status.icon.clone()),
        status_message: r
            .user_status
            .as_ref()
            .and_then(|status| status.message.clone()),
        participants: None,
        participant_names: Page::empty(),
        send: r.actions.send_message,
        attach: Decision::Unknown,
        draft_clipboard: if r.sensitive || r.classified || r.organization.classified == Some(true) {
            Decision::Denied
        } else if !r.unknown_restrictions && r.organization.classified == Some(false) {
            Decision::Allowed
        } else {
            Decision::Unknown
        },
        call: r.actions.join_call,
        call_active: r.call_active,
        call_scope: r.call_scope,
    }
}

// Single-room metadata and mutation replies do not request status. Preserve only
// the current same-room observation; the next authoritative list can clear it.
fn conversation_with_existing(
    room: &protocol::Conversation,
    scope: AccountScope,
    existing: &ConversationView,
) -> ConversationView {
    let mut row = conversation(room, scope);
    if room.user_status.is_none()
        && row.context == existing.context
        && row.kind == ConversationKind::Direct
        && existing.kind == ConversationKind::Direct
    {
        row.presence = existing.presence;
        row.status_icon = existing.status_icon.clone();
        row.status_message = existing.status_message.clone();
    }
    row
}

impl Purpose {
    fn diagnostic_operation(self) -> Option<DiagnosticOperation> {
        match self {
            Self::Login(_, work) => work.diagnostic_operation(),
            Self::Message(_, _, _, work) => work.diagnostic_operation(),
            Self::Send(..) => Some(DiagnosticOperation::Send),
            Self::Restore(_) | Self::RestoreAutomatic(_) => {
                Some(DiagnosticOperation::AccountRestore)
            }
            _ => None,
        }
    }
}
impl LoginWork {
    fn diagnostic_operation(self) -> Option<DiagnosticOperation> {
        match self {
            Self::Begin => Some(DiagnosticOperation::LoginBegin),
            Self::Poll => Some(DiagnosticOperation::LoginPoll),
            Self::Save => Some(DiagnosticOperation::LoginSave),
            Self::Browser => None,
        }
    }
}
fn diagnostic_storage(error: IdentityError) -> Option<(StorageBackend, StorageFailure)> {
    match error {
        IdentityError::Storage { backend, failure } => Some((backend, failure)),
        _ => None,
    }
}
fn diagnostic_profile(error: IdentityError) -> Option<ProfileFailure> {
    match error {
        IdentityError::Profile(failure) => Some(failure),
        _ => None,
    }
}
fn account_load_failure(error: IdentityError) -> AccountLoadFailure {
    use AccountLoadFailure as F;
    match error {
        IdentityError::Profile(failure) => match failure {
            ProfileFailure::UnsafeLocation => F::UnsafeLocation,
            ProfileFailure::Incomplete => F::Incomplete,
            ProfileFailure::BindingMismatch => F::BindingMismatch,
            ProfileFailure::DeviceChanged => F::DeviceChanged,
            ProfileFailure::UnsupportedSchema => F::UnsupportedVersion,
            ProfileFailure::InvalidMetadata => F::InvalidData,
        },
        IdentityError::Storage { failure, .. } => match failure {
            StorageFailure::Conflict => F::InUse,
            StorageFailure::AccessDenied => F::AccessDenied,
            StorageFailure::Corrupt => F::InvalidData,
            _ => F::Unavailable,
        },
        IdentityError::RecoveryRequired | IdentityError::SuppressionUnconfirmed => {
            F::RecoveryRequired
        }
        _ => F::Unavailable,
    }
}
fn login_completion_event(work: LoginWork, done: &ConnectionCompletion) -> Option<Diagnostic> {
    let operation = work.diagnostic_operation()?;
    let (outcome, error) = match &done.result {
        Err(IdentityError::SuppressionUnconfirmed) => (
            DiagnosticOutcome::Uncertain,
            Some(ErrorKind::StorageUnavailable),
        ),
        Err(IdentityError::Request(ErrorKind::Uncertain)) => {
            (DiagnosticOutcome::Uncertain, Some(ErrorKind::Uncertain))
        }
        Err(error) => (DiagnosticOutcome::Failed, Some(diagnostic_error(*error))),
        Ok(RuntimeReply::Pending { .. }) if work == LoginWork::Poll => {
            (DiagnosticOutcome::Loading, None)
        }
        Ok(RuntimeReply::Begun { .. }) if work == LoginWork::Begin => {
            (DiagnosticOutcome::Accepted, None)
        }
        Ok(RuntimeReply::Authorized { .. }) if work == LoginWork::Poll => {
            (DiagnosticOutcome::Accepted, None)
        }
        Ok(RuntimeReply::Active { .. }) if work == LoginWork::Save => {
            if done.scope_is_current {
                (DiagnosticOutcome::Accepted, None)
            } else {
                (DiagnosticOutcome::Ignored, Some(ErrorKind::WrongScope))
            }
        }
        _ => (DiagnosticOutcome::Failed, Some(ErrorKind::Malformed)),
    };
    Some(Diagnostic {
        profile: None,
        request: done.request.0,
        operation,
        outcome,
        error,
        count: 0,
        storage: done
            .result
            .as_ref()
            .err()
            .copied()
            .and_then(diagnostic_storage),
    })
}
fn diagnostic_error(error: IdentityError) -> ErrorKind {
    match error {
        IdentityError::Request(error) => error,
        IdentityError::Storage { .. }
        | IdentityError::Profile(_)
        | IdentityError::RecoveryRequired
        | IdentityError::SuppressionUnconfirmed => ErrorKind::StorageUnavailable,
        IdentityError::WorkerUnavailable => ErrorKind::Unconfigured,
    }
}
fn diagnostic_completion(completion: Completion) -> DiagnosticOutcome {
    match completion {
        Completion::Queued | Completion::ServerAccepted => DiagnosticOutcome::Accepted,
        Completion::Uncertain => DiagnosticOutcome::Uncertain,
        Completion::Failed(_) => DiagnosticOutcome::Failed,
    }
}

fn submit_error(e: IdentityError) -> SubmitError {
    if let IdentityError::Request(kind) = e
        && let Some(failure) = connection_failure(kind)
    {
        return SubmitError::Unavailable(UnavailableReason::Connection(failure));
    }
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
            ErrorKind::Transport
            | ErrorKind::Deadline
            | ErrorKind::ServerUnavailable
            | ErrorKind::ServerAccessDenied,
        ) => ContentState::Offline,
        _ => ContentState::Failed,
    }
}
fn login_error(e: IdentityError) -> LoginFailure {
    if let IdentityError::Request(kind) = e
        && let Some(failure) = connection_failure(kind)
    {
        return LoginFailure::Connection(failure);
    }
    match e {
        IdentityError::Storage { .. }
        | IdentityError::Profile(_)
        | IdentityError::RecoveryRequired
        | IdentityError::SuppressionUnconfirmed => LoginFailure::Storage,
        IdentityError::Request(
            ErrorKind::PermissionDenied | ErrorKind::ReauthenticationRequired,
        ) => LoginFailure::Denied,
        _ => LoginFailure::Unconfigured,
    }
}

fn connection_failure(kind: ErrorKind) -> Option<ConnectionFailure> {
    Some(match kind {
        ErrorKind::ServerAccessDenied => ConnectionFailure::AccessDenied,
        ErrorKind::Transport => ConnectionFailure::Unreachable,
        ErrorKind::Deadline => ConnectionFailure::TimedOut,
        ErrorKind::ServerUnavailable => ConnectionFailure::ServerUnavailable,
        ErrorKind::RateLimited => ConnectionFailure::RateLimited,
        ErrorKind::Malformed | ErrorKind::RedirectRejected | ErrorKind::TooLarge => {
            ConnectionFailure::InvalidResponse
        }
        _ => return None,
    })
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

fn storage_save_failure(backend: StorageBackend, failure: StorageFailure) -> StorageSaveFailure {
    if backend == StorageBackend::PrivateFile {
        return match failure {
            StorageFailure::AccessDenied => StorageSaveFailure::LocalDenied,
            StorageFailure::Corrupt => StorageSaveFailure::LocalCorrupt,
            _ => StorageSaveFailure::LocalUnavailable,
        };
    }
    match failure {
        StorageFailure::Locked => StorageSaveFailure::NativeLocked,
        StorageFailure::AccessDenied => StorageSaveFailure::NativeDenied,
        StorageFailure::Missing | StorageFailure::Unsupported | StorageFailure::Unavailable => {
            StorageSaveFailure::NativeUnavailable
        }
        StorageFailure::Corrupt => StorageSaveFailure::NativeCorrupt,
        StorageFailure::TemporarilyUnavailable => StorageSaveFailure::NativeTemporarilyUnavailable,
        StorageFailure::Cancelled => StorageSaveFailure::Cancelled,
        StorageFailure::Conflict => StorageSaveFailure::Conflict,
        StorageFailure::Other => StorageSaveFailure::Failed,
    }
}
