//! Disposable pictures kept as plain files, one directory per account inside the
//! private profile. A file is the picture itself (an avatar, a rendered preview),
//! readable by any image viewer; its modification time is the only metadata, and an
//! in-memory index built once per client decides eviction. Nothing here is a
//! credential, and nothing is trusted beyond what the owner's machine already holds.
use super::{ledger::Ledger, private_fs::PrivateDir};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};
use talk_core::ids::AccountId;

/// What one kind of file may occupy. Files outside every prefix are removed.
#[derive(Clone, Copy)]
pub(super) struct Budget {
    pub prefix: &'static str,
    pub max_file: usize,
    pub max_entries: usize,
    pub max_bytes: usize,
}

struct Entry {
    len: u64,
    at: u64,
}

/// Account-scoped directory plus the index of the files of this owner's prefixes.
pub(super) struct Files {
    ledger: Arc<Mutex<Ledger>>,
    account: AccountId,
    prefixes: &'static [&'static str],
    index: Option<BTreeMap<String, Entry>>,
}

fn directory_name(account: AccountId) -> String {
    format!("media-{:032x}", account.as_local())
}
/// Names are built from hex digits and fixed prefixes. Anything else is refused,
/// so a caller can never address a path outside the account directory.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 96
        && name
            .bytes()
            .all(|b| b.is_ascii_digit() || b.is_ascii_lowercase() || b"-._".contains(&b))
        && !name.starts_with('.')
}
fn modified(path: &Path) -> Option<u64> {
    fs::symlink_metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

impl Files {
    pub(super) fn new(
        ledger: Arc<Mutex<Ledger>>,
        account: AccountId,
        prefixes: &'static [&'static str],
    ) -> Self {
        Self {
            ledger,
            account,
            prefixes,
            index: None,
        }
    }
    /// Runs `f` on the account directory while this account is still durably
    /// eligible. A disconnected client or a session-only save never creates files
    /// through a stale store, and removal cannot race a write.
    pub(super) fn with<T>(
        &mut self,
        f: impl FnOnce(&mut Dir<'_>) -> Result<T, ()>,
    ) -> Result<T, ()> {
        let ledger = self.ledger.lock().map_err(|_| ())?;
        let account = ledger.account(self.account).map_err(|_| ())?;
        if account.disposition != 2 || account.eligible.is_none() {
            return Err(());
        }
        let path = open_directory(&ledger.dir, self.account)?;
        if self.index.is_none() {
            self.index = Some(scan(&path, self.prefixes)?);
        }
        let index = self.index.as_mut().ok_or(())?;
        f(&mut Dir { path, index })
    }
}

pub(super) struct Dir<'a> {
    path: PathBuf,
    index: &'a mut BTreeMap<String, Entry>,
}
impl Dir<'_> {
    /// The file's bytes and when it was saved or last used.
    pub(super) fn read(&mut self, name: &str, limit: usize) -> Result<Option<(Vec<u8>, u64)>, ()> {
        if !valid_name(name) {
            return Err(());
        }
        let Some(entry) = self.index.get(name) else {
            return Ok(None);
        };
        let at = entry.at;
        let Some(mut file) = open_read(&self.path.join(name)) else {
            // Gone or replaced behind our back: forget it.
            self.index.remove(name);
            return Ok(None);
        };
        let Ok(meta) = file.metadata() else {
            self.index.remove(name);
            return Ok(None);
        };
        if !meta.is_file() || meta.len() > limit as u64 {
            self.drop_file(name);
            return Ok(None);
        }
        let mut bytes = Vec::with_capacity(meta.len() as usize);
        if file.read_to_end(&mut bytes).is_err() || bytes.len() > limit {
            self.drop_file(name);
            return Ok(None);
        }
        Ok(Some((bytes, at)))
    }
    /// Marks a file as just used without rewriting it.
    pub(super) fn touch(&mut self, name: &str, now: u64) {
        if let Some(entry) = self.index.get_mut(name)
            && let Ok(file) = OpenOptions::new().write(true).open(self.path.join(name))
            && file
                .set_modified(UNIX_EPOCH + Duration::from_secs(now))
                .is_ok()
        {
            entry.at = now;
        }
    }
    /// Replaces `name` atomically and evicts the least recently saved or used files
    /// of the same prefix until the budget holds.
    pub(super) fn write(
        &mut self,
        budget: Budget,
        name: &str,
        bytes: &[u8],
        now: u64,
    ) -> Result<(), ()> {
        if !valid_name(name) || !name.starts_with(budget.prefix) {
            return Err(());
        }
        if bytes.is_empty() || bytes.len() > budget.max_file {
            return Ok(());
        }
        self.drop_file(name);
        let used = |dir: &Self| {
            dir.index
                .iter()
                .filter(|(n, _)| n.starts_with(budget.prefix))
                .fold((0usize, 0u64), |(count, size), (_, e)| {
                    (count + 1, size + e.len)
                })
        };
        loop {
            let (count, size) = used(self);
            if count < budget.max_entries && size + bytes.len() as u64 <= budget.max_bytes as u64 {
                break;
            }
            let oldest = self
                .index
                .iter()
                .filter(|(n, _)| n.starts_with(budget.prefix))
                .min_by(|a, b| a.1.at.cmp(&b.1.at).then_with(|| a.0.cmp(b.0)))
                .map(|(n, _)| n.clone());
            match oldest {
                Some(oldest) => self.drop_file(&oldest),
                None => return Ok(()),
            }
        }
        let stage = self.path.join(format!(".stage-{name}"));
        let result = (|| -> std::io::Result<()> {
            let mut file = create_private(&stage)?;
            file.write_all(bytes)?;
            file.set_modified(UNIX_EPOCH + Duration::from_secs(now))?;
            drop(file);
            fs::rename(&stage, self.path.join(name))
        })();
        if result.is_err() {
            let _ = fs::remove_file(&stage);
            return Err(());
        }
        self.index.insert(
            name.to_owned(),
            Entry {
                len: bytes.len() as u64,
                at: now,
            },
        );
        Ok(())
    }
    pub(super) fn drop_file(&mut self, name: &str) {
        if valid_name(name) {
            let _ = fs::remove_file(self.path.join(name));
        }
        self.index.remove(name);
    }
    /// Names of this owner's files that start with `prefix`.
    pub(super) fn names(&self, prefix: &str) -> Vec<String> {
        self.index
            .keys()
            .filter(|n| n.starts_with(prefix))
            .cloned()
            .collect()
    }
}

#[cfg(unix)]
fn create_private(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(path)
}
#[cfg(not(unix))]
fn create_private(path: &Path) -> std::io::Result<File> {
    // The profile directory is already restricted to its owner; the files inherit it.
    OpenOptions::new().write(true).create_new(true).open(path)
}
#[cfg(unix)]
fn open_read(path: &Path) -> Option<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(path)
        .ok()
}
#[cfg(not(unix))]
fn open_read(path: &Path) -> Option<File> {
    if fs::symlink_metadata(path).ok()?.file_type().is_symlink() {
        return None;
    }
    File::open(path).ok()
}

/// The account's directory inside the validated profile, created private on first use.
fn open_directory(dir: &PrivateDir, account: AccountId) -> Result<PathBuf, ()> {
    dir.check().map_err(|_| ())?;
    let path = dir.path().join(directory_name(account));
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return Err(()),
        Err(_) => {
            create_directory(&path).map_err(|_| ())?;
        }
    }
    Ok(path)
}
#[cfg(unix)]
fn create_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new().mode(0o700).create(path)
}
#[cfg(not(unix))]
fn create_directory(path: &Path) -> std::io::Result<()> {
    fs::create_dir(path)
}

/// Lists the owner's files, removing strays (interrupted writes, foreign names, links).
fn scan(path: &Path, prefixes: &[&str]) -> Result<BTreeMap<String, Entry>, ()> {
    let mut index = BTreeMap::new();
    for item in fs::read_dir(path).map_err(|_| ())? {
        let Ok(item) = item else { continue };
        let name = item.file_name();
        let Some(name) = name.to_str() else { continue };
        let full = path.join(name);
        let Ok(meta) = fs::symlink_metadata(&full) else {
            continue;
        };
        if name.starts_with(".stage-") {
            // An interrupted write of an earlier process; the owner is the only writer.
            let _ = fs::remove_file(&full);
            continue;
        }
        if !prefixes.iter().any(|p| name.starts_with(p)) {
            // Another owner's file (or `profile.json`), left alone.
            continue;
        }
        if !valid_name(name) || !meta.is_file() {
            let _ = fs::remove_file(&full);
            continue;
        }
        index.insert(
            name.to_owned(),
            Entry {
                len: meta.len(),
                at: modified(&full).unwrap_or(0),
            },
        );
    }
    Ok(index)
}

/// Deletes the account's pictures and any database of the former cache layout.
pub(super) fn remove(dir: &PrivateDir, account: AccountId) -> super::error::Result<()> {
    let path = dir.path().join(directory_name(account));
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_dir() => {
            fs::remove_dir_all(&path).map_err(super::error::io)?;
        }
        Ok(_) => fs::remove_file(&path).map_err(super::error::io)?,
        Err(_) => {}
    }
    purge_legacy(dir, account)
}
/// The previous cache layout kept pictures in SQLite files. They are disposable.
pub(super) fn purge_legacy(dir: &PrivateDir, account: AccountId) -> super::error::Result<()> {
    for kind in ["avatars", "previews"] {
        let name = format!("{kind}-{:032x}.sqlite", account.as_local());
        for suffix in ["-journal", "-wal", "-shm", ""] {
            dir.remove(&format!("{name}{suffix}"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMALL: Budget = Budget {
        prefix: "u-",
        max_file: 16,
        max_entries: 3,
        max_bytes: 40,
    };
    fn temporary(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "media-files-{name}-{}-{}",
            std::process::id(),
            UNIX_EPOCH.elapsed().unwrap().as_nanos()
        ));
        create_directory(&path).unwrap();
        path
    }
    fn open(path: &Path) -> (PathBuf, BTreeMap<String, Entry>) {
        (path.to_owned(), scan(path, &["u-", "m-"]).unwrap())
    }

    #[test]
    fn files_are_the_pictures_and_survive_a_rescan_with_their_age() {
        let path = temporary("rescan");
        let (p, mut index) = open(&path);
        let mut dir = Dir {
            path: p,
            index: &mut index,
        };
        dir.write(SMALL, "u-aa", b"picture", 1_000).unwrap();
        assert_eq!(fs::read(path.join("u-aa")).unwrap(), b"picture");
        assert_eq!(
            dir.read("u-aa", 16).unwrap(),
            Some((b"picture".to_vec(), 1_000))
        );
        let (_, rescanned) = open(&path);
        assert_eq!(rescanned["u-aa"].at, 1_000);
        assert_eq!(rescanned["u-aa"].len, 7);
        fs::remove_dir_all(path).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn files_and_directory_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let path = temporary("private");
        let (p, mut index) = open(&path);
        Dir {
            path: p,
            index: &mut index,
        }
        .write(SMALL, "u-aa", b"x", 1)
        .unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path.join("u-aa")), 0o600);
        assert_eq!(mode(&path), 0o700);
        fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn the_least_recently_used_file_leaves_first_and_budgets_hold() {
        let path = temporary("lru");
        let (p, mut index) = open(&path);
        let mut dir = Dir {
            path: p,
            index: &mut index,
        };
        for (n, name) in ["u-a", "u-b", "u-c"].into_iter().enumerate() {
            dir.write(SMALL, name, b"1234567890", 10 + n as u64)
                .unwrap();
        }
        // Three entries would be 30 bytes; the fourth needs the oldest gone.
        dir.touch("u-a", 100);
        dir.write(SMALL, "u-d", b"1234567890", 101).unwrap();
        assert!(
            dir.read("u-b", 16).unwrap().is_none(),
            "u-b was the least recently used"
        );
        assert!(dir.read("u-a", 16).unwrap().is_some());
        assert!(dir.read("u-d", 16).unwrap().is_some());
        assert!(!path.join("u-b").exists());
        // 40 bytes at most: five more bytes force another eviction.
        dir.write(SMALL, "u-e", b"1234567890", 102).unwrap();
        assert!(dir.names("u-").len() <= 3);
        // An oversized or empty file never lands.
        dir.write(SMALL, "u-big", &[0; 17], 103).unwrap();
        dir.write(SMALL, "u-empty", b"", 103).unwrap();
        assert!(!path.join("u-big").exists() && !path.join("u-empty").exists());
        fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn names_cannot_leave_the_directory_or_cross_prefixes() {
        let path = temporary("names");
        let (p, mut index) = open(&path);
        let mut dir = Dir {
            path: p,
            index: &mut index,
        };
        for bad in ["../u-x", "u-/x", "", ".hidden", "u-X", "u-a b"] {
            assert!(dir.write(SMALL, bad, b"x", 1).is_err(), "{bad:?}");
            assert!(dir.read(bad, 16).is_err(), "{bad:?}");
        }
        assert!(dir.write(SMALL, "m-aa", b"x", 1).is_err(), "other prefix");
        fs::remove_dir_all(path).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn a_planted_link_is_neither_followed_nor_kept() {
        let path = temporary("link");
        let secret = temporary("link-secret").join("secret");
        fs::write(&secret, b"secret").unwrap();
        std::os::unix::fs::symlink(&secret, path.join("u-aa")).unwrap();
        fs::write(path.join(".stage-u-old"), b"half").unwrap();
        fs::write(path.join("foreign"), b"left alone").unwrap();
        let (p, mut index) = open(&path);
        assert!(index.is_empty(), "the link is not an entry");
        assert!(
            !path.join("u-aa").exists(),
            "the link is removed, its target untouched"
        );
        assert!(
            !path.join(".stage-u-old").exists(),
            "interrupted writes are cleaned"
        );
        assert!(
            path.join("foreign").exists(),
            "foreign names are left alone"
        );
        assert_eq!(fs::read(&secret).unwrap(), b"secret");
        // A replacement behind the index's back is refused too.
        let mut dir = Dir {
            path: p,
            index: &mut index,
        };
        dir.write(SMALL, "u-bb", b"x", 5).unwrap();
        fs::remove_file(path.join("u-bb")).unwrap();
        std::os::unix::fs::symlink(&secret, path.join("u-bb")).unwrap();
        assert!(dir.read("u-bb", 16).unwrap().is_none());
        fs::remove_dir_all(path).unwrap();
        fs::remove_dir_all(secret.parent().unwrap()).unwrap();
    }
}
