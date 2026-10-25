//! Device-local recent Unicode reaction keys. No message or account content.
use crate::bounded::{LimitExceeded, Page, Text};

pub const RECENT_REACTIONS: usize = 12;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecentReactions(Page<Text<64>, RECENT_REACTIONS>);

impl Default for RecentReactions {
    fn default() -> Self {
        Self(Page::empty())
    }
}

impl RecentReactions {
    pub fn as_slice(&self) -> &[Text<64>] {
        self.0.as_slice()
    }

    pub fn from_ordered(keys: impl IntoIterator<Item = Text<64>>) -> Result<Self, LimitExceeded> {
        let mut result = Page::empty();
        for key in keys {
            if !valid_key(&key) || result.as_slice().contains(&key) {
                return Err(LimitExceeded);
            }
            result.try_push(key)?;
        }
        Ok(Self(result))
    }

    /// The latest explicit add goes first. Removing a reaction does not reorder history.
    pub fn remember(&mut self, key: Text<64>) -> Result<bool, LimitExceeded> {
        if !valid_key(&key) {
            return Err(LimitExceeded);
        }
        if self.0.as_slice().first() == Some(&key) {
            return Ok(false);
        }
        let mut next = Page::empty();
        next.try_push(key.clone())?;
        for previous in self
            .0
            .as_slice()
            .iter()
            .filter(|previous| **previous != key)
            .take(RECENT_REACTIONS - 1)
        {
            next.try_push(previous.clone())?;
        }
        self.0 = next;
        Ok(true)
    }
}

fn valid_key(key: &Text<64>) -> bool {
    !key.as_str().is_empty()
        && !key
            .as_str()
            .chars()
            .any(|c| c.is_control() || c.is_whitespace())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recency_deduplicates_and_evicts_only_the_oldest_at_capacity() {
        let mut recent = RecentReactions::default();
        for emoji in [
            "👍", "❤️", "😂", "🎉", "👏", "🙏", "👀", "✅", "🙌", "🔥", "💯", "😄", "🙂",
        ] {
            assert!(recent.remember(Text::new(emoji).unwrap()).unwrap());
        }
        assert_eq!(recent.as_slice().len(), RECENT_REACTIONS);
        assert!(!recent.as_slice().iter().any(|key| key.as_str() == "👍"));
        assert!(recent.remember(Text::new("😂").unwrap()).unwrap());
        assert_eq!(recent.as_slice()[0].as_str(), "😂");
        assert!(!recent.remember(Text::new("😂").unwrap()).unwrap());
        assert_eq!(recent.as_slice().len(), RECENT_REACTIONS);
    }

    #[test]
    fn persisted_order_rejects_duplicates_overflow_and_invalid_keys() {
        assert!(
            RecentReactions::from_ordered([Text::new("👍").unwrap(), Text::new("👍").unwrap()])
                .is_err()
        );
        assert!(
            RecentReactions::from_ordered((0..13).map(|n| Text::new(&format!("key-{n}")).unwrap()))
                .is_err()
        );
        for key in ["", "two keys", "\n"] {
            assert!(
                RecentReactions::default()
                    .remember(Text::new(key).unwrap())
                    .is_err()
            );
        }
    }
}
