//! Runtime-scoped history, never a credential/file store or authorization grant.
//! Ordinary confirmed content may be exported to the identity data owner;
//! restored content never restores action permissions or operation tickets.
//! At most 32 opened or prefetched account/runtime/host/room contexts retain 100
//! messages each. Eviction, access loss and account retirement also retire their
//! reply tickets: an old completion cannot recreate a removed cache entry.
use super::messaging::{Work, merge, message_view};
use super::*;
use talk_adapters::identity::{CacheHistory, CachedMessage, CachedReaction, CachedReply};
use talk_core::preferences::navigation::NavigationRoomKey;
use talk_core::protocol::messaging::MessagingResult;
use talk_core::ui::presentation::MessageView;
use talk_core::ui::{conversation::RemoteState, messaging::MessagePanel};

const ROOMS: usize = 32;
const IN_FLIGHT: usize = 16;
const REACTION_SNAPSHOTS: usize = 100;

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
struct Version(u64, bool);

struct Flight {
    context: ConversationRef,
    operation: OperationId,
    request: RequestId,
    work: Work,
    version: Version,
    started: Instant,
}

struct ReactionSnapshot {
    message: u128,
    version: Version,
    reactions: Page<protocol::messaging::Reaction, 64>,
}

pub(super) struct History {
    context: ConversationRef,
    opened: bool,
    pub(super) messages: Page<MessageView, 100>,
    pub(super) older: Option<u128>,
    pub(super) latest: Option<u128>,
    common_read: Option<u128>,
    common_version: Version,
    pub(super) state: RemoteState,
    state_owner: Work,
    pub(super) loaded: bool,
    pub(super) offline: bool,
    connectivity_version: Version,
    versions: Vec<(u128, Version)>,
    // Only while an older admitted read can overwrite a confirmed mutation.
    // This also protects thread/pin rows outside the main 100-message page.
    reaction_snapshots: Vec<ReactionSnapshot>,
    capabilities: Option<(Instant, Version, protocol::messaging::MessagingCapabilities)>,
    // Same room/runtime lifetime as history; at most 50 canonical pin excerpts.
    // A confirmed write fences all already-admitted pin reads, just like edits.
    pins: Page<MessageView, 50>,
    pins_version: Version,
    pins_state: RemoteState,
    pins_loaded: bool,
    pins_read_at: Option<Instant>,
}
impl History {
    fn new(context: ConversationRef) -> Self {
        Self {
            context,
            opened: false,
            messages: Page::empty(),
            older: None,
            latest: None,
            common_read: None,
            common_version: Version(0, false),
            state: RemoteState::Idle,
            state_owner: Work::History,
            loaded: false,
            offline: false,
            connectivity_version: Version(0, false),
            versions: Vec::new(),
            reaction_snapshots: Vec::new(),
            capabilities: None,
            pins: Page::empty(),
            pins_version: Version(0, false),
            pins_state: RemoteState::Idle,
            pins_loaded: false,
            pins_read_at: None,
        }
    }
    pub(super) fn project(&self, panel: &mut MessagePanel) {
        panel.history = self.state;
        panel.older = self.older;
        panel.latest = self.latest;
        panel.common_read = self.common_read;
        // Presentation may reuse a recently observed limit/features snapshot;
        // every mutation still revalidates its room in the protocol owner.
        panel.features = Page::empty();
        panel.message_limit = 4096;
        panel.file_limit = 0;
        panel.pin_change = Decision::Unknown;
        panel.pinned_messages = self.pins.clone();
        panel.pins_state = self.pins_state;
        panel.pins_loaded = self.pins_loaded;
        if let Some((expires, _, capabilities)) = &self.capabilities
            && Instant::now() < *expires
        {
            super::messaging::project_capabilities(panel, capabilities);
        }
    }
    pub(super) fn needs_pins(&self) -> bool {
        !self.pins_state.pending()
            && self
                .pins_read_at
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(30))
            && self
                .capabilities
                .as_ref()
                .is_some_and(|(expires, _, caps)| {
                    Instant::now() < *expires
                        && caps.actions.as_slice().iter().any(|(feature, value)| {
                            *feature == protocol::messaging::MessagingFeature::Pins
                                && value.implemented
                                && value.server == protocol::FeatureSupport::Supported
                                && value.allowed == Decision::Allowed
                        })
                })
    }
    fn merge(
        &mut self,
        incoming: impl Iterator<Item = MessageView>,
        version: Version,
        older: bool,
    ) {
        let incoming: Vec<_> = incoming
            .take(50)
            .filter(|message| {
                self.versions
                    .iter()
                    .find(|(id, _)| *id == message.local_id)
                    .is_none_or(|(_, known)| *known <= version)
            })
            .collect();
        for message in &incoming {
            if let Some((_, known)) = self
                .versions
                .iter_mut()
                .find(|(id, _)| *id == message.local_id)
            {
                *known = version;
            } else {
                self.versions.push((message.local_id, version));
            }
        }
        let first = self.messages.as_slice().first().map(|m| m.local_id);
        self.messages = merge(&self.messages, incoming.into_iter(), older);
        self.versions
            .retain(|(id, _)| self.messages.as_slice().iter().any(|m| m.local_id == *id));
        // Appending at capacity can evict the oldest retained row. Those rows
        // remain pageable even if the server previously exhausted older history.
        if let Some(new_first) = self.messages.as_slice().first().map(|m| m.local_id)
            && first.is_some_and(|old_first| new_first > old_first)
        {
            self.older = Some(new_first);
        }
    }
    fn expire(&mut self, now: u64) {
        self.pins = page(
            self.pins
                .as_slice()
                .iter()
                .filter(|m| !m.metadata.expires_at.is_some_and(|at| at > 0 && at <= now))
                .cloned(),
        );
        let expired: Vec<_> = self
            .messages
            .as_slice()
            .iter()
            .filter(|m| m.metadata.expires_at.is_some_and(|at| at > 0 && at <= now))
            .map(|m| m.local_id)
            .collect();
        if expired.is_empty() {
            return;
        }
        self.messages = page(self.messages.as_slice().iter().filter_map(|message| {
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
        }));
        self.versions.retain(|(id, _)| !expired.contains(id));
    }
}

#[derive(Default)]
pub(super) struct Cache {
    rooms: Vec<History>,
    flights: Vec<Flight>,
    last_admission: u64,
}
impl Cache {
    fn reap_reactions(&mut self) {
        let flights = &self.flights;
        for room in &mut self.rooms {
            room.reaction_snapshots.retain(|snapshot| {
                flights.iter().any(|flight| {
                    flight.context == room.context && flight.version < snapshot.version
                })
            });
        }
    }
    /// Reserve capacity before mutation dispatch. Confirmed results are never
    /// dropped to make room; settled old-read protection is reclaimed instead.
    pub(super) fn can_react(&mut self, context: ConversationRef, message: u128) -> bool {
        self.reap_reactions();
        self.get(context).is_some_and(|room| {
            room.reaction_snapshots.len() < REACTION_SNAPSHOTS
                || room
                    .reaction_snapshots
                    .iter()
                    .any(|snapshot| snapshot.message == message)
        })
    }
    /// Repair only reaction fields on reads admitted before an accepted change.
    /// Newer reads retain their own authority for reactions and all other fields.
    pub(super) fn reconcile_reactions(
        &self,
        context: ConversationRef,
        operation: OperationId,
        work: Work,
        result: &mut Result<MessagingResult, Completion>,
    ) {
        let Some(version) = self.flights.iter().find_map(|flight| {
            (flight.context == context && flight.operation == operation && flight.work == work)
                .then_some(flight.version)
        }) else {
            return;
        };
        let Some(room) = self.get(context) else {
            return;
        };
        let reconcile = |message: &mut protocol::messaging::MessageDetails| {
            if message.message.kind != protocol::MessageKind::Deleted
                && let Some(snapshot) = room.reaction_snapshots.iter().find(|snapshot| {
                    snapshot.message == u128::from(message.message.id.sequence)
                        && snapshot.version > version
                })
            {
                message.reactions = snapshot.reactions.clone();
            }
        };
        match result {
            Ok(MessagingResult::Messages { messages, .. }) => {
                let mut updated = Page::empty();
                for message in messages.as_slice() {
                    let mut message = message.clone();
                    reconcile(&mut message);
                    let _ = updated.try_push(message); // Same bounded page length.
                }
                *messages = updated;
            }
            Ok(MessagingResult::Message(message)) => reconcile(message),
            _ => (),
        }
    }
    /// Selection or bounded prefetch admission creates entries. Results never allocate a room.
    pub(super) fn open(&mut self, context: ConversationRef) {
        if let Some(index) = self.rooms.iter().position(|r| r.context == context) {
            let mut room = self.rooms.remove(index);
            room.opened = true;
            self.rooms.push(room);
        } else {
            if self.rooms.len() == ROOMS {
                let evict = self.rooms.iter().position(|r| !r.opened).unwrap_or(0);
                let oldest = self.rooms[evict].context;
                self.remove(oldest);
            }
            let mut room = History::new(context);
            room.opened = true;
            self.rooms.push(room);
        }
    }
    /// Speculation cannot evict history the user has already opened. At full
    /// capacity, discard that warm-up slot instead of disturbing the foreground.
    pub(super) fn prefetch(&mut self, context: ConversationRef) -> bool {
        if self.get(context).is_some() {
            return true;
        }
        if self.rooms.len() == ROOMS {
            return false;
        }
        self.rooms.push(History::new(context));
        true
    }
    pub(super) fn get(&self, context: ConversationRef) -> Option<&History> {
        self.rooms.iter().find(|r| r.context == context)
    }
    /// Durable projections contain confirmed content only, never live policy,
    /// transport identifiers, attachments or unresolved optimistic messages.
    pub(super) fn durable(
        &self,
        context: ConversationRef,
        key: NavigationRoomKey,
        now: u64,
    ) -> Option<CacheHistory> {
        let room = self.get(context).filter(|r| r.loaded)?;
        Some(CacheHistory {
            room: key,
            messages: page(
                room.messages
                    .as_slice()
                    .iter()
                    .filter(|m| {
                        matches!(
                            m.delivery,
                            talk_core::ui::presentation::DeliveryState::Received
                                | talk_core::ui::presentation::DeliveryState::ServerAccepted
                        )
                    })
                    .filter_map(cached_message),
            ),
            pins: page(
                room.pins
                    .as_slice()
                    .iter()
                    .filter_map(|m| u64::try_from(m.local_id).ok())
                    .filter(|id| *id > 0),
            ),
            pin_messages: page(room.pins.as_slice().iter().filter_map(cached_message)),
            pins_loaded: room.pins_loaded,
            older: room.older.and_then(|id| u64::try_from(id).ok()),
            latest: room.latest.and_then(|id| u64::try_from(id).ok()),
            saved_at: now,
        })
    }
    /// A cache read may warm an empty room during a pending network read, but
    /// cannot replace canonical observations or change admitted request state.
    /// Every restored action decision remains Unknown.
    pub(super) fn restore(&mut self, context: ConversationRef, cached: &CacheHistory) {
        if self
            .get(context)
            .is_some_and(|r| r.loaded || r.versions.iter().any(|(_, v)| v.0 > 0))
            || !self.prefetch(context)
        {
            return;
        }
        let Some(room) = self.get_mut(context) else {
            return;
        };
        room.messages = page(cached.messages.as_slice().iter().map(restored_message));
        if room.pins_version.0 == 0 {
            room.pins = page(cached.pins.as_slice().iter().filter_map(|id| {
                cached
                    .pin_messages
                    .as_slice()
                    .iter()
                    .find(|m| m.id == *id)
                    .map(restored_message)
                    .or_else(|| {
                        room.messages
                            .as_slice()
                            .iter()
                            .find(|m| m.local_id == u128::from(*id))
                            .cloned()
                    })
            }));
            room.pins_loaded =
                cached.pins_loaded && room.pins.as_slice().len() == cached.pins.as_slice().len();
        }
        room.older = cached.older.map(u128::from);
        room.latest = cached.latest.map(u128::from);
        room.loaded = true;
        // A disk observation says nothing about reachability: preserve both an
        // existing real failure and the default healthy/unknown observation.
        room.expire(now_unix());
    }
    fn get_mut(&mut self, context: ConversationRef) -> Option<&mut History> {
        self.rooms.iter_mut().find(|r| r.context == context)
    }
    pub(super) fn remove(&mut self, context: ConversationRef) {
        self.rooms.retain(|r| r.context != context);
        self.flights.retain(|f| f.context != context);
    }
    pub(super) fn remove_account(&mut self, account: AccountId) {
        self.rooms.retain(|r| r.context.account.account != account);
        self.flights
            .retain(|f| f.context.account.account != account);
    }
    pub(super) fn retain_accounts(&mut self, scopes: &[AccountScope]) {
        self.rooms.retain(|r| scopes.contains(&r.context.account));
        self.flights.retain(|f| scopes.contains(&f.context.account));
    }
    pub(super) fn expire(&mut self, now: u64) {
        for room in &mut self.rooms {
            room.expire(now);
        }
    }
    pub(super) fn admitted(
        &mut self,
        context: ConversationRef,
        operation: OperationId,
        work: Work,
        request: RequestId,
        serial: u64,
    ) {
        // Accounts admits at most 16 total operations; this owner keeps no queue
        // beyond those operations and does not restart an evicted request.
        if self.flights.len() >= IN_FLIGHT || self.get(context).is_none() {
            return;
        }
        self.last_admission = self.last_admission.max(serial);
        self.flights.push(Flight {
            context,
            operation,
            request,
            work,
            version: Version(serial, false),
            started: Instant::now(),
        });
        if matches!(work, Work::History | Work::Older)
            && let Some(room) = self.get_mut(context)
        {
            room.state = RemoteState::Loading(request);
            room.state_owner = work;
        }
        if work == Work::Pins
            && let Some(room) = self.get_mut(context)
        {
            room.pins_state = RemoteState::Loading(request);
            room.pins_read_at = Some(Instant::now());
        }
    }
    pub(super) fn admission_failed(
        &mut self,
        context: ConversationRef,
        request: RequestId,
        reason: UnavailableReason,
    ) {
        if let Some(room) = self.get_mut(context) {
            room.state = RemoteState::Failed(request, reason);
            room.state_owner = if room.latest.is_some() {
                Work::Poll
            } else {
                Work::History
            };
        }
    }
    pub(super) fn read_failure_is_current(
        &self,
        context: ConversationRef,
        operation: OperationId,
        work: Work,
    ) -> bool {
        self.get(context).is_some_and(|room| {
            self.flights.iter().any(|flight| {
                flight.context == context
                    && flight.operation == operation
                    && flight.work == work
                    && flight.version >= room.common_version
            })
        })
    }
    /// Returns false for cancelled/evicted tickets, even after the same room was
    /// opened again. The account owner separately checks current runtime authority.
    pub(super) fn complete(
        &mut self,
        context: ConversationRef,
        operation: OperationId,
        work: Work,
        request: RequestId,
        result: &Result<MessagingResult, Completion>,
        network_failure: bool,
    ) -> bool {
        let Some(index) = self.flights.iter().position(|f| {
            f.context == context
                && f.operation == operation
                && f.work == work
                && f.request == request
        }) else {
            return false;
        };
        let flight = self.flights.remove(index);
        let mutation_version = Version(self.last_admission, true);
        let Some(room) = self.get_mut(context) else {
            return false;
        };
        if work == Work::Pins && flight.version >= room.pins_version {
            match result {
                Ok(MessagingResult::Messages { messages, .. }) => {
                    room.pins = page(messages.as_slice().iter().map(message_view));
                    room.pins_loaded = true;
                    room.pins_state = RemoteState::Idle;
                    room.pins_version = flight.version;
                }
                Err(Completion::Failed(reason)) => {
                    room.pins_state = RemoteState::Failed(request, *reason)
                }
                _ => room.pins_state = RemoteState::Uncertain(request),
            }
        }
        if let Work::PinChange { message, pinned } = work
            && result.is_ok()
        {
            let target = room
                .messages
                .as_slice()
                .iter()
                .find(|m| m.local_id == message)
                .filter(|m| pinned && !m.metadata.deleted)
                .cloned();
            let retained = if target.is_some() { 49 } else { 50 };
            room.pins = page(
                room.pins
                    .as_slice()
                    .iter()
                    .filter(|m| m.local_id != message)
                    .take(retained)
                    .cloned()
                    .chain(target),
            );
            room.pins_version = mutation_version;
            room.pins_state = RemoteState::Idle;
            // Refresh membership/excerpts through a separately observed GET.
            // Busy admission leaves this eligible for the next bounded owner tick.
            room.pins_read_at = None;
        }
        let capabilities = match result {
            Ok(MessagingResult::Messages { capabilities, .. }) => capabilities.as_ref(),
            Ok(MessagingResult::Capabilities(capabilities)) => Some(capabilities),
            _ => None,
        };
        if let Some(capabilities) = capabilities
            && room
                .capabilities
                .as_ref()
                .is_none_or(|(_, version, _)| *version <= flight.version)
        {
            room.capabilities = Some((
                flight.started + background::SOURCE_LIFETIME,
                flight.version,
                capabilities.clone(),
            ));
        }
        let timeline = matches!(
            work,
            Work::History
                | Work::Latest
                | Work::Prefetch
                | Work::Older
                | Work::Poll
                | Work::Context(_)
                | Work::Recovery
        );
        // Retained content and reachability are independent. Only a newer
        // admitted timeline observation may change the offline projection;
        // a late failure cannot hide recovery, nor can an old success clear it.
        if timeline
            && flight.version >= room.connectivity_version
            && (network_failure || matches!(result, Ok(MessagingResult::Messages { .. })))
        {
            room.offline = network_failure;
            room.connectivity_version = flight.version;
        }
        match result {
            Ok(MessagingResult::Messages {
                messages,
                cursor,
                common_read,
                ..
            }) if timeline => {
                let was_loaded = room.loaded;
                room.merge(
                    messages.as_slice().iter().map(message_view),
                    flight.version,
                    work == Work::Older
                        || matches!(work, Work::Context(id)
                        if room.messages.as_slice().first().is_some_and(|m| id < m.local_id)),
                );
                if work == Work::Older
                    || work == Work::History && !was_loaded
                    || matches!(work, Work::Latest | Work::Prefetch) && room.latest.is_none()
                {
                    room.older = cursor.map(|id| u128::from(id.sequence));
                } else if matches!(work, Work::Context(_)) {
                    room.older = room.messages.as_slice().first().map(|m| m.local_id);
                }
                // The forward cursor is an event cursor, not necessarily a
                // retained message ID. Backward pages never rewind it.
                let latest = if work == Work::Poll {
                    cursor.map(|id| u128::from(id.sequence))
                } else if matches!(work, Work::History | Work::Latest | Work::Prefetch) {
                    messages
                        .as_slice()
                        .iter()
                        .map(|m| u128::from(m.message.id.sequence))
                        .max()
                } else {
                    None
                };
                room.latest = room.latest.max(latest);
                if flight.version >= room.common_version && work != Work::Recovery {
                    room.common_read = common_read.map(|id| u128::from(id.sequence));
                    room.common_version = flight.version;
                }
                room.loaded = true;
                if (!work.background()
                    || room.state_owner.background()
                    || room.state_owner == Work::History
                    || room.state == RemoteState::Idle)
                    && (!work.background() || flight.version >= room.common_version)
                {
                    room.state = RemoteState::Idle;
                    room.state_owner = work;
                }
            }
            Ok(MessagingResult::Message(message)) => {
                // A confirmed local edit/delete outranks every read already
                // admitted at completion, including a later-started slow poll.
                room.merge(
                    std::iter::once(message_view(message)),
                    mutation_version,
                    false,
                );
            }
            Ok(MessagingResult::Reactions { message, reactions }) if matches!(work, Work::Reaction(target) if target == u128::from(message.sequence)) =>
            {
                let target = u128::from(message.sequence);
                if let Some(mut current) = room
                    .messages
                    .as_slice()
                    .iter()
                    .find(|message| message.local_id == target && !message.metadata.deleted)
                    .cloned()
                {
                    current.metadata.reactions = super::messaging::reaction_views(reactions);
                    room.merge(std::iter::once(current), mutation_version, false);
                }
                if let Some(snapshot) = room
                    .reaction_snapshots
                    .iter_mut()
                    .find(|snapshot| snapshot.message == target)
                {
                    snapshot.version = mutation_version;
                    snapshot.reactions = reactions.clone();
                } else {
                    // can_react and the single admitted mutation per room reserve
                    // this slot before dispatch, including while navigation changes.
                    room.reaction_snapshots.push(ReactionSnapshot {
                        message: target,
                        version: mutation_version,
                        reactions: reactions.clone(),
                    });
                }
            }
            Err(completion)
                if timeline
                    && !matches!(work, Work::Context(_))
                    && (work == Work::Older || flight.version >= room.connectivity_version)
                    && (!work.background()
                        || (room.state_owner.background() || room.state == RemoteState::Idle)
                            && flight.version >= room.common_version) =>
            {
                room.state = match *completion {
                    Completion::Failed(reason) => RemoteState::Failed(request, reason),
                    Completion::Uncertain => RemoteState::Uncertain(request),
                    Completion::ServerAccepted | Completion::Queued => RemoteState::Idle,
                };
                room.state_owner = work;
            }
            _ => (),
        }
        room.pins = page(room.pins.as_slice().iter().filter_map(|pin| {
            let canonical = room
                .messages
                .as_slice()
                .iter()
                .find(|m| {
                    m.local_id == pin.local_id
                        && room.versions.iter().any(|(id, version)| {
                            *id == pin.local_id && *version >= room.pins_version
                        })
                })
                .unwrap_or(pin);
            if canonical.metadata.deleted {
                return None;
            }
            let mut canonical = canonical.clone();
            if let Ok(MessagingResult::Reactions { message, reactions }) = result
                && u128::from(message.sequence) == pin.local_id
            {
                canonical.metadata.reactions = super::messaging::reaction_views(reactions);
            }
            Some(canonical)
        }));
        self.reap_reactions();
        true
    }
}

fn cached_message(message: &MessageView) -> Option<CachedMessage> {
    let id = u64::try_from(message.local_id).ok().filter(|id| *id > 0)?;
    Some(CachedMessage {
        attachments: page(
            message
                .metadata
                .rich
                .as_slice()
                .iter()
                .filter(|o| !message.metadata.deleted && o.kind.as_str() == "file")
                .take(8)
                .map(|o| talk_adapters::identity::CachedAttachment {
                    placeholder: o.placeholder.clone(),
                    name: o.name.clone(),
                    mime_type: o.mime_type.clone(),
                    size: o.size,
                    id: o.id.clone(),
                }),
        ),
        id,
        author: text(message.author.as_str()),
        actor_key: message.metadata.author_key.clone(),
        text: message.body.clone(),
        own: message.own,
        timestamp: message.metadata.timestamp,
        expires_at: message.metadata.expires_at,
        edited: message.metadata.edited,
        deleted: message.metadata.deleted,
        system: message.metadata.system,
        truncated: message.metadata.truncated,
        replyable: message.metadata.replyable,
        parent: message.metadata.parent.as_ref().and_then(|p| {
            Some(CachedReply {
                id: u64::try_from(p.id).ok()?,
                author: p.author.clone(),
                text: p.text.clone(),
                unavailable: p.unavailable,
            })
        }),
        thread: message
            .metadata
            .thread
            .and_then(|id| u64::try_from(id).ok()),
        reactions: page(
            message
                .metadata
                .reactions
                .as_slice()
                .iter()
                .map(|r| CachedReaction {
                    emoji: r.emoji.clone(),
                    count: r.count,
                    own: r.own,
                }),
        ),
    })
}
fn restored_message(message: &CachedMessage) -> MessageView {
    MessageView {
        local_id: u128::from(message.id),
        author: text(message.author.as_str()),
        initials: initials(message.author.as_str()),
        time_label: text(&super::messaging::time_label(message.timestamp)),
        body: message.text.clone(),
        own: message.own,
        delivery: talk_core::ui::presentation::DeliveryState::Received,
        attachments: Page::empty(),
        reply_count: 0,
        metadata: talk_core::ui::messaging::MessageMetadata {
            rich: page(
                message
                    .attachments
                    .as_slice()
                    .iter()
                    .filter(|_| !message.deleted)
                    .map(|o| talk_core::ui::messaging::RichObject {
                        placeholder: o.placeholder.clone(),
                        kind: text("file"),
                        name: o.name.clone(),
                        mime_type: o.mime_type.clone(),
                        size: o.size,
                        id: o.id.clone(),
                        link: None,
                    }),
            ),
            author_key: message.actor_key.clone(),
            expires_at: message.expires_at,
            timestamp: message.timestamp,
            truncated: message.truncated,
            edited: message.edited,
            deleted: message.deleted,
            system: message.system,
            replyable: message.replyable,
            parent: message
                .parent
                .as_ref()
                .map(|p| talk_core::ui::messaging::Reply {
                    id: u128::from(p.id),
                    author: p.author.clone(),
                    text: p.text.clone(),
                    unavailable: p.unavailable,
                }),
            thread: message.thread.map(u128::from),
            reactions: page(message.reactions.as_slice().iter().map(|r| {
                talk_core::ui::messaging::Reaction {
                    emoji: r.emoji.clone(),
                    count: r.count,
                    own: r.own,
                }
            })),
            ..Default::default()
        },
    }
}

pub(super) fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests;
