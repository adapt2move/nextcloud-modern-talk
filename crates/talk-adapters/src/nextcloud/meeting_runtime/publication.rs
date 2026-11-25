//! The REST flags describe the shared audio/camera publisher. Screen shares
//! have their own publication protocol.
use super::{MediaWishes, ParticipantFlags, PeerSession, StreamKind, Write, mpsc, send};
use crate::nextcloud::call_policy::Peer;

/// A web peer that saw a receive-only session may never request its offer.
/// Remember each affected peer until its HPB route is ready. Later joins
/// see the published flags and follow the ordinary receiver-initiated path.
pub(super) struct Publication {
    advertised: ParticipantFlags,
    awaiting_publisher: bool,
    pending: Vec<PeerSession>,
}

impl Publication {
    pub(super) fn new(join_flags: ParticipantFlags) -> Self {
        Self {
            advertised: join_flags,
            awaiting_publisher: false,
            pending: Vec::new(),
        }
    }

    pub(super) fn update(
        &mut self,
        wishes: MediaWishes,
        writes: &mpsc::Sender<Write>,
    ) -> Result<(), ()> {
        let flags = ParticipantFlags::new(wishes.microphone, wishes.camera);
        if flags == self.advertised {
            return Ok(());
        }
        send(writes, Write::Flags(flags))?;
        if self.advertised == ParticipantFlags::new(false, false) {
            self.awaiting_publisher = true;
        } else if !wishes.microphone && !wishes.camera {
            self.awaiting_publisher = false;
            self.pending.clear();
        }
        self.advertised = flags;
        Ok(())
    }

    pub(super) fn announce(
        &mut self,
        peers: &[Peer],
        routes: &[PeerSession],
        own: &PeerSession,
        writes: &mpsc::Sender<Write>,
    ) -> Result<(), ()> {
        if self.awaiting_publisher {
            // Snapshot both bounded projections at the first connected tick.
            // Either observation can precede the other; only their intersection
            // may receive an offer.
            self.pending
                .extend(routes.iter().filter(|p| *p != own).cloned());
            for peer in peers {
                if !self.pending.contains(&peer.session) {
                    self.pending.push(peer.session.clone());
                }
            }
            self.awaiting_publisher = false;
        }
        let mut index = 0;
        while index < self.pending.len() {
            let target = &self.pending[index];
            let member = peers.iter().any(|p| p.session == *target);
            let routed = routes.contains(target);
            if member && routed {
                send(
                    writes,
                    Write::SendStreamOffer(target.clone(), StreamKind::Camera),
                )?;
            } else if member || routed {
                index += 1;
                continue;
            }
            self.pending.swap_remove(index);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::bounded::Text;

    fn peer(id: &str) -> Peer {
        Peer {
            session: PeerSession::parse(id).unwrap(),
            name: Text::new("Peer").unwrap(),
            microphone: false,
            camera: false,
            publish_media: false,
        }
    }

    #[test]
    fn early_hpb_membership_waits_for_rest_and_subsequent_joins_use_normal_subscriptions() {
        let own = PeerSession::parse("own").unwrap();
        let mut publication = Publication::new(ParticipantFlags::new(false, false));
        let (tx, mut rx) = mpsc::channel(4);
        let early = peer("early");
        let late = peer("late");
        let routes = [early.session.clone(), late.session.clone()];
        publication
            .update(
                MediaWishes {
                    camera: true,
                    ..Default::default()
                },
                &tx,
            )
            .unwrap();
        publication.announce(&[], &routes[..1], &own, &tx).unwrap();
        publication
            .announce(&[early, late], &routes, &own, &tx)
            .unwrap();
        assert!(matches!(rx.try_recv(), Ok(Write::Flags(_))));
        assert!(
            matches!(rx.try_recv(), Ok(Write::SendStreamOffer(id, StreamKind::Camera)) if id == routes[0])
        );
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn camera_only_publication_waits_for_each_route_and_retires_offers_on_mute() {
        let own = PeerSession::parse("own").unwrap();
        let mut publication = Publication::new(ParticipantFlags::new(false, false));
        let (tx, mut rx) = mpsc::channel(8);
        let peers = [peer("early"), peer("late")];
        let early = peers[0].session.clone();
        let late = peers[1].session.clone();
        let camera = MediaWishes {
            camera: true,
            ..Default::default()
        };
        publication.update(camera, &tx).unwrap();
        publication.announce(&peers, &[], &own, &tx).unwrap();
        assert!(matches!(rx.try_recv(), Ok(Write::Flags(flags)) if flags.bits() == 5));
        publication
            .announce(&peers, std::slice::from_ref(&early), &own, &tx)
            .unwrap();
        assert!(
            matches!(rx.try_recv(), Ok(Write::SendStreamOffer(id, StreamKind::Camera)) if id == early)
        );
        let routes = [early, late.clone()];
        publication.announce(&peers, &routes, &own, &tx).unwrap();
        publication.announce(&peers, &routes, &own, &tx).unwrap();
        assert!(
            matches!(rx.try_recv(), Ok(Write::SendStreamOffer(id, StreamKind::Camera)) if id == late)
        );
        assert!(rx.try_recv().is_err());

        publication.update(MediaWishes::default(), &tx).unwrap();
        publication.update(camera, &tx).unwrap();
        publication.announce(&peers, &[], &own, &tx).unwrap();
        publication.update(MediaWishes::default(), &tx).unwrap();
        publication.announce(&peers, &routes, &own, &tx).unwrap();
        for bits in [1, 5, 1] {
            assert!(matches!(rx.try_recv(), Ok(Write::Flags(flags)) if flags.bits() == bits));
        }
        assert!(rx.try_recv().is_err());
    }
}
