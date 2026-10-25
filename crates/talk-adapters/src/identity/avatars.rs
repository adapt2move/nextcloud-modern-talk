//! Disposable profile images, separate from restoration metadata and protected
//! chat/draft/attachment storage. Private account files, never credential files.
use super::{ledger::Ledger, private_fs::PrivateDir};
use crate::nextcloud::avatars::{AvatarStore, fresh};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use std::sync::{Arc, Mutex};
use talk_core::ids::AccountId;

const MAX_IMAGE: usize = 1024 * 1024;
const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_ENTRIES: usize = 128;

pub(super) struct Store {
    ledger: Arc<Mutex<Ledger>>,
    account: AccountId,
}
fn filename(account: AccountId) -> String {
    format!("avatars-{:032x}.sqlite", account.as_local())
}
impl Store {
    pub(super) fn new(ledger: Arc<Mutex<Ledger>>, account: AccountId) -> Self {
        Self { ledger, account }
    }
    fn connection(&self) -> Result<(std::sync::MutexGuard<'_, Ledger>, Connection), ()> {
        let ledger = self.ledger.lock().map_err(|_| ())?;
        let account = ledger.account(self.account).map_err(|_| ())?;
        // Recheck durable eligibility on every access. A disconnected client or
        // session-only save cannot create a durable cache via a stale store.
        if account.disposition != 2 || account.eligible.is_none() {
            return Err(());
        }
        let db = open(&ledger.dir, self.account)?;
        Ok((ledger, db))
    }
}
fn open(dir: &PrivateDir, account: AccountId) -> Result<Connection, ()> {
    let name = filename(account);
    dir.prepare_database(&name).map_err(|_| ())?;
    let db = Connection::open_with_flags(
        dir.path().join(name),
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(|_| ())?;
    db.busy_timeout(std::time::Duration::ZERO).map_err(|_| ())?;
    db.set_limit(
        rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH,
        (MAX_IMAGE + 1024) as i32,
    )
    .map_err(|_| ())?;
    // At most 16 MiB including SQLite overhead and free pages; logical payload
    // remains <= 8 MiB. DELETE journaling avoids long-lived WAL copies.
    db.execute_batch("PRAGMA page_size=4096; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;
        PRAGMA temp_store=MEMORY; PRAGMA max_page_count=4096; PRAGMA trusted_schema=OFF;
        PRAGMA secure_delete=ON;
        CREATE TABLE IF NOT EXISTS avatars(key TEXT PRIMARY KEY CHECK(length(key)=66), saved INTEGER NOT NULL, bytes BLOB NOT NULL);
        DELETE FROM avatars WHERE length(key)<>66 OR substr(key,1,2) NOT IN ('u-','r-');")
        .map_err(|_| ())?;
    let page_size: i64 = db
        .query_row("PRAGMA page_size", [], |r| r.get(0))
        .map_err(|_| ())?;
    let page_count: i64 = db
        .query_row("PRAGMA page_count", [], |r| r.get(0))
        .map_err(|_| ())?;
    if page_size != 4096 || page_count > 4096 {
        return Err(());
    }
    Ok(db)
}
impl AvatarStore for Store {
    fn retain_rooms(&mut self, allowed: &[String]) -> Result<(), ()> {
        if allowed.len() > 100 {
            return Err(());
        }
        let (_ledger, mut db) = self.connection()?;
        let tx = db.transaction().map_err(|_| ())?;
        let keys: Vec<String> = {
            let mut query = tx
                .prepare("SELECT key FROM avatars WHERE key GLOB 'r-*' LIMIT 129")
                .map_err(|_| ())?;
            query
                .query_map([], |r| r.get(0))
                .map_err(|_| ())?
                .collect::<Result<_, _>>()
                .map_err(|_| ())?
        };
        for key in keys {
            if !allowed.contains(&key) {
                tx.execute("DELETE FROM avatars WHERE key=?1", [key])
                    .map_err(|_| ())?;
            }
        }
        tx.commit().map_err(|_| ())
    }
    fn remove(&mut self, key: &str) -> Result<(), ()> {
        let (_ledger, db) = self.connection()?;
        db.execute("DELETE FROM avatars WHERE key=?1", [key])
            .map_err(|_| ())?;
        Ok(())
    }
    fn get(&mut self, key: &str, now: u64) -> Result<Option<(Vec<u8>, u64)>, ()> {
        let (_ledger, db) = self.connection()?;
        let row: Option<(i64, Vec<u8>)> = db
            .query_row(
                "SELECT saved,bytes FROM avatars WHERE key=?1 AND length(bytes)<=?2",
                params![key, MAX_IMAGE as i64],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|_| ())?;
        match row {
            Some((at, bytes)) if at >= 0 && fresh(at as u64, now) => Ok(Some((bytes, at as u64))),
            Some(_) => {
                db.execute("DELETE FROM avatars WHERE key=?1", [key])
                    .map_err(|_| ())?;
                Ok(None)
            }
            None => Ok(None),
        }
    }
    fn put(&mut self, key: &str, bytes: &[u8], now: u64) -> Result<(), ()> {
        if bytes.len() > MAX_IMAGE || key.len() != 66 || now > i64::MAX as u64 {
            return Ok(());
        }
        let (_ledger, mut db) = self.connection()?;
        let tx = db.transaction().map_err(|_| ())?;
        tx.execute(
            "DELETE FROM avatars WHERE key=?1 OR saved>?2 OR saved<=?3",
            params![
                key,
                now as i64,
                now.saturating_sub(crate::nextcloud::avatars::TTL) as i64
            ],
        )
        .map_err(|_| ())?;
        loop {
            let (count, size): (i64, i64) = tx
                .query_row(
                    "SELECT count(*),coalesce(sum(length(bytes)),0) FROM avatars",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(|_| ())?;
            if count < MAX_ENTRIES as i64 && size + bytes.len() as i64 <= MAX_BYTES as i64 {
                break;
            }
            tx.execute("DELETE FROM avatars WHERE key=(SELECT key FROM avatars ORDER BY saved,rowid LIMIT 1)", [])
                .map_err(|_| ())?;
        }
        tx.execute(
            "INSERT INTO avatars VALUES(?1,?2,?3)",
            params![key, now as i64, bytes],
        )
        .map_err(|_| ())?;
        tx.commit().map_err(|_| ())
    }
}
pub(super) fn remove(dir: &PrivateDir, account: AccountId) -> super::error::Result<()> {
    let name = filename(account);
    for suffix in ["-journal", "-wal", "-shm", ""] {
        dir.remove(&format!("{name}{suffix}"))?;
    }
    Ok(())
}
