//! Session-only history, never a credential/file store or an authorization grant.
//! At most 32 recently opened full account/runtime/host/room contexts retain 100
//! messages each. Eviction, access loss and account retirement also retire their
//! reply tickets: an old completion cannot recreate a removed cache entry.
use super::messaging::{Work, merge, message_view};
use super::*;
use talk_core::protocol::messaging::MessagingResult;
use talk_core::ui::presentation::MessageView;
use talk_core::ui::{conversation::RemoteState, messaging::MessagePanel};

const ROOMS: usize = 32;
const IN_FLIGHT: usize = 16;

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
struct Version(u64, bool);

struct Flight {
    context: ConversationRef,
    operation: OperationId,
    work: Work,
    version: Version,
}

pub(super) struct History {
    context: ConversationRef,
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
}
impl History {
    fn new(context: ConversationRef) -> Self {
        Self {
            context,
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
        }
    }
    pub(super) fn project(&self, panel: &mut MessagePanel) {
        panel.history = self.state;
        panel.older = self.older;
        panel.latest = self.latest;
        panel.common_read = self.common_read;
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
    /// Only deliberate selection creates entries. Results never allocate a room.
    pub(super) fn open(&mut self, context: ConversationRef) {
        if let Some(index) = self.rooms.iter().position(|r| r.context == context) {
            let room = self.rooms.remove(index);
            self.rooms.push(room);
        } else {
            if self.rooms.len() == ROOMS {
                let oldest = self.rooms[0].context;
                self.remove(oldest);
            }
            self.rooms.push(History::new(context));
        }
    }
    pub(super) fn get(&self, context: ConversationRef) -> Option<&History> {
        self.rooms.iter().find(|r| r.context == context)
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
            work,
            version: Version(serial, false),
        });
        if matches!(work, Work::History | Work::Older)
            && let Some(room) = self.get_mut(context)
        {
            room.state = RemoteState::Loading(request);
            room.state_owner = work;
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
        let Some(index) = self
            .flights
            .iter()
            .position(|f| f.context == context && f.operation == operation && f.work == work)
        else {
            return false;
        };
        let flight = self.flights.remove(index);
        let mutation_version = Version(self.last_admission, true);
        let Some(room) = self.get_mut(context) else {
            return false;
        };
        let timeline = matches!(
            work,
            Work::History | Work::Older | Work::Poll | Work::Context(_)
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
            }) if timeline => {
                let was_loaded = room.loaded;
                room.merge(
                    messages.as_slice().iter().map(message_view),
                    flight.version,
                    work == Work::Older
                        || matches!(work, Work::Context(id)
                        if room.messages.as_slice().first().is_some_and(|m| id < m.local_id)),
                );
                if work == Work::Older || work == Work::History && !was_loaded {
                    room.older = cursor.map(|id| u128::from(id.sequence));
                } else if matches!(work, Work::Context(_)) {
                    room.older = room.messages.as_slice().first().map(|m| m.local_id);
                }
                // The forward cursor is an event cursor, not necessarily a
                // retained message ID. Backward pages never rewind it.
                let latest = if work == Work::Poll {
                    cursor.map(|id| u128::from(id.sequence))
                } else if work == Work::History {
                    messages
                        .as_slice()
                        .iter()
                        .map(|m| u128::from(m.message.id.sequence))
                        .max()
                } else {
                    None
                };
                room.latest = room.latest.max(latest);
                if flight.version >= room.common_version {
                    room.common_read = common_read.map(|id| u128::from(id.sequence));
                    room.common_version = flight.version;
                }
                room.loaded = true;
                if work != Work::Poll
                    || room.state_owner == Work::Poll
                    || room.state == RemoteState::Idle
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
            Err(completion)
                if timeline
                    && (work != Work::Poll
                        || (room.state_owner == Work::Poll || room.state == RemoteState::Idle)
                            && flight.version >= room.common_version) =>
            {
                room.state = match *completion {
                    Completion::Failed(reason) => RemoteState::Failed(request, reason),
                    Completion::Uncertain => RemoteState::Uncertain(request),
                    Completion::ServerAccepted => RemoteState::Idle,
                };
                room.state_owner = work;
            }
            _ => (),
        }
        true
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
