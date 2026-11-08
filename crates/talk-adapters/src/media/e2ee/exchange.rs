//! Key exchange of Nextcloud Talk's call end-to-end encryption.
//!
//! Every participant owns one random 32 byte key and sends its frames with it. The
//! participant with the larger session id opens an Olm session with each peer
//! (`encryption.start`); the peer answers with its own key inside the first Olm
//! message (`encryption.finish`), and after that either side can replace its key
//! (`encryption.setkey`, acknowledged by `encryption.gotkey`).
//!
//! * A joining participant gets the current key and everybody ratchets their own key
//!   one step, so the newcomer cannot read what came before. Receivers that missed
//!   the step catch up by ratcheting on authentication failure ([`super::frame`]).
//! * A leaving participant triggers a fresh random key, distributed to all peers; the
//!   sender switches only after every peer confirmed or timed out.
//!
//! This is a deterministic state machine: callers pass the time, deliver signaling
//! messages in and send the produced [`Output`] on. Nothing here does I/O.
use super::frame::{KEY_LEN, ratchet};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use vodozemac::olm::{Account, OlmMessage, PreKeyMessage, Session, SessionConfig};
use vodozemac::{Curve25519PublicKey, base64_decode, base64_encode};
use zeroize::Zeroizing;

/// Quiet period before a join or leave changes the key.
const DEBOUNCE: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PEERS: usize = 256;
const MAX_REQUESTS: usize = 1024;
const MAX_PAYLOAD_TEXT: usize = 8 * 1024;

pub(crate) const START: &str = "encryption.start";
pub(crate) const FINISH: &str = "encryption.finish";
pub(crate) const SET_KEY: &str = "encryption.setkey";
pub(crate) const GOT_KEY: &str = "encryption.gotkey";
pub(crate) const ERROR: &str = "encryption.error";

/// Whose key an update is about.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Participant {
    Local,
    Remote(String),
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Output {
    /// A call message for one session; the payload is the signaling `payload`.
    Send { to: String, payload: Value },
    /// Use this key (and index) for the participant's frames from now on.
    SetKey {
        participant: Participant,
        key: [u8; KEY_LEN],
        index: u32,
    },
    /// The participant left; drop everything held for it.
    Forget(String),
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Error {
    Random,
}

fn base64_padded(bytes: &[u8]) -> String {
    let mut text = base64_encode(bytes);
    while !text.len().is_multiple_of(4) {
        text.push('=');
    }
    text
}
fn base64_key(text: &str) -> Option<[u8; KEY_LEN]> {
    // Talk's JavaScript sends padded base64; decoding accepts both.
    base64_decode(text.trim_end_matches('='))
        .ok()?
        .try_into()
        .ok()
}

#[derive(Default)]
struct Peer {
    session: Option<Session>,
    start_id: Option<String>,
    last_key: Option<[u8; KEY_LEN]>,
}
enum Request {
    Start { peer: String },
    SetKey { peer: String, rotation: bool },
}

struct Rotation {
    pending: usize,
    index: u32,
    key: Zeroizing<[u8; KEY_LEN]>,
}

pub(crate) struct Exchange {
    local: String,
    account: Account,
    identity: String,
    key: Zeroizing<[u8; KEY_LEN]>,
    index: u32,
    peers: HashMap<String, Peer>,
    requests: HashMap<String, (Request, Instant)>,
    rotate_at: Option<Instant>,
    ratchet_at: Option<Instant>,
    rotation: Option<Rotation>,
    outputs: Vec<Output>,
    next_id: u64,
}

impl Exchange {
    /// A new participant with its own random key, in use immediately.
    pub(crate) fn new(local_session: &str) -> Result<Self, Error> {
        let mut key = Zeroizing::new([0_u8; KEY_LEN]);
        getrandom::fill(key.as_mut_slice()).map_err(|_| Error::Random)?;
        let account = Account::new();
        let identity = account.curve25519_key().to_base64();
        let mut exchange = Self {
            local: local_session.to_owned(),
            account,
            identity,
            key,
            index: 0,
            peers: HashMap::new(),
            requests: HashMap::new(),
            rotate_at: None,
            ratchet_at: None,
            rotation: None,
            outputs: Vec::new(),
            next_id: 0,
        };
        exchange.announce_local();
        Ok(exchange)
    }

    pub(crate) fn take_outputs(&mut self) -> Vec<Output> {
        std::mem::take(&mut self.outputs)
    }

    fn announce_local(&mut self) {
        self.outputs.push(Output::SetKey {
            participant: Participant::Local,
            key: *self.key,
            index: self.index,
        });
    }

    fn request_id(&mut self) -> String {
        // Only unique among this participant's open requests; the peer echoes it.
        self.next_id += 1;
        format!("{:x}-{:x}", self.next_id, self.local.len())
    }

    /// Participants that are in the call. Larger session ids start the sessions.
    pub(crate) fn users_joined(&mut self, now: Instant, sessions: &[String]) {
        for session in sessions {
            if *session == self.local {
                continue;
            }
            if session.as_str() < self.local.as_str() {
                self.start_session(now, session);
            } else if self.peers.len() < MAX_PEERS {
                self.peers.entry(session.clone()).or_default();
            }
        }
        // A newcomer must not read the past: advance our key one step.
        self.ratchet_at = Some(now + DEBOUNCE);
    }

    pub(crate) fn users_left(&mut self, now: Instant, sessions: &[String]) {
        for session in sessions {
            if self.peers.remove(session).is_some() {
                self.outputs.push(Output::Forget(session.clone()));
            }
            self.requests.retain(|_, (request, _)| match request {
                Request::Start { peer } | Request::SetKey { peer, .. } => peer != session,
            });
        }
        // Whoever left must not read the future: replace our key.
        self.rotate_at = Some(now + DEBOUNCE);
        self.settle_rotation();
    }

    fn start_session(&mut self, now: Instant, peer_id: &str) {
        if self.peers.len() >= MAX_PEERS && !self.peers.contains_key(peer_id) {
            return;
        }
        let peer = self.peers.entry(peer_id.to_owned()).or_default();
        if peer.session.is_some() || peer.start_id.is_some() || self.requests.len() >= MAX_REQUESTS
        {
            return;
        }
        self.account.generate_one_time_keys(1);
        let Some(one_time) = self.account.one_time_keys().into_values().next() else {
            return;
        };
        self.account.mark_keys_as_published();
        let id = self.request_id();
        if let Some(peer) = self.peers.get_mut(peer_id) {
            peer.start_id = Some(id.clone());
        }
        self.requests.insert(
            id.clone(),
            (
                Request::Start {
                    peer: peer_id.to_owned(),
                },
                now + REQUEST_TIMEOUT,
            ),
        );
        self.outputs.push(Output::Send {
            to: peer_id.to_owned(),
            payload: json!({
                "id": id,
                "type": START,
                "identity": self.identity,
                "key": one_time.to_base64(),
            }),
        });
    }

    /// Timers: open requests that ran out, then debounced ratchet and rotation.
    pub(crate) fn poll(&mut self, now: Instant) {
        let expired: Vec<String> = self
            .requests
            .iter()
            .filter(|(_, (_, deadline))| *deadline <= now)
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            self.resolve(&id);
        }
        if self.rotate_at.is_some_and(|at| at <= now) {
            self.rotate_at = None;
            self.rotate(now);
        }
        if self.ratchet_at.is_some_and(|at| at <= now) {
            self.ratchet_at = None;
            self.advance_key();
        }
    }

    /// A request is finished by its answer or by running out of time.
    fn resolve(&mut self, id: &str) {
        let Some((request, _)) = self.requests.remove(id) else {
            return;
        };
        match request {
            Request::Start { peer } => {
                if let Some(peer) = self.peers.get_mut(&peer)
                    && peer.session.is_none()
                    && peer.start_id.as_deref() == Some(id)
                {
                    peer.start_id = None;
                }
            }
            Request::SetKey { rotation: true, .. } => {
                if let Some(rotation) = &mut self.rotation {
                    rotation.pending = rotation.pending.saturating_sub(1);
                }
                self.settle_rotation();
            }
            Request::SetKey { .. } => {}
        }
    }

    fn rotate(&mut self, now: Instant) {
        if self.rotation.is_some() {
            return;
        }
        let mut key = Zeroizing::new([0_u8; KEY_LEN]);
        if getrandom::fill(key.as_mut_slice()).is_err() {
            return;
        }
        // New peers receive the new key at once; our own frames switch later.
        self.key = key.clone();
        self.index = self.index.wrapping_add(1);
        let ready: Vec<String> = self
            .peers
            .iter()
            .filter(|(_, peer)| peer.session.is_some())
            .map(|(id, _)| id.clone())
            .collect();
        let mut pending = 0;
        for peer in ready {
            if self.send_key(now, &peer, true) {
                pending += 1;
            }
        }
        self.rotation = Some(Rotation {
            pending,
            index: self.index,
            key,
        });
        self.settle_rotation();
    }

    fn settle_rotation(&mut self) {
        if self
            .rotation
            .as_ref()
            .is_some_and(|rotation| rotation.pending == 0)
            && let Some(rotation) = self.rotation.take()
        {
            self.outputs.push(Output::SetKey {
                participant: Participant::Local,
                key: *rotation.key,
                index: rotation.index,
            });
        }
    }

    fn advance_key(&mut self) {
        if self.rotation.is_some() {
            return;
        }
        if let Ok(next) = ratchet(&self.key) {
            self.key = next;
            self.announce_local();
        }
    }

    fn encrypted_key(&mut self, peer: &str) -> Option<Value> {
        let data = json!({"key": base64_padded(&*self.key), "index": self.index}).to_string();
        let session = self.peers.get_mut(peer)?.session.as_mut()?;
        let (kind, body) = session.encrypt(data).ok()?.to_parts();
        Some(json!({"type": kind, "body": base64_encode(body)}))
    }

    /// Send our key; true when an answer is awaited.
    fn send_key(&mut self, now: Instant, peer: &str, rotation: bool) -> bool {
        if self.requests.len() >= MAX_REQUESTS {
            return false;
        }
        let Some(key) = self.encrypted_key(peer) else {
            return false;
        };
        let id = self.request_id();
        self.requests.insert(
            id.clone(),
            (
                Request::SetKey {
                    peer: peer.to_owned(),
                    rotation,
                },
                now + REQUEST_TIMEOUT,
            ),
        );
        self.outputs.push(Output::Send {
            to: peer.to_owned(),
            payload: json!({"id": id, "type": SET_KEY, "key": key}),
        });
        true
    }

    fn send_error(&mut self, to: &str, reason: &str) {
        self.outputs.push(Output::Send {
            to: to.to_owned(),
            payload: json!({"type": ERROR, "error": reason}),
        });
    }

    fn set_peer_key(&mut self, peer: &str, data: &Value) -> bool {
        let (Some(key), Some(index)) = (
            data["key"].as_str().and_then(base64_key),
            data["index"].as_u64().and_then(|n| u32::try_from(n).ok()),
        ) else {
            return false;
        };
        let Some(state) = self.peers.get_mut(peer) else {
            return false;
        };
        if state.last_key != Some(key) {
            state.last_key = Some(key);
            self.outputs.push(Output::SetKey {
                participant: Participant::Remote(peer.to_owned()),
                key,
                index,
            });
        }
        true
    }

    fn decrypt(&mut self, peer: &str, key: &Value) -> Option<Value> {
        let kind = usize::try_from(key["type"].as_u64()?).ok()?;
        let body = base64_decode(key["body"].as_str()?.trim_end_matches('=')).ok()?;
        let message = OlmMessage::from_parts(kind, &body).ok()?;
        let session = self.peers.get_mut(peer)?.session.as_mut()?;
        let plain = session.decrypt(&message).ok()?;
        serde_json::from_slice(&plain).ok()
    }

    /// A call message from another participant's session.
    pub(crate) fn message(&mut self, now: Instant, from: &str, payload: &Value) {
        let kind = payload["type"].as_str().unwrap_or_default();
        if !kind.starts_with("encryption.") || payload.to_string().len() > MAX_PAYLOAD_TEXT {
            return;
        }
        match kind {
            START => self.process_start(from, payload),
            FINISH => self.process_finish(now, from, payload),
            SET_KEY => self.process_set_key(from, payload),
            GOT_KEY => self.process_got_key(from, payload),
            _ => {}
        }
    }

    fn process_start(&mut self, from: &str, payload: &Value) {
        if self.peers.len() >= MAX_PEERS && !self.peers.contains_key(from) {
            return;
        }
        if self
            .peers
            .get(from)
            .is_some_and(|peer| peer.session.is_some())
        {
            self.send_error(from, "Session already created");
            return;
        }
        let (Some(id), Some(identity), Some(one_time)) = (
            payload["id"].as_str(),
            payload["identity"]
                .as_str()
                .and_then(|text| Curve25519PublicKey::from_base64(text).ok()),
            payload["key"]
                .as_str()
                .and_then(|text| Curve25519PublicKey::from_base64(text).ok()),
        ) else {
            self.send_error(from, "Invalid session request");
            return;
        };
        let Ok(session) =
            self.account
                .create_outbound_session(SessionConfig::version_1(), identity, one_time)
        else {
            self.send_error(from, "Session could not be created");
            return;
        };
        self.peers.entry(from.to_owned()).or_default().session = Some(session);
        if let Some(key) = self.encrypted_key(from) {
            self.outputs.push(Output::Send {
                to: from.to_owned(),
                payload: json!({"id": id, "type": FINISH, "key": key}),
            });
        }
    }

    fn process_finish(&mut self, now: Instant, from: &str, payload: &Value) {
        let Some(peer) = self.peers.get(from) else {
            self.send_error(from, "No session request");
            return;
        };
        if peer.session.is_some() {
            self.send_error(from, "Session already created");
            return;
        }
        let id = payload["id"].as_str().unwrap_or_default();
        if peer.start_id.as_deref() != Some(id) {
            self.send_error(from, "Finish has wrong id");
            return;
        }
        let key = &payload["key"];
        let parsed = (|| {
            if key["type"].as_u64()? != 0 {
                return None;
            }
            let body = base64_decode(key["body"].as_str()?.trim_end_matches('=')).ok()?;
            let prekey = PreKeyMessage::from_bytes(&body).ok()?;
            self.account
                .create_inbound_session(SessionConfig::version_1(), prekey.identity_key(), &prekey)
                .ok()
        })();
        let Some(created) = parsed else {
            self.send_error(from, "Session could not be created");
            return;
        };
        if let Some(peer) = self.peers.get_mut(from) {
            peer.session = Some(created.session);
            peer.start_id = None;
        }
        self.requests.remove(id);
        if let Ok(data) = serde_json::from_slice::<Value>(&created.plaintext) {
            self.set_peer_key(from, &data);
        }
        // Tell the peer which key we send with.
        self.send_key(now, from, false);
    }

    fn process_set_key(&mut self, from: &str, payload: &Value) {
        if self
            .peers
            .get(from)
            .is_none_or(|peer| peer.session.is_none())
        {
            self.send_error(from, "No session for setting key");
            return;
        }
        let Some(data) = self.decrypt(from, &payload["key"]) else {
            self.send_error(from, "Key could not be read");
            return;
        };
        if self.set_peer_key(from, &data)
            && let Some(key) = self.encrypted_key(from)
        {
            self.outputs.push(Output::Send {
                to: from.to_owned(),
                payload: json!({"id": payload["id"], "type": GOT_KEY, "key": key}),
            });
        }
    }

    fn process_got_key(&mut self, from: &str, payload: &Value) {
        if self
            .peers
            .get(from)
            .is_none_or(|peer| peer.session.is_none())
        {
            self.send_error(from, "No session for confirming key");
            return;
        }
        if let Some(data) = self.decrypt(from, &payload["key"]) {
            self.set_peer_key(from, &data);
        }
        if let Some(id) = payload["id"].as_str()
            && matches!(self.requests.get(id), Some((Request::SetKey { peer, .. }, _)) if peer == from)
        {
            self.resolve(id);
        }
    }
}

#[cfg(test)]
mod interop;
#[cfg(test)]
mod tests;
