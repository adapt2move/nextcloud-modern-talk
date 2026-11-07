//! Runtime-scoped history, never a credential/file store or authorization grant.
//! Ordinary confirmed content may be exported to the identity data owner;
//! restored content never restores action permissions or operation tickets.
//! At most 32 opened or prefetched account/runtime/host/room contexts retain 100
//! messages each. Eviction, access loss and account retirement also retire their
//! reply tickets: an old completion cannot recreate a removed cache entry.
use super::messaging::{Work, merge, message_view};
use super::*;
use talk_adapters::identity::{
    CacheHistory, CachedMessage, CachedReaction, CachedReply, CachedThread,
};
use talk_core::preferences::navigation::NavigationRoomKey;
use talk_core::protocol::messaging::MessagingResult;
use talk_core::ui::presentation::MessageView;
use talk_core::ui::{
    conversation::RemoteState,
    messaging::{MessagePanel, SystemEvent},
};

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
    poll_after: Option<u128>,
    older_before: Option<u128>,
    tail_repair: bool,
}

struct ReactionSnapshot {
    message: u128,
    version: Version,
    reactions: Page<protocol::messaging::Reaction, 64>,
}

struct ThreadObservation {
    details: talk_core::ui::messaging::ThreadDetails,
    version: Version,
    expires: Instant,
}

struct OpenThread {
    root: u128,
    rows: Page<MessageView, 50>,
    root_message: Option<MessageView>,
}

pub(super) struct History {
    context: ConversationRef,
    opened: bool,
    pub(super) messages: Page<MessageView, 100>,
    pub(super) older: Option<u128>,
    pub(super) latest: Option<u128>,
    common_read: Option<u128>,
    observation_version: Version,
    pub(super) state: RemoteState,
    state_owner: Work,
    pub(super) loaded: bool,
    pub(super) offline: bool,
    connectivity_version: Version,
    versions: Vec<(u128, Version)>,
    // At most 99 adjacent pairs in the retained 100-row page. Unlike mutation
    // versions, these preserve observed pagination continuity across polls.
    read_edges: Vec<(u128, u128)>,
    // Proof from an exhausted live older page. Never restored from disk.
    first_message: Option<u128>,
    // One proven hidden continuation beyond an older retained window. This
    // stores IDs only, never another message page, and is not restored from disk.
    read_tail: Option<(u128, u128)>,
    // One bounded live repair cursor, with a cooldown after empty/failing reads.
    // Cached continuity is never trusted, even when its content is displayed.
    tail_probe: Option<(u128, Instant)>,
    // One legacy classification target per retained room, with a retry cooldown.
    // This background read never establishes pagination or read continuity.
    metadata_probe: Option<(u128, Instant)>,
    // Only while an older admitted read can overwrite a confirmed mutation.
    // This also protects thread/pin rows outside the main 100-message page.
    reaction_snapshots: Vec<ReactionSnapshot>,
    // Four recently opened threads per scoped room; no permission is cached.
    threads: Vec<(u128, Page<MessageView, 50>, Option<MessageView>)>,
    // Canonical current page, including older pages that are not the cached tail.
    // Optimistic rows are projected only after this bounded baseline is restored.
    open_thread: Option<OpenThread>,
    // Current-runtime observations only; disk never restores rename authority.
    thread_details: Vec<ThreadObservation>,
    thread_activity: Page<talk_core::ui::messaging::ThreadActivity, 50>,
    thread_activity_version: Version,
    thread_activity_requested: Option<Instant>,
    capabilities: Option<(Instant, Version, protocol::messaging::MessagingCapabilities)>,
    // Same room/runtime lifetime as history; at most 50 canonical pin excerpts.
    // A confirmed write fences all already-admitted pin reads, just like edits.
    pins: Page<MessageView, 50>,
    pins_version: Version,
    pins_state: RemoteState,
    pins_loaded: bool,
    pins_read_at: Option<Instant>,
    schedules: Page<talk_core::ui::messaging::Scheduled, 100>,
    schedules_version: Version,
    translation_targets: Option<(Instant, Page<Text<64>, 64>)>,
    translations: Page<talk_core::ui::messaging::Translation, 50>,
    pub(super) summary: Option<talk_core::ui::messaging::Summary>,
    summary_dismissed: bool,
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
            observation_version: Version(0, false),
            state: RemoteState::Idle,
            state_owner: Work::History,
            loaded: false,
            offline: false,
            connectivity_version: Version(0, false),
            versions: Vec::new(),
            read_edges: Vec::new(),
            first_message: None,
            read_tail: None,
            tail_probe: None,
            metadata_probe: None,
            reaction_snapshots: Vec::new(),
            threads: Vec::new(),
            open_thread: None,
            thread_details: Vec::new(),
            thread_activity: Page::empty(),
            thread_activity_version: Version(0, false),
            thread_activity_requested: None,
            capabilities: None,
            pins: Page::empty(),
            pins_version: Version(0, false),
            pins_state: RemoteState::Idle,
            pins_loaded: false,
            pins_read_at: None,
            schedules: Page::empty(),
            schedules_version: Version(0, false),
            translation_targets: None,
            translations: Page::empty(),
            summary: None,
            summary_dismissed: false,
        }
    }
    pub(super) fn project(&self, panel: &mut MessagePanel) {
        panel.history = self.state;
        panel.older = self.older;
        panel.latest = self.latest;
        panel.common_read = self.common_read;
        self.project_open_thread(panel);
        if panel.thread_change_prior.is_none()
            && let Some(observed) = panel.thread_root.and_then(|root| {
                self.thread_details
                    .iter()
                    .find(|item| item.details.root == root)
            })
        {
            let mut details = observed.details.clone();
            if Instant::now() >= observed.expires && details.rename == Decision::Allowed {
                details.rename = Decision::Unknown;
            }
            panel.thread_details = Some(details);
        }
        // Presentation may reuse a recently observed limit/features snapshot;
        // every mutation still revalidates its room in the protocol owner.
        panel.features = Page::empty();
        panel.message_limit = 4096;
        panel.file_limit = 0;
        panel.pin_change = Decision::Unknown;
        panel.pinned_messages = self.pins.clone();
        panel.pins_state = self.pins_state;
        panel.pins_loaded = self.pins_loaded;
        let now = now_unix();
        let mut activities = self.thread_activity.as_slice().to_vec();
        for (root, rows, cached_root) in &self.threads {
            if activities.iter().any(|activity| activity.root == *root) {
                continue;
            }
            let last = rows.as_slice().iter().rev().find(|row| {
                row.local_id != *root
                    && !row.metadata.deleted
                    && !row
                        .metadata
                        .expires_at
                        .is_some_and(|at| at > 0 && at <= now)
            });
            let root_count = cached_root
                .as_ref()
                .or_else(|| {
                    self.messages
                        .as_slice()
                        .iter()
                        .find(|message| message.local_id == *root)
                })
                .map_or(0, |message| message.reply_count);
            let loaded = rows
                .as_slice()
                .iter()
                .filter(|row| row.local_id != *root)
                .count();
            if activities.len() >= 50 {
                activities.pop();
            }
            activities.push(talk_core::ui::messaging::ThreadActivity {
                root: *root,
                replies: root_count.max(u16::try_from(loaded).unwrap_or(u16::MAX)),
                last_initials: last.map(|message| message.initials.clone()),
                recent_initials: Page::empty(),
                last_time: last.map(|message| message.time_label.clone()),
                expires: last.and_then(|message| message.metadata.expires_at),
            });
        }
        panel.scheduled = self.schedules.clone();
        panel.summary = self.summary.clone();
        panel.translation_targets = self
            .translation_targets
            .as_ref()
            .filter(|(expires, _)| Instant::now() < *expires)
            .map(|(_, targets)| targets.clone());
        panel.translations = page(
            self.translations
                .as_slice()
                .iter()
                .filter(|translation| {
                    self.messages.as_slice().iter().any(|message| {
                        message.local_id == translation.message
                            && !message.metadata.deleted
                            && message.body == translation.source
                    })
                })
                .cloned(),
        );
        if let Some((expires, _, capabilities)) = &self.capabilities
            && Instant::now() < *expires
        {
            super::messaging::project_capabilities(panel, capabilities);
        }
        panel.thread_activity = page(activities.into_iter().map(|mut activity| {
            if activity.expires.is_some_and(|at| at > 0 && at <= now) {
                activity.last_initials = None;
                activity.last_time = None;
            }
            let mut recent = Vec::new();
            if let Some(initials) = &activity.last_initials {
                recent.push(initials.clone());
            }
            if let Some(rows) = self.thread(activity.root) {
                for reply in rows.as_slice().iter().rev().filter(|reply| {
                    reply.local_id != activity.root
                        && !reply.metadata.deleted
                        && !reply
                            .metadata
                            .expires_at
                            .is_some_and(|at| at > 0 && at <= now)
                }) {
                    if !recent.contains(&reply.initials) {
                        recent.push(reply.initials.clone());
                    }
                    if recent.len() == 3 {
                        break;
                    }
                }
            }
            activity.recent_initials = page(recent);
            activity
        }));
        if let Some((expires, _, capabilities)) = &self.capabilities {
            if Instant::now() < *expires {
                super::messaging::project_capabilities(panel, capabilities);
            } else {
                // The last capability bit is only a local staging hint.
                // Discard grants; dispatch always obtains current server policy.
                let mut prior = MessagePanel::new(self.context);
                super::messaging::project_capabilities(&mut prior, capabilities);
                use talk_core::ui::messaging::MessageFeature;
                for feature in [
                    MessageFeature::Threads,
                    MessageFeature::Reply,
                    MessageFeature::Edit,
                    MessageFeature::React,
                ] {
                    let mut value = prior.availability(feature);
                    if value.allowed == Decision::Allowed {
                        value.allowed = Decision::Unknown;
                    }
                    if value.implemented
                        && value.server != talk_core::ui::presentation::FeatureSupport::Unknown
                    {
                        let _ = panel.features.try_push((feature, value));
                    }
                }
            }
        }
    }
    pub(super) fn project_open_thread(&self, panel: &mut MessagePanel) {
        if let Some(thread) = self
            .open_thread
            .as_ref()
            .filter(|thread| panel.thread_root == Some(thread.root))
        {
            panel.thread_messages = thread.rows.clone();
            panel.thread_root_message = thread.root_message.clone();
        }
    }
    pub(super) fn open_thread_root(&self, root: u128) -> Option<&MessageView> {
        self.open_thread
            .as_ref()
            .filter(|thread| thread.root == root)
            .and_then(|thread| thread.root_message.as_ref())
    }
    pub(super) fn visible_messages(&self) -> Page<MessageView, 100> {
        page(
            self.messages
                .as_slice()
                .iter()
                .filter(|message| !message.metadata.pin_notice)
                .map(|message| {
                    let mut visible = message.clone();
                    if let Some(activity) = self
                        .thread_activity
                        .as_slice()
                        .iter()
                        .find(|activity| activity.root == message.local_id)
                        && !message.metadata.deleted
                    {
                        visible.reply_count = activity.replies;
                    } else if let Some(rows) = self.thread(message.local_id)
                        && !message.metadata.deleted
                    {
                        let loaded = rows
                            .as_slice()
                            .iter()
                            .filter(|row| row.local_id != message.local_id)
                            .count();
                        visible.reply_count = visible
                            .reply_count
                            .max(u16::try_from(loaded).unwrap_or(u16::MAX));
                    }
                    let target = self.read_through_pin_notices(message.local_id);
                    visible.metadata.read_through = (target != message.local_id).then_some(target);
                    visible.metadata.unread_after = self.predecessor(message.local_id);
                    visible
                }),
        )
    }

    pub(super) fn thread(&self, root: u128) -> Option<&Page<MessageView, 50>> {
        self.threads
            .iter()
            .find(|(id, _, _)| *id == root)
            .map(|(_, rows, _)| rows)
    }

    pub(super) fn thread_root(&self, root: u128) -> Option<&MessageView> {
        self.threads
            .iter()
            .find(|(id, _, _)| *id == root)
            .and_then(|(_, _, message)| message.as_ref())
    }

    pub(super) fn needs_thread_activity(&self, now: Instant) -> bool {
        self.thread_activity_requested
            .is_none_or(|at| now.saturating_duration_since(at) >= Duration::from_secs(15))
    }

    /// Only a live page's adjacent IDs can identify the server's preceding
    /// message. Cached pages and independently merged gaps confer no proof.
    pub(super) fn predecessor(&self, id: u128) -> Option<u128> {
        self.read_edges
            .iter()
            .find_map(|(previous, next)| (*next == id && *previous > 0).then_some(*previous))
            .or_else(|| {
                (self.first_message == Some(id)
                    && self
                        .messages
                        .as_slice()
                        .first()
                        .is_some_and(|row| row.local_id == id))
                .then_some(0)
            })
    }

    /// Invisible pin activity carries no additional user content. A deliberate
    /// read of its preceding visible row may include only that adjacent run,
    /// never the next ordinary message or an unloaded server gap.
    pub(super) fn read_through_pin_notices(&self, id: u128) -> u128 {
        let Some(index) = self
            .messages
            .as_slice()
            .iter()
            .position(|message| message.local_id == id)
        else {
            return id;
        };
        let mut target = id;
        for message in self.messages.as_slice().iter().skip(index + 1) {
            if !message.metadata.system
                || !message.metadata.pin_notice
                || !self.read_edges.contains(&(target, message.local_id))
            {
                break;
            }
            target = message.local_id;
        }
        self.read_tail
            .filter(|(anchor, _)| *anchor == target)
            .map_or(target, |(_, end)| end)
    }
    /// A response page proves its own adjacency. A forward poll additionally
    /// proves the edge from its admitted event cursor to the first returned row.
    /// Independently fetched pages, cached data and mutation results prove no gap.
    fn observe_read_page(&mut self, incoming: &[(u128, bool)], after: Option<u128>) {
        let mut edges = Vec::with_capacity(99);
        for pair in self.messages.as_slice().windows(2) {
            let edge = (pair[0].local_id, pair[1].local_id);
            if self.read_edges.contains(&edge)
                || incoming
                    .windows(2)
                    .any(|pair| (pair[0].0, pair[1].0) == edge)
                || after == Some(edge.0) && incoming.first().map(|row| row.0) == Some(edge.1)
            {
                edges.push(edge);
            }
        }
        self.read_edges = edges;
        let Some(anchor) = self
            .messages
            .as_slice()
            .last()
            .map(|message| message.local_id)
        else {
            self.read_tail = None;
            return;
        };
        self.read_tail = self.read_tail.filter(|(known, _)| *known == anchor);
        if incoming.iter().any(|(id, _)| *id > anchor) {
            let start = incoming
                .iter()
                .position(|(id, _)| *id == anchor)
                .map(|index| index + 1)
                .or_else(|| {
                    (after == Some(anchor)
                        || self.read_tail.is_some_and(|(_, end)| after == Some(end)))
                    .then_some(0)
                });
            self.read_tail = start.and_then(|index| {
                let suffix = incoming.get(index..)?;
                if suffix.iter().all(|(id, hidden)| *id > anchor && *hidden) {
                    suffix.last().map(|row| (anchor, row.0))
                } else {
                    None
                }
            });
        }
    }
    pub(super) fn poll_cursor(&self, now: Instant) -> Option<u128> {
        let latest = self.latest?;
        let Some(last_visible) = self
            .messages
            .as_slice()
            .iter()
            .rev()
            .find(|message| !message.metadata.pin_notice)
            .map(|message| message.local_id)
        else {
            return Some(latest);
        };
        let through = self.read_through_pin_notices(last_visible);
        let unknown_tail = self.messages.as_slice().last().is_some_and(|message| {
            message.metadata.system && message.metadata.pin_notice && message.local_id > through
        });
        if unknown_tail
            && self.tail_probe.is_none_or(|(cursor, at)| {
                cursor != through || now.saturating_duration_since(at) >= Duration::from_secs(30)
            })
        {
            Some(through)
        } else {
            Some(latest)
        }
    }
    fn metadata_target(&self) -> Option<u128> {
        self.loaded
            .then(|| {
                self.messages
                    .as_slice()
                    .iter()
                    .rev()
                    .find(|message| {
                        message.metadata.system
                            && message.metadata.system_event == SystemEvent::Unknown
                    })
                    .map(|message| message.local_id)
            })
            .flatten()
    }
    /// At most 50 server rows before the newest unclassified retained row.
    /// No-progress/failing targets wait 30 seconds; a changed target can progress
    /// on the next owner tick without retaining another page or a retry queue.
    pub(super) fn metadata_before(&self, now: Instant) -> Option<u128> {
        let target = self.metadata_target()?;
        if self.metadata_probe.is_some_and(|(known, at)| {
            known == target && now.saturating_duration_since(at) < Duration::from_secs(30)
        }) {
            return None;
        }
        u64::try_from(target).ok()?.checked_add(1).map(u128::from)
    }
    pub(super) fn read_reaches_latest(&self, id: u128) -> bool {
        self.loaded
            && self.latest.is_some_and(|latest| latest <= id)
            && self
                .messages
                .as_slice()
                .last()
                .is_none_or(|message| message.local_id <= id)
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
        let expired: Vec<_> = self
            .messages
            .as_slice()
            .iter()
            .chain(self.pins.as_slice())
            .chain(
                self.threads
                    .iter()
                    .flat_map(|(_, rows, root)| rows.as_slice().iter().chain(root.iter())),
            )
            .chain(self.open_thread.iter().flat_map(|thread| {
                thread
                    .rows
                    .as_slice()
                    .iter()
                    .chain(thread.root_message.iter())
            }))
            .filter(|m| m.metadata.expires_at.is_some_and(|at| at > 0 && at <= now))
            .map(|m| m.local_id)
            .collect();
        if expired.is_empty() {
            return;
        }
        let retain = |message: &MessageView| {
            if expired.contains(&message.local_id) {
                return None;
            }
            let mut message = message.clone();
            if let Some(parent) = message.metadata.parent.as_mut()
                && !parent.external
                && expired.contains(&parent.id)
            {
                parent.unavailable = true;
                parent.text = text("");
            }
            Some(message)
        };
        self.messages = page(self.messages.as_slice().iter().filter_map(retain));
        self.pins = page(self.pins.as_slice().iter().filter_map(retain));
        for (_, rows, root) in &mut self.threads {
            *rows = page(rows.as_slice().iter().filter_map(retain));
            *root = root.as_ref().and_then(retain);
        }
        if let Some(thread) = self.open_thread.as_mut() {
            thread.rows = page(thread.rows.as_slice().iter().filter_map(retain));
            thread.root_message = thread.root_message.as_ref().and_then(retain);
        }
        self.versions.retain(|(id, _)| !expired.contains(id));
        self.read_edges
            .retain(|(from, to)| !expired.contains(from) && !expired.contains(to));
        self.read_tail = self
            .read_tail
            .filter(|(anchor, _)| !expired.contains(anchor));
    }

    /// Deletions are terminal observations. Scrub every retained thread copy,
    /// including closed panes that do not currently receive row reconciliation.
    fn redact_threads(&mut self, deleted: &[u128]) {
        if deleted.is_empty() {
            return;
        }
        let redact = |message: &mut MessageView| {
            if deleted.contains(&message.local_id) {
                message.body = text("");
                message.attachments = Page::empty();
                message.metadata.deleted = true;
                message.metadata.reactions = Page::empty();
                message.metadata.rich = Page::empty();
                message.metadata.replyable = false;
            }
            if let Some(parent) = message.metadata.parent.as_mut()
                && !parent.external
                && deleted.contains(&parent.id)
            {
                parent.unavailable = true;
                parent.text = text("");
            }
        };
        for (_, rows, root) in &mut self.threads {
            *rows = page(rows.as_slice().iter().cloned().map(|mut message| {
                redact(&mut message);
                message
            }));
            if let Some(root) = root {
                redact(root);
            }
        }
        if let Some(thread) = self.open_thread.as_mut() {
            thread.rows = page(thread.rows.as_slice().iter().cloned().map(|mut message| {
                redact(&mut message);
                message
            }));
            if let Some(root) = thread.root_message.as_mut() {
                redact(root);
            }
        }
    }
}

#[derive(Default)]
pub(super) struct Cache {
    rooms: Vec<History>,
    flights: Vec<Flight>,
    last_admission: u64,
}
impl Cache {
    pub(super) fn ensure_open_thread(&mut self, context: ConversationRef, panel: &MessagePanel) {
        let Some(room) = self.get_mut(context) else {
            return;
        };
        let Some(root) = panel.thread_root else {
            room.open_thread = None;
            return;
        };
        if room
            .open_thread
            .as_ref()
            .is_some_and(|thread| thread.root == root)
        {
            return;
        }
        room.open_thread = Some(OpenThread {
            root,
            // Locally staged IDs occupy the upper half of u128; server IDs are
            // u64. A reopened baseline must never adopt projected intents.
            rows: page(
                panel
                    .thread_messages
                    .as_slice()
                    .iter()
                    .filter(|row| row.local_id <= u128::from(u64::MAX))
                    .cloned(),
            ),
            root_message: panel.thread_root_message.clone(),
        });
    }
    pub(super) fn remember_open_thread(
        &mut self,
        context: ConversationRef,
        root: u128,
        rows: Page<MessageView, 50>,
        root_message: Option<MessageView>,
    ) {
        if let Some(room) = self.get_mut(context)
            && room
                .open_thread
                .as_ref()
                .is_some_and(|thread| thread.root == root)
        {
            room.open_thread = Some(OpenThread {
                root,
                rows,
                root_message,
            });
        }
    }
    pub(super) fn remember_thread(
        &mut self,
        context: ConversationRef,
        root: u128,
        rows: Page<MessageView, 50>,
        root_message: Option<MessageView>,
    ) {
        let Some(room) = self.get_mut(context) else {
            return;
        };
        let previous_root = room.thread_root(root).cloned();
        room.threads.retain(|(id, _, _)| *id != root);
        if room.threads.len() >= 4 {
            room.threads.remove(0);
        }
        room.threads
            .push((root, rows, root_message.or(previous_root)));
    }
    /// Record the actual dispatched cursor, including a bounded hidden-tail
    /// repair. The flight must not infer it from a newer canonical event cursor.
    pub(super) fn older_admitted(
        &mut self,
        context: ConversationRef,
        operation: OperationId,
        before: u128,
    ) {
        if let Some(flight) = self.flights.iter_mut().find(|flight| {
            flight.context == context && flight.operation == operation && flight.work == Work::Older
        }) {
            flight.older_before = Some(before);
        }
    }
    pub(super) fn poll_admitted(
        &mut self,
        context: ConversationRef,
        operation: OperationId,
        after: u128,
    ) {
        let repair = self
            .get(context)
            .is_some_and(|room| room.latest.is_some_and(|latest| after < latest));
        let Some(flight) = self.flights.iter_mut().find(|flight| {
            flight.context == context
                && flight.operation == operation
                && matches!(flight.work, Work::Poll | Work::UnreadContext(_))
        }) else {
            return;
        };
        flight.poll_after = Some(after);
        let is_poll = flight.work == Work::Poll;
        flight.tail_repair = is_poll && repair;
        if is_poll
            && let Some(room) = self.get_mut(context)
            && room.latest.is_some_and(|latest| after < latest)
        {
            room.tail_probe = Some((after, Instant::now()));
        }
    }
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
    pub(super) fn dismiss_summary(&mut self, context: ConversationRef) {
        if let Some(room) = self.get_mut(context) {
            room.summary = None;
            room.summary_dismissed = true;
        }
    }
    pub(super) fn set_summary(
        &mut self,
        context: ConversationRef,
        value: talk_core::ui::messaging::Summary,
    ) -> bool {
        if let Some(room) = self.get_mut(context) {
            room.summary = Some(value);
            true
        } else {
            false
        }
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
            threads: page(
                room.threads
                    .iter()
                    .filter_map(|(root, replies, root_message)| {
                        let root = u64::try_from(*root).ok().filter(|root| *root > 0)?;
                        let confirmed =
                            |message: &MessageView| {
                                matches!(message.delivery,
                        talk_core::ui::presentation::DeliveryState::Received
                        | talk_core::ui::presentation::DeliveryState::ServerAccepted)
                            };
                        Some(CachedThread {
                            root,
                            root_message: root_message
                                .as_ref()
                                .filter(|message| confirmed(message))
                                .and_then(cached_message),
                            replies: page(
                                replies
                                    .as_slice()
                                    .iter()
                                    .filter(|message| confirmed(message))
                                    .filter_map(|message| {
                                        let mut cached = cached_message(message)?;
                                        if cached.id != root {
                                            cached.thread = Some(root);
                                        }
                                        Some(cached)
                                    }),
                            ),
                        })
                    }),
            ),
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
        room.threads = cached
            .threads
            .as_slice()
            .iter()
            .map(|thread| {
                (
                    u128::from(thread.root),
                    page(thread.replies.as_slice().iter().map(restored_message)),
                    thread.root_message.as_ref().map(restored_message),
                )
            })
            .collect();
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
        let poll_after = (work == Work::Poll)
            .then(|| self.get(context).and_then(|room| room.latest))
            .flatten();
        let started = Instant::now();
        self.flights.push(Flight {
            context,
            operation,
            request,
            work,
            version: Version(serial, false),
            started,
            poll_after,
            older_before: None,
            tail_repair: false,
        });
        if work == Work::Metadata
            && let Some(room) = self.get_mut(context)
            && let Some(target) = room.metadata_target()
        {
            room.metadata_probe = Some((target, started));
        }
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
        if work == Work::ThreadActivity
            && let Some(room) = self.get_mut(context)
        {
            room.thread_activity_requested = Some(started);
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
                    && flight.version >= room.observation_version
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
        let deleted = match result {
            Ok(MessagingResult::Message(message)) => vec![message.as_ref()],
            Ok(MessagingResult::Messages { messages, .. }) => messages.as_slice().iter().collect(),
            _ => Vec::new(),
        }
        .into_iter()
        .filter(|message| message.message.kind == protocol::MessageKind::Deleted)
        .map(|message| u128::from(message.message.id.sequence))
        .collect::<Vec<_>>();
        room.redact_threads(&deleted);
        if work == Work::Metadata
            && let Some((_, at)) = room.metadata_probe.as_mut()
        {
            // Failures and no-progress replies get a full cooldown after the
            // request settles, including when the GET itself was slow.
            *at = Instant::now();
        }
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
        if work == Work::ThreadActivity {
            // Slow requests receive the same cooldown as immediate failures.
            room.thread_activity_requested = Some(Instant::now());
            if flight.version >= room.thread_activity_version
                && let Ok(MessagingResult::Threads(threads)) = result
            {
                room.thread_activity = page(threads.as_slice().iter().map(|thread| {
                    let last = thread.last_reply.as_ref();
                    talk_core::ui::messaging::ThreadActivity {
                        root: u128::from(thread.root.sequence),
                        replies: u16::try_from(thread.replies).unwrap_or(u16::MAX),
                        last_initials: last.map(|last| initials(last.author.as_str())),
                        recent_initials: Page::empty(),
                        last_time: last
                            .map(|last| text(&super::messaging::time_label(last.timestamp))),
                        expires: last.and_then(|last| last.expires),
                    }
                }));
                room.thread_activity_version = flight.version;
            }
        }
        if let Work::ThreadInfo(root) | Work::ThreadChange(root) | Work::ThreadQueued(root) = work
            && let Ok(MessagingResult::Thread(thread)) = result
            && thread.root.conversation == context.conversation
            && u128::from(thread.root.sequence) == root
        {
            let version = if matches!(work, Work::ThreadChange(_) | Work::ThreadQueued(_)) {
                mutation_version
            } else {
                flight.version
            };
            let old = room
                .thread_details
                .iter()
                .position(|item| item.details.root == root);
            if old.is_none_or(|index| room.thread_details[index].version <= version) {
                if let Some(index) = old {
                    room.thread_details.remove(index);
                }
                if room.thread_details.len() >= 4 {
                    room.thread_details.remove(0);
                }
                room.thread_details.push(ThreadObservation {
                    details: talk_core::ui::messaging::ThreadDetails {
                        root,
                        title: thread.title.clone(),
                        notification_level: thread.notification_level,
                        rename: thread.rename,
                    },
                    version,
                    expires: flight.started + background::SOURCE_LIFETIME,
                });
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
        match result {
            Ok(MessagingResult::Scheduled(values))
                if matches!(work, Work::Scheduled | Work::ScheduledPoll)
                    && flight.version >= room.schedules_version =>
            {
                room.schedules = page(
                    values
                        .as_slice()
                        .iter()
                        .map(super::messaging::scheduled_view),
                );
                room.schedules_version = flight.version;
            }
            Ok(MessagingResult::TranslationLanguages(targets))
                if work == Work::TranslationOptions =>
            {
                room.translation_targets =
                    Some((Instant::now() + Duration::from_secs(600), targets.clone()));
            }
            Ok(MessagingResult::Translation {
                message,
                source,
                language,
                text,
            }) if matches!(work, Work::Translate(id) if id == u128::from(message.sequence)) => {
                let id = u128::from(message.sequence);
                if room
                    .messages
                    .as_slice()
                    .iter()
                    .any(|row| row.local_id == id && !row.metadata.deleted && row.body == *source)
                {
                    room.translations = page(
                        room.translations
                            .as_slice()
                            .iter()
                            .filter(|known| known.message != id || known.language != *language)
                            .take(49)
                            .cloned()
                            .chain(std::iter::once(talk_core::ui::messaging::Translation {
                                message: id,
                                source: source.clone(),
                                language: language.clone(),
                                text: text.clone(),
                            })),
                    );
                }
            }
            Ok(MessagingResult::Schedule(value)) => {
                let value = super::messaging::scheduled_view(value);
                let id = value.id;
                room.schedules = page(
                    room.schedules
                        .as_slice()
                        .iter()
                        .filter(|known| known.id != id)
                        .take(99)
                        .cloned()
                        .chain(std::iter::once(value)),
                );
                room.schedules_version = mutation_version;
            }
            Ok(MessagingResult::Changed) if matches!(work, Work::CancelScheduled(_)) => {
                if let Work::CancelScheduled(id) = work {
                    room.schedules = page(
                        room.schedules
                            .as_slice()
                            .iter()
                            .filter(|known| known.id != id)
                            .cloned(),
                    );
                    room.schedules_version = mutation_version;
                }
            }
            _ => {}
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
            if !room.summary_dismissed
                && room.summary.is_none()
                && let Some(offer) = &capabilities.summary_offer
                && offer.from.conversation == context.conversation
            {
                room.summary = Some(talk_core::ui::messaging::Summary::Offer {
                    from: u128::from(offer.from.sequence),
                    unread: offer.unread,
                });
            }
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
                | Work::UnreadContext(_)
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
            Ok(MessagingResult::Messages { messages, .. }) if work == Work::Metadata => {
                // Reconcile only IDs still retained at completion, under the
                // ordinary row/mutation Version fence. Unseen rows prove no gap
                // and cannot insert, evict, mark read or move either cursor.
                let retained: Vec<_> = messages
                    .as_slice()
                    .iter()
                    .take(50)
                    .filter(|message| {
                        room.messages
                            .as_slice()
                            .iter()
                            .any(|known| known.local_id == u128::from(message.message.id.sequence))
                    })
                    .map(message_view)
                    .collect();
                room.merge(retained.into_iter(), flight.version, false);
            }
            Ok(MessagingResult::Messages {
                messages,
                cursor,
                common_read,
                ..
            }) if timeline => {
                let was_loaded = room.loaded;
                if work == Work::Older
                    && messages.as_slice().is_empty()
                    && cursor.is_none()
                    && flight.older_before.is_some()
                    && room.messages.as_slice().first().map(|row| row.local_id)
                        == flight.older_before
                {
                    room.first_message = flight.older_before;
                }
                let incoming: Vec<_> = messages
                    .as_slice()
                    .iter()
                    .take(50)
                    .map(message_view)
                    .collect();
                let mut observed: Vec<_> = incoming
                    .iter()
                    .map(|message| {
                        (
                            message.local_id,
                            message.metadata.system && message.metadata.pin_notice,
                        )
                    })
                    .collect();
                observed.sort_unstable_by_key(|row| row.0);
                observed.dedup_by_key(|row| row.0);
                // Invisible activity must not evict the reader's visible
                // anchor. Its verified continuation uses only the ID proof
                // below; the retained message window stays capped at 100.
                let hidden_poll = work == Work::Poll
                    && incoming
                        .iter()
                        .all(|message| message.metadata.system && message.metadata.pin_notice);
                room.merge(
                    incoming.into_iter(),
                    flight.version,
                    flight.tail_repair
                        || hidden_poll
                        || work == Work::Older
                        || matches!(work, Work::Context(id) | Work::UnreadContext(id)
                        if room.messages.as_slice().first().is_some_and(|m| id < m.local_id)),
                );
                if !matches!(work, Work::Recovery | Work::Context(_)) {
                    room.observe_read_page(&observed, flight.poll_after);
                }
                if work == Work::Older
                    || work == Work::History && !was_loaded
                    || matches!(work, Work::Latest | Work::Prefetch) && room.latest.is_none()
                {
                    room.older = cursor.map(|id| u128::from(id.sequence));
                } else if matches!(work, Work::Context(_) | Work::UnreadContext(_)) {
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
                if flight.version >= room.observation_version && work != Work::Recovery {
                    // Missing headers on an older page are not a revocation.
                    // Explicit zero still clears receipts (e.g. read privacy).
                    if let Some(read) = common_read {
                        room.common_read = Some(u128::from(read.sequence));
                    }
                    room.observation_version = flight.version;
                }
                room.loaded = true;
                if (!work.background()
                    || room.state_owner.background()
                    || room.state_owner == Work::History
                    || room.state == RemoteState::Idle)
                    && (!work.background() || flight.version >= room.observation_version)
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
                    && !matches!(work, Work::Context(_) | Work::UnreadContext(_))
                    && (work == Work::Older || flight.version >= room.connectivity_version)
                    && (!work.background()
                        || (room.state_owner.background() || room.state == RemoteState::Idle)
                            && flight.version >= room.observation_version) =>
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
        locations: page(
            message
                .metadata
                .rich
                .as_slice()
                .iter()
                .filter(|o| !message.metadata.deleted && o.kind.as_str() == "geo-location")
                .filter_map(|o| {
                    o.id.as_ref()
                        .filter(|id| {
                            talk_core::ui::markup::location_coordinates(id.as_str()).is_some()
                        })
                        .map(|id| talk_adapters::identity::CachedLocation {
                            placeholder: o.placeholder.clone(),
                            name: o.name.clone(),
                            id: id.clone(),
                        })
                })
                .take(8),
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
        voice_message: message.metadata.voice_message,
        system_event: cached_system_event(message.metadata.system, message.metadata.system_event),
        pin_notice: message.metadata.system && message.metadata.pin_notice,
        truncated: message.metadata.truncated,
        replyable: message.metadata.replyable,
        parent: message.metadata.parent.as_ref().and_then(|p| {
            Some(CachedReply {
                id: u64::try_from(p.id).ok()?,
                external: p.external,
                author: p.author.clone(),
                text: p.text.clone(),
                unavailable: p.unavailable,
            })
        }),
        thread: message
            .metadata
            .thread
            .and_then(|id| u64::try_from(id).ok()),
        thread_replies: message.reply_count,
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
        reply_count: message.thread_replies,
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
                    })
                    .chain(
                        message
                            .locations
                            .as_slice()
                            .iter()
                            .filter(|o| {
                                !message.deleted
                                    && talk_core::ui::markup::location_coordinates(o.id.as_str())
                                        .is_some()
                            })
                            .map(|o| talk_core::ui::messaging::RichObject {
                                placeholder: o.placeholder.clone(),
                                kind: text("geo-location"),
                                name: o.name.clone(),
                                mime_type: None,
                                size: None,
                                id: Some(o.id.clone()),
                                link: None,
                            }),
                    ),
            ),
            author_key: message.actor_key.clone(),
            expires_at: message.expires_at,
            timestamp: message.timestamp,
            truncated: message.truncated,
            edited: message.edited,
            deleted: message.deleted,
            system: message.system,
            voice_message: message.voice_message,
            system_event: restored_system_event(message.system, message.system_event.as_ref()),
            pin_notice: message.system && message.pin_notice,
            replyable: message.replyable,
            parent: message
                .parent
                .as_ref()
                .map(|p| talk_core::ui::messaging::Reply {
                    id: u128::from(p.id),
                    external: p.external,
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

fn cached_system_event(system: bool, event: SystemEvent) -> Option<Text<32>> {
    if !system {
        return None;
    }
    Some(text(match event {
        SystemEvent::Unknown => return None,
        SystemEvent::Other => "other",
        SystemEvent::CallIncoming => "call_incoming",
        SystemEvent::CallOutgoing => "call_outgoing",
        SystemEvent::CallJoined => "call_joined",
        SystemEvent::CallLeft => "call_left",
        SystemEvent::CallEnded => "call_ended",
        SystemEvent::CallMissed => "call_missed",
        SystemEvent::CallUnanswered => "call_unanswered",
    }))
}

fn restored_system_event(system: bool, code: Option<&Text<32>>) -> SystemEvent {
    if !system {
        return SystemEvent::Unknown;
    }
    match code.map(Text::as_str) {
        Some("other") => SystemEvent::Other,
        Some("call_incoming") => SystemEvent::CallIncoming,
        Some("call_outgoing") => SystemEvent::CallOutgoing,
        Some("call_joined") => SystemEvent::CallJoined,
        Some("call_left") => SystemEvent::CallLeft,
        Some("call_ended") => SystemEvent::CallEnded,
        Some("call_missed") => SystemEvent::CallMissed,
        Some("call_unanswered") => SystemEvent::CallUnanswered,
        _ => SystemEvent::Unknown,
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
