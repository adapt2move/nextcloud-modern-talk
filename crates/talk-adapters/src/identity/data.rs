//! Bounded ordinary-chat observations and durable pending intents.
//!
//! This is private, unencrypted per-user storage, not an authorization source or
//! an at-rest encryption claim. The identity worker admits only confirmed ordinary
//! rooms here. Restricted/unknown rooms and session-only accounts stay in memory.
//! One account database owns room metadata, eight recent histories and 64 durable
//! intents; cache eviction never evicts an intent. No credential, transport URL,
//! room token, permission grant or original media payload belongs in these records.
//! Disposable, downscaled ordinary-room previews have a separate finite budget.
use super::{ledger::Ledger, private_fs::PrivateDir};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use talk_core::{
    bounded::{Page, Text},
    ids::{AccountId, OperationId},
    preferences::navigation::NavigationRoomKey,
};

mod codec;
mod intent;
mod thumbnails;
pub use intent::{OutboxIntent, OutboxOptions};

pub const CACHE_ROOM_LIMIT: usize = 100;
pub const CACHE_HISTORY_LIMIT: usize = 8;
pub const OUTBOX_LIMIT: usize = 64;
pub const CACHE_LIFETIME: u64 = 30 * 24 * 60 * 60;
pub const CACHE_THUMBNAIL_LIMIT: usize = 24;
pub const CACHE_THUMBNAIL_BYTES: usize = 256 * 1024;
pub const CACHE_THUMBNAIL_LIFETIME: u64 = 7 * 24 * 60 * 60;
const HISTORY_BYTES: usize = 4 * 1024 * 1024;
const CACHE_BYTES: usize = 8 * 1024 * 1024;
const OUTBOX_BYTES: usize = 512 * 1024;
const META_BYTES: usize = 8192;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DataError {
    Unavailable,
    Ineligible,
    InvalidInput,
    Corrupt,
    Capacity,
}
type Result<T> = std::result::Result<T, DataError>;
fn sql<T>(result: rusqlite::Result<T>) -> Result<T> {
    result.map_err(|_| DataError::Unavailable)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CachedRoomKind {
    Direct,
    Group,
    Public,
    NoteToSelf,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CacheRoom {
    #[serde(with = "codec::room_key")]
    pub key: NavigationRoomKey,
    #[serde(with = "codec::text")]
    pub title: Text<256>,
    pub kind: CachedRoomKind,
    pub favorite: Option<bool>,
    pub archived: Option<bool>,
    pub unread: u32,
    pub participant_count: Option<u32>,
    pub participant_count_at: Option<u64>,
    pub last_activity: Option<u64>,
    pub saved_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CachedReaction {
    #[serde(with = "codec::text")]
    pub emoji: Text<64>,
    pub count: u32,
    pub own: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CachedReply {
    pub id: u64,
    #[serde(with = "codec::text")]
    pub author: Text<256>,
    #[serde(with = "codec::text")]
    pub text: Text<4096>,
    pub unavailable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CachedAttachment {
    #[serde(with = "codec::text")]
    pub placeholder: Text<128>,
    #[serde(with = "codec::text")]
    pub name: Text<1024>,
    #[serde(with = "codec::optional_text")]
    pub mime_type: Option<Text<256>>,
    pub size: Option<u64>,
    #[serde(with = "codec::optional_text")]
    pub id: Option<Text<1024>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CachedMessage {
    pub id: u64,
    #[serde(with = "codec::text")]
    pub author: Text<256>,
    #[serde(with = "codec::optional_text")]
    pub actor_key: Option<Text<256>>,
    #[serde(with = "codec::text")]
    pub text: Text<4096>,
    pub own: bool,
    pub timestamp: u64,
    pub expires_at: Option<u64>,
    pub edited: bool,
    pub deleted: bool,
    pub system: bool,
    pub truncated: bool,
    pub replyable: bool,
    pub parent: Option<CachedReply>,
    pub thread: Option<u64>,
    #[serde(with = "codec::page")]
    pub reactions: Page<CachedReaction, 64>,
    #[serde(default = "Page::empty", with = "codec::page")]
    pub attachments: Page<CachedAttachment, 8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CacheHistory {
    #[serde(with = "codec::room_key")]
    pub room: NavigationRoomKey,
    #[serde(with = "codec::page")]
    pub messages: Page<CachedMessage, 100>,
    #[serde(with = "codec::page")]
    pub pins: Page<u64, 50>,
    #[serde(default = "Page::empty", with = "codec::page")]
    pub pin_messages: Page<CachedMessage, 50>,
    pub pins_loaded: bool,
    pub older: Option<u64>,
    pub latest: Option<u64>,
    pub saved_at: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutboxKind {
    Send,
    Edit,
    Delete,
    Reaction,
    Pin,
    Forward,
    Reminder,
    Schedule,
    CancelScheduled,
}
impl OutboxKind {
    fn code(self) -> i64 {
        match self {
            Self::Send => 0,
            Self::Edit => 1,
            Self::Delete => 2,
            Self::Reaction => 3,
            Self::Pin => 4,
            Self::Forward => 5,
            Self::Reminder => 6,
            Self::Schedule => 7,
            Self::CancelScheduled => 8,
        }
    }
    fn parse(code: i64) -> Result<Self> {
        Ok(match code {
            0 => Self::Send,
            1 => Self::Edit,
            2 => Self::Delete,
            3 => Self::Reaction,
            4 => Self::Pin,
            5 => Self::Forward,
            6 => Self::Reminder,
            7 => Self::Schedule,
            8 => Self::CancelScheduled,
            _ => return Err(DataError::Corrupt),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutboxPhase {
    Queued,
    Dispatched,
    Uncertain,
    /// Definitive failure. A text-bearing intent remains a recoverable draft;
    /// this operation identity can never silently become a new dispatch.
    Failed,
}
impl OutboxPhase {
    fn code(self) -> i64 {
        match self {
            Self::Queued => 0,
            Self::Dispatched => 1,
            Self::Uncertain => 2,
            Self::Failed => 3,
        }
    }
    fn parse(code: i64) -> Result<Self> {
        match code {
            0 => Ok(Self::Queued),
            1 => Ok(Self::Dispatched),
            2 => Ok(Self::Uncertain),
            3 => Ok(Self::Failed),
            _ => Err(DataError::Corrupt),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutboxRecord {
    pub operation: OperationId,
    pub room: NavigationRoomKey,
    pub kind: OutboxKind,
    pub phase: OutboxPhase,
    /// Typed action encoding is owned by the outbox reducer, never a credential.
    pub payload: Text<32768>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutboxEntry {
    pub ordinal: u64,
    pub record: OutboxRecord,
    pub saved_at: u64,
}
/// Reencoded ordinary-room preview, never authenticated/original media bytes.
/// The producer bounds the raster to 320 × 240; the store checks encoded format,
/// dimensions, message expiry, current room eligibility and account capacity.
#[derive(Clone, Eq, PartialEq)]
pub struct CacheThumbnail {
    pub room: NavigationRoomKey,
    pub message: u64,
    pub placeholder: Text<128>,
    pub bytes: Arc<[u8]>,
    pub saved_at: u64,
    pub expires_at: Option<u64>,
    pub attachment: CachedAttachment,
}
impl std::fmt::Debug for CacheThumbnail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CacheThumbnail")
            .field("message", &self.message)
            .field("bytes", &self.bytes.len())
            .field("saved_at", &self.saved_at)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataSnapshot {
    pub rooms: Page<CacheRoom, CACHE_ROOM_LIMIT>,
    pub histories: Page<CacheHistory, CACHE_HISTORY_LIMIT>,
    pub outbox: Page<OutboxEntry, OUTBOX_LIMIT>,
    pub thumbnails: Page<CacheThumbnail, CACHE_THUMBNAIL_LIMIT>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataCommand {
    Load,
    /// Complete freshly approved ordinary-room snapshot. Omitted cached rooms
    /// and histories are removed atomically; pending intents are retained.
    PutRooms(Page<CacheRoom, CACHE_ROOM_LIMIT>),
    PutHistory(CacheHistory),
    PutThumbnail(CacheThumbnail),
    RemoveThumbnail {
        room: NavigationRoomKey,
        message: u64,
        placeholder: Text<128>,
    },
    /// Bounded overload collapse for required preview revocations only.
    ClearThumbnails,
    MemberCount {
        room: NavigationRoomKey,
        count: u32,
    },
    /// Removes cached observations only; intents require explicit settlement.
    RemoveRoom(NavigationRoomKey),
    Enqueue(OutboxRecord),
    /// Atomically retires a definitively failed body intent and admits its exact
    /// payload with a new operation identity. A lost reply can be retried safely.
    Replace {
        previous: OperationId,
        record: OutboxRecord,
    },
    SetPhase {
        operation: OperationId,
        phase: OutboxPhase,
    },
    Acknowledge(OperationId),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataResult {
    Snapshot(DataSnapshot),
    Queued(OutboxEntry),
    Saved,
}

pub(super) struct Store {
    ledger: Arc<Mutex<Ledger>>,
    account: AccountId,
}
fn filename(account: AccountId) -> String {
    format!("data-{:032x}.sqlite", account.as_local())
}
impl Store {
    pub(super) fn new(ledger: Arc<Mutex<Ledger>>, account: AccountId) -> Self {
        Self { ledger, account }
    }
    pub(super) fn execute(&self, command: DataCommand, now: u64) -> Result<DataResult> {
        match command {
            DataCommand::Load => return self.load(now).map(DataResult::Snapshot),
            DataCommand::PutRooms(rooms) => self.put_rooms(&rooms, now)?,
            DataCommand::PutHistory(history) => self.put_history(&history, now)?,
            DataCommand::PutThumbnail(thumbnail) => self.put_thumbnail(&thumbnail, now)?,
            DataCommand::RemoveThumbnail {
                room,
                message,
                placeholder,
            } => self.remove_thumbnail(&room, message, &placeholder)?,
            DataCommand::ClearThumbnails => self.clear_thumbnails()?,
            DataCommand::MemberCount { room, count } => self.member_count(&room, count, now)?,
            DataCommand::RemoveRoom(room) => self.remove_cached_room(&room)?,
            DataCommand::Enqueue(record) => {
                return self.enqueue(&record, now).map(DataResult::Queued);
            }
            DataCommand::Replace { previous, record } => {
                return self.replace(previous, &record, now).map(DataResult::Queued);
            }
            DataCommand::SetPhase { operation, phase } => self.set_phase(operation, phase)?,
            DataCommand::Acknowledge(operation) => self.acknowledge(operation)?,
        }
        Ok(DataResult::Saved)
    }
    fn connection(&self) -> Result<(std::sync::MutexGuard<'_, Ledger>, Connection)> {
        let ledger = self.ledger.lock().map_err(|_| DataError::Unavailable)?;
        let account = ledger
            .account(self.account)
            .map_err(|_| DataError::Ineligible)?;
        // Eligibility, not a live scope: ordinary offline cache reads must work
        // before credential materialization. Nothing here grants network use.
        if account.disposition != 2 || account.eligible.is_none() {
            return Err(DataError::Ineligible);
        }
        let db = open(&ledger.dir, self.account)?;
        Ok((ledger, db))
    }
    pub(super) fn load(&self, now: u64) -> Result<DataSnapshot> {
        let (ledger, mut db) = self.connection()?;
        prune(&mut db, now)?;
        let rooms = read_rooms(&db, now)?;
        for room in rooms.as_slice() {
            self.check_room(&ledger, &room.key)?;
        }
        let (history_count, history_bytes): (i64, i64) = sql(db.query_row(
            "SELECT count(*),coalesce(sum(length(bytes)),0) FROM histories",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ))?;
        if history_count > CACHE_HISTORY_LIMIT as i64 || history_bytes > CACHE_BYTES as i64 {
            return Err(DataError::Corrupt);
        }
        let mut histories = Page::empty();
        let keys: Vec<String> = {
            let mut query =
                sql(db.prepare("SELECT room FROM histories ORDER BY saved DESC,room LIMIT 9"))?;
            sql(query.query_map([], |row| row.get(0)))?
                .collect::<rusqlite::Result<_>>()
                .map_err(|_| DataError::Corrupt)?
        };
        for key in keys {
            let key = NavigationRoomKey::new(&key).map_err(|_| DataError::Corrupt)?;
            if let Some(history) = read_history(&db, &key, now)? {
                histories
                    .try_push(history)
                    .map_err(|_| DataError::Corrupt)?;
            }
        }
        let outbox = read_outbox(&db)?;
        for entry in outbox.as_slice() {
            self.check_room(&ledger, &entry.record.room)?;
        }
        let thumbnails = thumbnails::read(&db, now)?;
        for thumbnail in thumbnails.as_slice() {
            self.check_room(&ledger, &thumbnail.room)?;
        }
        Ok(DataSnapshot {
            rooms,
            histories,
            outbox,
            thumbnails,
        })
    }
    pub(super) fn put_rooms(
        &self,
        rooms: &Page<CacheRoom, CACHE_ROOM_LIMIT>,
        now: u64,
    ) -> Result<()> {
        let (ledger, mut db) = self.connection()?;
        prune(&mut db, now)?;
        let tx = sql(db.transaction())?;
        let existing: Vec<String> = {
            let mut statement = sql(tx.prepare("SELECT room FROM rooms LIMIT 101"))?;
            let mut rows = sql(statement.query([]))?;
            let mut keys = Vec::new();
            while let Some(row) = sql(rows.next())? {
                if keys.len() == CACHE_ROOM_LIMIT {
                    return Err(DataError::Corrupt);
                }
                keys.push(row.get(0).map_err(|_| DataError::Corrupt)?);
            }
            keys
        };
        for key in existing {
            if !rooms.as_slice().iter().any(|room| room.key.as_str() == key) {
                sql(tx.execute("DELETE FROM histories WHERE room=?1", [key.as_str()]))?;
                sql(tx.execute("DELETE FROM rooms WHERE room=?1", [key.as_str()]))?;
            }
        }
        for room in rooms.as_slice() {
            self.check_room(&ledger, &room.key)?;
            if room.kind == CachedRoomKind::Unknown
                || !fresh(room.saved_at, now)
                || room.participant_count_at.is_some_and(|at| at > now)
            {
                return Err(DataError::InvalidInput);
            }
            let bytes = codec::encode(room, META_BYTES)?;
            thumbnails::reserve_for_write(&tx, bytes.len())?;
            sql(tx.execute("INSERT INTO rooms(room,saved,bytes) VALUES(?1,?2,?3) ON CONFLICT(room) DO UPDATE SET saved=excluded.saved,bytes=excluded.bytes",
                params![room.key.as_str(), integer(room.saved_at)?, bytes]))?;
        }
        loop {
            let count: i64 = sql(tx.query_row("SELECT count(*) FROM rooms", [], |row| row.get(0)))?;
            if count <= CACHE_ROOM_LIMIT as i64 {
                break;
            }
            let removed = sql(tx.execute("DELETE FROM rooms WHERE room=(SELECT room FROM rooms WHERE room NOT IN (SELECT room FROM outbox) ORDER BY saved,room LIMIT 1)", []))?;
            if removed != 1 {
                return Err(DataError::Capacity);
            }
        }
        sql(tx.execute(
            "DELETE FROM histories WHERE room NOT IN (SELECT room FROM rooms)",
            [],
        ))?;
        sql(tx.commit())
    }
    pub(super) fn put_history(&self, history: &CacheHistory, now: u64) -> Result<()> {
        if !fresh(history.saved_at, now) {
            return Err(DataError::InvalidInput);
        }
        let cleaned = clean_history(history, now)?;
        let bytes = codec::encode(&cleaned, HISTORY_BYTES)?;
        let (ledger, mut db) = self.connection()?;
        self.check_room(&ledger, &history.room)?;
        prune(&mut db, now)?;
        let tx = sql(db.transaction())?;
        let known: bool = sql(tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM rooms WHERE room=?1)",
            [history.room.as_str()],
            |row| row.get(0),
        ))?;
        if !known {
            return Err(DataError::InvalidInput);
        }
        thumbnails::remove_unavailable(&tx, history, now)?;
        sql(tx.execute(
            "DELETE FROM histories WHERE room=?1",
            [history.room.as_str()],
        ))?;
        loop {
            let (count, size): (i64, i64) = sql(tx.query_row(
                "SELECT count(*),coalesce(sum(length(bytes)),0) FROM histories",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            ))?;
            if count < CACHE_HISTORY_LIMIT as i64 && size + bytes.len() as i64 <= CACHE_BYTES as i64
            {
                break;
            }
            let removed = sql(tx.execute("DELETE FROM histories WHERE room=(SELECT room FROM histories ORDER BY saved,room LIMIT 1)", []))?;
            if removed != 1 {
                return Err(DataError::Capacity);
            }
        }
        thumbnails::reserve_for_write(&tx, bytes.len())?;
        sql(tx.execute(
            "INSERT INTO histories(room,saved,bytes) VALUES(?1,?2,?3)",
            params![history.room.as_str(), integer(history.saved_at)?, bytes],
        ))?;
        sql(tx.commit())
    }
    pub(super) fn member_count(
        &self,
        room: &NavigationRoomKey,
        count: u32,
        now: u64,
    ) -> Result<()> {
        let (ledger, mut db) = self.connection()?;
        self.check_room(&ledger, room)?;
        let tx = sql(db.transaction())?;
        let row: Option<Vec<u8>> = sql(tx
            .query_row(
                "SELECT bytes FROM rooms WHERE room=?1 AND length(bytes)<=?2",
                params![room.as_str(), META_BYTES as i64],
                |row| row.get(0),
            )
            .optional())?;
        let Some(bytes) = row else {
            return Err(DataError::InvalidInput);
        };
        let mut value: CacheRoom = codec::decode(&bytes, META_BYTES)?;
        value.participant_count = Some(count);
        value.participant_count_at = Some(now);
        let bytes = codec::encode(&value, META_BYTES)?;
        thumbnails::reserve_for_write(&tx, bytes.len())?;
        sql(tx.execute(
            "UPDATE rooms SET bytes=?2 WHERE room=?1",
            params![room.as_str(), bytes],
        ))?;
        sql(tx.commit())
    }
    /// Cache-only invalidation; the reducer must settle/reject retained intents.
    pub(super) fn remove_cached_room(&self, room: &NavigationRoomKey) -> Result<()> {
        let (_ledger, mut db) = self.connection()?;
        let tx = sql(db.transaction())?;
        sql(tx.execute("DELETE FROM histories WHERE room=?1", [room.as_str()]))?;
        sql(tx.execute("DELETE FROM rooms WHERE room=?1", [room.as_str()]))?;
        sql(tx.commit())
    }
    /// Transaction commits before the caller may acknowledge durable admission.
    /// The same operation is idempotent only if its entire record is unchanged.
    pub(super) fn enqueue(&self, record: &OutboxRecord, now: u64) -> Result<OutboxEntry> {
        if record.operation.as_local() == 0 || record.phase != OutboxPhase::Queued {
            return Err(DataError::InvalidInput);
        }
        let saved = integer(now)?;
        let (ledger, mut db) = self.connection()?;
        self.check_room(&ledger, &record.room)?;
        let tx = sql(db.transaction())?;
        let existing = read_outbox(&tx)?
            .as_slice()
            .iter()
            .find(|entry| entry.record.operation == record.operation)
            .cloned();
        if let Some(existing) = existing {
            return if existing.record == *record {
                Ok(existing)
            } else {
                Err(DataError::InvalidInput)
            };
        }
        let known: bool = sql(tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM rooms WHERE room=?1)",
            [record.room.as_str()],
            |row| row.get(0),
        ))?;
        if !known {
            return Err(DataError::InvalidInput);
        }
        let (count, size): (i64, i64) = sql(tx.query_row(
            "SELECT count(*),coalesce(sum(length(CAST(payload AS BLOB))),0) FROM outbox",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ))?;
        if count >= OUTBOX_LIMIT as i64
            || size + record.payload.as_str().len() as i64 > OUTBOX_BYTES as i64
        {
            return Err(DataError::Capacity);
        }
        thumbnails::reserve_for_write(&tx, record.payload.as_str().len())?;
        sql(tx.execute(
            "INSERT INTO outbox(operation,room,kind,phase,payload,saved) VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                operation(record.operation),
                record.room.as_str(),
                record.kind.code(),
                record.phase.code(),
                record.payload.as_str(),
                saved
            ],
        ))?;
        let ordinal = u64::try_from(tx.last_insert_rowid()).map_err(|_| DataError::Corrupt)?;
        sql(tx.commit())?;
        Ok(OutboxEntry {
            ordinal,
            record: record.clone(),
            saved_at: now,
        })
    }
    /// A dispatched/uncertain operation may not silently return to Queued.
    pub(super) fn set_phase(&self, operation_id: OperationId, phase: OutboxPhase) -> Result<()> {
        let (_ledger, mut db) = self.connection()?;
        let tx = sql(db.transaction())?;
        let old: Option<i64> = sql(tx
            .query_row(
                "SELECT phase FROM outbox WHERE operation=?1",
                [operation(operation_id)],
                |row| row.get(0),
            )
            .optional())?;
        let Some(old) = old else {
            return Err(DataError::InvalidInput);
        };
        let old = OutboxPhase::parse(old)?;
        if old == OutboxPhase::Failed && phase != OutboxPhase::Failed
            || phase == OutboxPhase::Failed
                && !matches!(
                    old,
                    OutboxPhase::Queued | OutboxPhase::Dispatched | OutboxPhase::Failed
                )
        {
            return Err(DataError::InvalidInput);
        }
        if phase == OutboxPhase::Queued && old != OutboxPhase::Queued {
            return Err(DataError::InvalidInput);
        }
        sql(tx.execute(
            "UPDATE outbox SET phase=?2 WHERE operation=?1",
            params![operation(operation_id), phase.code()],
        ))?;
        sql(tx.commit())
    }
    /// Failed drafts are replaced and admitted in one transaction: capacity or
    /// storage failure leaves the original intact, including at 64 records.
    pub(super) fn replace(
        &self,
        previous: OperationId,
        record: &OutboxRecord,
        now: u64,
    ) -> Result<OutboxEntry> {
        if previous.as_local() == 0
            || record.operation.as_local() == 0
            || previous == record.operation
            || record.phase != OutboxPhase::Queued
            || !matches!(
                record.kind,
                OutboxKind::Send | OutboxKind::Edit | OutboxKind::Schedule
            )
        {
            return Err(DataError::InvalidInput);
        }
        record.intent().map_err(|_| DataError::InvalidInput)?;
        let saved = integer(now)?;
        let (ledger, mut db) = self.connection()?;
        self.check_room(&ledger, &record.room)?;
        let tx = sql(db.transaction())?;
        let entries = read_outbox(&tx)?;
        let prior = entries
            .as_slice()
            .iter()
            .find(|e| e.record.operation == previous);
        let existing = entries
            .as_slice()
            .iter()
            .find(|e| e.record.operation == record.operation);
        if let Some(old) = prior {
            if old.record.phase != OutboxPhase::Failed
                || old.record.room != record.room
                || old.record.kind != record.kind
                || old.record.payload != record.payload
            {
                return Err(DataError::InvalidInput);
            }
            old.record.intent()?;
        }
        if let Some(existing) = existing {
            if existing.record != *record {
                return Err(DataError::InvalidInput);
            }
            // Normally `previous` is already absent after the successful commit.
            // If an exact new record exists alongside a valid failed draft, the
            // same atomic retirement still cannot delete an unrelated intent.
            if prior.is_some() {
                sql(tx.execute(
                    "DELETE FROM outbox WHERE operation=?1",
                    [operation(previous)],
                ))?;
                sql(tx.commit())?;
            }
            return Ok(existing.clone());
        }
        if prior.is_none() {
            return Err(DataError::InvalidInput);
        }
        let known: bool = sql(tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM rooms WHERE room=?1)",
            [record.room.as_str()],
            |r| r.get(0),
        ))?;
        if !known {
            return Err(DataError::InvalidInput);
        }
        sql(tx.execute(
            "DELETE FROM outbox WHERE operation=?1",
            [operation(previous)],
        ))?;
        let (count, size): (i64, i64) = sql(tx.query_row(
            "SELECT count(*),coalesce(sum(length(CAST(payload AS BLOB))),0) FROM outbox",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ))?;
        if count >= OUTBOX_LIMIT as i64
            || size + record.payload.as_str().len() as i64 > OUTBOX_BYTES as i64
        {
            return Err(DataError::Capacity);
        }
        thumbnails::reserve_for_write(&tx, record.payload.as_str().len())?;
        sql(tx.execute(
            "INSERT INTO outbox(operation,room,kind,phase,payload,saved) VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                operation(record.operation),
                record.room.as_str(),
                record.kind.code(),
                record.phase.code(),
                record.payload.as_str(),
                saved
            ],
        ))?;
        let ordinal = u64::try_from(tx.last_insert_rowid()).map_err(|_| DataError::Corrupt)?;
        sql(tx.commit())?;
        Ok(OutboxEntry {
            ordinal,
            record: record.clone(),
            saved_at: now,
        })
    }
    pub(super) fn acknowledge(&self, operation_id: OperationId) -> Result<()> {
        let (_ledger, db) = self.connection()?;
        sql(db.execute(
            "DELETE FROM outbox WHERE operation=?1",
            [operation(operation_id)],
        ))?;
        Ok(())
    }
    fn check_room(&self, ledger: &Ledger, room: &NavigationRoomKey) -> Result<()> {
        if !ledger
            .owns_room_key(self.account, room)
            .map_err(|_| DataError::Unavailable)?
        {
            return Err(DataError::InvalidInput);
        }
        Ok(())
    }
}

fn integer(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| DataError::InvalidInput)
}
fn operation(value: OperationId) -> String {
    format!("{:032x}", value.as_local())
}
fn fresh(saved: u64, now: u64) -> bool {
    saved <= now && now - saved < CACHE_LIFETIME
}
fn open(dir: &PrivateDir, account: AccountId) -> Result<Connection> {
    let name = filename(account);
    dir.prepare_database(&name)
        .map_err(|_| DataError::Unavailable)?;
    let db = sql(Connection::open_with_flags(
        dir.path().join(name),
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    ))?;
    sql(db.busy_timeout(std::time::Duration::ZERO))?;
    sql(db.set_limit(
        rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH,
        (HISTORY_BYTES + META_BYTES) as i32,
    ))?;
    let version: i64 = sql(db.query_row("PRAGMA user_version", [], |row| row.get(0)))?;
    if !(0..=3).contains(&version) {
        return Err(DataError::Corrupt);
    }
    sql(db.execute_batch("PRAGMA page_size=4096; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;
        PRAGMA temp_store=MEMORY; PRAGMA max_page_count=4096; PRAGMA trusted_schema=OFF; PRAGMA secure_delete=ON; PRAGMA foreign_keys=ON;
        CREATE TABLE IF NOT EXISTS rooms(room TEXT PRIMARY KEY CHECK(length(room)=35),saved INTEGER NOT NULL,bytes BLOB NOT NULL);
        CREATE TABLE IF NOT EXISTS histories(room TEXT PRIMARY KEY CHECK(length(room)=35),saved INTEGER NOT NULL,bytes BLOB NOT NULL);
        CREATE TABLE IF NOT EXISTS outbox(ordinal INTEGER PRIMARY KEY AUTOINCREMENT,operation TEXT NOT NULL UNIQUE CHECK(length(operation)=32),room TEXT NOT NULL CHECK(length(room)=35),kind INTEGER NOT NULL,phase INTEGER NOT NULL,payload TEXT NOT NULL,saved INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS thumbnails(room TEXT NOT NULL REFERENCES rooms(room) ON DELETE CASCADE CHECK(length(room)=35),message INTEGER NOT NULL CHECK(message>0),placeholder TEXT NOT NULL CHECK(length(CAST(placeholder AS BLOB)) BETWEEN 1 AND 128),saved INTEGER NOT NULL,expires INTEGER,used INTEGER NOT NULL,bytes BLOB NOT NULL CHECK(length(bytes) BETWEEN 1 AND 262144),attachment BLOB,PRIMARY KEY(room,message,placeholder));"))?;
    if version == 2 {
        // Schema two previews had no immutable descriptor. They are disposable;
        // never guess their association or block the required history/outbox.
        let columns: Vec<String> = {
            let mut q = sql(db.prepare("PRAGMA table_info(thumbnails)"))?;
            sql(q.query_map([], |r| r.get(1)))?
                .collect::<rusqlite::Result<_>>()
                .map_err(|_| DataError::Corrupt)?
        };
        if !columns.iter().any(|column| column == "attachment") {
            sql(db.execute_batch("ALTER TABLE thumbnails ADD COLUMN attachment BLOB;"))?;
        }
        sql(db.execute("DELETE FROM thumbnails", []))?;
    }
    sql(db.execute_batch("PRAGMA user_version=3;"))?;
    let page_size: i64 = sql(db.query_row("PRAGMA page_size", [], |row| row.get(0)))?;
    let page_count: i64 = sql(db.query_row("PRAGMA page_count", [], |row| row.get(0)))?;
    if page_size != 4096 || page_count > 4096 {
        return Err(DataError::Corrupt);
    }
    Ok(db)
}
fn prune(db: &mut Connection, now: u64) -> Result<()> {
    let now = integer(now)?;
    let oldest = now.saturating_sub(CACHE_LIFETIME as i64);
    let tx = sql(db.transaction())?;
    // Source metadata is part of the retained pending intent until explicit
    // settlement. Cache expiry still removes its message history after 30 days.
    sql(tx.execute(
        "DELETE FROM rooms WHERE (saved>?1 OR saved<=?2) AND room NOT IN (SELECT room FROM outbox)",
        params![now, oldest],
    ))?;
    thumbnails::prune(&tx, now as u64)?;
    sql(tx.execute(
        "DELETE FROM histories WHERE saved>?1 OR saved<=?2 OR room NOT IN (SELECT room FROM rooms)",
        params![now, oldest],
    ))?;
    sql(tx.commit())
}
fn read_rooms(db: &Connection, now: u64) -> Result<Page<CacheRoom, CACHE_ROOM_LIMIT>> {
    let mut query=sql(db.prepare("SELECT room,saved,bytes FROM rooms WHERE length(bytes)<=?1 ORDER BY saved DESC,room LIMIT 101"))?;
    let mut rows = sql(query.query([META_BYTES as i64]))?;
    let mut result = Page::empty();
    while let Some(row) = sql(rows.next())? {
        let key: String = row.get(0).map_err(|_| DataError::Corrupt)?;
        let saved: i64 = row.get(1).map_err(|_| DataError::Corrupt)?;
        let bytes: Vec<u8> = row.get(2).map_err(|_| DataError::Corrupt)?;
        let value: CacheRoom = codec::decode(&bytes, META_BYTES)?;
        let pending: bool = sql(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM outbox WHERE room=?1)",
            [key.as_str()],
            |r| r.get(0),
        ))?;
        if value.key.as_str() != key
            || integer(value.saved_at)? != saved
            || value.saved_at > now
            || (!fresh(value.saved_at, now) && !pending)
        {
            return Err(DataError::Corrupt);
        }
        result.try_push(value).map_err(|_| DataError::Corrupt)?;
    }
    Ok(result)
}
fn read_history(
    db: &Connection,
    room: &NavigationRoomKey,
    now: u64,
) -> Result<Option<CacheHistory>> {
    let bytes: Option<Vec<u8>> = sql(db
        .query_row(
            "SELECT bytes FROM histories WHERE room=?1 AND length(bytes)<=?2",
            params![room.as_str(), HISTORY_BYTES as i64],
            |row| row.get(0),
        )
        .optional())?;
    let Some(bytes) = bytes else { return Ok(None) };
    let value: CacheHistory = codec::decode(&bytes, HISTORY_BYTES)?;
    if value.room != *room || !fresh(value.saved_at, now) {
        return Err(DataError::Corrupt);
    }
    thumbnails::remove_unavailable(db, &value, now)?;
    let clean = clean_history(&value, now)?;
    if clean != value {
        let bytes = codec::encode(&clean, HISTORY_BYTES)?;
        sql(db.execute(
            "UPDATE histories SET bytes=?2 WHERE room=?1",
            params![room.as_str(), bytes],
        ))?;
    }
    Ok(Some(clean))
}
fn clean_history(history: &CacheHistory, now: u64) -> Result<CacheHistory> {
    let unavailable: Vec<u64> = history
        .messages
        .as_slice()
        .iter()
        .chain(history.pin_messages.as_slice())
        .filter(|m| m.deleted || m.expires_at.is_some_and(|at| at > 0 && at <= now))
        .map(|m| m.id)
        .collect();
    let mut result = history.clone();
    result.messages = Page::empty();
    let clean_message = |message: &CachedMessage| -> Result<Option<CachedMessage>> {
        if message.id == 0 || message.parent.as_ref().is_some_and(|p| p.id == 0) {
            return Err(DataError::InvalidInput);
        }
        if message.expires_at.is_some_and(|at| at > 0 && at <= now) {
            return Ok(None);
        }
        let mut message = message.clone();
        if message.deleted {
            message.text = Text::new("").map_err(|_| DataError::InvalidInput)?;
            message.reactions = Page::empty();
            message.attachments = Page::empty();
        }
        if let Some(parent) = message.parent.as_mut()
            && (parent.unavailable || unavailable.contains(&parent.id))
        {
            parent.unavailable = true;
            parent.text = Text::new("").map_err(|_| DataError::InvalidInput)?;
        }
        Ok(Some(message))
    };
    for message in history.messages.as_slice() {
        if let Some(message) = clean_message(message)? {
            result
                .messages
                .try_push(message)
                .map_err(|_| DataError::Capacity)?;
        }
    }
    result.pin_messages = Page::empty();
    for message in history.pin_messages.as_slice() {
        if !message.deleted
            && let Some(message) = clean_message(message)?
        {
            result
                .pin_messages
                .try_push(message)
                .map_err(|_| DataError::Capacity)?;
        }
    }
    result.pins = Page::empty();
    for &id in history.pins.as_slice() {
        if id == 0 {
            return Err(DataError::InvalidInput);
        }
        if !unavailable.contains(&id) {
            result.pins.try_push(id).map_err(|_| DataError::Capacity)?;
        }
    }
    Ok(result)
}
fn read_outbox(db: &Connection) -> Result<Page<OutboxEntry, OUTBOX_LIMIT>> {
    let mut query=sql(db.prepare("SELECT ordinal,operation,room,kind,phase,CASE WHEN length(CAST(payload AS BLOB))<=32768 THEN payload ELSE NULL END,saved FROM outbox ORDER BY ordinal LIMIT 65"))?;
    let mut rows = sql(query.query([]))?;
    let mut result = Page::empty();
    let mut bytes = 0;
    while let Some(row) = sql(rows.next())? {
        let ordinal: i64 = row.get(0).map_err(|_| DataError::Corrupt)?;
        let operation: String = row.get(1).map_err(|_| DataError::Corrupt)?;
        let room: String = row.get(2).map_err(|_| DataError::Corrupt)?;
        let kind: i64 = row.get(3).map_err(|_| DataError::Corrupt)?;
        let phase: i64 = row.get(4).map_err(|_| DataError::Corrupt)?;
        let payload: String = row.get(5).map_err(|_| DataError::Corrupt)?;
        let saved: i64 = row.get(6).map_err(|_| DataError::Corrupt)?;
        bytes += payload.len();
        if operation.len() != 32 || ordinal <= 0 || saved < 0 || bytes > OUTBOX_BYTES {
            return Err(DataError::Corrupt);
        }
        let operation = OperationId::from_local(
            u128::from_str_radix(&operation, 16).map_err(|_| DataError::Corrupt)?,
        );
        let record = OutboxRecord {
            operation,
            room: NavigationRoomKey::new(&room).map_err(|_| DataError::Corrupt)?,
            kind: OutboxKind::parse(kind)?,
            phase: OutboxPhase::parse(phase)?,
            payload: Text::new(&payload).map_err(|_| DataError::Corrupt)?,
        };
        result
            .try_push(OutboxEntry {
                ordinal: ordinal as u64,
                record,
                saved_at: saved as u64,
            })
            .map_err(|_| DataError::Corrupt)?;
    }
    Ok(result)
}
pub(super) fn remove(dir: &PrivateDir, account: AccountId) -> super::error::Result<()> {
    let name = filename(account);
    for suffix in ["-journal", "-wal", "-shm", ""] {
        dir.remove(&format!("{name}{suffix}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
