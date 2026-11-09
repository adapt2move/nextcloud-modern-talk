//! Disposable attachment previews, separate from restoration metadata and the chat
//! database. Private account files; a file per account, bounded in entries and bytes.
use super::{ledger::Ledger, private_fs::PrivateDir};
use crate::nextcloud::previews::{Preview, PreviewStore, TTL, fresh};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use std::sync::{Arc, Mutex};
use talk_core::ids::AccountId;

const MAX_IMAGE: usize = 4 * 1024 * 1024;
const MAX_BYTES: usize = 96 * 1024 * 1024;
const MAX_ENTRIES: usize = 128;
// 96 MiB of blobs plus SQLite overhead and free pages stay below 112 MiB.
const MAX_PAGES: i64 = 28_672;

pub(super) struct Store {
    ledger: Arc<Mutex<Ledger>>,
    account: AccountId,
}
fn filename(account: AccountId) -> String {
    format!("previews-{:032x}.sqlite", account.as_local())
}
impl Store {
    pub(super) fn new(ledger: Arc<Mutex<Ledger>>, account: AccountId) -> Self {
        Self { ledger, account }
    }
    fn connection(&self) -> Result<(std::sync::MutexGuard<'_, Ledger>, Connection), ()> {
        let ledger = self.ledger.lock().map_err(|_| ())?;
        let account = ledger.account(self.account).map_err(|_| ())?;
        // Recheck durable eligibility on every access, like the avatar store.
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
    db.execute_batch(&format!(
        "PRAGMA page_size=4096; PRAGMA journal_mode=DELETE; PRAGMA synchronous=NORMAL;
        PRAGMA temp_store=MEMORY; PRAGMA max_page_count={MAX_PAGES}; PRAGMA trusted_schema=OFF;
        PRAGMA secure_delete=ON;
        CREATE TABLE IF NOT EXISTS previews(key TEXT PRIMARY KEY CHECK(length(key)=64),
            saved INTEGER NOT NULL, used INTEGER NOT NULL, pages INTEGER, bytes BLOB NOT NULL);"
    ))
    .map_err(|_| ())?;
    let page_size: i64 = db
        .query_row("PRAGMA page_size", [], |r| r.get(0))
        .map_err(|_| ())?;
    if page_size != 4096 {
        return Err(());
    }
    Ok(db)
}
impl PreviewStore for Store {
    fn get(&mut self, key: &str, now: u64) -> Result<Option<Preview>, ()> {
        let (_ledger, db) = self.connection()?;
        let row: Option<(i64, Option<i64>, Vec<u8>)> = db
            .query_row(
                "SELECT saved,pages,bytes FROM previews WHERE key=?1 AND length(bytes)<=?2",
                params![key, MAX_IMAGE as i64],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|_| ())?;
        match row {
            Some((at, pages, bytes)) if at >= 0 && fresh(at as u64, now) => {
                db.execute(
                    "UPDATE previews SET used=?2 WHERE key=?1",
                    params![key, now as i64],
                )
                .map_err(|_| ())?;
                let pages = pages.and_then(|p| u32::try_from(p).ok());
                Ok(Some((bytes, pages)))
            }
            Some(_) => {
                db.execute("DELETE FROM previews WHERE key=?1", [key])
                    .map_err(|_| ())?;
                Ok(None)
            }
            None => Ok(None),
        }
    }
    fn put(&mut self, key: &str, preview: &Preview, now: u64) -> Result<(), ()> {
        let (bytes, pages) = preview;
        if bytes.is_empty() || bytes.len() > MAX_IMAGE || key.len() != 64 || now > i64::MAX as u64 {
            return Ok(());
        }
        let (_ledger, mut db) = self.connection()?;
        let tx = db.transaction().map_err(|_| ())?;
        tx.execute(
            "DELETE FROM previews WHERE key=?1 OR saved>?2 OR saved<=?3",
            params![key, now as i64, now.saturating_sub(TTL) as i64],
        )
        .map_err(|_| ())?;
        // Least recently opened goes first.
        loop {
            let (count, size): (i64, i64) = tx
                .query_row(
                    "SELECT count(*),coalesce(sum(length(bytes)),0) FROM previews",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(|_| ())?;
            if count < MAX_ENTRIES as i64 && size + bytes.len() as i64 <= MAX_BYTES as i64 {
                break;
            }
            tx.execute(
                "DELETE FROM previews WHERE key=(SELECT key FROM previews ORDER BY used,rowid LIMIT 1)",
                [],
            )
            .map_err(|_| ())?;
        }
        tx.execute(
            "INSERT INTO previews VALUES(?1,?2,?2,?3,?4)",
            params![key, now as i64, pages.map(i64::from), bytes],
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
