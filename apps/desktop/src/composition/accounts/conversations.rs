//! Account-owned settings bridge. No speculative mutation is projected as confirmed state.
use super::*;
use talk_core::protocol::conversation as wire;
use talk_core::ui::conversation as view;

#[cfg(test)]
#[path = "conversations_metadata_tests.rs"]
mod metadata_tests;

// The existing room-list schedule services invalidation and the safety fallback.
// No timer or request queue belongs to metadata. Each of eight accounts retains
// at most its 100 current rooms; removal/runtime replacement drops all tickets.
const METADATA_LIFETIME: Duration = Duration::from_secs(60);
const METADATA_RETRY: Duration = Duration::from_secs(2);

struct RoomObservation {
    context: ConversationRef,
    listed: protocol::Conversation,
    details: Option<view::RoomDetails>,
    observed_at: Option<u64>,
    observed: Option<Instant>,
    attempted: Option<Instant>,
    dirty: bool,
    operation: Option<OperationId>,
    reading_details: bool,
}
#[derive(Default)]
pub(super) struct Metadata {
    rooms: Vec<RoomObservation>,
    hinted: bool,
}
fn metadata_identity(room: &protocol::Conversation) -> protocol::Conversation {
    let mut identity = room.clone();
    identity.unread_messages = None;
    identity.last_activity = None;
    identity.call_active = None;
    identity.call_scope = None;
    identity
}
impl Metadata {
    pub(super) fn hint(&mut self) -> Vec<ConversationRef> {
        self.hinted = true;
        let mut retired = Vec::new();
        for room in &mut self.rooms {
            room.dirty = true;
            if room.reading_details && room.operation.take().is_some() {
                retired.push(room.context);
            }
        }
        retired
    }
    pub(super) fn reconcile(
        &mut self,
        scope: AccountScope,
        list: &protocol::ConversationList,
    ) -> Vec<(ConversationRef, bool)> {
        let mut retired = Vec::new();
        self.rooms.retain(|old| {
            old.context.account == scope
                && list
                    .rooms
                    .as_slice()
                    .iter()
                    .any(|r| r.id == old.context.conversation)
        });
        for room in list.rooms.as_slice() {
            let context = ConversationRef {
                account: scope,
                conversation: room.id,
            };
            let listed = metadata_identity(room);
            if let Some(old) = self.rooms.iter_mut().find(|r| r.context == context) {
                if self.hinted || old.listed != listed {
                    let policy_changed = old.listed.role != listed.role
                        || old.listed.actions != listed.actions
                        || old.listed.sensitive != listed.sensitive
                        || old.listed.classified != listed.classified
                        || old.listed.channel != listed.channel
                        || old.listed.unknown_restrictions != listed.unknown_restrictions
                        || old.listed.organization != listed.organization;
                    old.dirty = true;
                    if old.reading_details || policy_changed {
                        old.operation = None;
                    }
                    retired.push((context, policy_changed));
                }
                old.listed = listed;
            } else if self.rooms.len() < 100 {
                self.rooms.push(RoomObservation {
                    context,
                    listed,
                    details: None,
                    observed_at: None,
                    observed: None,
                    attempted: None,
                    dirty: true,
                    operation: None,
                    reading_details: false,
                });
            }
        }
        self.hinted = false;
        retired
    }
    pub(super) fn invalidate(&mut self, context: ConversationRef) {
        if let Some(room) = self.rooms.iter_mut().find(|r| r.context == context) {
            room.dirty = true;
        }
    }
    pub(super) fn needs_read(&self, context: ConversationRef, now: Instant) -> bool {
        self.rooms
            .iter()
            .find(|r| r.context == context)
            .is_some_and(|r| {
                r.listed.actions.read_messages == Decision::Allowed
                    && r.operation.is_none()
                    && (r.dirty
                        || !r.observed.is_some_and(|at| {
                            now.saturating_duration_since(at) < METADATA_LIFETIME
                        }))
                    && !r
                        .attempted
                        .is_some_and(|at| now.saturating_duration_since(at) < METADATA_RETRY)
            })
    }
    fn attempted(&mut self, context: ConversationRef, now: Instant) {
        if let Some(room) = self.rooms.iter_mut().find(|r| r.context == context) {
            room.attempted = Some(now);
        }
    }
    fn admitted(&mut self, context: ConversationRef, operation: OperationId, details: bool) {
        if let Some(room) = self.rooms.iter_mut().find(|r| r.context == context) {
            room.operation = Some(operation);
            room.reading_details = details;
            if details {
                room.dirty = false;
            }
        }
    }
    fn complete(&mut self, context: ConversationRef, operation: OperationId) -> bool {
        let Some(room) = self
            .rooms
            .iter_mut()
            .find(|r| r.context == context && r.operation == Some(operation))
        else {
            return false;
        };
        room.operation = None;
        true
    }
    fn remember(
        &mut self,
        context: ConversationRef,
        details: &view::RoomDetails,
        ordinary: bool,
        now: Instant,
    ) {
        if let Some(room) = self.rooms.iter_mut().find(|r| r.context == context) {
            room.observed = Some(now);
            room.observed_at = Some(history::now_unix());
            room.details = ordinary.then(|| readonly_details(details.clone()));
        }
    }
    fn restored(&self, context: ConversationRef) -> Option<view::RoomDetails> {
        let room = self.rooms.iter().find(|r| r.context == context)?;
        let at = room.observed_at?;
        super::cache::fresh_metadata(at, history::now_unix())
            .then(|| room.details.clone())
            .flatten()
    }
    pub(super) fn revoke(&mut self, context: ConversationRef) {
        if let Some(room) = self.rooms.iter_mut().find(|r| r.context == context) {
            room.details = None;
            room.observed_at = None;
            room.observed = None;
            room.operation = None;
        }
    }
}
fn readonly_details(mut details: view::RoomDetails) -> view::RoomDetails {
    details.controls = Page::empty();
    details.leave = Decision::Unknown;
    details.default_permissions = None;
    details.server_default_permissions = None;
    details.effective_permissions = None;
    details.has_password = None;
    details
}

#[derive(Clone, Copy)]
pub(super) enum Work {
    Read,
    ReadKnown,
    Revalidate,
    OtherRead,
    MemberCount,
    Write,
    Directory,
    Csv,
    Bots,
    BotChange { bot: view::PersonRef, enabled: bool },
}
impl Work {
    fn for_action(action: &view::ConversationAction) -> Self {
        match action {
            view::ConversationAction::Directory { .. } => Self::Directory,
            view::ConversationAction::PreviewCsv(_) => Self::Csv,
            view::ConversationAction::Bots => Self::Bots,
            view::ConversationAction::SetBotEnabled { bot, enabled } => Self::BotChange {
                bot: *bot,
                enabled: *enabled,
            },
            view::ConversationAction::Read => Self::Read,
            action if action.reads() => Self::OtherRead,
            _ => Self::Write,
        }
    }
    fn details(self) -> bool {
        matches!(self, Self::Read | Self::ReadKnown | Self::Revalidate)
    }
    fn read(self) -> bool {
        matches!(
            self,
            Self::Read
                | Self::ReadKnown
                | Self::Revalidate
                | Self::OtherRead
                | Self::MemberCount
                | Self::Directory
                | Self::Csv
                | Self::Bots
        )
    }
    fn preview(self) -> bool {
        matches!(self, Self::Directory | Self::Csv)
    }
    fn bots(self) -> bool {
        matches!(self, Self::Bots | Self::BotChange { .. })
    }
}

impl Accounts {
    /// Creation returns an authoritative room before the next complete list.
    /// Register it before opening, so its very first detail ticket can settle.
    pub(super) fn observe_created_room(
        &mut self,
        scope: AccountScope,
        room: protocol::Conversation,
    ) {
        let Some(account) = self.background.account_mut(scope) else {
            return;
        };
        let rooms = page(
            std::iter::once(room.clone()).chain(
                account
                    .metadata
                    .rooms
                    .iter()
                    .filter(|old| {
                        old.context.conversation != room.id
                            && self
                                .snapshot
                                .conversations
                                .as_slice()
                                .iter()
                                .any(|row| row.context == old.context)
                    })
                    .take(99)
                    .map(|old| old.listed.clone()),
            ),
        );
        let retired = account
            .metadata
            .reconcile(scope, &protocol::ConversationList { scope, rooms });
        for (context, policy_changed) in retired {
            if policy_changed {
                self.revoke_room_metadata(context, UnavailableReason::PolicyStale);
            } else {
                self.retire_room_metadata_read(context);
            }
        }
    }
    pub(super) fn restore_room_metadata(&mut self, context: ConversationRef) {
        let details = self
            .background
            .account(context.account)
            .and_then(|a| a.metadata.restored(context))
            .or_else(|| self.cached_room_details(context));
        let mut panel = view::ConversationPanel::new(context);
        panel.details_stale = details.is_some();
        panel.details = details;
        self.snapshot.conversation_panel = Some(panel);
        if let Some(account) = self.background.account_mut(context.account) {
            account.metadata.invalidate(context);
        }
    }
    pub(super) fn hydrate_room_metadata(&mut self, context: ConversationRef) {
        if !self
            .snapshot
            .conversation_panel
            .as_ref()
            .is_some_and(|p| p.context == context && p.details.is_none())
        {
            return;
        }
        if let Some(details) = self.cached_room_details(context)
            && let Some(panel) = self.snapshot.conversation_panel.as_mut()
        {
            panel.details = Some(details);
            panel.details_stale = true;
            self.touch();
        }
    }
    pub(super) fn revalidate_room_metadata(&mut self, context: ConversationRef) {
        let now = Instant::now();
        if !self
            .background
            .account(context.account)
            .is_some_and(|a| a.metadata.needs_read(context, now))
        {
            return;
        }
        if let Some(account) = self.background.account_mut(context.account) {
            account.metadata.attempted(context, now);
        }
        let _ = self.room_action_inner(RequestId(0), context, view::ConversationAction::Read, true);
    }
    pub(super) fn retire_room_metadata_read(&mut self, context: ConversationRef) {
        if let Ok(connection) = &mut self.connection {
            for pending in &self.pending {
                if matches!(pending.purpose, Purpose::Room(_, c, _, work) if c == context && work.details())
                {
                    let _ = connection.cancel_request(pending.id);
                }
            }
        }
        if let Some(panel) = self
            .snapshot
            .conversation_panel
            .as_mut()
            .filter(|p| p.context == context)
            && let view::RemoteState::Loading(request) = panel.state
        {
            panel.state = if panel.details.is_some() {
                view::RemoteState::Idle
            } else {
                view::RemoteState::Failed(request, UnavailableReason::PolicyStale)
            };
            self.touch();
        }
    }
    /// Retire authority before cancelling external work; an already dispatched
    /// reply must not restore controls after a later room/message denial.
    pub(super) fn revoke_room_metadata(
        &mut self,
        context: ConversationRef,
        reason: UnavailableReason,
    ) {
        if let Some(account) = self.background.account_mut(context.account) {
            account.metadata.revoke(context);
        }
        if let Ok(connection) = &mut self.connection {
            for pending in &self.pending {
                if matches!(pending.purpose, Purpose::Room(_, c, _, _) if c == context) {
                    let _ = connection.cancel_request(pending.id);
                }
            }
        }
        if let Some(panel) = self
            .snapshot
            .conversation_panel
            .as_mut()
            .filter(|p| p.context == context)
        {
            if let Some(details) = panel.details.as_mut() {
                *details = readonly_details(details.clone());
                if reason == UnavailableReason::PermissionDenied {
                    details.leave = Decision::Denied;
                }
            }
            panel.details_stale = true;
            panel.members = Page::empty();
            panel.members_loaded = false;
            panel.directory = Page::empty();
            panel.directory_loaded = false;
            panel.bots = Page::empty();
            panel.bots_loaded = false;
            panel.bans = Page::empty();
            panel.bans_loaded = false;
            panel.guest_link = None;
            panel.state = match panel.state {
                view::RemoteState::Saving(request) => view::RemoteState::Uncertain(request),
                view::RemoteState::Uncertain(request) => view::RemoteState::Uncertain(request),
                state @ view::RemoteState::Failed(_, UnavailableReason::PermissionDenied) => state,
                _ => view::RemoteState::Failed(RequestId(0), reason),
            };
        }
        self.forget_cached_room_details(context);
    }
    pub(super) fn operation(&self, context: ConversationRef) -> Result<OperationId, SubmitError> {
        self.current(context.account)?;
        if self.snapshot.selected != Some(context) {
            return Err(SubmitError::StaleScope);
        }
        self.connection
            .as_ref()
            .map_err(|e| submit_error(*e))?
            .new_operation_id(context.account)
            .map_err(submit_error)
    }
    pub(super) fn room_action(
        &mut self,
        request: RequestId,
        context: ConversationRef,
        action: view::ConversationAction,
    ) -> Result<(), SubmitError> {
        self.room_action_inner(request, context, action, false)
    }
    fn room_action_inner(
        &mut self,
        request: RequestId,
        context: ConversationRef,
        action: view::ConversationAction,
        automatic: bool,
    ) -> Result<(), SubmitError> {
        match action {
            view::ConversationAction::PickCsv { edit_generation } => {
                return self.queue_csv_picker(request, context, edit_generation);
            }
            view::ConversationAction::CancelCsvPick { pick_request } => {
                self.cancel_csv_picker(context, pick_request);
                return Ok(());
            }
            _ => {}
        }
        if self.cache.is_presentation(context) {
            return Err(SubmitError::Unavailable(UnavailableReason::PolicyStale));
        }
        let mut work = Work::for_action(&action);
        if automatic && matches!(action, view::ConversationAction::Members) {
            work = Work::MemberCount;
        }
        if matches!(work, Work::Read)
            && self
                .snapshot
                .conversation_panel
                .as_ref()
                .is_some_and(|p| p.context == context && p.details.is_some())
        {
            work = if automatic {
                Work::Revalidate
            } else {
                Work::ReadKnown
            };
        }
        let read = work.read();
        if work.bots() && self.snapshot.selected != Some(context) {
            return Err(SubmitError::StaleScope);
        }
        if work.bots() && !bots_online(&self.snapshot, context) {
            return Err(SubmitError::Unavailable(UnavailableReason::Connection(
                talk_core::ui::presentation::ConnectionFailure::Unreachable,
            )));
        }
        if self
            .pending
            .iter()
            .any(|p| matches!(p.purpose, Purpose::Room(_, c, _, _) if c == context))
        {
            return Err(SubmitError::Busy);
        }
        if !read
            && self.snapshot.conversation_panel.as_ref().is_some_and(|p| {
                p.context == context && matches!(p.state, view::RemoteState::Uncertain(_))
            })
        {
            return Err(SubmitError::Busy);
        }
        let operation = self.operation(context)?;
        let invited = if let view::ConversationAction::Invite(person) = &action {
            Some(*person)
        } else {
            None
        };
        let command = command(context, action)?;
        self.request(
            context.account,
            ProtocolRequest::ConversationSettings {
                conversation: context.conversation,
                operation,
                command,
            },
            Purpose::Room(request, context, operation, work),
        )?;
        if let Some(account) = self.background.account_mut(context.account) {
            account
                .metadata
                .admitted(context, operation, work.details());
        }
        if let Some(person) = invited {
            self.room_invite = Some((request, person));
        }
        let panel = self
            .snapshot
            .conversation_panel
            .get_or_insert_with(|| view::ConversationPanel::new(context));
        begin_room(panel, request, work);
        self.touch();
        Ok(())
    }
    pub(super) fn complete_room(
        &mut self,
        request: RequestId,
        context: ConversationRef,
        operation: OperationId,
        work: Work,
        reply: Result<RuntimeReply, IdentityError>,
    ) {
        let current = self
            .background
            .account_mut(context.account)
            .is_some_and(|a| a.metadata.complete(context, operation));
        if !current
            || self.snapshot.selected != Some(context)
            || self.current(context.account).is_err()
        {
            return;
        }
        let result = match reply {
            Ok(RuntimeReply::Protocol(ProtocolReply::ConversationSettings {
                conversation,
                operation: returned,
                result,
            })) if conversation == context.conversation && returned == operation => Ok(result),
            Err(error) => Err(completion_error(error)),
            _ => Err(Completion::Failed(UnavailableReason::PolicyStale)),
        };
        self.apply_room_result(request, context, work, result);
    }
    /// Projects an already scoped operation result, including explicit read
    /// completions consumed by the invitation preview.
    fn apply_room_result(
        &mut self,
        request: RequestId,
        context: ConversationRef,
        work: Work,
        result: Result<wire::ConversationResult, Completion>,
    ) {
        let read = work.read();
        let Some(panel) = self
            .snapshot
            .conversation_panel
            .as_mut()
            .filter(|p| p.context == context)
        else {
            return;
        };
        let quiet = matches!(work, Work::ReadKnown | Work::Revalidate | Work::MemberCount);
        let previous_state = panel.state;
        if !quiet
            && !matches!(panel.state, view::RemoteState::Loading(r) | view::RemoteState::Saving(r) if r == request)
        {
            return;
        }
        let result = match result {
            Ok(result) => match (&work, &result) {
                (Work::MemberCount, wire::ConversationResult::Members(_))
                | (Work::Bots, wire::ConversationResult::Bots(_))
                | (Work::BotChange { .. }, wire::ConversationResult::Changed)
                | (Work::Directory, wire::ConversationResult::Directory(_))
                | (Work::Csv, wire::ConversationResult::EmailCsvPreview { .. }) => Ok(result),
                (
                    Work::MemberCount
                    | Work::Bots
                    | Work::BotChange { .. }
                    | Work::Directory
                    | Work::Csv,
                    _,
                ) => Err(Completion::Failed(UnavailableReason::PolicyStale)),
                _ => Ok(result),
            },
            error => error,
        };
        let mut reload = false;
        let mut invalidate_history = None;
        let mut reload_messages = false;
        let mut load_members = false;
        let mut observed_count = None;
        let mut invalidate_cache = false;
        let mut observed_members = None;
        let mut observed_details = None;
        let completion = match result {
            Ok(result) => {
                match result {
                    wire::ConversationResult::Details(details) => {
                        if !details.controls.as_slice().iter().any(|(setting, value)| {
                            *setting == wire::Setting::Members
                                && value.implemented
                                && value.server == protocol::FeatureSupport::Supported
                                && value.allowed == Decision::Allowed
                        }) {
                            panel.members = Page::empty();
                            panel.members_loaded = false;
                            panel.directory = Page::empty();
                            panel.directory_loaded = false;
                        }
                        if !details.controls.as_slice().iter().any(|(setting, value)| {
                            *setting == wire::Setting::Bots
                                && value.implemented
                                && value.server == protocol::FeatureSupport::Supported
                                && value.allowed == Decision::Allowed
                        }) {
                            panel.bots = Page::empty();
                            panel.bots_loaded = false;
                            panel.bots_state = view::RemoteState::Idle;
                        }
                        invalidate_cache = details.conversation.actions.read_messages
                            != Decision::Allowed
                            || details.conversation.classified
                            || details.conversation.sensitive
                            || details.conversation.unknown_restrictions
                            || details.conversation.organization.classified == Some(true);
                        if details.conversation.actions.read_messages != Decision::Allowed {
                            invalidate_history = Some(
                                if details.conversation.actions.read_messages == Decision::Denied {
                                    ContentState::Denied
                                } else {
                                    ContentState::Failed
                                },
                            );
                        } else if self.snapshot.content == ContentState::Denied {
                            reload_messages = true;
                        }
                        let mut row = conversation(&details.conversation, context.account);
                        if let Some(count) = details
                            .participant_count
                            .and_then(|count| u16::try_from(count).ok())
                        {
                            self.background
                                .remember_member_count(context, count, Instant::now());
                            observed_count = Some(count);
                        }
                        row.participants = details
                            .participant_count
                            .and_then(|count| u16::try_from(count).ok())
                            .or_else(|| self.background.member_count(context))
                            .or_else(|| {
                                self.snapshot
                                    .conversations
                                    .as_slice()
                                    .iter()
                                    .find(|existing| existing.context == context)
                                    .and_then(|existing| existing.participants)
                            });
                        self.snapshot.conversations =
                            page(self.snapshot.conversations.as_slice().iter().map(|r| {
                                if r.context == context {
                                    row.clone()
                                } else {
                                    r.clone()
                                }
                            }));
                        load_members = details.participant_count.is_none()
                            && self
                                .background
                                .members_need_refresh(context, Instant::now())
                            && !matches!(
                                row.kind,
                                ConversationKind::Direct | ConversationKind::NoteToSelf
                            )
                            && details.controls.as_slice().iter().any(|(key, value)| {
                                *key == wire::Setting::Members
                                    && value.implemented
                                    && value.allowed == Decision::Allowed
                            });
                        let details = details_view(*details);
                        observed_details = Some((details.clone(), !invalidate_cache));
                        panel.details = Some(details);
                        panel.details_stale = false;
                    }
                    wire::ConversationResult::Members(people) => {
                        panel.members = page(people.as_slice().iter().map(|m| view::Member {
                            id: person(context, m.id.local),
                            name: m.name.clone(),
                            role: role_view(m.role),
                            source: source_view(m.source),
                            permissions: m.permissions,
                            self_user: m.is_self,
                        }));
                        panel.members_loaded = true;
                        observed_members = Some(people);
                        // This endpoint is authoritative and unpaged; the adapter rejects overflow.
                        let count = u16::try_from(panel.members.as_slice().len()).ok();
                        if let Some(count) = count {
                            self.background
                                .remember_member_count(context, count, Instant::now());
                            observed_count = Some(count);
                        }
                        self.snapshot.conversations =
                            page(self.snapshot.conversations.as_slice().iter().map(|row| {
                                let mut row = row.clone();
                                if row.context == context {
                                    row.participants = count;
                                }
                                row
                            }));
                    }
                    wire::ConversationResult::Directory(people) => {
                        panel.directory = recipients(context, people);
                        panel.directory_loaded = true;
                    }
                    wire::ConversationResult::EmailCsvPreview {
                        recipients: people,
                        duplicate_count,
                        invalid_count,
                    } => {
                        panel.directory = recipients(context, people);
                        panel.directory_loaded = true;
                        panel.csv_duplicates = duplicate_count;
                        panel.csv_invalid = invalid_count;
                    }
                    wire::ConversationResult::Bots(bots) => {
                        panel.bots = page(bots.as_slice().iter().map(|b| view::Bot {
                            id: person(context, b.id.local),
                            name: b.name.clone(),
                            description: b.description.clone(),
                            enabled: b.enabled,
                        }));
                        panel.bots_loaded = true;
                    }
                    wire::ConversationResult::Bans(bans) => {
                        panel.bans = page(bans.as_slice().iter().map(|b| view::Ban {
                            id: person(context, b.id.local),
                            name: b.name.clone(),
                            note: b.note.clone(),
                        }));
                        panel.bans_loaded = true;
                    }
                    wire::ConversationResult::Deleted | wire::ConversationResult::Left => {
                        invalidate_history = Some(ContentState::Denied);
                        self.load_rooms = Some(context.account);
                        self.snapshot.messages = Page::empty();
                        self.snapshot.files = Page::empty();
                        self.snapshot.participants = Page::empty();
                        self.snapshot.content = ContentState::Denied;
                        panel.members = Page::empty();
                        panel.directory = Page::empty();
                        panel.bots = Page::empty();
                        panel.bans = Page::empty();
                    }
                    wire::ConversationResult::HistoryCleared => {
                        invalidate_history = Some(ContentState::Loading);
                        reload_messages = true;
                        self.snapshot.messages = Page::empty();
                        reload = true;
                    }
                    wire::ConversationResult::Changed => {
                        reload = true;
                        panel.bans_loaded = false;
                    }
                    wire::ConversationResult::BreakoutRooms(_) => (),
                    wire::ConversationResult::GuestLink(link) => panel.guest_link = Some(link),
                }
                Completion::ServerAccepted
            }
            Err(error) => error,
        };
        let preserve_failure = match previous_state {
            view::RemoteState::Uncertain(_) => {
                !(matches!(work, Work::ReadKnown) && completion == Completion::ServerAccepted)
            }
            view::RemoteState::Failed(request, _) => panel.result.is_some_and(|result| {
                result.request == request
                    && matches!(
                        result.completion,
                        Completion::Failed(_) | Completion::Uncertain
                    )
            }),
            _ => false,
        };
        let transient_background_failure = matches!(work, Work::Revalidate | Work::MemberCount)
            && matches!(
                completion,
                Completion::Failed(UnavailableReason::Connection(_) | UnavailableReason::Busy)
            );
        panel.state = if quiet
            && (preserve_failure || transient_background_failure)
            && completion != Completion::Failed(UnavailableReason::PermissionDenied)
        {
            previous_state
        } else {
            match completion {
                Completion::ServerAccepted | Completion::Queued => view::RemoteState::Idle,
                Completion::Failed(reason) => view::RemoteState::Failed(request, reason),
                Completion::Uncertain => view::RemoteState::Uncertain(request),
            }
        };
        complete_bots(panel, work, completion);
        // Denial invalidates confidential membership data, including directory candidates.
        if completion == Completion::Failed(UnavailableReason::PermissionDenied) {
            if let Some(details) = panel.details.as_mut() {
                details.controls = page(details.controls.as_slice().iter().map(|(key, value)| {
                    (
                        *key,
                        view::Availability {
                            allowed: Decision::Denied,
                            ..*value
                        },
                    )
                }));
                details.leave = Decision::Denied;
            }
            panel.members = Page::empty();
            panel.members_loaded = false;
            self.background.forget_member_count(context);
            self.snapshot.conversations =
                page(self.snapshot.conversations.as_slice().iter().map(|row| {
                    let mut row = row.clone();
                    if row.context == context {
                        row.participants = None;
                    }
                    row
                }));
            panel.directory = Page::empty();
            panel.directory_loaded = false;
            panel.bots = Page::empty();
            panel.bots_loaded = false;
            panel.bots_state =
                view::RemoteState::Failed(request, UnavailableReason::PermissionDenied);
            panel.bans = Page::empty();
        }
        if !read
            && let Some((invitation, person)) = self.room_invite
            && invitation == request
            && person.context == context
        {
            if completion == Completion::ServerAccepted {
                panel.directory = page(
                    panel
                        .directory
                        .as_slice()
                        .iter()
                        .filter(|p| p.id != person)
                        .cloned(),
                );
            }
            self.room_invite = None;
        }
        // Directory and CSV reads have an explicit preview consumer. Publish
        // their completion without treating ordinary background reads as writes.
        if !read || work.preview() {
            panel.result = Some(view::RoomResult {
                request,
                completion,
            });
        }
        if let Some((details, ordinary)) = observed_details {
            if let Some(account) = self.background.account_mut(context.account) {
                account
                    .metadata
                    .remember(context, &details, ordinary, Instant::now());
            }
            if ordinary {
                self.remember_cached_room_details(context, &details);
            }
        }
        if let Some(members) = observed_members.as_ref() {
            self.observe_member_avatar_identities(context, members);
        }
        if completion == Completion::Failed(UnavailableReason::PermissionDenied) {
            self.forget_cached_member_count(context);
            self.revoke_room_metadata(context, UnavailableReason::PermissionDenied);
        }
        if invalidate_cache {
            self.invalidate_cached_room(context);
        }
        if let Some(count) = observed_count {
            self.remember_cached_member_count(context, count);
        }
        if let Some(state) = invalidate_history {
            self.invalidate_history(context, state);
        }
        if reload_messages {
            let _ = self.start_messages(context);
        }
        if reload {
            let _ = self.room_action(request, context, view::ConversationAction::Read);
        }
        if load_members {
            self.background
                .member_count_requested(context, Instant::now());
            let _ =
                self.room_action_inner(request, context, view::ConversationAction::Members, true);
        }
        self.touch();
    }
}
fn begin_room(panel: &mut view::ConversationPanel, request: RequestId, work: Work) {
    // A background refresh must not consume an unseen preview/write result.
    // Explicit replacement actions and acknowledgement own its retirement.
    if !work.read() || work.preview() {
        panel.result = None;
    }
    if matches!(work, Work::ReadKnown | Work::Revalidate | Work::MemberCount) {
        return;
    }
    panel.state = if work.read() {
        view::RemoteState::Loading(request)
    } else {
        view::RemoteState::Saving(request)
    };
    if work.bots() {
        panel.bots_state = panel.state;
    }
}
fn complete_bots(panel: &mut view::ConversationPanel, work: Work, completion: Completion) {
    if !work.bots() {
        return;
    }
    panel.bots_state = panel.state;
    if completion == Completion::ServerAccepted
        && let Work::BotChange { bot, enabled } = work
    {
        panel.bots = page(panel.bots.as_slice().iter().cloned().map(|mut row| {
            if row.id == bot {
                row.enabled = Some(enabled);
            }
            row
        }));
    }
}
fn bots_online(snapshot: &PresentationSnapshot, context: ConversationRef) -> bool {
    snapshot.selected == Some(context)
        && snapshot.content != ContentState::Offline
        && snapshot.conversation_list.content != ContentState::Offline
        && snapshot.accounts.as_slice().iter().any(|account| {
            account.scope == context.account
                && account.connection != AccountConnectionState::Disconnected
                && account.connection_failure.is_none()
        })
}
fn recipients(
    context: ConversationRef,
    people: Page<wire::Recipient, 100>,
) -> Page<view::Recipient, 100> {
    page(people.as_slice().iter().map(|p| view::Recipient {
        id: person(context, p.id.local),
        name: p.name.clone(),
        label: p.label.clone(),
        source: source_view(p.source),
    }))
}
fn person(context: ConversationRef, local: u128) -> view::PersonRef {
    view::PersonRef { context, local }
}
fn member(context: ConversationRef, id: view::PersonRef) -> Result<wire::MemberId, SubmitError> {
    if context != id.context {
        return Err(SubmitError::StaleScope);
    }
    Ok(wire::MemberId {
        conversation: context.conversation,
        local: id.local,
    })
}
fn integration(
    context: ConversationRef,
    id: view::PersonRef,
) -> Result<wire::IntegrationId, SubmitError> {
    member(context, id).map(|id| wire::IntegrationId {
        conversation: id.conversation,
        local: id.local,
    })
}
fn command(
    context: ConversationRef,
    action: view::ConversationAction,
) -> Result<wire::ConversationCommand, SubmitError> {
    use view::ConversationAction as V;
    use wire::ConversationCommand as W;
    Ok(match action {
        V::PickCsv { .. } | V::CancelCsvPick { .. } => return Err(SubmitError::StaleScope),
        V::Read => W::Read,
        V::SetName(v) => W::SetName(v),
        V::SetDescription(v) => W::SetDescription(v),
        V::SetEmojiAvatar(emoji) => W::SetEmojiAvatar { emoji, color: None },
        V::RemoveAvatar => W::RemoveAvatar,
        V::SetReadOnly(v) => W::SetReadOnly(v),
        V::SetNotifications(v) => W::SetNotifications(match v {
            view::NotificationLevel::Default => wire::NotificationLevel::Default,
            view::NotificationLevel::Always => wire::NotificationLevel::Always,
            view::NotificationLevel::Mention => wire::NotificationLevel::Mention,
            view::NotificationLevel::Never => wire::NotificationLevel::Never,
        }),
        V::SetCallNotifications(v) => W::SetCallNotifications(v),
        V::SetImportant(v) => W::SetImportant(v),
        V::SetSensitive(v) => W::SetSensitive(v),
        V::SetExpiration(v) => W::SetExpiration(v),
        V::GuestLink => W::GuestLink,
        V::SetGuestAccess(enabled) => W::SetGuestAccess {
            enabled,
            password: text(""),
        },
        V::SetPassword(v) => W::SetPassword(v),
        V::SetDefaultPermissions(v) => W::SetDefaultPermissions(v),
        V::SetLobby(enabled) => W::SetLobby {
            enabled,
            timer: None,
        },
        V::Delete => W::Delete,
        V::ClearHistory => W::ClearHistory,
        V::Leave => W::Leave,
        V::Members => W::Members,
        V::Directory { search, source } => W::Directory {
            search,
            source: source_wire(source),
        },
        V::Invite(id) => {
            member(context, id)?;
            W::Invite(wire::RecipientId {
                scope: context.account,
                local: id.local,
            })
        }
        V::RemoveMember(id) => W::RemoveMember(member(context, id)?),
        V::SetRole { member: id, role } => W::SetRole {
            member: member(context, id)?,
            role: role_wire(role),
        },
        V::Bots => W::Bots,
        V::SetBotEnabled { bot, enabled } => W::SetBotEnabled {
            bot: integration(context, bot)?,
            enabled,
        },
        V::Bans => W::Bans,
        V::Unban(id) => W::Unban(integration(context, id)?),
        V::PreviewCsv(csv) => W::PreviewEmailCsv(csv),
        V::ConfigureBreakouts { amount, automatic } => W::ConfigureBreakouts { amount, automatic },
    })
}
pub(super) fn availability(a: wire::ActionAvailability) -> view::Availability {
    view::Availability {
        implemented: a.implemented,
        server: support(a.server),
        allowed: a.allowed,
    }
}
fn details_view(d: wire::ConversationDetails) -> view::RoomDetails {
    view::RoomDetails {
        name: d.conversation.name,
        description: d.description,
        read_only: d.read_only,
        notifications: d.notifications.map(|v| match v {
            wire::NotificationLevel::Default => view::NotificationLevel::Default,
            wire::NotificationLevel::Always => view::NotificationLevel::Always,
            wire::NotificationLevel::Mention => view::NotificationLevel::Mention,
            wire::NotificationLevel::Never => view::NotificationLevel::Never,
        }),
        call_notifications: d.call_notifications,
        important: d.important,
        sensitive: d.conversation.sensitive,
        classified: d.conversation.classified,
        preserved: d.preserved,
        message_expiration: d.message_expiration,
        default_permissions: d.default_permissions,
        server_default_permissions: d.server_default_permissions,
        effective_permissions: d.effective_permissions,
        lobby: d.lobby,
        has_password: d.has_password,
        participant_count: d.participant_count,
        guest_access: Some(d.conversation.kind == RoomKind::Public),
        leave: d.conversation.actions.leave_conversation,
        controls: page(
            d.controls
                .as_slice()
                .iter()
                .map(|(key, a)| (setting(*key), availability(*a))),
        ),
    }
}
fn setting(s: wire::Setting) -> view::RoomSetting {
    use view::RoomSetting as V;
    use wire::Setting as W;
    match s {
        W::Metadata => V::Metadata,
        W::Avatar => V::Avatar,
        W::ReadOnly => V::ReadOnly,
        W::Notifications => V::Notifications,
        W::Important => V::Important,
        W::Sensitive => V::Sensitive,
        W::Expiration => V::Expiration,
        W::Preserve => V::Preserve,
        W::GuestAccess => V::GuestAccess,
        W::Password => V::Password,
        W::Discoverability => V::Discoverability,
        W::Mentions => V::Mentions,
        W::Permissions => V::Permissions,
        W::Lobby => V::Lobby,
        W::RecordingConsent => V::RecordingConsent,
        W::Delete => V::Delete,
        W::ClearHistory => V::ClearHistory,
        W::Keep => V::Keep,
        W::Members => V::Members,
        W::Invite => V::Invite,
        W::CsvImport => V::CsvImport,
        W::Roles => V::Roles,
        W::Bots => V::Bots,
        W::Bans => V::Bans,
        W::Breakouts => V::Breakouts,
    }
}
fn source_view(s: wire::RecipientSource) -> view::DirectorySource {
    match s {
        wire::RecipientSource::User => view::DirectorySource::User,
        wire::RecipientSource::Group => view::DirectorySource::Group,
        wire::RecipientSource::Team => view::DirectorySource::Team,
        wire::RecipientSource::Email => view::DirectorySource::Email,
        wire::RecipientSource::Federated => view::DirectorySource::Federated,
        wire::RecipientSource::Phone => view::DirectorySource::Phone,
    }
}
fn source_wire(s: view::DirectorySource) -> wire::RecipientSource {
    match s {
        view::DirectorySource::User => wire::RecipientSource::User,
        view::DirectorySource::Group => wire::RecipientSource::Group,
        view::DirectorySource::Team => wire::RecipientSource::Team,
        view::DirectorySource::Email => wire::RecipientSource::Email,
        view::DirectorySource::Federated => wire::RecipientSource::Federated,
        view::DirectorySource::Phone => wire::RecipientSource::Phone,
    }
}
fn role_view(r: protocol::Role) -> view::MemberRole {
    match r {
        protocol::Role::Owner => view::MemberRole::Owner,
        protocol::Role::Moderator => view::MemberRole::Moderator,
        protocol::Role::Member => view::MemberRole::Member,
        protocol::Role::Guest => view::MemberRole::Guest,
        protocol::Role::Following => view::MemberRole::Following,
        protocol::Role::GuestModerator => view::MemberRole::GuestModerator,
        protocol::Role::Unknown => view::MemberRole::Unknown,
    }
}
fn role_wire(r: view::MemberRole) -> protocol::Role {
    match r {
        view::MemberRole::Owner => protocol::Role::Owner,
        view::MemberRole::Moderator => protocol::Role::Moderator,
        view::MemberRole::Member => protocol::Role::Member,
        view::MemberRole::Guest => protocol::Role::Guest,
        view::MemberRole::Following => protocol::Role::Following,
        view::MemberRole::GuestModerator => protocol::Role::GuestModerator,
        view::MemberRole::Unknown => protocol::Role::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_and_csv_reads_publish_correlated_preview_completions() {
        for csv in [false, true] {
            let mut model = super::super::tests::model();
            model.snapshot = super::super::super::fixtures::snapshot(Some("chat"));
            let context = model.snapshot.selected.unwrap();
            let request = RequestId(27);
            let mut panel = view::ConversationPanel::new(context);
            panel.state = view::RemoteState::Loading(request);
            model.snapshot.conversation_panel = Some(panel);
            let action = if csv {
                view::ConversationAction::PreviewCsv(text("invitee@example.test"))
            } else {
                view::ConversationAction::Directory {
                    search: text("Synthetic"),
                    source: view::DirectorySource::User,
                }
            };
            let work = Work::for_action(&action);
            let people = page([wire::Recipient {
                id: wire::RecipientId {
                    scope: context.account,
                    local: 5,
                },
                name: text("Synthetic invitee"),
                label: text("example.test"),
                source: if csv {
                    wire::RecipientSource::Email
                } else {
                    wire::RecipientSource::User
                },
            }]);
            let result = if csv {
                wire::ConversationResult::EmailCsvPreview {
                    recipients: people,
                    duplicate_count: 1,
                    invalid_count: 2,
                }
            } else {
                wire::ConversationResult::Directory(people)
            };
            model.apply_room_result(request, context, work, Ok(result));
            let panel = model.snapshot.conversation_panel.as_ref().unwrap();
            assert!(panel.directory_loaded);
            assert_eq!(
                panel.directory.as_slice()[0].name.as_str(),
                "Synthetic invitee"
            );
            assert_eq!(panel.state, view::RemoteState::Idle);
            assert_eq!(
                panel.result,
                Some(view::RoomResult {
                    request,
                    completion: Completion::ServerAccepted
                })
            );
            assert_eq!(panel.context, context);
            if csv {
                assert_eq!((panel.csv_duplicates, panel.csv_invalid), (1, 2));
            }

            let panel = model.snapshot.conversation_panel.as_mut().unwrap();
            begin_room(panel, RequestId(28), Work::Read);
            assert_eq!(
                panel.result.as_ref().unwrap().request,
                request,
                "background admission cannot hide an unobserved preview"
            );
            begin_room(panel, RequestId(29), work);
            assert!(
                panel.result.is_none(),
                "an explicit replacement preview retires its predecessor"
            );
            let before = panel.clone();
            model.apply_room_result(
                request,
                context,
                work,
                Ok(wire::ConversationResult::Directory(Page::empty())),
            );
            assert_eq!(
                model.snapshot.conversation_panel.as_ref().unwrap(),
                &before,
                "an old preview cannot complete the next request"
            );
            model.apply_room_result(
                RequestId(29),
                context,
                work,
                Err(Completion::Failed(UnavailableReason::Unsupported)),
            );
            assert_eq!(
                model.snapshot.conversation_panel.as_ref().unwrap().result,
                Some(view::RoomResult {
                    request: RequestId(29),
                    completion: Completion::Failed(UnavailableReason::Unsupported)
                })
            );
        }
    }

    #[test]
    fn bot_actions_require_current_online_context_before_queue_admission() {
        let mut model = super::super::tests::model();
        model.snapshot = super::super::super::fixtures::snapshot(Some("chat"));
        let context = model.snapshot.selected.unwrap();
        model.snapshot.accounts = page([AccountView {
            scope: context.account,
            navigation_key: None,
            connection: AccountConnectionState::Connected,
            connection_failure: None,
            label: text("Synthetic account"),
            server_label: text("example.test"),
            self_name: text("Synthetic user"),
            initials: text("SY"),
            presence: Presence::Unknown,
            storage: StorageStatus::SessionOnly,
        }]);
        assert!(bots_online(&model.snapshot, context));
        for failure in 0..4 {
            let saved = model.snapshot.clone();
            match failure {
                0 => model.snapshot.content = ContentState::Offline,
                1 => model.snapshot.conversation_list.content = ContentState::Offline,
                2 => {
                    model.snapshot.accounts = page(saved.accounts.as_slice().iter().cloned().map(
                        |mut account| {
                            account.connection = AccountConnectionState::Disconnected;
                            account
                        },
                    ));
                }
                _ => model.snapshot.accounts = Page::empty(),
            }
            assert!(!bots_online(&model.snapshot, context));
            for action in [
                view::ConversationAction::Bots,
                view::ConversationAction::SetBotEnabled {
                    bot: person(context, 9),
                    enabled: true,
                },
            ] {
                assert_eq!(
                    model.room_action(RequestId(1), context, action),
                    Err(SubmitError::Unavailable(UnavailableReason::Connection(
                        ConnectionFailure::Unreachable
                    )))
                );
                assert!(model.pending.is_empty());
                assert!(model.snapshot.conversation_panel.is_none());
            }
            model.snapshot = saved;
        }
        let other = super::super::super::fixtures::context(2);
        assert!(!bots_online(&model.snapshot, other));
        assert_eq!(
            model.room_action(RequestId(2), other, view::ConversationAction::Bots),
            Err(SubmitError::StaleScope)
        );
    }

    #[test]
    fn bot_completion_preserves_loaded_list_and_changes_only_confirmed_target() {
        let context = super::super::super::fixtures::context(1);
        let mut panel = view::ConversationPanel::new(context);
        panel.bots_loaded = true;
        panel.bots = page([1, 2].map(|id| view::Bot {
            id: person(context, id),
            name: text("Synthetic bot"),
            description: text(""),
            enabled: Some(false),
        }));
        let work = Work::BotChange {
            bot: person(context, 2),
            enabled: true,
        };
        let reason = UnavailableReason::Connection(ConnectionFailure::Unreachable);
        let failed = Completion::Failed(reason);
        panel.state = view::RemoteState::Failed(RequestId(1), reason);
        complete_bots(&mut panel, work, failed);
        assert_eq!(panel.bots_state, panel.state);
        assert!(panel.bots_loaded);
        assert!(
            panel
                .bots
                .as_slice()
                .iter()
                .all(|bot| bot.enabled == Some(false))
        );
        panel.state = view::RemoteState::Idle;
        complete_bots(&mut panel, work, Completion::ServerAccepted);
        assert_eq!(panel.bots.as_slice()[0].enabled, Some(false));
        assert_eq!(panel.bots.as_slice()[1].enabled, Some(true));
        assert!(panel.bots_loaded);
        let retained = panel.bots.clone();
        panel.bots_state = view::RemoteState::Loading(RequestId(2));
        complete_bots(&mut panel, Work::Read, Completion::ServerAccepted);
        assert_eq!(panel.bots_state, view::RemoteState::Loading(RequestId(2)));
        assert_eq!(panel.bots, retained);
    }
}
