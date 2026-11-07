//! Mention observations are display state, never permission or navigation authority.

/// Preserve a previous observation only while the same unread count and last
/// activity prove that the room has not visibly changed. A missing server value
/// is otherwise unknown, not a negative mention result.
pub(in crate::composition::accounts) fn merge(
    observed: Option<bool>,
    unread: Option<u32>,
    activity: Option<u64>,
    previous: Option<(Option<bool>, u32, Option<u64>)>,
) -> Option<bool> {
    observed.or_else(|| {
        let (mention, previous_unread, previous_activity) = previous?;
        let activity = activity?;
        (unread == Some(previous_unread)
            && previous_unread > 0
            && previous_activity == Some(activity))
        .then_some(mention)
        .flatten()
    })
}

#[cfg(test)]
mod tests {
    use super::merge;

    #[test]
    fn missing_mention_state_only_retains_an_unchanged_unread_observation() {
        let old = Some((Some(true), 3, Some(100)));
        assert_eq!(merge(None, Some(3), Some(100), old), Some(true));
        assert_eq!(merge(Some(false), Some(3), Some(100), old), Some(false));
        assert_eq!(merge(None, Some(0), Some(100), old), None);
        assert_eq!(merge(None, Some(2), Some(100), old), None);
        assert_eq!(merge(None, Some(3), Some(101), old), None);
        assert_eq!(merge(None, None, Some(100), old), None);
        assert_eq!(
            merge(None, Some(3), None, Some((Some(true), 3, None))),
            None
        );
    }
}
