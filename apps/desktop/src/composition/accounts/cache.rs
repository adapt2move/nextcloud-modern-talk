//! Composition owns bounded cache projections and admission, never file I/O.
//! Local runtime/host zero is a read-only presentation identity: only a freshly
//! restored account and room observation can authorize protocol dispatch.
use super::*;
use talk_adapters::identity::{
    CACHE_THUMBNAIL_BYTES, CACHE_THUMBNAIL_LIFETIME, CACHE_THUMBNAIL_LIMIT, CacheHistory,
    CacheRoom, CacheThumbnail, CachedRoomKind, DataCommand, DataResult, DataSnapshot,
};
use talk_core::{
    ids::{ConversationId, HostId, RuntimeGeneration},
    preferences::navigation::NavigationRoomKey,
};

const WRITES: usize = 24;
const CLEANUP_RETRY: Duration = Duration::from_secs(5);
struct CleanupFailure {
    account: AccountId,
    generation: u64,
    retry_at: Instant,
}
#[derive(Clone, Copy)]
pub(super) enum Work {
    Load(AccountId, u64),
    Write(AccountId, u64),
}
struct Binding {
    context: ConversationRef,
    key: NavigationRoomKey,
    ordinary: bool,
}
struct Account {
    id: AccountId,
    generation: u64,
    scope: Option<AccountScope>,
    observed: bool,
    loaded: bool,
    snapshot: Option<DataSnapshot>,
    bindings: Vec<Binding>,
    hydrated: Option<AccountScope>,
    load_attempted: bool,
}
#[derive(Clone)]
struct Write {
    account: AccountId,
    generation: u64,
    command: DataCommand,
}
#[derive(Default)]
pub(super) struct Cache {
    accounts: Vec<Account>,
    writes: Vec<Write>,
    in_flight: Option<Write>,
    selection: Option<(AccountId, NavigationRoomKey)>,
    next_generation: u64,
    thumbnail_denials: Vec<(AccountId, u64, NavigationRoomKey, u64, Text<128>)>,
    thumbnail_load_blocked: Vec<(AccountId, u64)>,
    // At most one cooldown per retained account (eight). Cleanup itself stays
    // in the existing 24-write budget and collapses to a preview-only clear.
    cleanup_failures: Vec<CleanupFailure>,
}
fn required_cleanup(command: &DataCommand) -> bool {
    matches!(
        command,
        DataCommand::RemoveThumbnail { .. } | DataCommand::ClearThumbnails
    )
}
impl Cache {
    fn write_ready(&self, now: Instant) -> Option<usize> {
        let current = |w: &Write| {
            self.accounts
                .iter()
                .any(|a| a.id == w.account && a.generation == w.generation)
        };
        self.writes
            .iter()
            .position(|w| {
                current(w)
                    && required_cleanup(&w.command)
                    && !self.cleanup_failures.iter().any(|f| {
                        f.account == w.account && f.generation == w.generation && now < f.retry_at
                    })
            })
            .or_else(|| {
                self.writes
                    .iter()
                    .position(|w| current(w) && !required_cleanup(&w.command))
            })
    }
    fn cleanup_failed(&mut self, account: AccountId, generation: u64, now: Instant) -> bool {
        if !self
            .accounts
            .iter()
            .any(|a| a.id == account && a.generation == generation)
        {
            return false;
        }
        let first = if let Some(failure) = self
            .cleanup_failures
            .iter_mut()
            .find(|f| f.account == account && f.generation == generation)
        {
            failure.retry_at = now + CLEANUP_RETRY;
            false
        } else {
            self.cleanup_failures.push(CleanupFailure {
                account,
                generation,
                retry_at: now + CLEANUP_RETRY,
            });
            true
        };
        self.enqueue(account, DataCommand::ClearThumbnails);
        first
    }
    fn cleanup_saved(&mut self, account: AccountId, generation: u64) -> bool {
        let old = self.cleanup_failures.len();
        self.cleanup_failures
            .retain(|f| f.account != account || f.generation != generation);
        old != self.cleanup_failures.len()
    }
    pub(super) fn loaded(&self, account: AccountId) -> Option<&DataSnapshot> {
        self.accounts
            .iter()
            .find(|a| a.id == account && a.loaded)?
            .snapshot
            .as_ref()
    }
    pub(super) fn is_local(&self, scope: AccountScope) -> bool {
        scope.runtime.as_local() == 0 && self.accounts.iter().any(|a| a.id == scope.account)
    }
    pub(super) fn is_presentation(&self, context: ConversationRef) -> bool {
        context.conversation.host.as_local() == 0 && self.stable_room(context).is_some()
    }
    pub(super) fn local_scope(&self, account: AccountId) -> Option<AccountScope> {
        self.loaded(account)
            .filter(|s| !s.rooms.as_slice().is_empty())
            .map(|_| AccountScope {
                account,
                runtime: RuntimeGeneration::from_local(0),
            })
    }
    pub(super) fn stable_room(&self, context: ConversationRef) -> Option<NavigationRoomKey> {
        self.accounts
            .iter()
            .find(|a| a.id == context.account.account)?
            .bindings
            .iter()
            .find(|b| b.context == context)
            .map(|b| b.key.clone())
    }
    pub(super) fn context(
        &self,
        account: AccountId,
        key: &NavigationRoomKey,
    ) -> Option<ConversationRef> {
        self.accounts
            .iter()
            .find(|a| a.id == account)?
            .bindings
            .iter()
            .find(|b| {
                &b.key == key
                    && b.context.account.runtime.as_local() != 0
                    && b.context.conversation.host.as_local() != 0
            })
            .map(|b| b.context)
    }
    pub(super) fn persistable(&self, context: ConversationRef) -> bool {
        self.accounts
            .iter()
            .find(|a| a.id == context.account.account)
            .is_some_and(|a| {
                a.bindings
                    .iter()
                    .any(|b| b.context == context && b.ordinary)
            })
    }
    pub(super) fn metadata_pending(&self, account: AccountId, key: &NavigationRoomKey) -> bool {
        self.writes.iter().chain(self.in_flight.iter()).any(|w|w.account==account && matches!(&w.command,DataCommand::PutRooms(rooms) if rooms.as_slice().iter().any(|r|&r.key==key)))
    }
    /// Revocation requires a fresh complete room list in the current account
    /// runtime. Missing disk metadata and offline presentation are not denials.
    pub(super) fn revoked(&self, account: AccountId, key: &NavigationRoomKey) -> bool {
        self.accounts
            .iter()
            .find(|a| a.id == account)
            .is_some_and(|a| {
                a.observed
                    && a.scope.is_some_and(|s| s.runtime.as_local() != 0)
                    && !a.bindings.iter().any(|b| b.key == *key && b.ordinary)
            })
    }
    pub(super) fn enqueue(&mut self, account: AccountId, command: DataCommand) {
        let Some(generation) = self
            .accounts
            .iter()
            .find(|a| a.id == account)
            .map(|a| a.generation)
        else {
            return;
        };
        if matches!(command, DataCommand::RemoveThumbnail { .. })
            && self
                .writes
                .iter()
                .any(|w| w.account == account && matches!(w.command, DataCommand::ClearThumbnails))
        {
            return;
        }
        // Cache observations and required preview cleanup share this budget,
        // independently of user intents. Coalesce disposable projections while
        // retaining cleanup and the latest complete room policy under overload.
        self.writes.retain(|w| {
            if w.account != account {
                return true;
            }
            if matches!(command,DataCommand::ClearThumbnails) {return !matches!(w.command,DataCommand::PutThumbnail(_) | DataCommand::RemoveThumbnail {..} | DataCommand::ClearThumbnails);}
            if let DataCommand::RemoveThumbnail {room,message,placeholder}=&command
                && matches!(&w.command,DataCommand::PutThumbnail(t) if t.room==*room && t.message==*message && t.placeholder==*placeholder) {return false;}
            if let DataCommand::PutRooms(rooms) = &command {
                return match &w.command {
                    DataCommand::PutRooms(_) | DataCommand::RemoveRoom(_) => false,
                    DataCommand::PutHistory(history) => {
                        rooms.as_slice().iter().any(|r| r.key == history.room)
                    }
                    DataCommand::PutThumbnail(thumbnail) => {
                        rooms.as_slice().iter().any(|r| r.key == thumbnail.room)
                    }
                    DataCommand::MemberCount { .. } => false,
                    _ => true,
                };
            }
            if let DataCommand::PutHistory(history) = &command
                && let DataCommand::PutThumbnail(thumbnail) = &w.command
                && thumbnail.room == history.room
                && !thumbnail_history_matches(history, thumbnail, history::now_unix())
            {
                return false;
            }
            !same_slot(&w.command, &command)
        });
        if self.writes.len() == WRITES {
            // At most one complete policy snapshot per eight accounts. Never
            // discard that invalidation under history/metadata refresh pressure.
            if let Some(oldest) = self
                .writes
                .iter()
                .position(|w| matches!(w.command, DataCommand::PutThumbnail(_)))
            {
                self.writes.remove(oldest);
            } else if matches!(command, DataCommand::PutThumbnail(_)) {
                // A disposable image cannot displace a queued room policy or
                // history snapshot; the in-memory preview remains available.
                return;
            } else if let Some(oldest) = self.writes.iter().position(|w| {
                !matches!(
                    w.command,
                    DataCommand::PutRooms(_)
                        | DataCommand::RemoveThumbnail { .. }
                        | DataCommand::ClearThumbnails
                )
            }) {
                self.writes.remove(oldest);
            } else {
                // At most eight accounts: collapse exact preview removals into
                // one preview-only clear each. Required revocation is retained;
                // room/history/outbox records remain independently owned.
                let queued = std::mem::take(&mut self.writes);
                for mut write in queued {
                    if matches!(
                        write.command,
                        DataCommand::RemoveThumbnail { .. } | DataCommand::ClearThumbnails
                    ) {
                        if self.writes.iter().any(|old| {
                            old.account == write.account
                                && matches!(old.command, DataCommand::ClearThumbnails)
                        }) {
                            continue;
                        }
                        write.command = DataCommand::ClearThumbnails;
                    }
                    self.writes.push(write);
                }
                if matches!(command, DataCommand::RemoveThumbnail { .. })
                    && self.writes.iter().any(|w| {
                        w.account == account && matches!(w.command, DataCommand::ClearThumbnails)
                    })
                {
                    return;
                }
                if self.writes.len() >= WRITES {
                    return;
                }
            }
        }
        self.writes.push(Write {
            account,
            generation,
            command,
        });
    }
    pub(super) fn remember_thumbnail(
        &mut self,
        context: ConversationRef,
        thumbnail: CacheThumbnail,
    ) {
        if !self.persistable(context) || self.stable_room(context).as_ref() != Some(&thumbnail.room)
        {
            return;
        }
        let now = history::now_unix();
        let Some(account) = self
            .accounts
            .iter_mut()
            .find(|a| a.id == context.account.account)
        else {
            return;
        };
        let Some(snapshot) = account.snapshot.as_mut() else {
            return;
        };
        if !snapshot
            .rooms
            .as_slice()
            .iter()
            .any(|r| r.key == thumbnail.room)
            || !thumbnail_snapshot_matches(snapshot, &thumbnail, now)
        {
            return;
        }
        retain_thumbnail(&mut snapshot.thumbnails, thumbnail.clone(), now);
        self.thumbnail_denials
            .retain(|(id, _, room, message, placeholder)| {
                *id != context.account.account
                    || *room != thumbnail.room
                    || *message != thumbnail.message
                    || *placeholder != thumbnail.placeholder
            });
        self.enqueue(
            context.account.account,
            DataCommand::PutThumbnail(thumbnail),
        );
    }
    pub(super) fn forget_thumbnail(&mut self, attachment: &talk_core::ui::content::AttachmentRef) {
        let Some(room) = self.stable_room(attachment.context) else {
            return;
        };
        let Ok(message) = u64::try_from(attachment.message) else {
            return;
        };
        let account = attachment.context.account.account;
        let Some(owner) = self.accounts.iter_mut().find(|a| a.id == account) else {
            return;
        };
        let generation = owner.generation;
        if let Some(snapshot) = owner.snapshot.as_mut() {
            snapshot.thumbnails = page(
                snapshot
                    .thumbnails
                    .as_slice()
                    .iter()
                    .filter(|t| {
                        !(t.room == room
                            && t.message == message
                            && t.placeholder == attachment.placeholder)
                    })
                    .cloned(),
            );
        }
        let mut clear = false;
        if !self.thumbnail_denials.iter().any(|(id, g, key, m, p)| {
            *id == account
                && *g == generation
                && *key == room
                && *m == message
                && *p == attachment.placeholder
        }) {
            if self
                .thumbnail_denials
                .iter()
                .filter(|(id, g, ..)| *id == account && *g == generation)
                .count()
                < 64
            {
                self.thumbnail_denials.push((
                    account,
                    generation,
                    room.clone(),
                    message,
                    attachment.placeholder.clone(),
                ));
            } else if !self.thumbnail_load_blocked.contains(&(account, generation)) {
                self.thumbnail_load_blocked.push((account, generation));
                if let Some(snapshot) = owner.snapshot.as_mut() {
                    snapshot.thumbnails = Page::empty();
                }
                clear = true;
            }
        }
        if clear {
            self.enqueue(account, DataCommand::ClearThumbnails);
        }
        self.enqueue(
            account,
            DataCommand::RemoveThumbnail {
                room,
                message,
                placeholder: attachment.placeholder.clone(),
            },
        );
    }
}
fn thumbnail_fresh(thumbnail: &CacheThumbnail, now: u64) -> bool {
    thumbnail.message > 0
        && !thumbnail.placeholder.as_str().is_empty()
        && !thumbnail.bytes.is_empty()
        && thumbnail.bytes.len() <= CACHE_THUMBNAIL_BYTES
        && thumbnail.saved_at <= now
        && now - thumbnail.saved_at < CACHE_THUMBNAIL_LIFETIME
        && thumbnail.expires_at.is_none_or(|at| at == 0 || at > now)
}
fn thumbnail_history_matches(history: &CacheHistory, thumbnail: &CacheThumbnail, now: u64) -> bool {
    history
        .messages
        .as_slice()
        .iter()
        .chain(history.pin_messages.as_slice())
        .find(|m| m.id == thumbnail.message)
        .is_none_or(|message| {
            !message.deleted
                && message.expires_at.is_none_or(|at| at == 0 || at > now)
                && message
                    .attachments
                    .as_slice()
                    .iter()
                    .any(|a| a == &thumbnail.attachment)
        })
}
fn thumbnail_snapshot_matches(
    snapshot: &DataSnapshot,
    thumbnail: &CacheThumbnail,
    now: u64,
) -> bool {
    thumbnail_fresh(thumbnail, now)
        && snapshot
            .histories
            .as_slice()
            .iter()
            .find(|h| h.room == thumbnail.room)
            .is_none_or(|h| thumbnail_history_matches(h, thumbnail, now))
}
fn retain_thumbnail(
    values: &mut Page<CacheThumbnail, CACHE_THUMBNAIL_LIMIT>,
    thumbnail: CacheThumbnail,
    now: u64,
) {
    if !thumbnail_fresh(&thumbnail, now) {
        return;
    }
    let mut entries: Vec<_> = values
        .as_slice()
        .iter()
        .filter(|old| {
            thumbnail_fresh(old, now)
                && !(old.room == thumbnail.room
                    && old.message == thumbnail.message
                    && old.placeholder == thumbnail.placeholder)
        })
        .cloned()
        .collect();
    let mut size: usize = entries.iter().map(|t| t.bytes.len()).sum();
    while !entries.is_empty()
        && (entries.len() >= CACHE_THUMBNAIL_LIMIT
            || size + thumbnail.bytes.len() > 4 * 1024 * 1024)
    {
        size -= entries.remove(0).bytes.len();
    }
    entries.push(thumbnail);
    *values = page(entries);
}
fn same_slot(a: &DataCommand, b: &DataCommand) -> bool {
    match (a, b) {
        (DataCommand::PutRooms(_), DataCommand::PutRooms(_)) => true,
        (DataCommand::PutHistory(a), DataCommand::PutHistory(b)) => a.room == b.room,
        (DataCommand::PutThumbnail(a), DataCommand::PutThumbnail(b)) => {
            a.room == b.room && a.message == b.message && a.placeholder == b.placeholder
        }
        (
            DataCommand::RemoveThumbnail {
                room: a,
                message: am,
                placeholder: ap,
            },
            DataCommand::RemoveThumbnail {
                room: b,
                message: bm,
                placeholder: bp,
            },
        ) => a == b && am == bm && ap == bp,
        (DataCommand::ClearThumbnails, DataCommand::ClearThumbnails) => true,
        (DataCommand::MemberCount { room: a, .. }, DataCommand::MemberCount { room: b, .. }) => {
            a == b
        }
        (DataCommand::RemoveRoom(a), DataCommand::RemoveRoom(b)) => a == b,
        _ => false,
    }
}
fn cleanup_diagnostic(outcome: DiagnosticOutcome) {
    let mut event = Diagnostic::new(DiagnosticOperation::ThumbnailCleanup, outcome);
    if matches!(outcome, DiagnosticOutcome::Failed) {
        event.error = Some(ErrorKind::StorageUnavailable);
    }
    diagnostics::record(event);
}
fn eligible(a: &talk_adapters::identity::ConnectionAccount) -> bool {
    matches!(a.summary.storage, StoredState::Eligible(_))
        && !a.summary.suppression_unconfirmed
        && !a.summary.cleanup_pending
}
fn local_context(account: AccountScope, key: &NavigationRoomKey) -> Option<ConversationRef> {
    let local = u128::from_str_radix(key.as_str().strip_prefix("r1:")?, 16).ok()?;
    Some(ConversationRef {
        account,
        conversation: ConversationId {
            account: account.account,
            host: HostId::from_local(0),
            local,
        },
    })
}
fn row(room: &CacheRoom, context: ConversationRef) -> ConversationView {
    ConversationView {
        context,
        navigation_key: Some(room.key.clone()),
        title: room.title.clone(),
        initials: initials(room.title.as_str()),
        subtitle: text(""),
        updated_label: text(""),
        kind: match room.kind {
            CachedRoomKind::Direct => ConversationKind::Direct,
            CachedRoomKind::Group => ConversationKind::Group,
            CachedRoomKind::Public => ConversationKind::GuestAccessible,
            CachedRoomKind::NoteToSelf => ConversationKind::NoteToSelf,
            CachedRoomKind::Unknown => ConversationKind::Unknown,
        },
        unread: room.unread,
        favorite: room.favorite,
        tags: None,
        organization: ConversationOrganization {
            category: ConversationCategory::Unknown,
            classified: None,
            favorites: FeatureSupport::Unknown,
            favorite_change: Decision::Unknown,
            tags: FeatureSupport::Unknown,
            tag_change: Decision::Unknown,
        },
        archived: room.archived,
        archive: Decision::Unknown,
        presence: Presence::Unknown,
        participants: room.participant_count.and_then(|n| u16::try_from(n).ok()),
        send: Decision::Unknown,
        attach: Decision::Unknown,
        draft_clipboard: Decision::Unknown,
        call: Decision::Unknown,
        call_scope: None,
    }
}
impl Accounts {
    pub(super) fn project_cache(&mut self, context: ConversationRef) {
        if let Some(panel) = self
            .snapshot
            .message_panel
            .as_mut()
            .filter(|p| p.context == context)
        {
            panel.queue_available = self.cache.persistable(context);
        }
    }
    pub(super) fn sync_cache_accounts(&mut self, state: &ConnectionSnapshot) {
        if let Some(context) = self.snapshot.selected
            && let Some(key) = self.cache.stable_room(context)
        {
            self.cache.selection = Some((context.account.account, key));
        }
        self.cache.accounts.retain(|a| {
            state
                .accounts
                .as_slice()
                .iter()
                .any(|s| s.summary.account == a.id && eligible(s))
        });
        self.cache.writes.retain(|w| {
            self.cache
                .accounts
                .iter()
                .any(|a| a.id == w.account && a.generation == w.generation)
        });
        self.cache.thumbnail_denials.retain(|(id, g, ..)| {
            self.cache
                .accounts
                .iter()
                .any(|a| a.id == *id && a.generation == *g)
        });
        self.cache.thumbnail_load_blocked.retain(|(id, g)| {
            self.cache
                .accounts
                .iter()
                .any(|a| a.id == *id && a.generation == *g)
        });
        self.cache.cleanup_failures.retain(|f| {
            self.cache
                .accounts
                .iter()
                .any(|a| a.id == f.account && a.generation == f.generation)
        });
        if self.cache.in_flight.as_ref().is_some_and(|w| {
            !self
                .cache
                .accounts
                .iter()
                .any(|a| a.id == w.account && a.generation == w.generation)
        }) {
            self.cache.in_flight = None;
        }
        if self
            .cache
            .selection
            .as_ref()
            .is_some_and(|(id, _)| !self.cache.accounts.iter().any(|a| a.id == *id))
        {
            self.cache.selection = None;
        }
        for a in state
            .accounts
            .as_slice()
            .iter()
            .filter(|a| eligible(a))
            .take(8)
        {
            if let Some(cache) = self
                .cache
                .accounts
                .iter_mut()
                .find(|c| c.id == a.summary.account)
            {
                if cache.scope != a.summary.scope {
                    cache.scope = a.summary.scope;
                    cache.bindings.clear();
                    cache.hydrated = None;
                    cache.observed = false;
                }
            } else {
                let Some(generation) = self.cache.next_generation.checked_add(1) else {
                    continue;
                };
                self.cache.next_generation = generation;
                self.cache.accounts.push(Account {
                    id: a.summary.account,
                    generation,
                    scope: a.summary.scope,
                    observed: false,
                    loaded: false,
                    snapshot: None,
                    bindings: Vec::new(),
                    hydrated: None,
                    load_attempted: false,
                });
            }
        }
        if let Ok(connection) = &mut self.connection {
            for pending in &self.pending {
                let (account, generation) = match pending.purpose {
                    Purpose::Data(Work::Load(a, g) | Work::Write(a, g)) => (a, g),
                    _ => continue,
                };
                if !self
                    .cache
                    .accounts
                    .iter()
                    .any(|a| a.id == account && a.generation == generation)
                {
                    let _ = connection.cancel_request(pending.id);
                }
            }
        }
    }
    pub(super) fn tick_cache(&mut self) {
        if self
            .pending
            .iter()
            .any(|p| matches!(p.purpose, Purpose::Data(_)))
        {
            return;
        }
        if let Some((account, generation)) = self
            .cache
            .accounts
            .iter()
            .find(|a| !a.load_attempted)
            .map(|a| (a.id, a.generation))
        {
            match self.admit(
                RuntimeCommand::AccountData {
                    account,
                    command: DataCommand::Load,
                },
                Purpose::Data(Work::Load(account, generation)),
            ) {
                Err(IdentityError::Request(ErrorKind::Busy)) => (),
                result => {
                    if let Some(a) = self.cache.accounts.iter_mut().find(|a| a.id == account) {
                        a.load_attempted = true;
                    }
                    if result.is_err() {
                        self.touch();
                    }
                }
            }
            return;
        }
        let now = Instant::now();
        let Some(index) = self.cache.write_ready(now) else {
            return;
        };
        let Some(write) = self.cache.writes.get(index) else {
            return;
        };
        let account = write.account;
        let generation = write.generation;
        let command = write.command.clone();
        let cleanup = required_cleanup(&command);
        match self.admit(
            RuntimeCommand::AccountData { account, command },
            Purpose::Data(Work::Write(account, generation)),
        ) {
            Ok(_) => self.cache.in_flight = Some(self.cache.writes.remove(index)),
            Err(IdentityError::Request(ErrorKind::Busy)) => (),
            Err(_) => {
                self.cache.writes.remove(index);
                if cleanup && self.cache.cleanup_failed(account, generation, now) {
                    cleanup_diagnostic(DiagnosticOutcome::Failed);
                }
            }
        }
    }
    pub(super) fn complete_data(
        &mut self,
        work: Work,
        result: Result<RuntimeReply, IdentityError>,
    ) {
        if let Work::Write(account, generation) = work
            && self
                .cache
                .in_flight
                .as_ref()
                .is_some_and(|w| w.account == account && w.generation == generation)
        {
            let Some(write) = self.cache.in_flight.take() else {
                return;
            };
            if required_cleanup(&write.command)
                && self
                    .cache
                    .accounts
                    .iter()
                    .any(|a| a.id == account && a.generation == generation)
            {
                if matches!(&result,Ok(RuntimeReply::AccountData {account:returned,result:Ok(DataResult::Saved)}) if *returned==account)
                {
                    if self.cache.cleanup_saved(account, generation) {
                        cleanup_diagnostic(DiagnosticOutcome::Recovered);
                    }
                } else if self
                    .cache
                    .cleanup_failed(account, generation, Instant::now())
                {
                    cleanup_diagnostic(DiagnosticOutcome::Failed);
                }
            }
        }
        if let Work::Load(account, generation) = work
            && let Ok(RuntimeReply::AccountData {
                account: returned,
                result: Ok(DataResult::Snapshot(mut snapshot)),
            }) = result
            && account == returned
            && let Some(a) = self
                .cache
                .accounts
                .iter_mut()
                .find(|a| a.id == account && a.generation == generation)
        {
            // The store returns most recent first; composition keeps an oldest
            // first disposable list so additions and late merges evict LRU.
            snapshot.thumbnails = page(snapshot.thumbnails.as_slice().iter().rev().cloned());
            snapshot.thumbnails = page(
                snapshot
                    .thumbnails
                    .as_slice()
                    .iter()
                    .filter(|t| {
                        !self
                            .cache
                            .thumbnail_load_blocked
                            .contains(&(account, generation))
                            && !self.cache.thumbnail_denials.iter().any(
                                |(id, g, room, message, placeholder)| {
                                    *id == account
                                        && *g == generation
                                        && *room == t.room
                                        && *message == t.message
                                        && *placeholder == t.placeholder
                                },
                            )
                    })
                    .cloned(),
            );
            if a.observed {
                // A late disk read cannot resurrect revoked room metadata or
                // overwrite newer confirmed content from this runtime.
                if let Some(current) = a.snapshot.as_mut() {
                    let now = history::now_unix();
                    let newer = current.thumbnails.clone();
                    current.thumbnails = Page::empty();
                    for thumbnail in snapshot
                        .thumbnails
                        .as_slice()
                        .iter()
                        .chain(newer.as_slice())
                    {
                        if a.bindings
                            .iter()
                            .any(|b| b.ordinary && b.key == thumbnail.room)
                            && thumbnail_snapshot_matches(current, thumbnail, now)
                        {
                            retain_thumbnail(&mut current.thumbnails, thumbnail.clone(), now);
                        }
                    }
                    current.outbox = snapshot.outbox;
                    let mut histories: Vec<_> = current.histories.as_slice().to_vec();
                    for h in snapshot.histories.as_slice() {
                        if histories.len() < 8
                            && !histories.iter().any(|old| old.room == h.room)
                            && a.bindings.iter().any(|b| b.ordinary && b.key == h.room)
                        {
                            histories.push(h.clone());
                        }
                    }
                    current.histories = page(histories);
                }
            } else {
                a.snapshot = Some(snapshot);
            }
            a.loaded = true;
            a.hydrated = None;
            self.touch();
        }
        // Required preview cleanup retains its account-owned retry above.
        // Other cache failures retain observations without failing a server
        // action; required durable user actions belong to the outbox owner.
    }
    pub(super) fn restore_cache_views(&mut self) {
        let scopes: Vec<_> = self
            .snapshot
            .accounts
            .as_slice()
            .iter()
            .map(|a| a.scope)
            .collect();
        let now = history::now_unix();
        let instant = Instant::now();
        for scope in scopes {
            let Some(account) = self
                .cache
                .accounts
                .iter_mut()
                .find(|a| a.id == scope.account && a.hydrated != Some(scope))
            else {
                continue;
            };
            let Some(snapshot) = &account.snapshot else {
                continue;
            };
            // A cached room always has a synthetic host, including while its
            // account has restored but the fresh room list has not arrived.
            if account
                .bindings
                .iter()
                .any(|b| b.context.account == scope && b.context.conversation.host.as_local() != 0)
            {
                continue;
            }
            account.bindings = snapshot
                .rooms
                .as_slice()
                .iter()
                .filter_map(|r| {
                    Some(Binding {
                        context: local_context(scope, &r.key)?,
                        key: r.key.clone(),
                        ordinary: true,
                    })
                })
                .collect();
            let rows = page(
                snapshot
                    .rooms
                    .as_slice()
                    .iter()
                    .filter_map(|r| Some(row(r, local_context(scope, &r.key)?))),
            );
            if let Some(background) = self.background.account_mut(scope) {
                background.rooms = rows.clone();
                background.state = ConversationListState {
                    content: ContentState::Ready,
                    complete: false,
                };
            }
            for r in snapshot.rooms.as_slice() {
                let Some(context) = local_context(scope, &r.key) else {
                    continue;
                };
                if let Some(count) = r.participant_count.and_then(|n| u16::try_from(n).ok()) {
                    self.background.restore_member_count(
                        context,
                        count,
                        r.participant_count_at
                            .and_then(|at| now.checked_sub(at))
                            .map(Duration::from_secs),
                        instant,
                    );
                }
                if let Some(history) = snapshot
                    .histories
                    .as_slice()
                    .iter()
                    .find(|h| h.room == r.key)
                {
                    self.history.restore(context, history);
                }
            }
            account.hydrated = Some(scope);
            if self.snapshot.active_account == Some(scope) {
                self.snapshot.conversations = rows;
                self.snapshot.conversation_list = ConversationListState {
                    content: ContentState::Ready,
                    complete: false,
                };
            }
        }
    }
    pub(super) fn restore_cache_selection(&mut self) {
        let Some((account, key)) = &self.cache.selection else {
            return;
        };
        let Some(scope) = self
            .snapshot
            .active_account
            .filter(|s| s.account == *account)
        else {
            return;
        };
        let Some(context) = self
            .snapshot
            .conversations
            .as_slice()
            .iter()
            .find(|r| r.context.account == scope && r.navigation_key.as_ref() == Some(key))
            .map(|r| r.context)
        else {
            return;
        };
        if self.snapshot.selected == Some(context) {
            return;
        }
        if self.snapshot.selected.is_none()
            || self.snapshot.selected.is_some_and(|c| {
                // Fresh room observations already retired the synthetic
                // bindings. Recognize the former presentation identity
                // without consulting that registry; the captured stable
                // key and currently published row choose the new context.
                // Host zero grants only this UI transition, never runtime
                // authority. select_conversation validates the new target.
                c.is_consistent()
                    && c.conversation.host.as_local() == 0
                    && c.account.account == *account
            })
        {
            let _ = self.select_conversation(context);
        }
    }
    pub(super) fn observe_cache_rooms(
        &mut self,
        scope: AccountScope,
        list: &protocol::ConversationList,
    ) {
        let now = history::now_unix();
        let Some(account) = self
            .cache
            .accounts
            .iter_mut()
            .find(|a| a.id == scope.account)
        else {
            return;
        };
        if account.scope != Some(scope) {
            return;
        }
        let prior = account
            .snapshot
            .as_ref()
            .map(|s| s.rooms.clone())
            .unwrap_or_else(Page::empty);
        account.bindings.clear();
        let mut rooms = Page::empty();
        for room in list.rooms.as_slice() {
            let context = ConversationRef {
                account: scope,
                conversation: room.id,
            };
            let Some(key) = room.navigation_key.clone() else {
                continue;
            };
            let ordinary = room.actions.read_messages == Decision::Allowed
                && !room.sensitive
                && !room.classified
                && !room.unknown_restrictions
                && room.organization.classified != Some(true)
                && matches!(
                    room.kind,
                    RoomKind::Direct
                        | RoomKind::FormerDirect
                        | RoomKind::Group
                        | RoomKind::Public
                        | RoomKind::NoteToSelf
                );
            account.bindings.push(Binding {
                context,
                key: key.clone(),
                ordinary,
            });
            if !ordinary {
                continue;
            }
            let old = prior.as_slice().iter().find(|r| r.key == key);
            let count = self
                .background
                .member_count(context)
                .map(u32::from)
                .or_else(|| old.and_then(|r| r.participant_count));
            if self.background.member_count(context).is_none()
                && let Some(count) = count.and_then(|n| u16::try_from(n).ok())
            {
                self.background.restore_member_count(
                    context,
                    count,
                    old.and_then(|r| r.participant_count_at)
                        .and_then(|at| now.checked_sub(at))
                        .map(Duration::from_secs),
                    Instant::now(),
                );
            }
            if let Some(history) = account
                .snapshot
                .as_ref()
                .and_then(|s| s.histories.as_slice().iter().find(|h| h.room == key))
            {
                self.history.restore(context, history);
            }
            let _ = rooms.try_push(CacheRoom {
                key,
                title: room.name.clone(),
                kind: match room.kind {
                    RoomKind::Direct | RoomKind::FormerDirect => CachedRoomKind::Direct,
                    RoomKind::Group => CachedRoomKind::Group,
                    RoomKind::Public => CachedRoomKind::Public,
                    RoomKind::NoteToSelf => CachedRoomKind::NoteToSelf,
                    _ => CachedRoomKind::Unknown,
                },
                favorite: room.favorite,
                archived: room.archived,
                unread: room.unread_messages.unwrap_or(0),
                participant_count: count,
                participant_count_at: old.and_then(|r| r.participant_count_at),
                last_activity: room.last_activity,
                saved_at: now,
            });
        }
        account.observed = true;
        let snapshot = account.snapshot.get_or_insert_with(|| DataSnapshot {
            thumbnails: Page::empty(),
            rooms: Page::empty(),
            histories: Page::empty(),
            outbox: Page::empty(),
        });
        {
            snapshot.rooms = rooms.clone();
            snapshot.histories = page(
                snapshot
                    .histories
                    .as_slice()
                    .iter()
                    .filter(|h| rooms.as_slice().iter().any(|r| r.key == h.room))
                    .cloned(),
            );
            snapshot.thumbnails = page(
                snapshot
                    .thumbnails
                    .as_slice()
                    .iter()
                    .filter(|t| rooms.as_slice().iter().any(|r| r.key == t.room))
                    .cloned(),
            );
        }
        self.cache
            .enqueue(scope.account, DataCommand::PutRooms(rooms));
    }
    pub(super) fn remember_cached_member_count(&mut self, context: ConversationRef, count: u16) {
        if !self.cache.persistable(context) {
            return;
        }
        let Some(key) = self.cache.stable_room(context) else {
            return;
        };
        if let Some(snapshot) = self
            .cache
            .accounts
            .iter_mut()
            .find(|a| a.id == context.account.account)
            .and_then(|a| a.snapshot.as_mut())
        {
            snapshot.rooms = page(snapshot.rooms.as_slice().iter().map(|r| {
                let mut r = r.clone();
                if r.key == key {
                    r.participant_count = Some(u32::from(count));
                    r.participant_count_at = Some(history::now_unix());
                }
                r
            }));
        }
        self.cache.enqueue(
            context.account.account,
            DataCommand::MemberCount {
                room: key,
                count: u32::from(count),
            },
        );
    }
    pub(super) fn forget_cached_member_count(&mut self, context: ConversationRef) {
        let Some(key) = self.cache.stable_room(context) else {
            return;
        };
        let Some(snapshot) = self
            .cache
            .accounts
            .iter_mut()
            .find(|a| a.id == context.account.account)
            .and_then(|a| a.snapshot.as_mut())
        else {
            return;
        };
        snapshot.rooms = page(snapshot.rooms.as_slice().iter().map(|r| {
            let mut r = r.clone();
            if r.key == key {
                r.participant_count = None;
                r.participant_count_at = None;
            }
            r
        }));
        let rooms = snapshot.rooms.clone();
        self.cache
            .enqueue(context.account.account, DataCommand::PutRooms(rooms));
    }
    /// An explicit current policy revocation also retires the on-disk projection.
    /// It cannot allocate or affect a room from an obsolete runtime completion.
    pub(super) fn invalidate_cached_room(&mut self, context: ConversationRef) {
        let Some(account) = self
            .cache
            .accounts
            .iter_mut()
            .find(|a| a.id == context.account.account && a.scope == Some(context.account))
        else {
            return;
        };
        let Some(binding) = account.bindings.iter_mut().find(|b| b.context == context) else {
            return;
        };
        binding.ordinary = false;
        let key = binding.key.clone();
        let Some(snapshot) = account.snapshot.as_mut() else {
            return;
        };
        snapshot.rooms = page(
            snapshot
                .rooms
                .as_slice()
                .iter()
                .filter(|r| r.key != key)
                .cloned(),
        );
        snapshot.histories = page(
            snapshot
                .histories
                .as_slice()
                .iter()
                .filter(|h| h.room != key)
                .cloned(),
        );
        snapshot.thumbnails = page(
            snapshot
                .thumbnails
                .as_slice()
                .iter()
                .filter(|t| t.room != key)
                .cloned(),
        );
        let rooms = snapshot.rooms.clone();
        self.cache
            .enqueue(context.account.account, DataCommand::PutRooms(rooms));
    }
    pub(super) fn cache_history(&mut self, context: ConversationRef) {
        if !self.cache.persistable(context) || self.cache.is_local(context.account) {
            return;
        }
        let Some(key) = self.cache.stable_room(context) else {
            return;
        };
        let Some(mut cached) = self.history.durable(context, key, history::now_unix()) else {
            return;
        };
        if let Some(snapshot) = self
            .cache
            .accounts
            .iter_mut()
            .find(|a| a.id == context.account.account)
            .and_then(|a| a.snapshot.as_mut())
        {
            let now = history::now_unix();
            snapshot.thumbnails = page(
                snapshot
                    .thumbnails
                    .as_slice()
                    .iter()
                    .filter(|t| t.room != cached.room || thumbnail_history_matches(&cached, t, now))
                    .cloned(),
            );
            if let Some(old) = snapshot
                .histories
                .as_slice()
                .iter()
                .find(|h| h.room == cached.room)
            {
                cached.saved_at = old.saved_at;
                if old == &cached {
                    return;
                }
                cached.saved_at = history::now_unix();
            }
            let mut histories: Vec<_> = snapshot
                .histories
                .as_slice()
                .iter()
                .filter(|h| h.room != cached.room)
                .cloned()
                .collect();
            histories.push(cached.clone());
            histories.sort_by_key(|h| std::cmp::Reverse(h.saved_at));
            snapshot.histories = page(histories.into_iter().take(8));
        }
        self.cache
            .enqueue(context.account.account, DataCommand::PutHistory(cached));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[path = "thumbnail_tests.rs"]
    mod thumbnail_tests;
    fn stable() -> NavigationRoomKey {
        NavigationRoomKey::new("r1:00000000000000000000000000000007").unwrap()
    }
    fn metadata() -> CacheRoom {
        CacheRoom {
            key: stable(),
            title: text("Cached room"),
            kind: CachedRoomKind::Group,
            favorite: Some(true),
            archived: Some(false),
            unread: 3,
            participant_count: Some(7),
            participant_count_at: Some(history::now_unix()),
            last_activity: Some(1),
            saved_at: history::now_unix(),
        }
    }
    fn install(model: &mut Accounts, scope: AccountScope) {
        model.cache.next_generation = 1;
        model.cache.accounts.push(Account {
            id: scope.account,
            generation: 1,
            scope: Some(scope),
            observed: false,
            loaded: true,
            snapshot: Some(DataSnapshot {
                thumbnails: Page::empty(),
                rooms: page([metadata()]),
                histories: Page::empty(),
                outbox: Page::empty(),
            }),
            bindings: Vec::new(),
            hydrated: None,
            load_attempted: true,
        });
    }
    fn observed(scope: AccountScope) -> protocol::Conversation {
        protocol::Conversation {
            id: ConversationId {
                account: scope.account,
                host: HostId::from_local(9),
                local: 3,
            },
            call_scope: None,
            navigation_key: Some(stable()),
            favorite: Some(true),
            organization: protocol::ConversationOrganization {
                favorites: protocol::FeatureSupport::Supported,
                favorite_change: Decision::Allowed,
                tags: protocol::FeatureSupport::Unsupported,
                tag_change: Decision::Denied,
                classified: Some(false),
                channel: Some(false),
                voice_room: Some(false),
            },
            tag_ids: None,
            name: text("Fresh room"),
            kind: RoomKind::Group,
            role: protocol::Role::Member,
            archived: Some(false),
            archive: Decision::Allowed,
            actions: protocol::RoomActions {
                read_messages: Decision::Allowed,
                send_message: Decision::Allowed,
                list_participants: Decision::Allowed,
                leave_conversation: Decision::Allowed,
                join_call: Decision::Unknown,
            },
            unread_messages: Some(1),
            last_activity: Some(1),
            call_active: Some(false),
            sensitive: false,
            classified: false,
            channel: false,
            unknown_restrictions: false,
        }
    }
    #[test]
    fn cached_projection_is_immediate_and_never_a_dispatch_scope() {
        let mut model = super::super::tests::model();
        model.snapshot = super::super::super::fixtures::snapshot(Some("chat"));
        let scope = model.snapshot.active_account.unwrap();
        install(&mut model, scope);
        model.background.sync(&[scope], Instant::now());
        model.restore_cache_views();
        let row = &model.snapshot.conversations.as_slice()[0];
        let context = row.context;
        assert_eq!(row.title.as_str(), "Cached room");
        assert_eq!(row.participants, Some(7));
        assert_eq!(row.send, Decision::Unknown);
        assert_eq!(row.archive, Decision::Unknown);
        assert_eq!(row.call, Decision::Unknown);
        assert!(model.cache.is_presentation(context) && model.cache.persistable(context));
        assert_eq!(model.cache.context(scope.account, &stable()), None);
        assert_eq!(model.cache.stable_room(context), Some(stable()));
        assert!(
            !model
                .background
                .members_need_refresh(context, Instant::now())
        );
    }
    #[test]
    fn selected_cached_room_rebinds_after_its_synthetic_binding_has_been_retired() {
        use talk_adapters::identity::{CacheHistory, CachedMessage};
        use talk_core::protocol::{
            Message, MessageId, MessageKind,
            messaging::{MessageDetails, MessagingResult},
        };
        let mut model = super::super::tests::model();
        model.snapshot = super::super::super::fixtures::snapshot(Some("chat"));
        let scope = model.snapshot.active_account.unwrap();
        install(&mut model, scope);
        model.background.sync(&[scope], Instant::now());
        let history = CacheHistory {
            room: stable(),
            messages: page([CachedMessage {
                attachments: Page::empty(),
                id: 7,
                author: text("Synthetic actor"),
                actor_key: None,
                text: text("Cached original"),
                own: true,
                timestamp: 1,
                expires_at: None,
                edited: false,
                deleted: false,
                system: false,
                truncated: false,
                replyable: true,
                parent: None,
                thread: None,
                reactions: Page::empty(),
            }]),
            pins: Page::empty(),
            pin_messages: Page::empty(),
            pins_loaded: false,
            older: None,
            latest: Some(7),
            saved_at: history::now_unix(),
        };
        model.cache.accounts[0].snapshot.as_mut().unwrap().histories = page([history.clone()]);
        model.restore_cache_views();
        let cached = model.snapshot.conversations.as_slice()[0].context;
        model.select_conversation(cached).unwrap();
        model.sync_cache_accounts(&connection_state(Some(scope), false));
        assert_eq!(model.cache.selection, Some((scope.account, stable())));
        assert!(model.cache.is_presentation(cached));
        assert_eq!(
            model.snapshot.messages.as_slice()[0].body.as_str(),
            "Cached original"
        );
        let room = observed(scope);
        let live = ConversationRef {
            account: scope,
            conversation: room.id,
        };
        model.observe_cache_rooms(
            scope,
            &protocol::ConversationList {
                scope,
                rooms: page([room.clone()]),
            },
        );
        assert!(
            !model.cache.is_presentation(cached),
            "the old binding is gone before selection reconciliation"
        );
        assert_eq!(model.snapshot.selected, Some(cached));
        model.snapshot.conversations = page([conversation(&room, scope)]);
        model.restore_cache_selection();
        assert_eq!(model.snapshot.selected, Some(live));
        assert_eq!(model.snapshot.message_panel.as_ref().unwrap().context, live);
        assert_eq!(
            model.snapshot.messages.as_slice()[0].body.as_str(),
            "Cached original"
        );
        assert!(
            model.current(scope).is_err(),
            "rebinding presentation cannot create account authority"
        );
        assert_eq!(
            model
                .cache
                .loaded(scope.account)
                .unwrap()
                .histories
                .as_slice(),
            &[history]
        );
        let operation = OperationId::from_local(42);
        model
            .history
            .admitted(live, operation, messaging::Work::Action, RequestId(42), 42);
        model.apply_message(
            RequestId(42),
            live,
            operation,
            messaging::Work::Action,
            Ok(RuntimeReply::Protocol(ProtocolReply::Messaging {
                conversation: live.conversation,
                operation,
                result: MessagingResult::Message(Box::new(MessageDetails {
                    author_key: None,
                    message: Message {
                        id: MessageId {
                            conversation: live.conversation,
                            sequence: 7,
                        },
                        author: text("Synthetic actor"),
                        text: text("Fresh update"),
                        kind: MessageKind::Comment,
                        timestamp: 1,
                        expires_at: None,
                        edited_at: Some(2),
                        own_operation: None,
                    },
                    notify: Decision::Denied,
                    own: true,
                    replyable: Some(true),
                    parent: None,
                    thread: None,
                    reactions: Page::empty(),
                    rich: Page::empty(),
                    system: None,
                    edit: Decision::Unknown,
                    delete: Decision::Unknown,
                })),
            })),
        );
        assert_eq!(
            model.snapshot.messages.as_slice()[0].body.as_str(),
            "Fresh update",
            "live-room results now project into the selected room"
        );
    }
    #[test]
    fn fresh_ordinary_observation_rebinds_stable_keys_and_restrictions_revoke_durable_admission() {
        let mut model = super::super::tests::model();
        let scope = super::super::super::fixtures::context(1).account;
        install(&mut model, scope);
        model.background.sync(&[scope], Instant::now());
        let room = observed(scope);
        let context = ConversationRef {
            account: scope,
            conversation: room.id,
        };
        model.observe_cache_rooms(
            scope,
            &protocol::ConversationList {
                scope,
                rooms: page([room.clone()]),
            },
        );
        assert_eq!(model.cache.context(scope.account, &stable()), Some(context));
        assert!(model.cache.persistable(context));
        assert!(model.cache.metadata_pending(scope.account, &stable()));
        assert_eq!(model.background.member_count(context), Some(7));
        for restricted in 0..3 {
            let mut room = room.clone();
            room.sensitive = restricted == 0;
            room.classified = restricted == 1;
            room.unknown_restrictions = restricted == 2;
            model.observe_cache_rooms(
                scope,
                &protocol::ConversationList {
                    scope,
                    rooms: page([room]),
                },
            );
            assert!(!model.cache.persistable(context));
            assert!(
                model
                    .cache
                    .loaded(scope.account)
                    .unwrap()
                    .rooms
                    .as_slice()
                    .is_empty()
            );
            assert!(model.cache.writes.iter().any(
                |w| matches!(&w.command,DataCommand::PutRooms(rooms) if rooms.as_slice().is_empty())
            ));
        }
    }
    #[test]
    fn cache_refresh_overload_is_bounded_and_coalesces_without_admitting_intents() {
        let account = AccountId::from_local(1);
        let mut cache = Cache::default();
        cache.accounts.push(Account {
            id: account,
            generation: 1,
            scope: None,
            observed: false,
            loaded: false,
            snapshot: None,
            bindings: Vec::new(),
            hydrated: None,
            load_attempted: false,
        });
        for n in 1..=40 {
            cache.enqueue(
                account,
                DataCommand::RemoveRoom(NavigationRoomKey::new(&format!("r1:{n:032x}")).unwrap()),
            );
        }
        assert_eq!(cache.writes.len(), WRITES);
        cache.enqueue(account, DataCommand::PutRooms(page([metadata()])));
        cache.enqueue(account, DataCommand::PutRooms(page([metadata()])));
        assert_eq!(
            cache.writes.len(),
            1,
            "one complete snapshot subsumes all earlier room invalidations"
        );
        for n in 1..=40 {
            cache.enqueue(
                account,
                DataCommand::RemoveRoom(NavigationRoomKey::new(&format!("r1:{n:032x}")).unwrap()),
            );
        }
        assert_eq!(cache.writes.len(), WRITES);
        assert_eq!(
            cache
                .writes
                .iter()
                .filter(|w| matches!(w.command, DataCommand::PutRooms(_)))
                .count(),
            1
        );
    }
    fn connection_state(scope: Option<AccountScope>, suppressed: bool) -> ConnectionSnapshot {
        use talk_adapters::identity::{AccountPhase, ConnectionAccount};
        use talk_core::{
            identity::{ProviderBinding, QualifiedItem},
            ids::{ProfileId, VaultGeneration},
            preferences::navigation::NavigationAccountKey,
        };
        let account = super::super::super::fixtures::context(1).account.account;
        ConnectionSnapshot {
            state: ConnectionState::Ready,
            accounts: page([ConnectionAccount {
                state: if scope.is_some() {
                    SavedAccountState::Active
                } else {
                    SavedAccountState::Dormant
                },
                summary: AccountSummary {
                    account,
                    navigation_key: NavigationAccountKey::new(&format!(
                        "a1:{:032x}:00000000000000000000000000000001",
                        account.as_local()
                    ))
                    .unwrap(),
                    server_label: Some(text("fixture.invalid")),
                    account_label: Some(text("Synthetic")),
                    connected_once: true,
                    attempt: None,
                    scope,
                    phase: if scope.is_some() {
                        AccountPhase::Active
                    } else {
                        AccountPhase::Disconnected
                    },
                    display_name: text("Synthetic"),
                    storage: if suppressed {
                        StoredState::Suppressed
                    } else {
                        StoredState::Eligible(QualifiedItem {
                            account,
                            generation: VaultGeneration::from_local(1),
                            provider: ProviderBinding {
                                backend: StorageBackend::PrivateFile,
                                profile: ProfileId::from_local(1),
                                store: 1,
                            },
                        })
                    },
                    cleanup_pending: false,
                    suppression_unconfirmed: false,
                    connection_degraded: false,
                    connection_error: None,
                    recovery: None,
                    revision: 1,
                },
            }]),
            pending: Page::empty(),
            retained: 0,
        }
    }
    fn disk_snapshot() -> DataSnapshot {
        DataSnapshot {
            thumbnails: Page::empty(),
            rooms: page([metadata()]),
            histories: Page::empty(),
            outbox: Page::empty(),
        }
    }
    #[test]
    fn logout_and_reauthentication_fence_late_disk_results_and_cached_scope_rebinds_without_authority()
     {
        let mut model = super::super::tests::model();
        let account = super::super::super::fixtures::context(1).account.account;
        model.sync_accounts(&connection_state(None, false));
        let old_generation = model.cache.accounts[0].generation;
        model.sync_accounts(&connection_state(None, true));
        assert!(model.cache.accounts.is_empty());
        assert!(model.snapshot.accounts.as_slice().is_empty());
        model.sync_accounts(&connection_state(None, false));
        let generation = model.cache.accounts[0].generation;
        assert_ne!(generation, old_generation);
        model.complete_data(
            Work::Load(account, old_generation),
            Ok(RuntimeReply::AccountData {
                account,
                result: Ok(DataResult::Snapshot(disk_snapshot())),
            }),
        );
        assert!(model.cache.loaded(account).is_none());
        assert!(model.cache.accounts[0].snapshot.is_none());
        model.complete_data(
            Work::Load(account, generation),
            Ok(RuntimeReply::AccountData {
                account,
                result: Ok(DataResult::Snapshot(disk_snapshot())),
            }),
        );
        model.sync_accounts(&connection_state(None, false));
        let cached_scope = model.snapshot.active_account.unwrap();
        assert!(model.cache.is_local(cached_scope));
        let cached = model.snapshot.conversations.as_slice()[0].context;
        assert!(model.cache.is_presentation(cached));
        assert!(!model.cache.revoked(account, &stable()));
        let real_scope = AccountScope {
            account,
            runtime: RuntimeGeneration::from_local(9),
        };
        model.sync_accounts(&connection_state(Some(real_scope), false));
        let pending = model.snapshot.conversations.as_slice()[0].context;
        assert_eq!(pending.account, real_scope);
        assert!(model.cache.is_presentation(pending));
        assert!(!model.cache.persistable(cached));
        assert_eq!(model.cache.context(account, &stable()), None);
        let room = observed(real_scope);
        model.apply_room_refresh(
            real_scope,
            Ok(RuntimeReply::Protocol(ProtocolReply::Conversations(
                protocol::ConversationList {
                    scope: real_scope,
                    rooms: page([room.clone()]),
                },
            ))),
        );
        let fresh = ConversationRef {
            account: real_scope,
            conversation: room.id,
        };
        assert_eq!(model.cache.context(account, &stable()), Some(fresh));
        assert!(model.cache.persistable(fresh));
        assert!(!model.cache.persistable(pending));
        // A former generation has no policy deletion or persistence authority.
        model.invalidate_cached_room(cached);
        assert!(model.cache.persistable(fresh));
        model.invalidate_cached_room(fresh);
        assert!(model.cache.revoked(account, &stable()));
        assert!(!model.cache.persistable(fresh));
    }
    #[test]
    fn late_disk_load_cannot_restore_revoked_rooms_or_replace_fresh_metadata() {
        let mut model = super::super::tests::model();
        let scope = super::super::super::fixtures::context(1).account;
        install(&mut model, scope);
        model.cache.accounts[0].loaded = false;
        model.cache.accounts[0].snapshot = None;
        model.background.sync(&[scope], Instant::now());
        let room = observed(scope);
        model.observe_cache_rooms(
            scope,
            &protocol::ConversationList {
                scope,
                rooms: page([room]),
            },
        );
        assert!(
            model.cache.loaded(scope.account).is_none(),
            "outbox recovery cannot observe an invented empty load"
        );
        model.complete_data(
            Work::Load(scope.account, 1),
            Ok(RuntimeReply::AccountData {
                account: scope.account,
                result: Ok(DataResult::Snapshot(disk_snapshot())),
            }),
        );
        assert_eq!(
            model.cache.loaded(scope.account).unwrap().rooms.as_slice()[0]
                .title
                .as_str(),
            "Fresh room"
        );
        model.observe_cache_rooms(
            scope,
            &protocol::ConversationList {
                scope,
                rooms: Page::empty(),
            },
        );
        assert!(model.cache.revoked(scope.account, &stable()));
        model.complete_data(
            Work::Load(scope.account, 1),
            Ok(RuntimeReply::AccountData {
                account: scope.account,
                result: Ok(DataResult::Snapshot(disk_snapshot())),
            }),
        );
        assert!(
            model
                .cache
                .loaded(scope.account)
                .unwrap()
                .rooms
                .as_slice()
                .is_empty()
        );
        model.restore_cache_views();
        assert_eq!(model.cache.context(scope.account, &stable()), None);
        let replacement = AccountScope {
            runtime: RuntimeGeneration::from_local(99),
            ..scope
        };
        model.sync_cache_accounts(&connection_state(Some(replacement), false));
        assert!(
            !model.cache.revoked(scope.account, &stable()),
            "new runtime needs a new policy observation"
        );
    }
}
