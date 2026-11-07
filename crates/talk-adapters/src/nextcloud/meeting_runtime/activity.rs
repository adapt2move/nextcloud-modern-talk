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

/// Re-announce the last deliberate state after membership/policy refreshes.
/// The lower state matters too: a peer may have missed the earlier signal.
pub(super) fn refresh_hand(
    raised: bool,
    timestamp: u64,
) -> Option<nextcloud_talk_api::signaling::activity::Activity> {
    (timestamp != 0).then_some(
        nextcloud_talk_api::signaling::activity::Activity::RaisedHand {
            state: raised,
            timestamp,
        },
    )
}

/// Existing web clients announce every received raised-hand message. Send an
/// already raised hand only to newly observed peers, while repeating a lowered
/// state to all peers so a missed lower can converge without repeated notices.
#[derive(Default)]
pub(super) struct HandAnnouncements(Vec<nextcloud_talk_api::signaling::PeerSession>);

impl HandAnnouncements {
    pub(super) fn changed(&mut self, peers: &[nextcloud_talk_api::signaling::PeerSession]) {
        self.0 = peers.to_vec();
    }

    pub(super) fn refresh(
        &mut self,
        peers: &[nextcloud_talk_api::signaling::PeerSession],
        raised: bool,
        timestamp: u64,
    ) -> Option<(
        nextcloud_talk_api::signaling::activity::Activity,
        Vec<nextcloud_talk_api::signaling::PeerSession>,
    )> {
        let activity = refresh_hand(raised, timestamp)?;
        self.0.retain(|peer| peers.contains(peer));
        let targets = peers
            .iter()
            .filter(|peer| !raised || !self.0.contains(peer))
            .cloned()
            .collect::<Vec<_>>();
        self.0 = peers.to_vec();
        (!targets.is_empty()).then_some((activity, targets))
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

    #[test]
    fn refresh_reannounces_the_latest_lowered_state_without_inventing_one() {
        use nextcloud_talk_api::signaling::activity::Activity;
        assert_eq!(refresh_hand(false, 0), None);
        assert_eq!(
            refresh_hand(true, 10),
            Some(Activity::RaisedHand {
                state: true,
                timestamp: 10
            })
        );
        assert_eq!(
            refresh_hand(false, 11),
            Some(Activity::RaisedHand {
                state: false,
                timestamp: 11
            })
        );
        let mut remote = RaisedHand::default();
        remote.update(true, 10);
        if let Some(Activity::RaisedHand { state, timestamp }) = refresh_hand(false, 11) {
            remote.update(state, timestamp);
        }
        assert!(!remote.raised());
    }

    #[test]
    fn raised_refresh_targets_new_peers_but_lowered_refresh_repairs_all() {
        use nextcloud_talk_api::signaling::PeerSession;
        let first = PeerSession::parse("first").unwrap();
        let second = PeerSession::parse("second").unwrap();
        let mut announcements = HandAnnouncements::default();
        announcements.changed(std::slice::from_ref(&first));
        assert!(
            announcements
                .refresh(std::slice::from_ref(&first), true, 10)
                .is_none()
        );
        let peers = [first.clone(), second.clone()];
        assert_eq!(
            announcements.refresh(&peers, true, 10).unwrap().1,
            vec![second]
        );
        assert_eq!(announcements.refresh(&peers, false, 11).unwrap().1, peers);
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
