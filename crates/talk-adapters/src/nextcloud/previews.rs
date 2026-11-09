//! Encoded attachment previews for the detail view, kept between opens and restarts.
//! The store is installed only for a durably eligible account and is consulted only
//! after the message and room were read fresh: a hit grants no authority. Classified
//! and sensitive rooms never reach it (`durable` is decided by the caller).
use sha2::{Digest, Sha256};

/// Cache life of one preview (the bytes of an immutable chat attachment).
pub(crate) const TTL: u64 = 30 * 24 * 60 * 60;

/// Encoded image bytes plus the page count of a rendered PDF's first page.
pub(crate) type Preview = (Vec<u8>, Option<u32>);

pub(crate) trait PreviewStore: Send + Sync {
    fn get(&mut self, key: &str, now: u64) -> Result<Option<Preview>, ()>;
    fn put(&mut self, key: &str, preview: &Preview, now: u64) -> Result<(), ()>;
}

/// File id, kind of preview, size and type identify the bytes. A different server
/// file or a replaced attachment changes the key.
pub(crate) fn key(file: u64, large: bool, size: Option<u64>, mime: Option<&str>) -> String {
    let mut hash = Sha256::new();
    hash.update(b"preview-v1\0");
    hash.update(file.to_le_bytes());
    hash.update([u8::from(large)]);
    hash.update(size.unwrap_or(0).to_le_bytes());
    hash.update(mime.unwrap_or("").as_bytes());
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn fresh(at: u64, now: u64) -> bool {
    at <= now && now - at < TTL
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keys_separate_files_sizes_kinds_and_types() {
        let base = key(7, true, Some(10), Some("image/png"));
        assert_eq!(base.len(), 64);
        assert_eq!(base, key(7, true, Some(10), Some("image/png")));
        for other in [
            key(8, true, Some(10), Some("image/png")),
            key(7, false, Some(10), Some("image/png")),
            key(7, true, Some(11), Some("image/png")),
            key(7, true, Some(10), Some("image/jpeg")),
        ] {
            assert_ne!(base, other);
        }
    }
    #[test]
    fn freshness_expires_and_rejects_clock_rollback() {
        assert!(fresh(100, 100));
        assert!(fresh(100, 100 + TTL - 1));
        assert!(!fresh(100, 100 + TTL));
        assert!(!fresh(101, 100));
    }
}
