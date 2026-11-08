//! One bounded account observation schedule. Push hints and timers share these
//! slots; neither creates another network owner or an unbounded event queue.
use super::*;

const ACCOUNTS: usize = 8;
const IN_FLIGHT: usize = 4;
const MIN_INTERVAL: Duration = Duration::from_secs(2);
const LIVE_FALLBACK_INTERVAL: Duration = Duration::from_secs(30);
// Notifications API v2 asks clients to poll at most hourly after HTTP 204.
const UNAVAILABLE_NOTIFICATION_INTERVAL: Duration = Duration::from_secs(60 * 60);
pub(super) const SOURCE_LIFETIME: Duration = Duration::from_secs(12);
const MEMBER_COUNT_LIFETIME: Duration = Duration::from_secs(10 * 60);
const MEMBER_COUNT_RETRY: Duration = Duration::from_secs(60);

struct MemberCount {
    context: ConversationRef,
    count: Option<u16>,
    observed: Option<Instant>,
    requested: Option<Instant>,
    names: Page<Text<256>, 16>,
    names_observed: Option<Instant>,
    names_requested: Option<Instant>,
}

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
    notification_failures: [u8; 2],
    retry_not_before: [Option<Instant>; 2],
    prefetch: Vec<ConversationRef>,
    prefetch_planned: bool,
    prefetch_deadline: Option<Instant>,
    live: Option<Result<protocol::AccountLiveState, IdentityError>>,
    first_transport_failure: Option<ErrorKind>,
    transport_failures: u8,
    transport_version: u64,
    background_paused_until: Option<Instant>,
    member_counts: Vec<MemberCount>,
    names_next: Instant,
    pub(super) metadata: conversations::Metadata,
    pub(super) rooms: Page<ConversationView, 256>,
    pub(super) state: ConversationListState,
}
#[derive(Default)]
pub(super) struct Background {
    accounts: Vec<Account>,
}
impl Background {
    pub(super) fn resume(&mut self, now: Instant) {
        for a in &mut self.accounts {
            a.background_paused_until = None;
            for kind in Observation::ALL {
                let i = kind.index();
                a.due[i] = a
                    .retry_not_before
                    .get(i)
                    .copied()
                    .flatten()
                    .unwrap_or(now)
                    .max(now);
            }
        }
    }
    pub(super) fn sync(&mut self, scopes: &[AccountScope], now: Instant) {
        self.accounts.retain(|a| scopes.contains(&a.scope));
        for &scope in scopes.iter().take(ACCOUNTS) {
            if !self.accounts.iter().any(|a| a.scope == scope) {
                self.accounts.push(Account {
                    scope,
                    due: [now; 4],
                    last: [None; 4],
                    sources: [None; 2],
                    notification_failures: [0; 2],
                    retry_not_before: [None; 2],
                    prefetch: Vec::new(),
                    prefetch_planned: false,
                    prefetch_deadline: None,
                    live: None,
                    first_transport_failure: None,
                    transport_failures: 0,
                    transport_version: 0,
                    background_paused_until: None,
                    member_counts: Vec::new(),
                    names_next: now,
                    metadata: conversations::Metadata::default(),
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
    pub(super) fn background_ready(&self, scope: AccountScope, now: Instant) -> bool {
        self.account(scope)
            .is_none_or(|a| a.background_paused_until.is_none_or(|at| now >= at))
    }
    pub(super) fn transport_result(
        &mut self,
        scope: AccountScope,
        version: u64,
        success: bool,
        now: Instant,
    ) {
        let Some(account) = self.account_mut(scope) else {
            return;
        };
        if version < account.transport_version {
            return;
        }
        account.transport_version = version;
        self.transport_observed(scope, success, now);
    }
    pub(super) fn transport_observed(&mut self, scope: AccountScope, success: bool, now: Instant) {
        if let Some(a) = self.account_mut(scope) {
            if success {
                a.transport_failures = 0;
                a.background_paused_until = None;
            } else {
                a.transport_failures = a.transport_failures.saturating_add(1).min(6);
                if a.transport_failures >= 2 {
                    // Stable per-runtime jitter needs neither a new random owner
                    // nor a task. A failed probe cannot reset another account.
                    let jitter = ((scope.account.as_local() ^ scope.runtime.as_local()) as u64)
                        .wrapping_mul(u64::from(a.transport_failures) * 7919)
                        % 1000;
                    let delay = Duration::from_secs((5u64 << (a.transport_failures - 2)).min(120))
                        + Duration::from_millis(jitter);
                    a.background_paused_until = Some(now + delay);
                }
            }
        }
    }
    pub(super) fn member_count(&self, context: ConversationRef) -> Option<u16> {
        self.account(context.account)?
            .member_counts
            .iter()
            .find(|m| m.context == context)?
            .count
    }
    pub(super) fn members_need_refresh(&self, context: ConversationRef, now: Instant) -> bool {
        let Some(count) = self
            .account(context.account)
            .and_then(|a| a.member_counts.iter().find(|m| m.context == context))
        else {
            return true;
        };
        !count
            .observed
            .is_some_and(|at| now.saturating_duration_since(at) < MEMBER_COUNT_LIFETIME)
            && !count
                .requested
                .is_some_and(|at| now.saturating_duration_since(at) < MEMBER_COUNT_RETRY)
    }
    pub(super) fn member_count_requested(&mut self, context: ConversationRef, now: Instant) {
        if let Some(slot) = self.member_slot(context) {
            slot.requested = Some(now);
        }
    }
    fn names_need_refresh(&self, context: ConversationRef, now: Instant) -> bool {
        self.account(context.account)
            .and_then(|a| a.member_counts.iter().find(|m| m.context == context))
            .is_none_or(|m| {
                !m.names_observed
                    .is_some_and(|at| now.saturating_duration_since(at) < MEMBER_COUNT_LIFETIME)
                    && !m
                        .names_requested
                        .is_some_and(|at| now.saturating_duration_since(at) < MEMBER_COUNT_RETRY)
            })
    }
    pub(super) fn remember_member_names(
        &mut self,
        context: ConversationRef,
        names: Page<Text<256>, 16>,
        now: Instant,
    ) {
        if let Some(slot) = self.member_slot(context) {
            slot.names = names.clone();
            slot.names_observed = Some(now);
        }
        if let Some(account) = self.account_mut(context.account) {
            account.rooms = page(account.rooms.as_slice().iter().map(|row| {
                let mut row = row.clone();
                if row.context == context {
                    row.participant_names = names.clone();
                }
                row
            }));
        }
    }
    pub(super) fn forget_member_names(&mut self, context: ConversationRef) {
        if let Some(account) = self.account_mut(context.account) {
            if let Some(slot) = account
                .member_counts
                .iter_mut()
                .find(|m| m.context == context)
            {
                slot.names = Page::empty();
                slot.names_observed = None;
            }
            account.rooms = page(account.rooms.as_slice().iter().map(|row| {
                let mut row = row.clone();
                if row.context == context {
                    row.participant_names = Page::empty();
                }
                row
            }));
        }
    }
    pub(super) fn remember_member_count(
        &mut self,
        context: ConversationRef,
        count: u16,
        now: Instant,
    ) {
        self.restore_member_count(context, count, Some(Duration::ZERO), now);
    }
    /// Restored observations affect presentation only. A missing/old wall-clock
    /// age keeps the value visible while permitting a silent fresh observation.
    pub(super) fn restore_member_count(
        &mut self,
        context: ConversationRef,
        count: u16,
        age: Option<Duration>,
        now: Instant,
    ) {
        if let Some(slot) = self.member_slot(context) {
            slot.count = Some(count);
            slot.observed = age.and_then(|age| now.checked_sub(age));
        }
        if let Some(account) = self.account_mut(context.account) {
            account.rooms = page(account.rooms.as_slice().iter().map(|row| {
                let mut row = row.clone();
                if row.context == context {
                    row.participants = Some(count);
                }
                row
            }));
        }
    }
    pub(super) fn forget_member_count(&mut self, context: ConversationRef) {
        if let Some(account) = self.account_mut(context.account) {
            account.member_counts.retain(|m| m.context != context);
            account.rooms = page(account.rooms.as_slice().iter().map(|row| {
                let mut row = row.clone();
                if row.context == context {
                    row.participants = None;
                    row.participant_names = Page::empty();
                }
                row
            }));
        }
    }
    fn member_slot(&mut self, context: ConversationRef) -> Option<&mut MemberCount> {
        let account = self.account_mut(context.account)?;
        let index = if let Some(index) = account
            .member_counts
            .iter()
            .position(|m| m.context == context)
        {
            index
        } else {
            if account.member_counts.len() == 100 {
                let oldest = account
                    .member_counts
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, m)| m.observed)
                    .map(|(i, _)| i)?;
                account.member_counts.remove(oldest);
            }
            account.member_counts.push(MemberCount {
                context,
                count: None,
                observed: None,
                requested: None,
                names: Page::empty(),
                names_observed: None,
                names_requested: None,
            });
            account.member_counts.len() - 1
        };
        account.member_counts.get_mut(index)
    }
    pub(super) fn invalidate(&mut self, scope: AccountScope, kind: Observation, now: Instant) {
        if let Some(a) = self.account_mut(scope) {
            let i = kind.index();
            // Fresh admitted hints permit a probe, never a freshness renewal.
            let mut earliest = a.last[i].map_or(now, |at| (at + MIN_INTERVAL).max(now));
            // Hints cannot reset failures or bypass backoff, Retry-After or HTTP 204.
            if let Some(Some(retry)) = a.retry_not_before.get(i) {
                earliest = earliest.max(*retry);
            }
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
    pub(super) fn notification_source(
        &mut self,
        scope: AccountScope,
        kind: Observation,
        source: protocol::NotificationSourceState,
        now: Instant,
    ) {
        use protocol::NotificationSourceState;
        let Some(a) = self.account_mut(scope) else {
            return;
        };
        let i = kind.index();
        let Some(observed) = a.sources.get_mut(i) else {
            return;
        };
        *observed = None;
        match source {
            NotificationSourceState::Available => {
                a.sources[i] = Some(now);
                a.notification_failures[i] = 0;
                a.retry_not_before[i] = None;
            }
            NotificationSourceState::Unsupported => {
                a.notification_failures[i] = 0;
                let retry = a.retry_not_before[i]
                    .unwrap_or(now)
                    .max(now + UNAVAILABLE_NOTIFICATION_INTERVAL);
                a.retry_not_before[i] = Some(retry);
                a.due[i] = a.due[i].max(retry);
            }
            NotificationSourceState::Paused {
                retry_after_millis, ..
            } => {
                a.notification_failures[i] = a.notification_failures[i].saturating_add(1).min(6);
                let backoff =
                    Duration::from_secs((5u64 << (a.notification_failures[i] - 1)).min(120));
                let server_delay =
                    Duration::from_millis(u64::from(retry_after_millis.unwrap_or(0).min(300_000)));
                let retry = a.retry_not_before[i]
                    .unwrap_or(now)
                    .max(now + backoff.max(server_delay));
                a.retry_not_before[i] = Some(retry);
                a.due[i] = a.due[i].max(retry);
            }
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
            .filter(|(a, kind)| {
                a.due[kind.index()] <= now
                    && (matches!(kind, Observation::Calls | Observation::LiveEvents)
                        || a.background_paused_until.is_none_or(|at| now >= at))
            })
            .min_by_key(|(a, kind)| (a.due[kind.index()], Some(a.scope) != active, kind.index()))
            .map(|(a, kind)| (a.scope, kind))
    }
    fn admitted(&mut self, scope: AccountScope, kind: Observation, active: bool, now: Instant) {
        if let Some(a) = self.account_mut(scope) {
            let i = kind.index();
            a.last[i] = Some(now);
            a.due[i] = now
                + match kind {
                    Observation::Calls | Observation::Messages => Duration::from_secs(3),
                    Observation::Rooms => Duration::from_secs(if active { 15 } else { 30 }),
                    Observation::LiveEvents => live_interval(a.live),
                };
            if let Some(Some(retry)) = a.retry_not_before.get(i) {
                a.due[i] = a.due[i].max(*retry);
            }
        }
    }
}

fn recent_rooms(scope: AccountScope, list: &protocol::ConversationList) -> Vec<ConversationRef> {
    let mut rooms: Vec<_> = list
        .rooms
        .as_slice()
        .iter()
        .filter(|r| {
            r.actions.read_messages == Decision::Allowed
                && !r.classified
                && !r.sensitive
                && !r.unknown_restrictions
                && r.archived != Some(true)
                && r.last_activity.is_some_and(|at| at > 0)
        })
        .collect();
    rooms.sort_by_key(|r| {
        std::cmp::Reverse((
            r.unread_messages.unwrap_or(0) > 0,
            r.favorite == Some(true),
            r.last_activity,
        ))
    });
    rooms
        .into_iter()
        .take(4)
        .map(|r| ConversationRef {
            account: scope,
            conversation: r.id,
        })
        .collect()
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
    pub(super) fn priority(self) -> u8 {
        match self {
            Self::Message(
                _,
                _,
                _,
                messaging::Work::Prefetch
                | messaging::Work::Metadata
                | messaging::Work::ThreadActivity
                | messaging::Work::ScheduledPoll
                | messaging::Work::TranslationOptions
                | messaging::Work::SummaryStatus
                | messaging::Work::Typing,
            )
            | Self::Rooms(_)
            | Self::Reference(..)
            | Self::ParticipantIndex(..)
            | Self::MessageNotifications(_)
            | Self::AccountReminders(..)
            | Self::FederationInvitations(..)
            | Self::DiscoverProbe(..)
            | Self::Typing(..) => 2,
            Self::IncomingCalls(_) | Self::LiveEvents(_) | Self::Threads(..) | Self::Tags(_) => 1,
            Self::Message(
                _,
                _,
                _,
                messaging::Work::History
                | messaging::Work::Poll
                | messaging::Work::Latest
                | messaging::Work::Older
                | messaging::Work::Thread
                | messaging::Work::ThreadOlder(_)
                | messaging::Work::ThreadRoot(_)
                | messaging::Work::ThreadInfo(_)
                | messaging::Work::Pins
                | messaging::Work::Recovery
                | messaging::Work::Context(_)
                | messaging::Work::UnreadContext(_),
            ) => 1,
            _ => 0,
        }
    }
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
            Self::QueuedSetting(c, _)
            | Self::ParticipantIndex(c, _)
            | Self::Reference(c, _, _, _)
            | Self::Room(_, c, _, _)
            | Self::Message(_, c, _, _)
            | Self::Mutation(_, c)
            | Self::Send(_, c, _)
            | Self::Typing(c, _) => Some(c),
            Self::Call(c) => Some(ConversationRef {
                account: c.account,
                conversation: c.conversation,
            }),
            _ => None,
        }
    }
    pub(super) fn account(self) -> Option<AccountScope> {
        self.room().map(|c| c.account).or(match self {
            Self::Create(_, s, _)
            | Self::Search(_, s, _)
            | Self::Discover(_, s, _)
            | Self::DiscoverProbe(s, _)
            | Self::IncomingCalls(s)
            | Self::MessageNotifications(s)
            | Self::AccountReminders(s, _)
            | Self::FederationInvitations(s, _)
            | Self::FederationChange(s, _, _)
            | Self::LiveEvents(s)
            | Self::Rooms(s)
            | Self::Threads(s, _)
            | Self::Tags(s)
            | Self::TagMutation(_, s)
            | Self::Preferences(s, _) => Some(s),
            _ => None,
        })
    }
}
impl Accounts {
    pub(super) fn remember_room_unread(&mut self, context: ConversationRef) {
        if let Some(account) = self.background.account_mut(context.account) {
            account.rooms = page(account.rooms.as_slice().iter().cloned().map(|mut row| {
                if row.context == context {
                    row.unread = row.unread.max(1);
                }
                row
            }));
        }
        if self.snapshot.active_account == Some(context.account) {
            self.snapshot.conversations = page(
                self.snapshot
                    .conversations
                    .as_slice()
                    .iter()
                    .cloned()
                    .map(|mut row| {
                        if row.context == context {
                            row.unread = row.unread.max(1);
                        }
                        row
                    }),
            );
            self.load_rooms = Some(context.account);
        }
        self.touch();
    }
    /// Entry point for account transport hints. It deliberately accepts no wire
    /// JSON, content, media state or authority. Repeated hints occupy the same slot.
    pub fn invalidate_account(&mut self, scope: AccountScope, change: protocol::AccountChange) {
        if self.current(scope).is_err() {
            return;
        }
        if change == protocol::AccountChange::Conversations
            && let Some(account) = self.background.account_mut(scope)
        {
            let retired = account.metadata.hint();
            for context in retired {
                self.retire_room_metadata_read(context);
            }
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
    /// Four recent policy-approved rooms per newly connected runtime, one
    /// newest page (at most 50 messages) each. One global speculative read,
    /// no retries/queue growth, and no receipt, typing, join or mutation request.
    pub(super) fn tick_prefetch(&mut self) {
        let now = Instant::now();
        for account in &mut self.background.accounts {
            if account.prefetch_deadline.is_some_and(|at| now >= at) {
                account.prefetch.clear();
                account.prefetch_deadline = None;
            }
        }
        if self.pending.len() >= 8
            || self.pending.iter().any(|p| {
                matches!(
                    p.purpose,
                    Purpose::Message(_, _, _, messaging::Work::Prefetch)
                )
            })
        {
            return;
        }
        let active = self.snapshot.active_account;
        let next = self
            .background
            .accounts
            .iter()
            .filter(|a| {
                !a.prefetch.is_empty()
                    && Some(a.scope) == active
                    && self.background.background_ready(a.scope, now)
            })
            .filter(|a| {
                !self
                    .pending
                    .iter()
                    .any(|p| p.purpose.account() == Some(a.scope))
            })
            .min_by_key(|a| Some(a.scope) != active)
            .map(|a| (a.scope, a.prefetch[0]));
        let Some((scope, context)) = next else {
            return;
        };
        if self.snapshot.selected == Some(context)
            || self.history.get(context).is_some_and(|h| h.loaded)
            || self.current(scope).is_err()
            || !self.history.prefetch(context)
        {
            if let Some(account) = self.background.account_mut(scope) {
                account.prefetch.remove(0);
            }
            return;
        }
        let result = self.start_message(
            RequestId(0),
            context,
            protocol::messaging::MessagingCommand::History {
                before: None,
                thread: None,
            },
            messaging::Work::Prefetch,
        );
        if result != Err(SubmitError::Busy)
            && let Some(account) = self.background.account_mut(scope)
        {
            account.prefetch.remove(0);
        }
    }
    /// Populate the session-only switcher index opportunistically. One read at
    /// a time, one attempt per second, and a bounded retry interval keep this
    /// subordinate to explicit navigation and message work.
    pub(super) fn tick_participant_index(&mut self) {
        let now = Instant::now();
        if self.pending.len() >= 8
            || self
                .pending
                .iter()
                .any(|p| matches!(p.purpose, Purpose::ParticipantIndex(..)))
        {
            return;
        }
        let Some(scope) = self.snapshot.active_account else {
            return;
        };
        if !self
            .snapshot
            .accounts
            .as_slice()
            .iter()
            .any(|a| a.scope == scope && a.connection == AccountConnectionState::Connected)
            || self.pending.iter().any(|p| {
                p.purpose.account() == Some(scope) && !matches!(p.purpose, Purpose::LiveEvents(_))
            })
        {
            return;
        }
        let Some(account) = self
            .background
            .account(scope)
            .filter(|a| now >= a.names_next)
        else {
            return;
        };
        let next = account
            .rooms
            .as_slice()
            .iter()
            .filter(|room| {
                matches!(
                    room.kind,
                    ConversationKind::Group | ConversationKind::GuestAccessible
                )
            })
            .filter_map(|room| {
                account
                    .metadata
                    .participant_index_ticket(room.context)
                    .map(|ticket| (room.context, ticket))
            })
            .find(|(context, _)| self.background.names_need_refresh(*context, now));
        let Some((context, ticket)) = next else {
            return;
        };
        if let Some(slot) = self.background.member_slot(context) {
            slot.names_requested = Some(now);
        }
        if let Some(account) = self.background.account_mut(scope) {
            account.names_next = now + Duration::from_secs(1);
        }
        let _ = self.request(
            scope,
            ProtocolRequest::Participants(context.conversation),
            Purpose::ParticipantIndex(context, ticket),
        );
    }

    pub(super) fn complete_participant_index(
        &mut self,
        context: ConversationRef,
        ticket: u64,
        result: Result<RuntimeReply, IdentityError>,
    ) {
        if self.current(context.account).is_err()
            || self.snapshot.active_account != Some(context.account)
            || self
                .background
                .account(context.account)
                .and_then(|a| a.metadata.participant_index_ticket(context))
                != Some(ticket)
        {
            return;
        }
        let Ok(RuntimeReply::Protocol(ProtocolReply::Participants(people))) = result else {
            return;
        };
        if people.conversation != context.conversation {
            return;
        }
        let names = page(
            people
                .people
                .as_slice()
                .iter()
                .filter(|p| !p.is_self && !p.display_name.as_str().trim().is_empty())
                .take(16)
                .map(|p| p.display_name.clone()),
        );
        self.background
            .remember_member_names(context, names.clone(), Instant::now());
        self.snapshot.conversations =
            page(self.snapshot.conversations.as_slice().iter().map(|row| {
                let mut row = row.clone();
                if row.context == context {
                    row.participant_names = names.clone();
                }
                row
            }));
        self.touch();
    }
    pub(super) fn sync_background(&mut self) {
        let scopes: Vec<_> = self
            .snapshot
            .accounts
            .as_slice()
            .iter()
            .map(|a| a.scope)
            .collect();
        if let Ok(connection) = &mut self.connection {
            for pending in &self.pending {
                if matches!(pending.purpose, Purpose::Message(_, c, _, messaging::Work::Prefetch) if Some(c.account) != self.snapshot.active_account)
                {
                    let _ = connection.cancel_request(pending.id);
                }
            }
        }
        self.background.sync(&scopes, Instant::now());
        self.history.retain_accounts(&scopes);
        self.notices.retain_accounts(&scopes);
        self.reminders.retain_accounts(&scopes);
        self.federation.retain_accounts(&scopes);
        self.translation.retain_accounts(&scopes);
        self.summary.retain_accounts(&scopes);
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
        if self.snapshot.active_account == Some(scope)
            && self.snapshot.conversation_list.content == ContentState::Ready
            && self.snapshot.threads.support == FeatureSupport::Unknown
        {
            let _ = self.refresh_threads(scope);
        }
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
                gate: None,
                profile: None,
                request: 0,
                operation: DiagnosticOperation::LiveConnection,
                outcome,
                error,
                count: 0,
                storage: None,
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
            let messages_changed = batch
                .changes
                .as_slice()
                .contains(&protocol::AccountChange::Messages);
            for &change in batch.changes.as_slice() {
                if change == protocol::AccountChange::Conversations
                    && let Some(account) = self.background.account_mut(scope)
                {
                    let retired = account.metadata.hint();
                    for context in retired {
                        self.retire_room_metadata_read(context);
                    }
                }
                self.background
                    .invalidate(scope, Observation::change(change), now);
            }
            if messages_changed {
                self.hint_thread_refresh(scope, now);
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
        let cached_rooms = self
            .cache
            .loaded(scope.account)
            .map(|snapshot| snapshot.rooms.clone())
            .unwrap_or_else(Page::empty);
        let Some(cache) = self.background.account_mut(scope) else {
            return;
        };
        match list {
            Ok(list) => {
                let retired = cache.metadata.reconcile(scope, &list);
                let rows = page(list.rooms.as_slice().iter().map(|r| {
                    let mut row = conversation(r, scope);
                    if r.actions.read_messages == Decision::Allowed
                        && !r.sensitive
                        && !r.classified
                        && !r.unknown_restrictions
                    {
                        let live = cache
                            .rooms
                            .as_slice()
                            .iter()
                            .find(|old| old.context == row.context && old.unread_mention.is_some())
                            .map(|old| (old.unread_mention, old.unread, old.last_activity));
                        let disk = r
                            .navigation_key
                            .as_ref()
                            .and_then(|key| {
                                cached_rooms.as_slice().iter().find(|old| &old.key == key)
                            })
                            .map(|old| (old.unread_mention, old.unread, old.last_activity));
                        row.unread_mention = super::cache::mentions::merge(
                            r.unread_mention,
                            r.unread_messages,
                            r.last_activity,
                            live.or(disk),
                        );
                    }
                    if r.actions.read_messages == Decision::Allowed {
                        row.participants = cache
                            .member_counts
                            .iter()
                            .find(|m| m.context == row.context)
                            .and_then(|m| m.count)
                            .or(row.participants);
                    }
                    if cache
                        .metadata
                        .participant_index_ticket(row.context)
                        .is_some()
                        && let Some(observed) = cache
                            .member_counts
                            .iter()
                            .find(|m| m.context == row.context)
                    {
                        row.participant_names = observed.names.clone();
                    }
                    row
                }));
                cache.member_counts.retain(|m| {
                    list.rooms.as_slice().iter().any(|r| {
                        r.id == m.context.conversation
                            && r.actions.read_messages == Decision::Allowed
                            && r.actions.list_participants == Decision::Allowed
                            && !r.classified
                            && !r.sensitive
                            && !r.unknown_restrictions
                    })
                });
                let removed: Vec<_> = cache
                    .rooms
                    .as_slice()
                    .iter()
                    .filter(|old| !rows.as_slice().iter().any(|r| r.context == old.context))
                    .map(|r| r.context)
                    .collect();
                let recent = recent_rooms(scope, &list);
                cache.prefetch.retain(|c| recent.contains(c));
                if !cache.prefetch_planned {
                    cache.prefetch = recent;
                    cache.prefetch_planned = true;
                    cache.prefetch_deadline = Some(Instant::now() + Duration::from_secs(60));
                }
                cache.rooms = rows.clone();
                cache.state = ConversationListState {
                    content: ContentState::Ready,
                    complete: true,
                };
                let state = cache.state;
                self.observe_cache_rooms(scope, &list);
                for (context, policy_changed) in retired {
                    if policy_changed {
                        self.revoke_room_metadata(context, UnavailableReason::PolicyStale);
                    } else {
                        self.retire_room_metadata_read(context);
                    }
                }
                let rows = self
                    .background
                    .account(scope)
                    .map_or(rows, |a| a.rooms.clone());
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
                    let context = ConversationRef {
                        account: scope,
                        conversation: room.id,
                    };
                    self.revoke_room_metadata(context, UnavailableReason::PermissionDenied);
                    self.invalidate_history(
                        context,
                        if room.actions.read_messages == Decision::Denied {
                            ContentState::Denied
                        } else {
                            ContentState::Failed
                        },
                    );
                }
                if self.snapshot.active_account == Some(scope) {
                    if let Some(panel) = self
                        .snapshot
                        .discover_panel
                        .as_mut()
                        .filter(|panel| panel.account == scope && panel.uncertain)
                        && let Some(context) = panel.joining
                        && list
                            .rooms
                            .as_slice()
                            .iter()
                            .any(|room| room.id == context.conversation)
                    {
                        panel.uncertain = false;
                        panel.joining = None;
                        panel.state = talk_core::ui::conversation::RemoteState::Idle;
                        panel.rooms = page(
                            panel
                                .rooms
                                .as_slice()
                                .iter()
                                .filter(|room| room.context != context)
                                .cloned(),
                        );
                    }
                    if let Some(panel) = self
                        .snapshot
                        .search_panel
                        .as_mut()
                        .filter(|p| p.account == scope)
                    {
                        panel.hits = page(
                            panel
                                .hits
                                .as_slice()
                                .iter()
                                .filter(|hit| {
                                    list.rooms.as_slice().iter().any(|room| {
                                        room.id == hit.context.conversation
                                            && room.actions.read_messages == Decision::Allowed
                                    })
                                })
                                .cloned(),
                        );
                    }
                    self.snapshot.conversations = rows;
                    self.snapshot.conversation_list = state;
                    self.restore_optimistic_join(scope);
                    self.restore_pending_notifications(scope);
                    self.restore_cache_selection();
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
                            // Events, relevant changes and a bounded safety fallback
                            // revalidate details without making activity-only lists busy.
                            self.revalidate_room_metadata(selected);
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
        self.revoke_room_metadata(context, UnavailableReason::PermissionDenied);
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

    #[test]
    fn running_background_notifications_are_due_within_three_seconds() {
        let now = Instant::now();
        let mut schedule = Background::default();
        schedule.sync(&[scope(1)], now);
        for kind in [Observation::Calls, Observation::Messages] {
            schedule.admitted(scope(1), kind, false, now);
            assert_eq!(
                schedule.account(scope(1)).unwrap().due[kind.index()],
                now + Duration::from_secs(3)
            );
        }
    }

    #[test]
    fn notification_backoff_is_bounded_and_does_not_renew_sources_or_block_other_accounts() {
        let now = Instant::now();
        for kind in [Observation::Calls, Observation::Messages] {
            let mut schedule = Background::default();
            schedule.sync(&[scope(1), scope(2)], now);
            schedule.notification_source(
                scope(2),
                kind,
                protocol::NotificationSourceState::Available,
                now,
            );
            schedule.admitted(scope(1), kind, true, now);
            for seconds in [5, 10, 20, 40, 80, 120, 120] {
                schedule.notification_source(
                    scope(1),
                    kind,
                    protocol::NotificationSourceState::Paused {
                        reason: ErrorKind::Transport,
                        retry_after_millis: None,
                    },
                    now,
                );
                assert_eq!(
                    schedule.account(scope(1)).unwrap().due[kind.index()],
                    now + Duration::from_secs(seconds)
                );
                assert!(!schedule.source(scope(1), kind, now));
                assert!(schedule.source(scope(2), kind, now));
                schedule.invalidate(scope(1), kind, now + Duration::from_secs(1));
                assert_eq!(
                    schedule.account(scope(1)).unwrap().due[kind.index()],
                    now + Duration::from_secs(seconds),
                    "a hint cannot bypass backoff or reset consecutive failures"
                );
            }
            assert_eq!(
                schedule.account(scope(1)).unwrap().notification_failures[kind.index()],
                6
            );
            schedule.notification_source(
                scope(1),
                kind,
                protocol::NotificationSourceState::Available,
                now,
            );
            assert_eq!(
                schedule.account(scope(1)).unwrap().notification_failures[kind.index()],
                0
            );
        }
    }

    #[test]
    fn server_cooldowns_survive_push_hints_and_scope_replacement_retires_them() {
        let now = Instant::now();
        for kind in [Observation::Calls, Observation::Messages] {
            let mut schedule = Background::default();
            schedule.sync(&[scope(1)], now);
            schedule.admitted(scope(1), kind, true, now);
            schedule.notification_source(
                scope(1),
                kind,
                protocol::NotificationSourceState::Paused {
                    reason: ErrorKind::RateLimited,
                    retry_after_millis: Some(300_000),
                },
                now,
            );
            schedule.invalidate(scope(1), kind, now + Duration::from_secs(1));
            assert_eq!(
                schedule.account(scope(1)).unwrap().due[kind.index()],
                now + Duration::from_secs(300)
            );
            assert!(!schedule.source(scope(1), kind, now));
            schedule.notification_source(
                scope(1),
                kind,
                protocol::NotificationSourceState::Unsupported,
                now,
            );
            {
                let a = schedule.account_mut(scope(1)).unwrap();
                for other in Observation::ALL {
                    if other != kind {
                        a.due[other.index()] = now + Duration::from_secs(7200);
                    }
                }
            }
            assert!(
                schedule
                    .next(Some(scope(1)), &[], true, now + Duration::from_secs(600))
                    .is_none()
            );
            schedule.invalidate(scope(1), kind, now + Duration::from_secs(601));
            assert_eq!(
                schedule.next(Some(scope(1)), &[], true, now + Duration::from_secs(601)),
                None
            );
            assert_eq!(
                schedule.next(
                    Some(scope(1)),
                    &[],
                    true,
                    now + UNAVAILABLE_NOTIFICATION_INTERVAL
                ),
                Some((scope(1), kind))
            );
            assert!(
                !schedule.source(scope(1), kind, now),
                "a wakeup is not fresh source authority"
            );
            let replacement = AccountScope {
                runtime: RuntimeGeneration::from_local(2),
                ..scope(1)
            };
            schedule.sync(&[replacement], now);
            assert!(schedule.account(scope(1)).is_none());
            assert!(
                schedule
                    .account(replacement)
                    .unwrap()
                    .retry_not_before
                    .iter()
                    .all(Option::is_none)
            );
        }
    }

    fn scope(n: u128) -> AccountScope {
        AccountScope {
            account: AccountId::from_local(n),
            runtime: RuntimeGeneration::from_local(1),
        }
    }
    #[test]
    fn member_counts_remain_visible_during_refresh_and_retry_on_a_bounded_schedule() {
        let now = Instant::now();
        let mut model = super::super::tests::model();
        model.background.sync(&[scope(1)], now);
        model.snapshot.active_account = Some(scope(1));
        model.apply_room_refresh(scope(1), rooms(vec![room(1)]));
        let context = model.snapshot.conversations.as_slice()[0].context;
        model.background.remember_member_count(context, 7, now);
        assert!(!model.background.members_need_refresh(
            context,
            now + MEMBER_COUNT_LIFETIME - Duration::from_secs(1)
        ));
        model.apply_room_refresh(scope(1), rooms(vec![room(1)]));
        assert_eq!(
            model.snapshot.conversations.as_slice()[0].participants,
            Some(7)
        );
        model.apply_room_refresh(scope(1), Err(IdentityError::Request(ErrorKind::Transport)));
        assert_eq!(
            model.snapshot.conversations.as_slice()[0].participants,
            Some(7)
        );
        let stale = now + MEMBER_COUNT_LIFETIME;
        assert!(model.background.members_need_refresh(context, stale));
        model.background.member_count_requested(context, stale);
        assert!(
            !model
                .background
                .members_need_refresh(context, stale + MEMBER_COUNT_RETRY - Duration::from_secs(1))
        );
        assert!(
            model
                .background
                .members_need_refresh(context, stale + MEMBER_COUNT_RETRY)
        );
        assert_eq!(model.background.member_count(context), Some(7));
        model.background.forget_member_count(context);
        assert_eq!(model.background.member_count(context), None);
        assert_eq!(
            model.background.account(scope(1)).unwrap().rooms.as_slice()[0].participants,
            None
        );
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
            user_status: None,
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
            unread_direct_mention: Some(false),
            last_read_message: Some(3),
            unread_mention: None,
            notification_muted: None,
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
            model.snapshot.conversation_list.complete,
            "selection preserves the last confirmed projection while revalidation runs in the background"
        );
        assert_eq!(
            model.load_rooms,
            Some(scope(1)),
            "selection still schedules a fresh policy observation"
        );
    }
    #[test]
    fn prefetch_plans_four_recent_safe_rooms_once_per_runtime_and_retires_access_loss() {
        let mut model = super::super::tests::model();
        model.background.sync(&[scope(1)], Instant::now());
        let mut items: Vec<_> = (1..=10)
            .map(|n| {
                let mut r = room(n);
                r.archived = Some(false);
                r.last_activity = Some(n as u64);
                r
            })
            .collect();
        items[9].classified = true;
        items[8].sensitive = true;
        items[7].unknown_restrictions = true;
        items[6].actions.read_messages = Decision::Denied;
        model.apply_room_refresh(scope(1), rooms(items.clone()));
        let planned = &model.background.account(scope(1)).unwrap().prefetch;
        assert_eq!(
            planned
                .iter()
                .map(|c| c.conversation.local)
                .collect::<Vec<_>>(),
            vec![6, 5, 4, 3]
        );
        assert!(model.snapshot.selected.is_none());
        assert!(model.snapshot.messages.as_slice().is_empty());
        items[5].classified = true;
        model.apply_room_refresh(scope(1), rooms(items));
        assert_eq!(
            model.background.account(scope(1)).unwrap().prefetch.len(),
            3
        );
        model
            .background
            .account_mut(scope(1))
            .unwrap()
            .prefetch_deadline = Some(Instant::now() - Duration::from_secs(1));
        model.tick_prefetch();
        assert!(
            model
                .background
                .account(scope(1))
                .unwrap()
                .prefetch
                .is_empty(),
            "queued reads expire without another admission"
        );
        model.apply_room_refresh(scope(1), rooms(vec![room(1)]));
        assert!(
            model
                .background
                .account(scope(1))
                .unwrap()
                .prefetch
                .is_empty()
        );
        model.background.sync(&[], Instant::now());
        assert!(model.background.accounts.is_empty());
    }
}

#[cfg(test)]
mod cooldown_tests {
    use super::*;
    #[test]
    fn automatic_observation_cannot_take_the_mutation_lane() {
        let context = ConversationRef {
            account: scope(1),
            conversation: talk_core::ids::ConversationId {
                account: scope(1).account,
                host: talk_core::ids::HostId::from_local(1),
                local: 1,
            },
        };
        let message =
            |work| Purpose::Message(RequestId(0), context, OperationId::from_local(1), work);
        let mutation = message(messaging::Work::Send).priority();
        for automatic in [
            Purpose::Rooms(scope(1)),
            Purpose::IncomingCalls(scope(1)),
            Purpose::AccountReminders(scope(1), 1),
            Purpose::FederationInvitations(scope(1), 1),
            message(messaging::Work::ScheduledPoll),
            message(messaging::Work::ThreadActivity),
            message(messaging::Work::Pins),
            message(messaging::Work::TranslationOptions),
            message(messaging::Work::SummaryStatus),
            message(messaging::Work::Typing),
        ] {
            assert!(automatic.priority() > mutation);
        }
        assert!(
            message(messaging::Work::Latest).priority()
                < message(messaging::Work::Prefetch).priority()
        );
    }
    fn scope(id: u128) -> AccountScope {
        AccountScope {
            account: AccountId::from_local(id),
            runtime: talk_core::ids::RuntimeGeneration::from_local(1),
        }
    }
    #[test]
    fn repeated_transport_failures_pause_only_the_affected_accounts_background_work() {
        let now = Instant::now();
        let a = AccountScope {
            account: AccountId::from_local(1),
            runtime: talk_core::ids::RuntimeGeneration::from_local(11),
        };
        let b = AccountScope {
            account: AccountId::from_local(2),
            ..a
        };
        let mut owner = Background::default();
        owner.sync(&[a, b], now);
        owner.transport_observed(a, false, now);
        assert!(owner.background_ready(a, now));
        owner.transport_observed(a, false, now);
        assert!(!owner.background_ready(a, now + Duration::from_secs(4)));
        assert!(owner.background_ready(b, now));
        assert!(owner.background_ready(a, now + Duration::from_secs(6)));
        owner.transport_observed(a, false, now + Duration::from_secs(6));
        assert!(!owner.background_ready(a, now + Duration::from_secs(15)));
        owner.transport_observed(a, true, now + Duration::from_secs(15));
        assert!(owner.background_ready(a, now + Duration::from_secs(15)));
        assert_eq!(owner.account(a).unwrap().transport_failures, 0);
        owner.transport_observed(a, false, now);
        owner.transport_observed(a, false, now);
        let replacement = AccountScope {
            runtime: talk_core::ids::RuntimeGeneration::from_local(12),
            ..a
        };
        owner.sync(&[replacement, b], now);
        assert!(owner.background_ready(replacement, now));
    }
    #[test]
    fn circuit_ignores_stale_completions_and_resume_reopens_background_without_losing_server_cooldown()
     {
        let now = Instant::now();
        let mut owner = Background::default();
        owner.sync(&[scope(1), scope(2)], now);
        owner.transport_result(scope(1), 1, false, now);
        owner.transport_result(scope(1), 2, false, now);
        assert!(!owner.background_ready(scope(1), now));
        assert!(owner.background_ready(scope(2), now));
        owner.transport_result(scope(1), 1, true, now);
        assert!(!owner.background_ready(scope(1), now));
        let retry = now + Duration::from_secs(45);
        owner.account_mut(scope(1)).unwrap().retry_not_before[0] = Some(retry);
        owner.resume(now);
        assert!(owner.background_ready(scope(1), now));
        assert_eq!(owner.account(scope(1)).unwrap().due[0], retry);
        owner.transport_result(scope(1), 3, true, now);
        owner.transport_result(scope(1), 2, false, now);
        assert!(owner.background_ready(scope(1), now));
    }
}
