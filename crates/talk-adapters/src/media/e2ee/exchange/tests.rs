use super::*;
use crate::media::e2ee::frame::{Decryptor, Encryptor, FrameKind};
use std::collections::BTreeMap;

struct Node {
    exchange: Exchange,
    /// What this node's media layer was told.
    local: ([u8; KEY_LEN], u32),
    remote: BTreeMap<String, ([u8; KEY_LEN], u32)>,
    forgotten: Vec<String>,
}
struct Net {
    nodes: BTreeMap<String, Node>,
    now: Instant,
    /// Sessions whose messages are dropped, to model a peer that never answers.
    silent: Vec<String>,
    log: Vec<(String, String, Value)>,
}
impl Net {
    fn new(sessions: &[&str]) -> Self {
        let mut net = Self {
            nodes: BTreeMap::new(),
            now: Instant::now(),
            silent: Vec::new(),
            log: Vec::new(),
        };
        for session in sessions {
            net.add(session);
        }
        net
    }
    fn add(&mut self, session: &str) {
        let exchange = Exchange::new(session).unwrap();
        self.nodes.insert(
            session.to_owned(),
            Node {
                exchange,
                local: ([0; KEY_LEN], 0),
                remote: BTreeMap::new(),
                forgotten: Vec::new(),
            },
        );
        self.pump();
    }
    fn join_all(&mut self) {
        let all: Vec<String> = self.nodes.keys().cloned().collect();
        let now = self.now;
        for node in self.nodes.values_mut() {
            node.exchange.users_joined(now, &all);
        }
        self.pump();
    }
    fn advance(&mut self, by: Duration) {
        self.now += by;
        let now = self.now;
        for node in self.nodes.values_mut() {
            node.exchange.poll(now);
        }
        self.pump();
    }
    /// Deliver everything until no node has output left.
    fn pump(&mut self) {
        for _ in 0..200 {
            let mut sends = Vec::new();
            for (id, node) in &mut self.nodes {
                for output in node.exchange.take_outputs() {
                    match output {
                        Output::Send { to, payload } => sends.push((id.clone(), to, payload)),
                        Output::SetKey {
                            participant: Participant::Local,
                            key,
                            index,
                        } => node.local = (key, index),
                        Output::SetKey {
                            participant: Participant::Remote(peer),
                            key,
                            index,
                        } => {
                            node.remote.insert(peer, (key, index));
                        }
                        Output::Forget(peer) => {
                            node.remote.remove(&peer);
                            node.forgotten.push(peer);
                        }
                    }
                }
            }
            if sends.is_empty() {
                return;
            }
            let now = self.now;
            for (from, to, payload) in sends {
                self.log.push((from.clone(), to.clone(), payload.clone()));
                if self.silent.contains(&to) {
                    continue;
                }
                if let Some(node) = self.nodes.get_mut(&to) {
                    node.exchange.message(now, &from, &payload);
                }
            }
        }
        panic!("key exchange did not settle");
    }
    fn leave(&mut self, session: &str) {
        self.nodes.remove(session);
        let now = self.now;
        for node in self.nodes.values_mut() {
            node.exchange.users_left(now, &[session.to_owned()]);
        }
        self.pump();
    }
    fn sent(&self, kind: &str) -> usize {
        self.log
            .iter()
            .filter(|(_, _, payload)| payload["type"] == kind)
            .count()
    }
}

#[test]
fn two_participants_end_up_with_each_others_keys() {
    let mut net = Net::new(&["a-low", "b-high"]);
    net.join_all();
    let (a, b) = (&net.nodes["a-low"], &net.nodes["b-high"]);
    assert_eq!(a.remote["b-high"], b.local);
    assert_eq!(b.remote["a-low"], a.local);
    assert_ne!(a.local.0, b.local.0);
    assert_ne!(a.local.0, [0; KEY_LEN]);
    // The larger session id starts, the smaller answers and finishes.
    assert_eq!(net.sent("encryption.start"), 1);
    assert_eq!(net.log[0].0, "b-high");
    assert_eq!(net.sent("encryption.finish"), 1);
}

#[test]
fn a_group_agrees_on_every_pair_and_late_joiners_receive_current_keys() {
    let mut net = Net::new(&["s1", "s2", "s3"]);
    net.join_all();
    for (me, node) in &net.nodes {
        for (other, peer) in &net.nodes {
            if me != other {
                assert_eq!(node.remote[other], peer.local, "{me} sees {other}");
            }
        }
    }
    net.add("s4");
    let now = net.now;
    let all: Vec<String> = net.nodes.keys().cloned().collect();
    for node in net.nodes.values_mut() {
        node.exchange.users_joined(now, &all);
    }
    net.pump();
    for (me, node) in &net.nodes {
        for (other, peer) in &net.nodes {
            if me != other {
                assert_eq!(node.remote[other], peer.local, "{me} sees {other}");
            }
        }
    }
}

#[test]
fn a_join_ratchets_every_key_and_receivers_follow_without_a_message() {
    let mut net = Net::new(&["a", "b"]);
    net.join_all();
    let before_a = net.nodes["a"].local;
    let before_b = net.nodes["b"].local;
    let messages = net.log.len();
    net.advance(Duration::from_secs(4));
    assert_eq!(net.nodes["a"].local, before_a, "debounce still running");
    net.advance(Duration::from_secs(2));
    let (a, b) = (net.nodes["a"].local, net.nodes["b"].local);
    assert_ne!(a.0, before_a.0);
    assert_eq!(a.1, before_a.1, "a ratchet keeps the key index");
    assert_eq!(a.0, *ratchet(&before_a.0).unwrap());
    assert_eq!(b.0, *ratchet(&before_b.0).unwrap());
    assert_eq!(net.log.len(), messages, "ratcheting needs no message");

    // B still holds A's old key; A's frames under the ratcheted key still open.
    let mut encryptor = Encryptor::default();
    encryptor.set_key(&a.0, a.1).unwrap();
    let sealed = encryptor
        .encrypt(9, 9, FrameKind::VideoDelta, &[5; 50])
        .unwrap();
    let mut decryptor = Decryptor::default();
    let held = net.nodes["b"].remote["a"];
    decryptor.set_key(&held.0, held.1).unwrap();
    assert_eq!(
        decryptor.decrypt(FrameKind::VideoDelta, &sealed).unwrap(),
        vec![5; 50]
    );
}

#[test]
fn a_departure_replaces_the_key_and_switches_after_the_peers_confirmed() {
    let mut net = Net::new(&["a", "b", "c"]);
    net.join_all();
    net.advance(Duration::from_secs(6));
    let old_a = net.nodes["a"].local;
    net.leave("c");
    assert_eq!(net.nodes["a"].forgotten, ["c"]);
    assert!(!net.nodes["a"].remote.contains_key("c"));
    net.advance(Duration::from_secs(4));
    assert_eq!(net.nodes["a"].local, old_a, "debounce still running");
    net.advance(Duration::from_secs(2));
    let a = net.nodes["a"].local;
    assert_eq!(a.1, old_a.1 + 1);
    assert_ne!(a.0, old_a.0);
    assert_ne!(
        a.0,
        *ratchet(&old_a.0).unwrap(),
        "rotation is not derived from the old key"
    );
    assert_eq!(
        net.nodes["b"].remote["a"], a,
        "the peer already holds the new key"
    );
    assert_eq!(net.nodes["a"].remote["b"], net.nodes["b"].local);
    assert!(net.sent("encryption.gotkey") >= 2);
}

#[test]
fn a_peer_that_never_answers_delays_the_switch_by_the_request_timeout_only() {
    let mut net = Net::new(&["a", "b", "c"]);
    net.join_all();
    net.advance(Duration::from_secs(6));
    let old = net.nodes["a"].local;
    net.silent.push("c".to_owned());
    net.leave("zz-not-there");
    net.advance(Duration::from_secs(6));
    assert_eq!(net.nodes["a"].local, old, "waiting for the silent peer");
    net.advance(Duration::from_secs(5));
    assert_eq!(net.nodes["a"].local.1, old.1 + 1);
    assert_ne!(net.nodes["a"].local.0, old.0);
}

#[test]
fn a_ratchet_never_overtakes_a_rotation_in_progress() {
    let mut net = Net::new(&["a", "b"]);
    net.join_all();
    net.advance(Duration::from_secs(6));
    net.silent.push("b".to_owned());
    let before = net.nodes["a"].local;
    net.leave("zz-gone");
    // The rotation starts at eleven seconds and waits for b's answer until sixteen.
    net.advance(Duration::from_secs(6));
    assert_eq!(net.nodes["a"].local, before);
    let now = net.now;
    net.nodes
        .get_mut("a")
        .unwrap()
        .exchange
        .users_joined(now, &["b".to_owned()]);
    net.advance(Duration::from_secs(3));
    assert_eq!(
        net.nodes["a"].local, before,
        "still waiting for the rotation"
    );
    net.advance(Duration::from_secs(3));
    // The rotation settled by timeout; the join's ratchet ran after it, so the
    // new index is in use and the old key chain is not.
    let after = net.nodes["a"].local;
    assert_eq!(after.1, before.1 + 1);
    assert_ne!(after.0, before.0);
    assert_ne!(after.0, *ratchet(&before.0).unwrap());
}

#[test]
fn malformed_unexpected_and_oversized_messages_change_nothing() {
    let mut net = Net::new(&["a", "b"]);
    let now = net.now;
    let a = &mut net.nodes.get_mut("a").unwrap().exchange;
    a.take_outputs();
    for payload in [
        json!({"type": "encryption.start"}),
        json!({"type": "encryption.start", "id": "x", "identity": "!!", "key": "??"}),
        json!({"type": "encryption.finish", "id": "x", "key": {"type": 0, "body": "AAAA"}}),
        json!({"type": "encryption.setkey", "id": "x", "key": {"type": 1, "body": "AAAA"}}),
        json!({"type": "encryption.gotkey", "id": "x", "key": {"type": 1, "body": "AAAA"}}),
        json!({"type": "encryption.setkey", "key": 5}),
        json!({"type": "encryption.error", "error": "x"}),
        json!({"type": "offer", "sdp": "x"}),
        json!({"type": "encryption.start", "padding": "x".repeat(MAX_PAYLOAD_TEXT)}),
    ] {
        a.message(now, "b", &payload);
    }
    let outputs = a.take_outputs();
    assert!(
        outputs.iter().all(
            |output| matches!(output, Output::Send { payload, .. } if payload["type"] == ERROR)
        ),
        "{outputs:?}"
    );
    // Nothing above established a session or changed a key.
    net.join_all();
    assert_eq!(net.nodes["a"].remote["b"], net.nodes["b"].local);
}

#[test]
fn a_second_start_from_the_same_peer_is_refused() {
    let mut net = Net::new(&["a", "b"]);
    net.join_all();
    let start = net
        .log
        .iter()
        .find(|(_, _, payload)| payload["type"] == "encryption.start")
        .cloned()
        .unwrap();
    let now = net.now;
    let a = &mut net.nodes.get_mut("a").unwrap().exchange;
    a.take_outputs();
    a.message(now, "b", &start.2);
    let outputs = a.take_outputs();
    assert!(matches!(
        outputs.as_slice(),
        [Output::Send { payload, .. }] if payload["error"] == "Session already created"
    ));
}

#[test]
fn padded_base64_round_trips_like_the_javascript_encoder() {
    for length in 0..=40 {
        let bytes: Vec<u8> = (0..length).map(|n| (n as u8).wrapping_mul(7)).collect();
        let text = base64_padded(&bytes);
        assert_eq!(text.len() % 4, 0);
        assert_eq!(base64_decode(text.trim_end_matches('=')).unwrap(), bytes);
    }
    assert_eq!(base64_padded(b"Man"), "TWFu");
    assert_eq!(base64_padded(b"Ma"), "TWE=");
    assert_eq!(base64_padded(b"M"), "TQ==");
}
