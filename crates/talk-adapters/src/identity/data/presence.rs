//! Brief display-only status snapshots for ordinary direct conversations.
use super::codec;
use serde::{Deserialize, Serialize};
use talk_core::bounded::Text;

/// Status is too volatile to restore after a long offline interval.
pub const CACHE_PRESENCE_LIFETIME: u64 = 5 * 60;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CachedPresence {
    Available,
    Away,
    DoNotDisturb,
    Offline,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheUserStatus {
    pub presence: CachedPresence,
    #[serde(default, with = "codec::optional_text")]
    pub icon: Option<Text<64>>,
    #[serde(default, with = "codec::optional_text")]
    pub message: Option<Text<320>>,
    pub observed_at: u64,
}
impl CacheUserStatus {
    pub fn fresh(&self, now: u64) -> bool {
        self.observed_at <= now && now - self.observed_at < CACHE_PRESENCE_LIFETIME
    }
}
