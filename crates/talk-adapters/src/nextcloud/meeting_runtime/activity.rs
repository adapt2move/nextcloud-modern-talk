//! Per-peer activity is retired with its call peer and never grants media rights.
#[derive(Default)]
pub(super) struct RaisedHand(Option<(bool, u64)>);

impl RaisedHand {
    pub(super) fn update(&mut self, state: bool, timestamp: u64) {
        if self.0.is_none_or(|(_, previous)| timestamp > previous) {
            self.0 = Some((state, timestamp));
        }
    }

    pub(super) fn raised(&self) -> bool {
        self.0.is_some_and(|(state, _)| state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delayed_messages_cannot_restore_a_lowered_hand() {
        let mut hand = RaisedHand::default();
        hand.update(true, 10);
        hand.update(false, 11);
        hand.update(true, 10);
        assert!(!hand.raised());
        hand.update(true, 12);
        assert!(hand.raised());
    }
}
