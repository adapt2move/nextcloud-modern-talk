//! Encoded attachment previews for the detail view, kept as plain files between
//! opens and restarts. The store is installed only for a durably eligible account.
//! A cached preview is shown without asking the server again: the account's own
//! machine is trusted with what it already received. Classified and sensitive rooms
//! never reach it, and a fresh room list retires the entries of rooms that became
//! restricted or disappeared.
use sha2::{Digest, Sha256};

/// Encoded image bytes plus the page count of a rendered PDF's first page.
pub(crate) type Preview = (Vec<u8>, Option<u32>);

pub(crate) trait PreviewStore: Send + Sync {
    /// The preview kept under `key`. Opening it counts as use for eviction.
    fn get(&mut self, key: &str, now: u64) -> Result<Option<Preview>, ()>;
    fn put(&mut self, key: &str, preview: &Preview, now: u64) -> Result<(), ()>;
    /// Keeps only entries of the listed room prefixes.
    fn retain_rooms(&mut self, allowed: &[String]) -> Result<(), ()>;
}

/// Room, message, placeholder and kind identify the bytes without a server round
/// trip. A chat message and its attachment never change, so the key stays valid.
pub(crate) fn key(room: &str, sequence: u64, placeholder: &str, large: bool) -> String {
    let mut hash = Sha256::new();
    hash.update(b"preview-v2\0");
    hash.update(room.as_bytes());
    hash.update([0]);
    hash.update(sequence.to_le_bytes());
    hash.update([u8::from(large)]);
    hash.update(placeholder.as_bytes());
    let digest: String = hash.finalize()[..24]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("p-{}-{digest}", super::avatars::room_prefix(room))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keys_separate_rooms_messages_placeholders_and_kinds() {
        let base = key("room", 7, "file", true);
        assert_eq!(base.len(), 2 + 16 + 1 + 48);
        assert_eq!(base, key("room", 7, "file", true));
        assert!(base.starts_with(&format!(
            "p-{}-",
            super::super::avatars::room_prefix("room")
        )));
        for other in [
            key("other", 7, "file", true),
            key("room", 8, "file", true),
            key("room", 7, "file", false),
            key("room", 7, "file-1", true),
        ] {
            assert_ne!(base, other);
        }
    }
}
