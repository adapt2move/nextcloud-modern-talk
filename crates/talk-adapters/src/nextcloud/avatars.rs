//! Canonical account-local avatar identities, independent of message/row IDs.
//! Encoded images only: 128 entries / 8 MiB in memory, 24-hour freshness. Room
//! policy is checked by content before lookup; a cache hit grants no authority.
use sha2::{Digest, Sha256};

pub(crate) const TTL: u64 = 24 * 60 * 60;
const ENTRIES: usize = 128;
const BYTES: usize = 8 * 1024 * 1024;

/// The identity owner installs this only for a durably eligible account. Errors
/// disable disk caching for this client, retaining the ordinary network path.
pub(crate) trait AvatarStore: Send + Sync {
    fn get(&mut self, key: &str, now: u64) -> Result<Option<(Vec<u8>, u64)>, ()>;
    fn put(&mut self, key: &str, bytes: &[u8], now: u64) -> Result<(), ()>;
    fn remove(&mut self, key: &str) -> Result<(), ()>;
    fn retain_rooms(&mut self, allowed: &[String]) -> Result<(), ()>;
}

#[derive(Default)]
pub(super) struct Avatars {
    entries: Vec<(String, Vec<u8>, u64)>,
    pub(super) store: Option<Box<dyn AvatarStore>>,
}
pub(super) fn key(kind: &str, identity: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(kind.as_bytes());
    hash.update([0]);
    hash.update(identity.as_bytes());
    let digest: String = hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{}-{digest}", if kind == "room" { "r" } else { "u" })
}
impl Avatars {
    /// A complete fresh room list is the only source with removal authority.
    /// Hash namespaces let disk cleanup work across process restarts without
    /// persisting room tokens or an additional identity map.
    pub(super) fn retain_rooms(&mut self, allowed: &[String]) {
        self.entries
            .retain(|(key, _, _)| !key.starts_with("r-") || allowed.contains(key));
        if let Some(store) = self.store.as_mut()
            && store.retain_rooms(allowed).is_err()
        {
            self.disable_disk();
        }
    }
    pub(super) fn get(&mut self, key: &str, now: u64, durable: bool) -> Option<Vec<u8>> {
        // A newly observed restriction also retires an older unrestricted disk
        // entry. It must not survive merely because this lookup uses memory.
        if !durable
            && let Some(store) = self.store.as_mut()
            && store.remove(key).is_err()
        {
            self.disable_disk();
        }
        self.entries.retain(|(_, _, at)| fresh(*at, now));
        if let Some(index) = self.entries.iter().position(|(k, _, _)| k == key) {
            let entry = self.entries.remove(index);
            let bytes = entry.1.clone();
            self.entries.push(entry);
            return Some(bytes);
        }
        if durable && let Some(store) = self.store.as_mut() {
            match store.get(key, now) {
                Ok(Some((bytes, at))) if bytes.len() <= BYTES && fresh(at, now) => {
                    self.retain(key, &bytes, at);
                    return Some(bytes);
                }
                Ok(_) => (),
                Err(()) => self.disable_disk(),
            }
        }
        None
    }
    pub(super) fn put(&mut self, key: &str, bytes: &[u8], now: u64, durable: bool) {
        self.retain(key, bytes, now);
        if durable
            && let Some(store) = self.store.as_mut()
            && store.put(key, bytes, now).is_err()
        {
            self.disable_disk();
        }
    }
    fn disable_disk(&mut self) {
        self.store = None;
        // Fixed, content-free category, once per client. Never print a DB error,
        // account, avatar key, filesystem path or server-supplied value.
        eprintln!("Avatar cache unavailable; using session memory.");
    }
    fn retain(&mut self, key: &str, bytes: &[u8], now: u64) {
        if bytes.len() > BYTES {
            return;
        }
        self.entries.retain(|(k, _, _)| k != key);
        while !self.entries.is_empty()
            && (self.entries.len() >= ENTRIES
                || self.entries.iter().map(|(_, b, _)| b.len()).sum::<usize>() + bytes.len()
                    > BYTES)
        {
            self.entries.remove(0);
        }
        self.entries.push((key.to_owned(), bytes.to_vec(), now));
    }
}
pub(crate) fn fresh(at: u64, now: u64) -> bool {
    at <= now && now - at < TTL
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_identity_lru_expiry_and_clock_rollback() {
        let mut cache = Avatars::default();
        let person = key("user", "synthetic-user");
        assert_ne!(person, key("room", "synthetic-user"));
        cache.put(&person, b"avatar", 10, false);
        assert_eq!(cache.get(&person, 11, false).unwrap(), b"avatar");
        for n in 0..ENTRIES {
            cache.put(&key("user", &n.to_string()), b"x", 11, false);
        }
        assert!(cache.get(&person, 12, false).is_none());
        assert_eq!(cache.entries.len(), ENTRIES);
        assert!(cache.get(&key("user", "127"), 11 + TTL, false).is_none());
        cache.put(&person, b"avatar", 100, false);
        assert!(cache.get(&person, 99, false).is_none());
    }
    #[test]
    fn restricted_images_never_write_disk_and_store_failure_is_not_retried_forever() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        struct Failed(Arc<AtomicUsize>);
        impl AvatarStore for Failed {
            fn get(&mut self, _: &str, _: u64) -> Result<Option<(Vec<u8>, u64)>, ()> {
                self.0.fetch_add(1, Ordering::Relaxed);
                Err(())
            }
            fn put(&mut self, _: &str, _: &[u8], _: u64) -> Result<(), ()> {
                panic!("restricted avatar persisted")
            }
            fn remove(&mut self, _: &str) -> Result<(), ()> {
                Ok(())
            }
            fn retain_rooms(&mut self, _: &[String]) -> Result<(), ()> {
                Ok(())
            }
        }
        let reads = Arc::new(AtomicUsize::new(0));
        let mut cache = Avatars {
            store: Some(Box::new(Failed(reads.clone()))),
            ..Default::default()
        };
        let key = key("room", "restricted-room");
        assert!(cache.get(&key, 100, false).is_none());
        cache.put(&key, b"memory-only", 100, false);
        assert_eq!(cache.get(&key, 101, false).unwrap(), b"memory-only");
        assert_eq!(reads.load(Ordering::Relaxed), 0);
        assert!(cache.get("missing", 101, true).is_none());
        assert!(cache.get("missing", 102, true).is_none());
        assert_eq!(reads.load(Ordering::Relaxed), 1);
        assert!(cache.store.is_none());
    }
}
