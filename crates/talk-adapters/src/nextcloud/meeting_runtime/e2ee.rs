//! Call end-to-end encryption for one meeting: the key exchange with every
//! participant, the keys handed to the media engines, and the roster that drives
//! key rotation. The state machine lives in [`crate::media::e2ee`]; this owner
//! connects it to signaling and to the call's peer connections.
use crate::media::e2ee::{
    Keys,
    exchange::{Exchange, Output, Participant},
    frame::{Decryptor, Encryptor},
};
use nextcloud_talk_api::signaling::PeerSession;
use serde_json::Value;
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::sync::mpsc;

const MAX_PARTICIPANTS: usize = 256;

pub(super) struct Call {
    exchange: Exchange,
    /// Talk identity to the session id both clients order by (see `WireResolver`).
    resolver: Arc<dyn Fn(&PeerSession) -> Option<PeerSession> + Send + Sync>,
    send: Arc<Mutex<Encryptor>>,
    /// One decryptor per remote participant, shared by its camera and screen.
    receivers: Vec<(PeerSession, Arc<Mutex<Decryptor>>)>,
    /// Wire session ids (what the exchange knows) to Talk identities.
    wires: Vec<(String, PeerSession)>,
    roster: Vec<String>,
    outbox: mpsc::Sender<(PeerSession, Value)>,
}

impl Call {
    pub(super) fn start(
        own: &PeerSession,
        resolver: Arc<dyn Fn(&PeerSession) -> Option<PeerSession> + Send + Sync>,
        outbox: mpsc::Sender<(PeerSession, Value)>,
    ) -> Result<Self, ()> {
        let mut call = Self {
            exchange: Exchange::new(own.wire_text()).map_err(|_| ())?,
            resolver,
            send: Arc::new(Mutex::new(Encryptor::default())),
            receivers: Vec::with_capacity(17),
            wires: Vec::with_capacity(17),
            roster: Vec::with_capacity(17),
            outbox,
        };
        call.drain();
        // The sender key must exist before any frame leaves.
        if !call.send.lock().map_err(|_| ())?.has_key() {
            return Err(());
        }
        Ok(call)
    }

    /// Key holders for a peer connection with `peer` (the local sender key is
    /// shared by every connection of this call).
    pub(super) fn keys(&mut self, peer: &PeerSession) -> Keys {
        let receive = match self.receivers.iter().find(|(owner, _)| owner == peer) {
            Some((_, receive)) => receive.clone(),
            None => {
                let receive = Arc::new(Mutex::new(Decryptor::default()));
                if self.receivers.len() < MAX_PARTICIPANTS {
                    self.receivers.push((peer.clone(), receive.clone()));
                }
                receive
            }
        };
        Keys {
            send: self.send.clone(),
            receive,
        }
    }

    /// Follow the call's participants: a join starts key sessions, a departure
    /// replaces our key.
    pub(super) fn sync(&mut self, now: Instant, peers: &[PeerSession]) {
        let mut present = Vec::with_capacity(peers.len());
        for peer in peers {
            let Some(wire) = (self.resolver)(peer) else {
                continue;
            };
            let wire = wire.wire_text().to_owned();
            if !self.wires.iter().any(|(known, _)| *known == wire)
                && self.wires.len() < MAX_PARTICIPANTS
            {
                self.wires.push((wire.clone(), peer.clone()));
            }
            present.push(wire);
        }
        let joined: Vec<String> = present
            .iter()
            .filter(|wire| !self.roster.contains(wire))
            .cloned()
            .collect();
        let left: Vec<String> = self
            .roster
            .iter()
            .filter(|wire| !present.contains(wire))
            .cloned()
            .collect();
        if !left.is_empty() {
            self.exchange.users_left(now, &left);
            self.wires.retain(|(wire, _)| !left.contains(wire));
        }
        if !joined.is_empty() {
            self.exchange.users_joined(now, &joined);
        }
        self.roster = present;
        self.drain();
    }

    pub(super) fn message(
        &mut self,
        now: Instant,
        from: &PeerSession,
        wire: &PeerSession,
        payload: &Value,
    ) {
        if !self
            .wires
            .iter()
            .any(|(known, _)| known == wire.wire_text())
            && self.wires.len() < MAX_PARTICIPANTS
        {
            self.wires.push((wire.wire_text().to_owned(), from.clone()));
        }
        self.exchange.message(now, wire.wire_text(), payload);
        self.drain();
    }

    pub(super) fn poll(&mut self, now: Instant) {
        self.exchange.poll(now);
        self.drain();
    }

    fn talk(&self, wire: &str) -> Option<&PeerSession> {
        self.wires
            .iter()
            .find(|(known, _)| known == wire)
            .map(|(_, talk)| talk)
    }

    /// Hand the exchange's results to the media engines and to signaling.
    fn drain(&mut self) {
        for output in self.exchange.take_outputs() {
            match output {
                Output::Send { to, payload } => {
                    if let Some(peer) = self.talk(&to).cloned() {
                        // Full queue: the exchange's own timers recover.
                        let _ = self.outbox.try_send((peer, payload));
                    }
                }
                Output::SetKey {
                    participant: Participant::Local,
                    key,
                    index,
                } => {
                    if let Ok(mut send) = self.send.lock() {
                        let _ = send.set_key(&key, index);
                    }
                }
                Output::SetKey {
                    participant: Participant::Remote(wire),
                    key,
                    index,
                } => {
                    if let Some(peer) = self.talk(&wire).cloned() {
                        let receive = self.keys(&peer).receive;
                        if let Ok(mut receive) = receive.lock() {
                            let _ = receive.set_key(&key, index);
                        }
                    }
                }
                Output::Forget(wire) => {
                    if let Some(peer) = self.talk(&wire).cloned()
                        && let Some((_, receive)) =
                            self.receivers.iter().find(|(owner, _)| *owner == peer)
                        && let Ok(mut receive) = receive.lock()
                    {
                        receive.clear();
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::e2ee::frame::FrameKind;
    use std::time::Duration;

    fn peer(text: &str) -> PeerSession {
        PeerSession::parse(text).unwrap()
    }
    /// Talk identity "t<x>" is reached on the wire as "w<x>".
    fn resolver() -> Arc<dyn Fn(&PeerSession) -> Option<PeerSession> + Send + Sync> {
        Arc::new(|talk| {
            talk.wire_text()
                .strip_prefix('t')
                .and_then(|rest| PeerSession::parse(&format!("w{rest}")).ok())
        })
    }
    struct Side {
        talk: PeerSession,
        wire: PeerSession,
        call: Call,
        outbox: mpsc::Receiver<(PeerSession, Value)>,
    }
    fn side(letter: &str) -> Side {
        let (tx, outbox) = mpsc::channel(64);
        let wire = peer(&format!("w{letter}"));
        Side {
            talk: peer(&format!("t{letter}")),
            call: Call::start(&wire, resolver(), tx).unwrap(),
            wire,
            outbox,
        }
    }
    /// Deliver every queued message to its recipient until all are idle.
    fn pump(now: Instant, sides: &mut [&mut Side]) {
        for _ in 0..50 {
            let mut deliveries = Vec::new();
            for side in sides.iter_mut() {
                while let Ok((to, payload)) = side.outbox.try_recv() {
                    deliveries.push((side.talk.clone(), side.wire.clone(), to, payload));
                }
            }
            if deliveries.is_empty() {
                return;
            }
            for (from, wire, to, payload) in deliveries {
                if let Some(target) = sides.iter_mut().find(|side| side.talk == to) {
                    target.call.message(now, &from, &wire, &payload);
                }
            }
        }
        panic!("no convergence");
    }
    fn seal_open(from: &mut Side, to: &mut Side) -> bool {
        let sealed = from.call.keys(&to.talk).send.lock().unwrap().encrypt(
            1,
            1,
            FrameKind::VideoDelta,
            &[9; 40],
        );
        let Ok(sealed) = sealed else { return false };
        let opened = to
            .call
            .keys(&from.talk)
            .receive
            .lock()
            .unwrap()
            .decrypt(FrameKind::VideoDelta, &sealed);
        opened.is_ok_and(|plain| plain == vec![9; 40])
    }

    #[test]
    fn participants_that_join_a_call_can_open_each_others_frames() {
        let now = Instant::now();
        let (mut a, mut b) = (side("a"), side("b"));
        // Nothing is shared before the roster says they are together.
        assert!(!seal_open(&mut a, &mut b));
        a.call.sync(now, &[b.talk.clone()]);
        b.call.sync(now, &[a.talk.clone()]);
        pump(now, &mut [&mut a, &mut b]);
        assert!(seal_open(&mut a, &mut b));
        assert!(seal_open(&mut b, &mut a));
    }

    #[test]
    fn a_departure_rotates_the_key_and_the_remaining_participants_follow() {
        let now = Instant::now();
        let (mut a, mut b, mut c) = (side("a"), side("b"), side("c"));
        let everyone = |me: &Side, others: &[&Side]| {
            let _ = me;
            others
                .iter()
                .map(|side| side.talk.clone())
                .collect::<Vec<_>>()
        };
        let roster_a = everyone(&a, &[&b, &c]);
        let roster_b = everyone(&b, &[&a, &c]);
        let roster_c = everyone(&c, &[&a, &b]);
        a.call.sync(now, &roster_a);
        b.call.sync(now, &roster_b);
        c.call.sync(now, &roster_c);
        pump(now, &mut [&mut a, &mut b, &mut c]);
        assert!(
            seal_open(&mut a, &mut b) && seal_open(&mut b, &mut c) && seal_open(&mut c, &mut a)
        );

        // c leaves; after the debounce a and b switch to fresh keys.
        a.call.sync(now, &[b.talk.clone()]);
        b.call.sync(now, &[a.talk.clone()]);
        let later = now + Duration::from_secs(6);
        a.call.poll(later);
        b.call.poll(later);
        pump(later, &mut [&mut a, &mut b]);
        assert!(seal_open(&mut a, &mut b) && seal_open(&mut b, &mut a));
        // What a sends now cannot be opened with what c held when it left.
        let sealed = a
            .call
            .keys(&b.talk)
            .send
            .lock()
            .unwrap()
            .encrypt(1, 3, FrameKind::VideoDelta, &[9; 40])
            .unwrap();
        let held_by_c = c.call.keys(&a.talk).receive;
        assert!(
            held_by_c
                .lock()
                .unwrap()
                .decrypt(FrameKind::VideoDelta, &sealed)
                .is_err()
        );
    }
}
