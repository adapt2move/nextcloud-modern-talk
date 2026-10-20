//! One bounded account observation schedule. Push hints and timers share these
//! slots; neither creates another network owner or an unbounded event queue.
use super::*;

const ACCOUNTS: usize = 8;
const IN_FLIGHT: usize = 4;
const MIN_INTERVAL: Duration = Duration::from_secs(2);
const LIVE_FALLBACK_INTERVAL: Duration = Duration::from_secs(30);
pub(super) const SOURCE_LIFETIME: Duration = Duration::from_secs(12);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Observation {
    Calls,
    Messages,
    Rooms,
    LiveEvents,
}
impl Observation {
    const ALL: [Self; 4] = [Self::Calls, Self::Messages, Self::Rooms, Self::LiveEvents];

    fn index(self) -> usize {
        match self {
            Self::Calls => 0,
            Self::Messages => 1,
            Self::Rooms => 2,
            Self::LiveEvents => 3,
        }
    }
    fn purpose(self, scope: AccountScope) -> Purpose {
        match self {
            Self::Calls => Purpose::IncomingCalls(scope),
            Self::Messages => Purpose::MessageNotifications(scope),
            Self::Rooms => Purpose::Rooms(scope),
            Self::LiveEvents => Purpose::LiveEvents(scope),
        }
    }
    fn request(self) -> ProtocolRequest {
        match self {
            Self::Calls => ProtocolRequest::PollIncomingCalls,
            Self::Messages => ProtocolRequest::PollMessageNotifications,
            Self::Rooms => ProtocolRequest::ListConversations,
            Self::LiveEvents => ProtocolRequest::PollLiveEvents,
        }
    }
    fn change(change: protocol::AccountChange) -> Self {
        match change {
            protocol::AccountChange::Messages => Self::Messages,
            protocol::AccountChange::Conversations => Self::Rooms,
            protocol::AccountChange::IncomingCalls => Self::Calls,
        }
    }
}
pub(super) struct Account {
    scope: AccountScope,
    due: [Instant; 4],
    last: [Option<Instant>; 4],
    sources: [Option<Instant>; 2],
    live: Option<Result<protocol::AccountLiveState, IdentityError>>,
    first_transport_failure: Option<ErrorKind>,
    pub(super) rooms: Page<ConversationView, 100>,
    pub(super) state: ConversationListState,
}
#[derive(Default)]
pub(super) struct Background {
    accounts: Vec<Account>,
}
impl Background {
    pub(super) fn sync(&mut self, scopes: &[AccountScope], now: Instant) {
        self.accounts.retain(|a| scopes.contains(&a.scope));
        for &scope in scopes.iter().take(ACCOUNTS) {
            if !self.accounts.iter().any(|a| a.scope == scope) {
                self.accounts.push(Account {
                    scope,
                    due: [now; 4],
                    last: [None; 4],
                    sources: [None; 2],
                    live: None,
                    first_transport_failure: None,
                    rooms: Page::empty(),
                    state: ConversationListState {
                        content: ContentState::Loading,
                        complete: false,
                    },
                });
            }
        }
    }
    pub(super) fn account(&self, scope: AccountScope) -> Option<&Account> {
        self.accounts.iter().find(|a| a.scope == scope)
    }
    pub(super) fn account_mut(&mut self, scope: AccountScope) -> Option<&mut Account> {
        self.accounts.iter_mut().find(|a| a.scope == scope)
    }
    pub(super) fn invalidate(&mut self, scope: AccountScope, kind: Observation, now: Instant) {
        if let Some(a) = self.account_mut(scope) {
            let i = kind.index();
            let earliest = a.last[i].map_or(now, |at| (at + MIN_INTERVAL).max(now));
            a.due[i] = a.due[i].min(earliest);
        }
    }
    pub(super) fn source(&self, scope: AccountScope, kind: Observation, now: Instant) -> bool {
        self.account(scope)
            .and_then(|a| a.sources.get(kind.index()).copied().flatten())
            .is_some_and(|at| now.saturating_duration_since(at) < SOURCE_LIFETIME)
    }
    pub(super) fn observed(&mut self, scope: AccountScope, kind: Observation, success: bool) {
        if let Some(a) = self.account_mut(scope)
            && let Some(source) = a.sources.get_mut(kind.index())
        {
            *source = success.then(Instant::now);
        }
    }
    fn next(
        &self,
        active: Option<AccountScope>,
        busy: &[AccountScope],
        calls: bool,
        now: Instant,
    ) -> Option<(AccountScope, Observation)> {
        // Oldest due first: active account wins ties and has a shorter room period,
        // but repeated active-account invalidations cannot starve older due work.
        self.accounts
            .iter()
            .filter(|a| !busy.contains(&a.scope))
            .flat_map(|a| {
                Observation::ALL
                    .into_iter()
                    .filter(move |kind| calls || *kind != Observation::Calls)
                    .map(move |kind| (a, kind))
            })
            .filter(|(a, kind)| a.due[kind.index()] <= now)
            .min_by_key(|(a, kind)| (a.due[kind.index()], Some(a.scope) != active, kind.index()))
            .map(|(a, kind)| (a.scope, kind))
    }
    fn admitted(&mut self, scope: AccountScope, kind: Observation, active: bool, now: Instant) {
        if let Some(a) = self.account_mut(scope) {
            let i = kind.index();
            a.last[i] = Some(now);
            a.due[i] = now
                + match kind {
                    Observation::Calls | Observation::Messages => Duration::from_secs(5),
                    Observation::Rooms => Duration::from_secs(if active { 15 } else { 30 }),
                    Observation::LiveEvents => live_interval(a.live),
                };
        }
    }
}

fn live_interval(state: Option<Result<protocol::AccountLiveState, IdentityError>>) -> Duration {
    // Missing push support only slows mailbox drains; HTTP observations keep
    // their own periodic deadlines and remain the only source of authority.
    match state {
        Some(Ok(protocol::AccountLiveState::Internal | protocol::AccountLiveState::Stopped(_)))
        | Some(Err(_)) => LIVE_FALLBACK_INTERVAL,
        _ => MIN_INTERVAL,
    }
}

impl Purpose {
    fn observation(self) -> Option<(AccountScope, Observation)> {
        match self {
            Self::IncomingCalls(s) => Some((s, Observation::Calls)),
            Self::MessageNotifications(s) => Some((s, Observation::Messages)),
            Self::Rooms(s) => Some((s, Observation::Rooms)),
            Self::LiveEvents(s) => Some((s, Observation::LiveEvents)),
            _ => None,
        }
    }
    pub(super) fn room(self) -> Option<ConversationRef> {
        match self {
            Self::Room(_, c, _, _)
            | Self::Message(_, c, _, _)
            | Self::Mutation(_, c)
            | Self::Send(_, c, _) => Some(c),
            Self::Call(c) => Some(ConversationRef {
                account: c.account,
                conversation: c.conversation,
            }),
            _ => None,
        }
    }
    fn account(self) -> Option<AccountScope> {
        self.room().map(|c| c.account).or(match self {
            Self::Create(_, s, _)
            | Self::IncomingCalls(s)
            | Self::MessageNotifications(s)
            | Self::LiveEvents(s)
            | Self::Rooms(s)
            | Self::Tags(s)
            | Self::TagMutation(_, s)
            | Self::Preferences(s, _) => Some(s),
            _ => None,
        })
    }
}
impl Accounts {
    /// Entry point for account transport hints. It deliberately accepts no wire
    /// JSON, content, media state or authority. Repeated hints occupy the same slot.
    pub fn invalidate_account(&mut self, scope: AccountScope, change: protocol::AccountChange) {
        if self.current(scope).is_err() {
            return;
        }
        self.background
            .invalidate(scope, Observation::change(change), Instant::now());
    }
    pub(super) fn tick_background(&mut self) {
        let now = Instant::now();
        if let Some(scope) = self.load_rooms.take() {
            self.invalidate_account(scope, protocol::AccountChange::Conversations);
        }
        if self.pending.len() >= 12
            || self
                .pending
                .iter()
                .filter(|p| p.purpose.observation().is_some())
                .count()
                >= IN_FLIGHT
        {
            return;
        }
        let busy: Vec<_> = self
            .pending
            .iter()
            .filter_map(|p| p.purpose.account())
            .collect();
        let Some((scope, kind)) =
            self.background
                .next(self.snapshot.active_account, &busy, true, now)
        else {
            return;
        };
        // Schedule attempts too: a rejected admission must not spin every UI tick.
        self.background.admitted(
            scope,
            kind,
            self.snapshot.active_account == Some(scope),
            now,
        );
        let _ = self.request(scope, kind.request(), kind.purpose(scope));
    }
    pub(super) fn sync_background(&mut self) {
        let scopes: Vec<_> = self
            .snapshot
            .accounts
            .as_slice()
            .iter()
            .map(|a| a.scope)
            .collect();
        self.background.sync(&scopes, Instant::now());
        self.history.retain_accounts(&scopes);
        self.notices.retain_accounts(&scopes);
        if let Ok(connection) = &mut self.connection {
            for pending in &self.pending {
                if pending
                    .purpose
                    .account()
                    .is_some_and(|s| !scopes.contains(&s))
                {
                    let _ = connection.cancel_request(pending.id);
                }
            }
        }
    }
    pub(super) fn complete_rooms(
        &mut self,
        scope: AccountScope,
        result: Result<RuntimeReply, IdentityError>,
    ) {
        if self.current(scope).is_err() {
            return;
        }
        self.apply_room_refresh(scope, result);
    }
    pub(super) fn complete_live_events(
        &mut self,
        scope: AccountScope,
        result: Result<RuntimeReply, IdentityError>,
    ) {
        if self.current(scope).is_err() {
            return;
        }
        self.apply_live_events(scope, result, Instant::now());
    }
    pub(super) fn apply_live_events(
        &mut self,
        scope: AccountScope,
        result: Result<RuntimeReply, IdentityError>,
        now: Instant,
    ) {
        let batch = match result {
            Ok(RuntimeReply::Protocol(ProtocolReply::LiveEvents(batch))) => {
                if batch.scope != scope {
                    return;
                }
                Ok(batch)
            }
            Err(IdentityError::Request(ErrorKind::WrongScope | ErrorKind::Cancelled)) => return,
            Err(error) => Err(error),
            _ => Err(IdentityError::Request(ErrorKind::Malformed)),
        };
        let Some(account) = self.background.account_mut(scope) else {
            return;
        };
        let live = Some(batch.as_ref().map(|b| b.state).map_err(|e| *e));
        if account.live != live {
            let (outcome, error) = match live {
                Some(Ok(protocol::AccountLiveState::Internal)) => {
                    (DiagnosticOutcome::Internal, None)
                }
                Some(Ok(protocol::AccountLiveState::Connecting)) => {
                    (DiagnosticOutcome::Loading, None)
                }
                Some(Ok(protocol::AccountLiveState::Connected)) => {
                    (DiagnosticOutcome::Connected, None)
                }
                Some(Ok(protocol::AccountLiveState::Reconnecting)) => (
                    DiagnosticOutcome::Reconnecting,
                    batch.as_ref().ok().and_then(|b| b.first_transport_failure),
                ),
                Some(Ok(protocol::AccountLiveState::Stopped(error))) => {
                    (DiagnosticOutcome::Stopped, Some(error))
                }
                Some(Err(error)) => (DiagnosticOutcome::Failed, Some(diagnostic_error(error))),
                None => (DiagnosticOutcome::Stopped, None),
            };
            diagnostics::record(Diagnostic {
                request: 0,
                operation: DiagnosticOperation::LiveConnection,
                outcome,
                error,
                count: 0,
            });
        }
        account.live = live;
        account.due[Observation::LiveEvents.index()] = now + live_interval(account.live);
        if let Ok(batch) = batch {
            account.first_transport_failure = account
                .first_transport_failure
                .or(batch.first_transport_failure);
            // Invalidations share the HTTP slots and their minimum interval. A
            // reconnect cannot enqueue room polls, replay alerts or change authority.
            for &change in batch.changes.as_slice() {
                self.background
                    .invalidate(scope, Observation::change(change), now);
            }
        }
    }
    // Applying an already admitted observation is separate from submitting I/O,
    // so failure/removal and selection invariants can be tested deterministically.
    pub(super) fn apply_room_refresh(
        &mut self,
        scope: AccountScope,
        result: Result<RuntimeReply, IdentityError>,
    ) {
        let list = match result {
            Ok(RuntimeReply::Protocol(ProtocolReply::Conversations(list)))
                if list.scope == scope && valid_rooms(&list) =>
            {
                Ok(list)
            }
            Err(error) => Err(content_error(error)),
            _ => Err(ContentState::Failed),
        };
        let Some(cache) = self.background.account_mut(scope) else {
            return;
        };
        match list {
            Ok(list) => {
                let rows = page(list.rooms.as_slice().iter().map(|r| conversation(r, scope)));
                let removed: Vec<_> = cache
                    .rooms
                    .as_slice()
                    .iter()
                    .filter(|old| !rows.as_slice().iter().any(|r| r.context == old.context))
                    .map(|r| r.context)
                    .collect();
                cache.rooms = rows.clone();
                cache.state = ConversationListState {
                    content: ContentState::Ready,
                    complete: true,
                };
                let state = cache.state;
                for room in removed {
                    self.cancel_room_work(room);
                }
                // A successful policy observation can revoke retained content
                // even for an inactive room. A failed list has no such authority.
                for room in list
                    .rooms
                    .as_slice()
                    .iter()
                    .filter(|room| room.actions.read_messages != Decision::Allowed)
                {
                    self.invalidate_history(
                        ConversationRef {
                            account: scope,
                            conversation: room.id,
                        },
                        if room.actions.read_messages == Decision::Denied {
                            ContentState::Denied
                        } else {
                            ContentState::Failed
                        },
                    );
                }
                if self.snapshot.active_account == Some(scope) {
                    self.snapshot.conversations = rows;
                    self.snapshot.conversation_list = state;
                    if list
                        .rooms
                        .as_slice()
                        .iter()
                        .any(|r| r.organization.tags == protocol::FeatureSupport::Supported)
                    {
                        self.load_tags = Some(scope);
                    }
                    if let Some(selected) = self.snapshot.selected {
                        if !self
                            .snapshot
                            .conversations
                            .as_slice()
                            .iter()
                            .any(|r| r.context == selected)
                        {
                            self.cancel_room_work(selected);
                            self.clear_content();
                        } else {
                            // Membership/policy may change without a selected-room action.
                            // Refresh settings only; preserve history, drafts and scroll state.
                            let _ = self.room_action(
                                RequestId(0),
                                selected,
                                talk_core::ui::conversation::ConversationAction::Read,
                            );
                        }
                    }
                    if self.snapshot.selected.is_none() {
                        self.snapshot.content = ContentState::Ready;
                    }
                }
            }
            Err(content) => {
                // Failed, oversized, malformed or partial lists have no deletion authority.
                cache.state = ConversationListState {
                    content,
                    complete: false,
                };
                if self.snapshot.active_account == Some(scope) {
                    self.snapshot.conversation_list = cache.state;
                }
            }
        }
        self.touch();
    }
    fn cancel_room_work(&mut self, context: ConversationRef) {
        self.history.remove(context);
        self.notices.remove_room(context);
        if let Ok(connection) = &mut self.connection {
            for pending in &self.pending {
                if pending.purpose.room() == Some(context) {
                    let _ = connection.cancel_request(pending.id);
                }
            }
        }
        if self.snapshot.call.as_ref().is_some_and(|c| {
            c.scope.account == context.account && c.scope.conversation == context.conversation
        }) {
            if let Some(call) = &mut self.snapshot.call {
                self.media.close_call(call.scope);
                call.stage = talk_core::ui::presentation::CallStage::Ended;
                call.accept = Decision::Denied;
                call.microphone = talk_core::ui::presentation::MediaState::Off;
                call.camera = talk_core::ui::presentation::MediaState::Off;
                call.share = talk_core::ui::presentation::MediaState::Off;
            }
            self.call_actions.clear();
            self.call_ui = None;
        }
    }
}
fn valid_rooms(list: &protocol::ConversationList) -> bool {
    list.rooms.as_slice().iter().enumerate().all(|(i, r)| {
        r.id.account == list.scope.account
            && !list.rooms.as_slice()[..i].iter().any(|old| old.id == r.id)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::ids::{HostId, RuntimeGeneration};

    fn scope(n: u128) -> AccountScope {
        AccountScope {
            account: AccountId::from_local(n),
            runtime: RuntimeGeneration::from_local(1),
        }
    }
    #[test]
    fn eight_accounts_get_all_slots_even_with_continuous_active_hints() {
        let now = Instant::now();
        let scopes: Vec<_> = (1..=8).map(scope).collect();
        let mut schedule = Background::default();
        schedule.sync(&scopes, now);
        let active = scopes[7];
        assert_eq!(
            schedule.next(Some(active), &[], true, now),
            Some((active, Observation::Calls))
        );
        let mut seen = Vec::new();
        for i in 0..32 {
            let at = now + Duration::from_millis(i * 10);
            for kind in Observation::ALL {
                schedule.invalidate(active, kind, at);
            }
            let (scope, kind) = schedule.next(Some(active), &[], true, at).unwrap();
            assert!(
                !seen.contains(&(scope, kind)),
                "an initial slot cannot starve"
            );
            seen.push((scope, kind));
            schedule.admitted(scope, kind, scope == active, at);
        }
        assert_eq!(seen.len(), 32);
        for scope in scopes {
            for kind in Observation::ALL {
                assert!(seen.contains(&(scope, kind)));
            }
        }
    }
    #[test]
    fn busy_account_does_not_block_others_and_runtime_replacement_has_no_old_freshness() {
        let now = Instant::now();
        let mut schedule = Background::default();
        schedule.sync(&[scope(1), scope(2)], now);
        assert_eq!(
            schedule.next(Some(scope(1)), &[scope(1)], true, now),
            Some((scope(2), Observation::Calls))
        );
        schedule.observed(scope(1), Observation::Calls, true);
        schedule.observed(scope(2), Observation::Messages, true);
        schedule.observed(scope(2), Observation::Calls, false);
        assert!(schedule.source(scope(1), Observation::Calls, now));
        let replacement = AccountScope {
            runtime: RuntimeGeneration::from_local(2),
            ..scope(1)
        };
        schedule.sync(&[replacement, scope(2)], now);
        assert!(!schedule.source(replacement, Observation::Calls, now));
        assert!(schedule.account(scope(1)).is_none());
        schedule.observed(scope(1), Observation::Calls, true);
        assert!(!schedule.source(replacement, Observation::Calls, now));
        assert!(schedule.source(scope(2), Observation::Messages, now));
        assert!(!schedule.source(
            scope(2),
            Observation::Messages,
            now + SOURCE_LIFETIME + Duration::from_secs(1)
        ));
    }
    fn live(
        scope: AccountScope,
        state: protocol::AccountLiveState,
        changes: &[protocol::AccountChange],
    ) -> Result<RuntimeReply, IdentityError> {
        Ok(RuntimeReply::Protocol(ProtocolReply::LiveEvents(
            protocol::AccountLiveEvents {
                scope,
                state,
                first_transport_failure: Some(ErrorKind::Transport),
                changes: page(changes.iter().copied()),
            },
        )))
    }
    #[test]
    fn reconnect_bursts_only_coalesce_http_due_times_without_authority_or_ui_changes() {
        let now = Instant::now();
        let mut model = super::super::tests::model();
        model.background.sync(&[scope(1)], now);
        for kind in Observation::ALL {
            model.background.admitted(scope(1), kind, true, now);
        }
        let snapshot = model.snapshot.clone();
        model.dirty = false;
        let changes = [
            protocol::AccountChange::IncomingCalls,
            protocol::AccountChange::Messages,
            protocol::AccountChange::Conversations,
        ];
        for _ in 0..100 {
            model.apply_live_events(
                scope(1),
                live(scope(1), protocol::AccountLiveState::Reconnecting, &changes),
                now,
            );
        }
        let account = model.background.account(scope(1)).unwrap();
        assert_eq!(&account.due[..3], &[now + MIN_INTERVAL; 3]);
        assert_eq!(
            account.sources, [None; 2],
            "hints cannot refresh HTTP authority"
        );
        assert_eq!(account.first_transport_failure, Some(ErrorKind::Transport));
        assert_eq!(model.snapshot, snapshot);
        let Ok(RuntimeReply::Protocol(ProtocolReply::LiveEvents(mut recovered))) =
            live(scope(1), protocol::AccountLiveState::Connected, &[])
        else {
            panic!("live fixture");
        };
        recovered.first_transport_failure = Some(ErrorKind::PermissionDenied);
        model.apply_live_events(
            scope(1),
            Ok(RuntimeReply::Protocol(ProtocolReply::LiveEvents(recovered))),
            now,
        );
        assert_eq!(
            model
                .background
                .account(scope(1))
                .unwrap()
                .first_transport_failure,
            Some(ErrorKind::Transport),
            "later drain metadata cannot replace the first failure"
        );
        assert!(model.pending.is_empty());
        assert!(!model.dirty);
        // Servicing an invalidation keeps the minimum interval even through another burst.
        let next = now + MIN_INTERVAL;
        for kind in Observation::ALL {
            model.background.admitted(scope(1), kind, true, next);
        }
        for _ in 0..100 {
            model.apply_live_events(
                scope(1),
                live(scope(1), protocol::AccountLiveState::Connected, &changes),
                next,
            );
        }
        assert_eq!(
            &model.background.account(scope(1)).unwrap().due[..3],
            &[next + MIN_INTERVAL; 3]
        );
        assert_eq!(model.snapshot, snapshot);
    }
    #[test]
    fn internal_stopped_and_failed_drains_keep_http_fallback_and_source_freshness() {
        let now = Instant::now();
        for result in [
            live(scope(1), protocol::AccountLiveState::Internal, &[]),
            live(
                scope(1),
                protocol::AccountLiveState::Stopped(ErrorKind::Transport),
                &[],
            ),
            Err(IdentityError::Request(ErrorKind::ServerUnavailable)),
        ] {
            let mut model = super::super::tests::model();
            model.background.sync(&[scope(1)], now);
            for kind in Observation::ALL {
                model.background.admitted(scope(1), kind, true, now);
            }
            model.background.account_mut(scope(1)).unwrap().sources = [Some(now); 2];
            let due = model.background.account(scope(1)).unwrap().due;
            model.apply_live_events(scope(1), result, now);
            let account = model.background.account(scope(1)).unwrap();
            assert_eq!(&account.due[..3], &due[..3]);
            assert_eq!(account.sources, [Some(now); 2]);
            assert_eq!(
                account.due[Observation::LiveEvents.index()],
                now + LIVE_FALLBACK_INTERVAL
            );
            assert_eq!(
                model
                    .background
                    .next(None, &[], true, now + Duration::from_secs(5)),
                Some((scope(1), Observation::Calls))
            );
        }
    }
    #[test]
    fn wrong_scope_cancelled_or_retired_live_results_cannot_change_the_schedule() {
        let now = Instant::now();
        let mut model = super::super::tests::model();
        model.background.sync(&[scope(1)], now);
        for result in [
            live(
                scope(2),
                protocol::AccountLiveState::Connected,
                &[protocol::AccountChange::Messages],
            ),
            Err(IdentityError::Request(ErrorKind::WrongScope)),
            Err(IdentityError::Request(ErrorKind::Cancelled)),
        ] {
            model.apply_live_events(scope(1), result, now);
            let account = model.background.account(scope(1)).unwrap();
            assert_eq!(account.due, [now; 4]);
            assert!(account.live.is_none());
            assert!(account.first_transport_failure.is_none());
        }
        let replacement = AccountScope {
            runtime: RuntimeGeneration::from_local(2),
            ..scope(1)
        };
        model.background.sync(&[replacement], now);
        model.apply_live_events(
            scope(1),
            live(
                scope(1),
                protocol::AccountLiveState::Reconnecting,
                &[protocol::AccountChange::Messages],
            ),
            now,
        );
        let account = model.background.account(replacement).unwrap();
        assert_eq!(account.due, [now; 4]);
        assert!(account.live.is_none());
        assert!(model.background.account(scope(1)).is_none());
    }
    #[test]
    fn live_drain_shares_observation_caps_and_skips_accounts_with_pending_user_work() {
        let now = Instant::now();
        let mut model = super::super::tests::model();
        model.background.sync(&[scope(1), scope(2)], now);
        for account in &mut model.background.accounts {
            account.due = [now + Duration::from_secs(60); 4];
            account.due[Observation::LiveEvents.index()] = now;
        }
        let context = ConversationRef {
            account: scope(1),
            conversation: room(1).id,
        };
        model.pending.push(Pending {
            id: ConnectionRequestId(1),
            purpose: Purpose::Send(RequestId(1), context, OperationId::from_local(1)),
        });
        model.tick_background();
        assert!(model.background.account(scope(1)).unwrap().last[3].is_none());
        assert!(model.background.account(scope(2)).unwrap().last[3].is_some());
        for (count, purpose) in [
            (IN_FLIGHT, Purpose::LiveEvents(scope(2))),
            (
                12,
                Purpose::Send(RequestId(1), context, OperationId::from_local(1)),
            ),
        ] {
            model.pending = (0..count)
                .map(|id| Pending {
                    id: ConnectionRequestId(id as u64),
                    purpose,
                })
                .collect();
            model.background.account_mut(scope(1)).unwrap().due = [now; 4];
            model.tick_background();
            assert_eq!(model.background.account(scope(1)).unwrap().last, [None; 4]);
        }
    }
    fn room(n: u128) -> protocol::Conversation {
        protocol::Conversation {
            id: talk_core::ids::ConversationId {
                account: scope(1).account,
                host: HostId::from_local(1),
                local: n,
            },
            call_scope: None,
            navigation_key: Some(
                talk_core::preferences::navigation::NavigationRoomKey::new(&format!("r1:{n:032x}"))
                    .unwrap(),
            ),
            favorite: Some(true),
            organization: protocol::ConversationOrganization {
                favorites: protocol::FeatureSupport::Supported,
                favorite_change: Decision::Allowed,
                tags: protocol::FeatureSupport::Supported,
                tag_change: Decision::Allowed,
                classified: Some(false),
                channel: Some(false),
                voice_room: Some(false),
            },
            tag_ids: Some(page([protocol::ConversationTagId {
                scope: scope(1),
                local: 7,
            }])),
            name: text("Remote room"),
            kind: RoomKind::Group,
            role: protocol::Role::Member,
            archived: Some(true),
            archive: Decision::Allowed,
            actions: protocol::RoomActions {
                read_messages: Decision::Allowed,
                send_message: Decision::Allowed,
                list_participants: Decision::Allowed,
                leave_conversation: Decision::Allowed,
                join_call: Decision::Unknown,
            },
            unread_messages: Some(4),
            last_activity: Some(1),
            call_active: Some(false),
            sensitive: false,
            classified: false,
            channel: false,
            unknown_restrictions: false,
        }
    }
    fn rooms(items: Vec<protocol::Conversation>) -> Result<RuntimeReply, IdentityError> {
        Ok(RuntimeReply::Protocol(ProtocolReply::Conversations(
            protocol::ConversationList {
                scope: scope(1),
                rooms: page(items),
            },
        )))
    }
    #[test]
    fn full_reconciliation_preserves_selected_history_and_metadata_failed_lists_never_remove() {
        let mut model = super::super::tests::model();
        model.background.sync(&[scope(1), scope(2)], Instant::now());
        model.snapshot.active_account = Some(scope(1));
        model.apply_room_refresh(scope(1), rooms(vec![room(1)]));
        let context = model.snapshot.conversations.as_slice()[0].context;
        model.history.open(context);
        model.snapshot.selected = Some(context);
        model.snapshot.message_panel = Some(talk_core::ui::messaging::MessagePanel::new(context));
        model.snapshot.message_panel.as_mut().unwrap().latest = Some(42);
        let messages = model.snapshot.messages.clone();
        model.apply_room_refresh(scope(1), rooms(vec![room(1), room(2)]));
        assert_eq!(model.snapshot.selected, Some(context));
        assert_eq!(model.snapshot.messages, messages);
        assert_eq!(
            model.snapshot.message_panel.as_ref().unwrap().latest,
            Some(42)
        );
        let row = &model.snapshot.conversations.as_slice()[0];
        assert_eq!(row.navigation_key, room(1).navigation_key);
        assert_eq!(row.favorite, Some(true));
        assert_eq!(row.archived, Some(true));
        assert_eq!(row.unread, 4);
        assert_eq!(row.tags.as_ref().unwrap().as_slice()[0].local, 7);
        for failure in [
            ErrorKind::TooLarge,
            ErrorKind::Malformed,
            ErrorKind::Transport,
        ] {
            model.apply_room_refresh(scope(1), Err(IdentityError::Request(failure)));
            assert_eq!(model.snapshot.selected, Some(context));
            assert_eq!(model.snapshot.conversations.as_slice().len(), 2);
            assert!(!model.snapshot.conversation_list.complete);
            assert!(
                model.history.get(context).is_some(),
                "failed lists cannot purge retained history"
            );
            assert_eq!(
                model.snapshot.message_panel.as_ref().unwrap().latest,
                Some(42)
            );
        }
        // Wrong-scope/duplicate data is incomplete even if its HTTP request succeeded.
        model.apply_room_refresh(scope(1), rooms(vec![room(1), room(1)]));
        assert_eq!(model.snapshot.conversations.as_slice().len(), 2);
        assert!(!model.snapshot.conversation_list.complete);
        model.apply_room_refresh(scope(1), rooms(vec![room(2)]));
        assert!(model.snapshot.conversation_list.complete);
        assert!(model.snapshot.selected.is_none());
        assert!(model.snapshot.message_panel.is_none());
        assert!(
            model.history.get(context).is_none(),
            "authoritative removal purges retained history"
        );
    }
    #[test]
    fn inactive_conversation_refresh_updates_only_its_cache() {
        let mut model = super::super::tests::model();
        model.background.sync(&[scope(1), scope(2)], Instant::now());
        model.snapshot.active_account = Some(scope(2));
        let visible = model.snapshot.conversations.clone();
        let selected = model.snapshot.selected;
        model.apply_room_refresh(scope(1), rooms(vec![room(1)]));
        assert_eq!(model.snapshot.conversations, visible);
        assert_eq!(model.snapshot.selected, selected);
        assert_eq!(
            model
                .background
                .account(scope(1))
                .unwrap()
                .rooms
                .as_slice()
                .len(),
            1
        );
        model.select(Some(scope(1)));
        assert_eq!(model.snapshot.conversations.as_slice()[0].unread, 4);
        assert!(
            !model.snapshot.conversation_list.complete,
            "selection revalidates cached state"
        );
    }
}
