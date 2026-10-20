//! Single-owner private metadata SQLite ledger (no app credentials). Provider writes never run on this owner.
use super::{
    error::{IdentityError, Result},
    private_fs::PrivateDir,
};
use rusqlite::{Connection, OpenFlags, params};
use std::{fs::File, sync::Arc};
use talk_core::{
    bounded::Text,
    identity::{ProviderBinding, QualifiedItem, StorageBackend, StorageFailure},
    ids::{AccountId, HostId, LoginAttemptId, OperationId, ProfileId, VaultGeneration},
    preferences::navigation::{NavigationAccountKey, NavigationRoomKey},
};

pub(super) fn random() -> Result<u128> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| IdentityError::WorkerUnavailable)?;
    Ok(u128::from_le_bytes(bytes))
}
fn id(n: u128) -> String {
    format!("{n:032x}")
}
fn number(s: String) -> Result<u128> {
    if s.len() != 32 {
        return Err(IdentityError::RecoveryRequired);
    }
    u128::from_str_radix(&s, 16).map_err(|_| IdentityError::RecoveryRequired)
}
fn revision(value: u64) -> String {
    format!("{value:016x}")
}
fn parse_revision(value: String) -> Result<u64> {
    if value.len() != 16 {
        return Err(IdentityError::RecoveryRequired);
    }
    u64::from_str_radix(&value, 16).map_err(|_| IdentityError::RecoveryRequired)
}
fn next_revision(value: u64) -> Result<String> {
    value
        .checked_add(1)
        .map(revision)
        .ok_or(IdentityError::RecoveryRequired)
}
fn db<T>(v: rusqlite::Result<T>) -> Result<T> {
    v.map_err(|_| IdentityError::RecoveryRequired)
}
fn unsafe_label_character(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}
fn backend_id(b: StorageBackend) -> i64 {
    match b {
        StorageBackend::PrivateFile => 1,
        StorageBackend::MacKeychain => 2,
        StorageBackend::WindowsCredential => 3,
        StorageBackend::LinuxSecretService => 4,
    }
}
fn backend(n: i64) -> Result<StorageBackend> {
    match n {
        1 => Ok(StorageBackend::PrivateFile),
        2 => Ok(StorageBackend::MacKeychain),
        3 => Ok(StorageBackend::WindowsCredential),
        4 => Ok(StorageBackend::LinuxSecretService),
        _ => Err(IdentityError::RecoveryRequired),
    }
}
const FAILURES: [StorageFailure; 10] = [
    StorageFailure::Missing,
    StorageFailure::Locked,
    StorageFailure::AccessDenied,
    StorageFailure::Corrupt,
    StorageFailure::TemporarilyUnavailable,
    StorageFailure::Unavailable,
    StorageFailure::Unsupported,
    StorageFailure::Cancelled,
    StorageFailure::Conflict,
    StorageFailure::Other,
];
fn failure_id(failure: StorageFailure) -> i64 {
    FAILURES
        .iter()
        .position(|item| *item == failure)
        .expect("complete allowlist") as i64
}
fn failure(value: i64) -> Result<StorageFailure> {
    usize::try_from(value)
        .ok()
        .and_then(|index| FAILURES.get(index))
        .copied()
        .ok_or(IdentityError::RecoveryRequired)
}
fn category(error: IdentityError) -> StorageFailure {
    match error {
        IdentityError::Storage { failure, .. } => failure,
        _ => StorageFailure::Other,
    }
}
#[derive(Clone)]
pub(super) struct Account {
    pub id: AccountId,
    pub host: HostId,
    pub origin: String,
    pub user: String,
    pub revision: u64,
    /// 0 suppressed, 1 unresolved, 2 sole eligible; mode 0 session, 1 native, 2 file.
    pub disposition: u8,
    pub mode: u8,
    pub eligible: Option<OperationId>,
}

#[cfg(test)]
mod boundaries {
    use super::*;
    #[test]
    fn sqlite_roundtrips_entire_identifier_and_revision_domains() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE boundaries(id TEXT NOT NULL, revision TEXT NOT NULL)")
            .unwrap();
        for identity in [
            0,
            1,
            u64::MAX as u128,
            (u64::MAX as u128) + 1,
            1u128 << 127,
            u128::MAX,
        ] {
            for rev in [0, 1, i64::MAX as u64, (i64::MAX as u64) + 1, u64::MAX] {
                db.execute("DELETE FROM boundaries", []).unwrap();
                db.execute(
                    "INSERT INTO boundaries VALUES(?1,?2)",
                    params![id(identity), revision(rev)],
                )
                .unwrap();
                let row: (String, String) = db
                    .query_row("SELECT id,revision FROM boundaries", [], |r| {
                        Ok((r.get(0)?, r.get(1)?))
                    })
                    .unwrap();
                assert_eq!(number(row.0).unwrap(), identity);
                assert_eq!(parse_revision(row.1).unwrap(), rev);
                if rev == u64::MAX {
                    assert!(next_revision(rev).is_err());
                } else {
                    assert_eq!(
                        parse_revision(next_revision(rev).unwrap()).unwrap(),
                        rev + 1
                    );
                }
            }
        }
        assert!(number("fffffffffffffffffffffffffffffffff".into()).is_err());
        assert!(parse_revision("10000000000000000".into()).is_err());
    }
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Operation {
    pub id: OperationId,
    pub attempt: LoginAttemptId,
    pub item: QualifiedItem,
    pub retained: Option<OperationId>,
    /// 0 dispatched/uncertain, 1 present and no writer, 2 known absent/nonwrite.
    pub settlement: u8,
    pub abandoned: bool,
    pub cleanup: bool,
    pub activated: bool,
    pub retired: bool,
    pub last_failure: Option<StorageFailure>,
}
pub(super) struct Ledger {
    db: Connection,
    pub dir: Arc<PrivateDir>,
    pub lock: Arc<File>,
    pub profile: ProfileId,
    pub store: u128,
}
impl Ledger {
    pub fn open(dir: Arc<PrivateDir>) -> Result<Self> {
        let lock = dir.lock()?;
        let marker = dir.read("profile.id", 128);
        let binding = dir.binding()?;
        let fresh = matches!(
            marker,
            Err(IdentityError::Storage {
                failure: talk_core::identity::StorageFailure::Missing,
                ..
            })
        );
        let profile = if fresh {
            if dir.path().join("restoration.sqlite").exists() {
                return Err(IdentityError::RecoveryRequired);
            }
            ProfileId::from_local(random()?)
        } else {
            ProfileId::from_local(number(
                String::from_utf8(marker?.to_vec()).map_err(|_| IdentityError::RecoveryRequired)?,
            )?)
        };
        if !fresh && !dir.path().join("restoration.sqlite").exists() {
            return Err(IdentityError::RecoveryRequired);
        }
        dir.prepare_database("restoration.sqlite")?;
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW;
        let connection = db(Connection::open_with_flags(
            dir.path().join("restoration.sqlite"),
            flags,
        ))?;
        db(connection.busy_timeout(std::time::Duration::ZERO))?;
        db(connection.set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH, 32768))?;
        db(connection.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=EXTRA; PRAGMA fullfsync=ON; PRAGMA foreign_keys=ON; PRAGMA temp_store=MEMORY; PRAGMA max_page_count=1024; PRAGMA trusted_schema=OFF;"))?;
        let mode: String = db(connection.query_row("PRAGMA journal_mode", [], |r| r.get(0)))?;
        let sync: i64 = db(connection.query_row("PRAGMA synchronous", [], |r| r.get(0)))?;
        let full: i64 = db(connection.query_row("PRAGMA fullfsync", [], |r| r.get(0)))?;
        if mode != "delete" || sync != 3 || full != 1 {
            return Err(IdentityError::RecoveryRequired);
        }
        let store = if fresh {
            let store = random()?;
            db(connection.execute_batch("BEGIN IMMEDIATE;
            CREATE TABLE meta(schema INTEGER NOT NULL,profile TEXT NOT NULL,store TEXT NOT NULL,root TEXT NOT NULL);
            CREATE TABLE accounts(id TEXT PRIMARY KEY,host TEXT NOT NULL,origin TEXT NOT NULL,user TEXT NOT NULL,revision TEXT NOT NULL,disposition INTEGER NOT NULL,mode INTEGER NOT NULL,eligible TEXT);
            CREATE UNIQUE INDEX canonical_identity ON accounts(origin,user) WHERE user<>'';
            CREATE TABLE operations(id TEXT PRIMARY KEY,account TEXT NOT NULL REFERENCES accounts(id),attempt TEXT NOT NULL,backend INTEGER NOT NULL,generation TEXT NOT NULL,retained TEXT,settlement INTEGER NOT NULL,abandoned INTEGER NOT NULL,cleanup INTEGER NOT NULL,activated INTEGER NOT NULL,login_name TEXT NOT NULL,retired INTEGER NOT NULL,last_failure INTEGER);
            CREATE UNIQUE INDEX qualified_generation ON operations(account,backend,generation);
            "))?;
            db(connection.execute(
                "INSERT INTO meta VALUES(5,?1,?2,?3)",
                params![id(profile.as_local()), id(store), binding],
            ))?;
            db(connection.execute_batch("COMMIT;"))?;
            dir.create("profile.id", id(profile.as_local()).as_bytes())?;
            store
        } else {
            let tuple: (i64, String, String, String) = db(connection.query_row(
                "SELECT schema,profile,store,root FROM meta",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            ))?;
            if !matches!(tuple.0, 5 | 6) || tuple.1 != id(profile.as_local()) || tuple.3 != binding
            {
                return Err(IdentityError::RecoveryRequired);
            }
            let integrity: String =
                db(connection.query_row("PRAGMA quick_check", [], |r| r.get(0)))?;
            if integrity != "ok" {
                return Err(IdentityError::RecoveryRequired);
            }
            number(tuple.2)?
        };
        let schema: i64 =
            db(connection.query_row("SELECT schema FROM meta", [], |row| row.get(0)))?;
        if schema == 5 {
            // Additive, transactional migration. Existing eligibility, exact provider
            // bindings and pending writers are untouched; labels confer no authority.
            db(connection.execute_batch("BEGIN IMMEDIATE;
                CREATE TABLE account_labels(account TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE, display_name TEXT);
                CREATE TABLE navigation_rooms(account TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE, server_id TEXT NOT NULL, local_key TEXT NOT NULL UNIQUE, PRIMARY KEY(account,server_id));
                UPDATE meta SET schema=6 WHERE schema=5;
                COMMIT;"))?;
        }
        let ledger = Self {
            db: connection,
            dir,
            lock: Arc::new(lock),
            profile,
            store,
        };
        if ledger.accounts()?.len() > 8 {
            return Err(IdentityError::RecoveryRequired);
        }
        Ok(ledger)
    }
    pub fn account_key(&self, account: AccountId) -> Result<NavigationAccountKey> {
        NavigationAccountKey::new(&format!(
            "a1:{}:{}",
            id(self.profile.as_local()),
            id(account.as_local())
        ))
        .map_err(|_| IdentityError::RecoveryRequired)
    }
    pub fn account_label(&self, account: AccountId) -> Result<Option<Text<128>>> {
        use rusqlite::OptionalExtension;
        let label: Option<Option<String>> = db(self
            .db
            .query_row(
                "SELECT display_name FROM account_labels WHERE account=?1",
                [id(account.as_local())],
                |row| row.get(0),
            )
            .optional())?;
        label
            .flatten()
            .map(|value| {
                if value.is_empty() || value.chars().any(unsafe_label_character) {
                    return Err(IdentityError::RecoveryRequired);
                }
                Text::new(&value).map_err(|_| IdentityError::RecoveryRequired)
            })
            .transpose()
    }
    pub fn set_account_label(&mut self, account: AccountId, display: &Text<256>) -> Result<()> {
        self.account(account)?;
        let value = display.as_str().trim();
        let label = if value.is_empty() || value.chars().any(unsafe_label_character) {
            None
        } else {
            // Preserve complete UTF-8 characters; a display label is not an identifier.
            let end = value
                .char_indices()
                .map(|(i, _)| i)
                .chain(std::iter::once(value.len()))
                .take_while(|i| *i <= 128)
                .last()
                .unwrap_or(0);
            Some(&value[..end])
        };
        self.dir.check()?;
        db(self.db.execute(
            "INSERT INTO account_labels(account,display_name) VALUES(?1,?2) ON CONFLICT(account) DO UPDATE SET display_name=excluded.display_name",
            params![id(account.as_local()), label],
        ))?;
        Ok(())
    }
    /// The server's numeric room ID is private and authoritative. Missing IDs have
    /// no stable key; random per-client ConversationId.local is never persisted.
    pub fn room_key(
        &mut self,
        account: AccountId,
        server_id: u64,
    ) -> Result<Option<NavigationRoomKey>> {
        use rusqlite::OptionalExtension;
        if server_id == 0 {
            return Ok(None);
        }
        self.account(account)?;
        let key: Option<String> = db(self
            .db
            .query_row(
                "SELECT local_key FROM navigation_rooms WHERE account=?1 AND server_id=?2",
                params![id(account.as_local()), revision(server_id)],
                |row| row.get(0),
            )
            .optional())?;
        if let Some(key) = key {
            return NavigationRoomKey::new(&format!("r1:{key}"))
                .map(Some)
                .map_err(|_| IdentityError::RecoveryRequired);
        }
        let count: i64 = db(self.db.query_row(
            "SELECT count(*) FROM navigation_rooms WHERE account=?1",
            [id(account.as_local())],
            |row| row.get(0),
        ))?;
        if count >= 256 {
            // No eviction/reassignment can silently apply an old layout to a new room.
            return Ok(None);
        }
        let key = id(random()?);
        self.dir.check()?;
        db(self.db.execute(
            "INSERT INTO navigation_rooms(account,server_id,local_key) VALUES(?1,?2,?3)",
            params![id(account.as_local()), revision(server_id), key],
        ))?;
        NavigationRoomKey::new(&format!("r1:{key}"))
            .map(Some)
            .map_err(|_| IdentityError::RecoveryRequired)
    }
    pub fn accounts(&self) -> Result<Vec<Account>> {
        let mut query = db(self.db.prepare(
            "SELECT id,host,origin,user,revision,disposition,mode,eligible FROM accounts LIMIT 9",
        ))?;
        let mut rows = db(query.query([]))?;
        let mut result = Vec::new();
        while let Some(row) = db(rows.next())? {
            let raw: Option<String> = db(row.get(7))?;
            let a = Account {
                id: AccountId::from_local(number(db(row.get(0))?)?),
                host: HostId::from_local(number(db(row.get(1))?)?),
                origin: db(row.get(2))?,
                user: db(row.get(3))?,
                revision: parse_revision(db(row.get(4))?)?,
                disposition: db(row.get(5))?,
                mode: db(row.get(6))?,
                eligible: raw.map(number).transpose()?.map(OperationId::from_local),
            };
            if a.origin.len() > 2048
                || a.user.len() > 256
                || a.disposition > 2
                || a.mode > 2
                || (a.disposition == 2) != a.eligible.is_some()
            {
                return Err(IdentityError::RecoveryRequired);
            }
            result.push(a);
        }
        Ok(result)
    }
    pub fn account(&self, account: AccountId) -> Result<Account> {
        self.accounts()?
            .into_iter()
            .find(|a| a.id == account)
            .ok_or(IdentityError::RecoveryRequired)
    }
    pub fn reserve(&mut self, origin: String) -> Result<Account> {
        if self.accounts()?.len() >= 8 {
            return Err(talk_core::ports::ErrorKind::Busy.into());
        }
        let a = Account {
            id: AccountId::from_local(random()?),
            host: HostId::from_local(random()?),
            origin,
            user: String::new(),
            revision: 0,
            disposition: 0,
            mode: 0,
            eligible: None,
        };
        self.dir.check()?;
        db(self.db.execute(
            "INSERT INTO accounts VALUES(?1,?2,?3,'','0000000000000000',0,0,NULL)",
            params![id(a.id.as_local()), id(a.host.as_local()), a.origin],
        ))?;
        Ok(a)
    }
    pub fn bind_user(&mut self, account: AccountId, user: &str) -> Result<()> {
        let a = self.account(account)?;
        if user.is_empty()
            || user.len() > 256
            || user.chars().any(char::is_control)
            || (!a.user.is_empty() && a.user != user)
        {
            return Err(talk_core::ports::ErrorKind::WrongScope.into());
        }
        self.dir.check()?;
        let n = db(self.db.execute(
            "UPDATE accounts SET user=?1 WHERE id=?2 AND revision=?3",
            params![user, id(account.as_local()), revision(a.revision)],
        ))?;
        if n != 1 {
            return Err(IdentityError::RecoveryRequired);
        }
        Ok(())
    }
    pub fn operations(&self, account: AccountId) -> Result<Vec<Operation>> {
        let mut query=db(self.db.prepare("SELECT id,attempt,backend,generation,retained,settlement,abandoned,cleanup,activated,retired,last_failure FROM operations WHERE account=?1 ORDER BY rowid LIMIT 17"))?;
        let mut rows = db(query.query([id(account.as_local())]))?;
        let mut list = Vec::new();
        while let Some(row) = db(rows.next())? {
            let retained: Option<String> = db(row.get(4))?;
            list.push(Operation {
                id: OperationId::from_local(number(db(row.get(0))?)?),
                attempt: LoginAttemptId::from_local(number(db(row.get(1))?)?),
                item: QualifiedItem {
                    provider: ProviderBinding {
                        backend: backend(db(row.get(2))?)?,
                        profile: self.profile,
                        store: self.store,
                    },
                    account,
                    generation: VaultGeneration::from_local(number(db(row.get(3))?)?),
                },
                retained: retained
                    .map(number)
                    .transpose()?
                    .map(OperationId::from_local),
                settlement: db(row.get(5))?,
                abandoned: db(row.get(6))?,
                cleanup: db(row.get(7))?,
                activated: db(row.get(8))?,
                retired: db(row.get(9))?,
                last_failure: db(row.get::<_, Option<i64>>(10))?
                    .map(failure)
                    .transpose()?,
            });
        }
        if list.len() > 16 || list.iter().any(|o| o.settlement > 2) {
            return Err(IdentityError::RecoveryRequired);
        }
        Ok(list)
    }
    pub fn operation(&self, account: AccountId, op: OperationId) -> Result<Operation> {
        self.operations(account)?
            .into_iter()
            .find(|o| o.id == op)
            .ok_or(IdentityError::RecoveryRequired)
    }
    pub fn login_name(&self, op: Operation) -> Result<zeroize::Zeroizing<String>> {
        let stored = self.operation(op.item.account, op.id)?;
        if stored.item != op.item || stored.attempt != op.attempt {
            return Err(IdentityError::RecoveryRequired);
        }
        let value: String = db(self.db.query_row(
            "SELECT login_name FROM operations WHERE id=?1 AND account=?2",
            params![id(op.id.as_local()), id(op.item.account.as_local())],
            |r| r.get(0),
        ))?;
        if value.is_empty()
            || value.len() > 256
            || value.contains(':')
            || value.chars().any(char::is_control)
        {
            return Err(IdentityError::RecoveryRequired);
        }
        Ok(zeroize::Zeroizing::new(value))
    }
    /// The conservative dispatched marker and unresolved eligibility commit BEFORE I/O.
    pub fn stage(
        &mut self,
        account: AccountId,
        attempt: LoginAttemptId,
        backend: StorageBackend,
        login_name: &str,
    ) -> Result<Operation> {
        if login_name.is_empty()
            || login_name.len() > 256
            || login_name.contains(':')
            || login_name.chars().any(char::is_control)
        {
            return Err(talk_core::ports::ErrorKind::Malformed.into());
        }
        let a = self.account(account)?;
        self.collect(account)?;
        let old = self.operations(account)?;
        let records: usize = old
            .iter()
            .filter(|o| !o.cleanup)
            .map(|o| if o.settlement == 0 { 2 } else { 1 })
            .sum();
        if old.iter().any(|o| o.settlement == 0) || records + 2 > 16 || old.len() >= 16 {
            return Err(talk_core::ports::ErrorKind::Busy.into());
        }
        let retained = a.eligible.or_else(|| {
            if a.disposition == 1 {
                old.last().and_then(|o| o.retained)
            } else {
                None
            }
        });
        let op = Operation {
            id: OperationId::from_local(random()?),
            attempt,
            item: QualifiedItem {
                provider: ProviderBinding {
                    backend,
                    profile: self.profile,
                    store: self.store,
                },
                account,
                generation: VaultGeneration::from_local(random()?),
            },
            retained,
            settlement: 0,
            abandoned: false,
            cleanup: false,
            activated: false,
            retired: false,
            last_failure: None,
        };
        self.dir.check()?;
        let tx = db(self.db.transaction())?;
        // Starting another explicit save abandons only earlier unactivated, settled N.
        // Ever-activated P remains untouched until named retirement/removal authority.
        db(tx.execute("UPDATE operations SET abandoned=1 WHERE account=?1 AND activated=0 AND settlement<>0 AND cleanup=0",[id(account.as_local())]))?;
        db(tx.execute(
            "INSERT INTO operations VALUES(?1,?2,?3,?4,?5,?6,0,0,0,0,?7,0,NULL)",
            params![
                id(op.id.as_local()),
                id(account.as_local()),
                id(attempt.as_local()),
                backend_id(backend),
                id(op.item.generation.as_local()),
                retained.map(|o| id(o.as_local())),
                login_name
            ],
        ))?;
        let n=db(tx.execute("UPDATE accounts SET disposition=1,eligible=NULL,revision=?3 WHERE id=?1 AND revision=?2",params![id(account.as_local()),revision(a.revision),next_revision(a.revision)?]))?;
        if n != 1 {
            return Err(IdentityError::RecoveryRequired);
        }
        db(tx.commit())?;
        Ok(op)
    }
    #[cfg(test)]
    pub fn settle(&mut self, op: Operation, present: bool) -> Result<()> {
        self.settle_result(op, present, None)
    }
    pub fn settle_result(
        &mut self,
        op: Operation,
        present: bool,
        error: Option<IdentityError>,
    ) -> Result<()> {
        let current = self.operation(op.item.account, op.id)?;
        if current.item != op.item || current.attempt != op.attempt {
            return Err(IdentityError::RecoveryRequired);
        }
        self.dir.check()?;
        let n = db(self.db.execute(
            "UPDATE operations SET settlement=?1,last_failure=?4 WHERE id=?2 AND account=?3 AND settlement=0",
            params![
                if present { 1 } else { 2 },
                id(op.id.as_local()),
                id(op.item.account.as_local()),error.map(category).map(failure_id)
            ],
        ))?;
        if n != 1 {
            return Err(IdentityError::RecoveryRequired);
        }
        Ok(())
    }
    /// Safe typed failure metadata never changes settlement or eligibility.
    pub fn record_failure(&mut self, op: Operation, error: IdentityError) -> Result<()> {
        let current = self.operation(op.item.account, op.id)?;
        if current.item != op.item || current.attempt != op.attempt {
            return Err(IdentityError::RecoveryRequired);
        }
        self.dir.check()?;
        db(self.db.execute(
            "UPDATE operations SET last_failure=?1 WHERE id=?2 AND account=?3",
            params![
                failure_id(category(error)),
                id(op.id.as_local()),
                id(op.item.account.as_local())
            ],
        ))?;
        Ok(())
    }
    /// Caller separately proves live attempt, canonical validation and fresh item read.
    pub fn activate(&mut self, op: Operation, expected: u64) -> Result<()> {
        let current = self.operation(op.item.account, op.id)?;
        if current.abandoned
            || current.settlement != 1
            || current.item != op.item
            || current.attempt != op.attempt
        {
            return Err(IdentityError::RecoveryRequired);
        }
        self.dir.check()?;
        let tx = db(self.db.transaction())?;
        let n=db(tx.execute("UPDATE accounts SET disposition=2,mode=?1,eligible=?2,revision=?5 WHERE id=?3 AND revision=?4 AND disposition=1",params![if op.item.provider.backend==StorageBackend::PrivateFile{2}else{1},id(op.id.as_local()),id(op.item.account.as_local()),revision(expected),next_revision(expected)?]))?;
        if n != 1 {
            return Err(IdentityError::RecoveryRequired);
        }
        db(tx.execute(
            "UPDATE operations SET activated=1 WHERE id=?1",
            [id(op.id.as_local())],
        ))?;
        db(tx.commit())
    }
    pub fn suppress(
        &mut self,
        account: AccountId,
        session: bool,
        attempted: Option<OperationId>,
        cleanup_all: bool,
    ) -> Result<()> {
        self.dir.check()?;
        let a = self.account(account)?;
        let tx = db(self.db.transaction())?;
        let n=db(tx.execute("UPDATE accounts SET disposition=0,eligible=NULL,mode=?1,revision=?4 WHERE id=?2 AND revision=?3",params![if session{0}else{a.mode},id(account.as_local()),revision(a.revision),next_revision(a.revision)?]))?;
        if n != 1 {
            return Err(IdentityError::SuppressionUnconfirmed);
        }
        if cleanup_all {
            db(tx.execute(
                "UPDATE operations SET abandoned=1 WHERE account=?1 AND cleanup=0 AND activated=0",
                [id(account.as_local())],
            ))?;
        } else if let Some(op) = attempted {
            db(tx.execute("UPDATE operations SET abandoned=1 WHERE account=?1 AND id=?2 AND cleanup=0 AND activated=0",params![id(account.as_local()),id(op.as_local())]))?;
        }
        db(tx.commit()).map_err(|_| IdentityError::SuppressionUnconfirmed)
    }
    /// Cancels N and deliberately retains the exact validated P; never issued by timeout.
    pub fn retain(
        &mut self,
        attempted: Operation,
        previous: Operation,
        expected_revision: u64,
    ) -> Result<()> {
        let now = self.operation(attempted.item.account, attempted.id)?;
        let stored = self.operation(previous.item.account, previous.id)?;
        if now.item != attempted.item
            || now.attempt != attempted.attempt
            || stored.item != previous.item
            || stored.attempt != previous.attempt
            || now.retained != Some(previous.id)
            || previous.item.account != attempted.item.account
            || previous.item == attempted.item
            || stored.settlement != 1
            || stored.cleanup
            || stored.abandoned
        {
            return Err(IdentityError::RecoveryRequired);
        }
        self.dir.check()?;
        let a = self.account(attempted.item.account)?;
        if a.revision != expected_revision || a.disposition != 1 || now.activated || now.abandoned {
            return Err(IdentityError::RecoveryRequired);
        }
        let tx = db(self.db.transaction())?;
        db(tx.execute(
            "UPDATE operations SET abandoned=1 WHERE id=?1",
            [id(attempted.id.as_local())],
        ))?;
        let n=db(tx.execute("UPDATE accounts SET disposition=2,mode=?1,eligible=?2,revision=?5 WHERE id=?3 AND revision=?4",params![if previous.item.provider.backend==StorageBackend::PrivateFile{2}else{1},id(previous.id.as_local()),id(a.id.as_local()),revision(a.revision),next_revision(a.revision)?]))?;
        if n != 1 {
            return Err(IdentityError::RecoveryRequired);
        }
        db(tx.commit())
    }
    /// Explicit removal authority, separate from abandoning a save attempt.
    pub fn retire_all(&mut self, account: AccountId) -> Result<()> {
        let a = self.account(account)?;
        self.dir.check()?;
        let tx = db(self.db.transaction())?;
        let n=db(tx.execute("UPDATE accounts SET disposition=0,eligible=NULL,revision=?3 WHERE id=?1 AND revision=?2",params![id(account.as_local()),revision(a.revision),next_revision(a.revision)?]))?;
        if n != 1 {
            return Err(IdentityError::SuppressionUnconfirmed);
        }
        db(tx.execute(
            "UPDATE operations SET abandoned=1,retired=1 WHERE account=?1 AND cleanup=0",
            [id(account.as_local())],
        ))?;
        db(tx.commit())
    }
    /// Retire one explicitly named residual source, never eligible or retained P.
    pub fn retire(&mut self, item: QualifiedItem) -> Result<()> {
        let a = self.account(item.account)?;
        let operations = self.operations(item.account)?;
        let op = operations
            .iter()
            .find(|o| o.item == item)
            .ok_or(IdentityError::RecoveryRequired)?;
        if a.eligible == Some(op.id)
            || op.settlement == 0
            || a.disposition == 1 && operations.last().is_some_and(|n| n.retained == Some(op.id))
        {
            return Err(talk_core::ports::ErrorKind::Conflict.into());
        }
        self.dir.check()?;
        db(self.db.execute(
            "UPDATE operations SET abandoned=1,retired=1 WHERE id=?1 AND account=?2",
            params![id(op.id.as_local()), id(item.account.as_local())],
        ))?;
        Ok(())
    }
    /// Completed cleanup records may be collected only after no unresolved record
    /// needs their exact correlation. No provider operation or absence inference occurs.
    fn collect(&mut self, account: AccountId) -> Result<()> {
        self.dir.check()?;
        db(self.db.execute("DELETE FROM operations WHERE account=?1 AND cleanup=1 AND settlement=2
            AND id NOT IN (SELECT eligible FROM accounts WHERE eligible IS NOT NULL)
            AND id NOT IN (SELECT retained FROM operations WHERE cleanup=0 AND retained IS NOT NULL)
            AND id NOT IN (SELECT id FROM operations WHERE account=?1 AND (SELECT disposition FROM accounts WHERE id=?1)=1 ORDER BY rowid DESC LIMIT 1)",[id(account.as_local())]))?;
        Ok(())
    }
    /// Account metadata removal follows confirmed suppression and exact cleanup only.
    pub fn forget(&mut self, account: AccountId) -> Result<()> {
        let a = self.account(account)?;
        if a.disposition != 0
            || a.eligible.is_some()
            || self
                .operations(account)?
                .iter()
                .any(|o| !o.cleanup || o.settlement != 2)
        {
            return Err(IdentityError::RecoveryRequired);
        }
        self.dir.check()?;
        let tx = db(self.db.transaction())?;
        db(tx.execute(
            "DELETE FROM operations WHERE account=?1",
            [id(account.as_local())],
        ))?;
        let n = db(tx.execute(
            "DELETE FROM accounts WHERE id=?1 AND revision=?2 AND disposition=0",
            params![id(account.as_local()), revision(a.revision)],
        ))?;
        if n != 1 {
            return Err(IdentityError::RecoveryRequired);
        }
        db(tx.commit())
    }
    pub fn cleaned(&mut self, op: Operation) -> Result<()> {
        let current = self.operation(op.item.account, op.id)?;
        if current.item != op.item
            || current.attempt != op.attempt
            || (current.activated && !current.retired)
            || !current.abandoned
            || current.settlement == 0
            || self.account(op.item.account)?.eligible == Some(op.id)
        {
            return Err(IdentityError::RecoveryRequired);
        }
        self.dir.check()?;
        db(self.db.execute(
            "UPDATE operations SET cleanup=1,settlement=2 WHERE id=?1",
            [id(op.id.as_local())],
        ))?;
        Ok(())
    }
}
