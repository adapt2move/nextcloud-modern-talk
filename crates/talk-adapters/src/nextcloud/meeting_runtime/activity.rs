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

/// Call membership is independent of a media subscription. In particular, an
/// HPB participant with both devices off still has a tile and can raise a hand.
pub(super) struct Participant {
    pub session: nextcloud_talk_api::signaling::PeerSession,
    pub local: u128,
    pub hand: RaisedHand,
}

#[derive(Default)]
pub(super) struct Participants(pub Vec<Participant>);

impl Participants {
    pub fn reconcile(
        &mut self,
        sessions: &[nextcloud_talk_api::signaling::PeerSession],
        reserved: &[u128],
        mut allocate: impl FnMut() -> Result<u128, ()>,
    ) -> Result<(), ()> {
        if sessions.len() > 16
            || sessions
                .iter()
                .enumerate()
                .any(|(i, s)| sessions[..i].contains(s))
        {
            return Err(());
        }
        self.0.retain(|person| sessions.contains(&person.session));
        for session in sessions {
            if self.0.iter().any(|person| person.session == *session) {
                continue;
            }
            let local = allocate()?;
            if local == 0 || reserved.contains(&local) || self.0.iter().any(|p| p.local == local) {
                return Err(());
            }
            self.0.push(Participant {
                session: session.clone(),
                local,
                hand: RaisedHand::default(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod participant_tests {
    use super::*;
    use nextcloud_talk_api::signaling::PeerSession;

    #[test]
    fn silent_members_keep_their_identity_and_hand_without_a_media_peer() {
        let first = PeerSession::parse("first").unwrap();
        let silent = PeerSession::parse("silent").unwrap();
        let mut people = Participants::default();
        let mut next = 0;
        let mut allocate = || {
            next += 1;
            Ok(next)
        };
        people
            .reconcile(&[first.clone(), silent.clone()], &[], &mut allocate)
            .unwrap();
        people.0[1].hand.update(true, 8);
        people
            .reconcile(std::slice::from_ref(&silent), &[], &mut allocate)
            .unwrap();
        assert_eq!(people.0[0].local, 2);
        assert!(people.0[0].hand.raised());
        people
            .reconcile(&[first, silent], &[], &mut allocate)
            .unwrap();
        assert_eq!(people.0[0].local, 2);
        assert_eq!(people.0[1].local, 3);
        assert!(!people.0[1].hand.raised());
    }
}
