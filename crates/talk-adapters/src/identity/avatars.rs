//! Disposable profile images as plain files in the account's picture directory,
//! separate from restoration metadata and protected chat/draft/attachment storage.
//! Private account files, never credential files.
use super::{
    ledger::Ledger,
    media_files::{Budget, Files},
};
use crate::nextcloud::avatars::{AvatarStore, Profile};
use std::sync::{Arc, Mutex};
use talk_core::ids::AccountId;

const IMAGES: Budget = Budget {
    prefix: "",
    max_file: 1024 * 1024,
    max_entries: 128,
    max_bytes: 8 * 1024 * 1024,
};
const USERS: Budget = Budget {
    prefix: "u-",
    ..IMAGES
};
const ROOMS: Budget = Budget {
    prefix: "r-",
    ..IMAGES
};
const AUTHORS: Budget = Budget {
    prefix: "m-",
    max_file: 80,
    max_entries: 4096,
    max_bytes: 4096 * 80,
};
const PROFILE: &str = "profile.json";

pub(super) struct Store {
    files: Files,
}
impl Store {
    pub(super) fn new(ledger: Arc<Mutex<Ledger>>, account: AccountId) -> Self {
        Self {
            files: Files::new(ledger, account, &["u-", "r-", "m-", PROFILE]),
        }
    }
}
/// `u-` or `r-` plus 64 hex digits.
fn image_key(key: &str) -> Result<Budget, ()> {
    let hex = key.get(2..).filter(|h| {
        h.len() == 64
            && h.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    });
    match (key.get(..2), hex) {
        (Some("u-"), Some(_)) => Ok(USERS),
        (Some("r-"), Some(_)) => Ok(ROOMS),
        _ => Err(()),
    }
}
/// `m-`, sixteen hex digits of the room, a dash and 48 hex digits.
fn author_key(key: &str) -> bool {
    let mut parts = key.splitn(3, '-');
    let hex = |s: Option<&str>, n: usize| {
        s.is_some_and(|s| {
            s.len() == n
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
    };
    parts.next() == Some("m") && hex(parts.next(), 16) && hex(parts.next(), 48)
}
impl AvatarStore for Store {
    fn profile(&mut self, _now: u64) -> Result<Option<Profile>, ()> {
        self.files.with(|dir| {
            let Some((bytes, _)) = dir.read(PROFILE, 4096)? else {
                return Ok(None);
            };
            let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                dir.drop_file(PROFILE);
                return Ok(None);
            };
            let text = |field: &str| match value.get(field) {
                Some(serde_json::Value::String(s)) => {
                    talk_core::bounded::Text::new(s).map(Some).map_err(|_| ())
                }
                _ => Ok(None),
            };
            let Some(saved_at) = value.get("saved").and_then(serde_json::Value::as_u64) else {
                dir.drop_file(PROFILE);
                return Ok(None);
            };
            Ok(Some(Profile {
                name: text("name")?,
                organization: text("organization")?,
                saved_at,
            }))
        })
    }
    fn put_profile(&mut self, profile: &Profile) -> Result<(), ()> {
        let value = serde_json::json!({
            "saved": profile.saved_at,
            "name": profile.name.as_ref().map(|s| s.as_str()),
            "organization": profile.organization.as_ref().map(|s| s.as_str()),
        });
        let bytes = serde_json::to_vec(&value).map_err(|_| ())?;
        self.files.with(|dir| {
            dir.write(
                Budget {
                    prefix: PROFILE,
                    max_file: 4096,
                    max_entries: 1,
                    max_bytes: 4096,
                },
                PROFILE,
                &bytes,
                profile.saved_at,
            )
        })
    }
    fn retain_rooms(&mut self, allowed: &[String]) -> Result<(), ()> {
        if allowed.len() > super::data::CACHE_ROOM_LIMIT {
            return Err(());
        }
        // `r-<64 hex>`: its first sixteen digits are the room prefix of `m-` entries.
        let prefixes: Vec<&str> = allowed.iter().filter_map(|k| k.get(2..18)).collect();
        self.files.with(|dir| {
            for name in dir.names("r-") {
                if !allowed.contains(&name) {
                    dir.drop_file(&name);
                }
            }
            for name in dir.names("m-") {
                if !name.get(2..18).is_some_and(|p| prefixes.contains(&p)) {
                    dir.drop_file(&name);
                }
            }
            Ok(())
        })
    }
    fn remove(&mut self, key: &str) -> Result<(), ()> {
        image_key(key)?;
        self.files.with(|dir| {
            dir.drop_file(key);
            Ok(())
        })
    }
    fn get(&mut self, key: &str, _now: u64) -> Result<Option<(Vec<u8>, u64)>, ()> {
        let budget = image_key(key)?;
        self.files.with(|dir| dir.read(key, budget.max_file))
    }
    fn put(&mut self, key: &str, bytes: &[u8], now: u64) -> Result<(), ()> {
        let budget = image_key(key)?;
        self.files.with(|dir| dir.write(budget, key, bytes, now))
    }
    fn actor(&mut self, message: &str) -> Result<Option<String>, ()> {
        if !author_key(message) {
            return Err(());
        }
        self.files.with(|dir| {
            let Some((bytes, _)) = dir.read(message, AUTHORS.max_file)? else {
                return Ok(None);
            };
            Ok(String::from_utf8(bytes)
                .ok()
                .filter(|key| image_key(key).is_ok()))
        })
    }
    fn put_actor(&mut self, message: &str, avatar: &str, now: u64) -> Result<(), ()> {
        if !author_key(message) || image_key(avatar).is_err() {
            return Err(());
        }
        self.files
            .with(|dir| dir.write(AUTHORS, message, avatar.as_bytes(), now))
    }
}
