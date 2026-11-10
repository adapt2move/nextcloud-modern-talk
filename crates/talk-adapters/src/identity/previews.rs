//! Disposable attachment previews as plain files in the account's picture directory,
//! separate from restoration metadata and the chat database. Bounded in entries and
//! bytes, least recently opened first.
use super::{
    ledger::Ledger,
    media_files::{Budget, Files},
};
use crate::nextcloud::previews::{Preview, PreviewStore};
use std::sync::{Arc, Mutex};
use talk_core::ids::AccountId;

const PREVIEWS: Budget = Budget {
    prefix: "p-",
    max_file: 8 * 1024 * 1024,
    max_entries: 512,
    max_bytes: 256 * 1024 * 1024,
};

pub(super) struct Store {
    files: Files,
}
impl Store {
    pub(super) fn new(ledger: Arc<Mutex<Ledger>>, account: AccountId) -> Self {
        Self {
            files: Files::new(ledger, account, &["p-"]),
        }
    }
}
/// `p-`, sixteen hex digits of the room, a dash and 48 hex digits.
fn valid(key: &str) -> bool {
    let mut parts = key.splitn(3, '-');
    let hex = |s: Option<&str>, n: usize| {
        s.is_some_and(|s| {
            s.len() == n
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
    };
    parts.next() == Some("p") && hex(parts.next(), 16) && hex(parts.next(), 48)
}
/// A rendered PDF page keeps its page count in the file name (`<key>.<pages>`), so
/// the file stays the bare image. Images have no suffix.
fn file(key: &str, pages: Option<u32>) -> String {
    pages.map_or_else(|| key.to_owned(), |pages| format!("{key}.{pages}"))
}
impl PreviewStore for Store {
    fn get(&mut self, key: &str, now: u64) -> Result<Option<Preview>, ()> {
        if !valid(key) {
            return Err(());
        }
        self.files.with(|dir| {
            let found = dir
                .names(key)
                .into_iter()
                .find(|n| n == key || n.strip_prefix(key).is_some_and(|s| s.starts_with('.')));
            let Some(name) = found else {
                return Ok(None);
            };
            let pages = name
                .strip_prefix(key)
                .and_then(|s| s.strip_prefix('.'))
                .map(|s| s.parse::<u32>().map_err(|_| ()))
                .transpose();
            let Ok(pages) = pages else {
                dir.drop_file(&name);
                return Ok(None);
            };
            let Some((bytes, _)) = dir.read(&name, PREVIEWS.max_file)? else {
                return Ok(None);
            };
            dir.touch(&name, now);
            Ok(Some((bytes, pages)))
        })
    }
    fn put(&mut self, key: &str, preview: &Preview, now: u64) -> Result<(), ()> {
        if !valid(key) {
            return Err(());
        }
        let (bytes, pages) = preview;
        self.files.with(|dir| {
            for name in dir.names(key) {
                if name == key || name.strip_prefix(key).is_some_and(|s| s.starts_with('.')) {
                    dir.drop_file(&name);
                }
            }
            dir.write(PREVIEWS, &file(key, *pages), bytes, now)
        })
    }
    fn retain_rooms(&mut self, allowed: &[String]) -> Result<(), ()> {
        if allowed.len() > super::data::CACHE_ROOM_LIMIT {
            return Err(());
        }
        self.files.with(|dir| {
            for name in dir.names("p-") {
                if !name
                    .get(2..18)
                    .is_some_and(|p| allowed.iter().any(|a| a == p))
                {
                    dir.drop_file(&name);
                }
            }
            Ok(())
        })
    }
}
