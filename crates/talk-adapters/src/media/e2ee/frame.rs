//! Frame cipher of Nextcloud Talk's call end-to-end encryption.
//!
//! The format is the one the Talk web client produces with encoded transforms
//! (a SFrame-like trailer, originally from Jitsi): the first bytes of a frame stay
//! readable for the SFU and codec, the rest is AES-128-GCM sealed with a key
//! derived from the participant's 32 byte secret by HKDF-SHA256.
//!
//! ```text
//! clear header | ciphertext + tag | IV (12) | 12 | key index
//! ```
//!
//! Nothing here reads a clock, the network or a device; callers own key
//! lifetime and the policy for frames that cannot be protected.
use ring::{aead, hkdf};
use zeroize::Zeroizing;

pub(crate) const KEY_LEN: usize = 32;
/// Four bits of the key index travel in the trailer.
pub(crate) const KEYRING_SIZE: usize = 16;
const IV_LEN: usize = 12;
const TAG_LEN: usize = 16;
const TRAILER_LEN: usize = 2;
/// Receivers try this many ratchet steps ahead of the key they hold.
const RATCHET_WINDOW: usize = 8;
const ENCRYPTION_SALT: &[u8] = b"TalkFrameEncryptionKey";
const RATCHET_SALT: &[u8] = b"TalkFrameRatchetKey";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FrameKind {
    Audio,
    VideoKey,
    VideoDelta,
}
impl FrameKind {
    /// Bytes left readable: the Opus TOC byte, or the VP8 frame tag (and, for
    /// key frames, the start code and dimensions).
    const fn clear_bytes(self) -> usize {
        match self {
            Self::VideoKey => 10,
            Self::VideoDelta => 3,
            Self::Audio => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FrameError {
    /// No key is held for the frame's key index (or for sending).
    NoKey,
    /// The frame is too short or its trailer is not understood.
    Malformed,
    /// Authentication failed with every key that could apply.
    Rejected,
    /// The system random source or the cipher failed.
    Crypto,
}

struct Length(usize);
impl hkdf::KeyType for Length {
    fn len(&self) -> usize {
        self.0
    }
}
fn hkdf_into(salt: &[u8], secret: &[u8], out: &mut [u8]) -> Result<(), FrameError> {
    hkdf::Salt::new(hkdf::HKDF_SHA256, salt)
        .extract(secret)
        .expand(&[], Length(out.len()))
        .and_then(|okm| okm.fill(out))
        .map_err(|_| FrameError::Crypto)
}
fn derive(secret: &[u8; KEY_LEN]) -> Result<aead::LessSafeKey, FrameError> {
    let mut key = Zeroizing::new([0_u8; 16]);
    hkdf_into(ENCRYPTION_SALT, secret, key.as_mut_slice())?;
    aead::UnboundKey::new(&aead::AES_128_GCM, key.as_slice())
        .map(aead::LessSafeKey::new)
        .map_err(|_| FrameError::Crypto)
}
/// The next key of the one-way chain; used when participants join.
pub(crate) fn ratchet(secret: &[u8; KEY_LEN]) -> Result<Zeroizing<[u8; KEY_LEN]>, FrameError> {
    let mut next = Zeroizing::new([0_u8; KEY_LEN]);
    hkdf_into(RATCHET_SALT, secret, next.as_mut_slice())?;
    Ok(next)
}

struct Slot {
    secret: Zeroizing<[u8; KEY_LEN]>,
    key: aead::LessSafeKey,
}
impl Slot {
    fn new(secret: &[u8; KEY_LEN]) -> Result<Self, FrameError> {
        Ok(Self {
            key: derive(secret)?,
            secret: Zeroizing::new(*secret),
        })
    }
}

/// Sixteen keys addressed by the index in the frame trailer.
pub(crate) struct KeyRing {
    slots: [Option<Slot>; KEYRING_SIZE],
    current: usize,
}
impl Default for KeyRing {
    fn default() -> Self {
        Self {
            slots: std::array::from_fn(|_| None),
            current: 0,
        }
    }
}
impl KeyRing {
    /// Install a key and make its index the one used for sending.
    pub(crate) fn set(&mut self, secret: &[u8; KEY_LEN], index: u32) -> Result<(), FrameError> {
        let slot = index as usize % KEYRING_SIZE;
        self.slots[slot] = Some(Slot::new(secret)?);
        self.current = slot;
        Ok(())
    }
    /// Install a key for receiving without changing the sending index.
    pub(crate) fn set_receive(
        &mut self,
        secret: &[u8; KEY_LEN],
        index: u32,
    ) -> Result<(), FrameError> {
        self.slots[index as usize % KEYRING_SIZE] = Some(Slot::new(secret)?);
        Ok(())
    }
    pub(crate) fn has_key(&self) -> bool {
        self.slots[self.current].is_some()
    }
    pub(crate) fn clear(&mut self) {
        self.slots = std::array::from_fn(|_| None);
    }
}

fn nonce(iv: &[u8]) -> Result<aead::Nonce, FrameError> {
    aead::Nonce::try_assume_unique_for_key(iv).map_err(|_| FrameError::Malformed)
}

/// Seals this participant's outgoing frames.
#[derive(Default)]
pub(crate) struct Encryptor {
    ring: KeyRing,
    counters: Vec<(u32, u32)>,
}
impl Encryptor {
    pub(crate) fn set_key(&mut self, secret: &[u8; KEY_LEN], index: u32) -> Result<(), FrameError> {
        self.ring.set(secret, index)
    }
    pub(crate) fn has_key(&self) -> bool {
        self.ring.has_key()
    }
    pub(crate) fn clear(&mut self) {
        self.ring.clear();
        self.counters.clear();
    }

    /// The IV is `ssrc | timestamp | counter`; it only has to be unique per key,
    /// because receivers read it from the trailer.
    fn next_iv(&mut self, ssrc: u32, timestamp: u32) -> Result<[u8; IV_LEN], FrameError> {
        let position = match self.counters.iter().position(|(owner, _)| *owner == ssrc) {
            Some(position) => position,
            None => {
                let mut seed = [0_u8; 2];
                getrandom::fill(&mut seed).map_err(|_| FrameError::Crypto)?;
                if self.counters.len() >= 8 {
                    self.counters.remove(0);
                }
                self.counters
                    .push((ssrc, u32::from(u16::from_be_bytes(seed))));
                self.counters.len() - 1
            }
        };
        let count = self.counters[position].1;
        self.counters[position].1 = count.wrapping_add(1);
        let mut iv = [0_u8; IV_LEN];
        iv[..4].copy_from_slice(&ssrc.to_be_bytes());
        iv[4..8].copy_from_slice(&timestamp.to_be_bytes());
        iv[8..].copy_from_slice(&(count % 0xFFFF).to_be_bytes());
        Ok(iv)
    }

    /// Protect one encoded frame. With no key the frame is not sent in the clear.
    pub(crate) fn encrypt(
        &mut self,
        ssrc: u32,
        timestamp: u32,
        kind: FrameKind,
        frame: &[u8],
    ) -> Result<Vec<u8>, FrameError> {
        let iv = self.next_iv(ssrc, timestamp)?;
        self.seal(kind, frame, iv)
    }

    fn seal(&self, kind: FrameKind, frame: &[u8], iv: [u8; IV_LEN]) -> Result<Vec<u8>, FrameError> {
        let slot = self.ring.slots[self.ring.current]
            .as_ref()
            .ok_or(FrameError::NoKey)?;
        let clear = kind.clear_bytes();
        let (header, body) = frame.split_at_checked(clear).ok_or(FrameError::Malformed)?;
        let mut sealed = body.to_vec();
        slot.key
            .seal_in_place_append_tag(nonce(&iv)?, aead::Aad::from(header), &mut sealed)
            .map_err(|_| FrameError::Crypto)?;
        let mut out = Vec::with_capacity(clear + sealed.len() + IV_LEN + TRAILER_LEN);
        out.extend_from_slice(header);
        out.extend_from_slice(&sealed);
        out.extend_from_slice(&iv);
        out.push(IV_LEN as u8);
        out.push(self.ring.current as u8);
        Ok(out)
    }
}

/// Opens one remote participant's frames.
#[derive(Default)]
pub(crate) struct Decryptor {
    ring: KeyRing,
}
impl Decryptor {
    pub(crate) fn set_key(&mut self, secret: &[u8; KEY_LEN], index: u32) -> Result<(), FrameError> {
        self.ring.set_receive(secret, index)
    }
    pub(crate) fn clear(&mut self) {
        self.ring.clear();
    }

    /// Open a frame. A sender that ratcheted its key without telling us
    /// authenticates only after the same steps here; the key then advances.
    pub(crate) fn decrypt(&mut self, kind: FrameKind, data: &[u8]) -> Result<Vec<u8>, FrameError> {
        let clear = kind.clear_bytes();
        let length = data.len();
        if length < clear + TAG_LEN + IV_LEN + TRAILER_LEN {
            return Err(FrameError::Malformed);
        }
        if usize::from(data[length - 2]) != IV_LEN {
            return Err(FrameError::Malformed);
        }
        let index = usize::from(data[length - 1]);
        let slot = self
            .ring
            .slots
            .get_mut(index)
            .and_then(Option::as_mut)
            .ok_or(FrameError::NoKey)?;
        let iv = &data[length - TRAILER_LEN - IV_LEN..length - TRAILER_LEN];
        let header = &data[..clear];
        let sealed = &data[clear..length - TRAILER_LEN - IV_LEN];

        let open = |key: &aead::LessSafeKey| -> Option<Vec<u8>> {
            let mut buffer = sealed.to_vec();
            let plain_len = key
                .open_in_place(nonce(iv).ok()?, aead::Aad::from(header), &mut buffer)
                .ok()?
                .len();
            buffer.truncate(plain_len);
            let mut out = Vec::with_capacity(clear + plain_len);
            out.extend_from_slice(header);
            out.extend_from_slice(&buffer);
            Some(out)
        };
        if let Some(frame) = open(&slot.key) {
            return Ok(frame);
        }
        let mut secret = slot.secret.clone();
        for _ in 0..RATCHET_WINDOW {
            secret = ratchet(&secret)?;
            let candidate = Slot::new(&secret)?;
            if let Some(frame) = open(&candidate.key) {
                *slot = candidate;
                return Ok(frame);
            }
        }
        Err(FrameError::Rejected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Case {
        kind: String,
        key_index: u32,
        iv: String,
        plain: String,
        encrypted: String,
        ratchet_steps: Option<usize>,
    }
    #[derive(Deserialize)]
    struct Vectors {
        key: String,
        ratchet_1: String,
        ratchet_2: String,
        cases: Vec<Case>,
    }

    fn unhex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
            .collect()
    }
    fn key(text: &str) -> [u8; KEY_LEN] {
        unhex(text).try_into().unwrap()
    }
    fn kind(name: &str) -> FrameKind {
        match name {
            "audio" => FrameKind::Audio,
            "key" => FrameKind::VideoKey,
            _ => FrameKind::VideoDelta,
        }
    }
    fn vectors() -> Vectors {
        // Produced by tools/e2ee/gen_frame_vectors.mjs with Talk's own WebCrypto steps.
        serde_json::from_str(include_str!("fixtures/frame_vectors.json")).unwrap()
    }

    #[test]
    fn ratchet_matches_the_web_client() {
        let vectors = vectors();
        let first = ratchet(&key(&vectors.key)).unwrap();
        assert_eq!(*first, key(&vectors.ratchet_1));
        assert_eq!(*ratchet(&first).unwrap(), key(&vectors.ratchet_2));
    }

    #[test]
    fn frames_sealed_by_the_web_client_open_here() {
        let vectors = vectors();
        for case in vectors.cases.iter().filter(|c| c.ratchet_steps.is_none()) {
            let mut decryptor = Decryptor::default();
            decryptor
                .set_key(&key(&vectors.key), case.key_index)
                .unwrap();
            let plain = decryptor
                .decrypt(kind(&case.kind), &unhex(&case.encrypted))
                .unwrap();
            assert_eq!(
                plain,
                unhex(&case.plain),
                "{} {}",
                case.kind,
                case.key_index
            );
        }
    }

    #[test]
    fn frames_sealed_here_are_byte_identical_to_the_web_client() {
        let vectors = vectors();
        for case in vectors.cases.iter().filter(|c| c.ratchet_steps.is_none()) {
            let mut encryptor = Encryptor::default();
            encryptor
                .set_key(&key(&vectors.key), case.key_index)
                .unwrap();
            let iv: [u8; IV_LEN] = unhex(&case.iv).try_into().unwrap();
            let sealed = encryptor
                .seal(kind(&case.kind), &unhex(&case.plain), iv)
                .unwrap();
            assert_eq!(
                sealed,
                unhex(&case.encrypted),
                "{} {}",
                case.kind,
                case.key_index
            );
        }
    }

    #[test]
    fn a_receiver_behind_by_ratchet_steps_catches_up_and_keeps_the_new_key() {
        let vectors = vectors();
        let case = vectors
            .cases
            .iter()
            .find(|c| c.ratchet_steps == Some(2))
            .unwrap();
        let mut decryptor = Decryptor::default();
        decryptor
            .set_key(&key(&vectors.key), case.key_index)
            .unwrap();
        let plain = decryptor
            .decrypt(kind(&case.kind), &unhex(&case.encrypted))
            .unwrap();
        assert_eq!(plain, unhex(&case.plain));
        // The advanced key now authenticates frames without further ratcheting.
        let slot = decryptor.ring.slots[case.key_index as usize]
            .as_ref()
            .unwrap();
        assert_eq!(*slot.secret, key(&vectors.ratchet_2));
    }

    #[test]
    fn unprotectable_frames_are_refused_not_passed_through() {
        let mut encryptor = Encryptor::default();
        assert_eq!(
            encryptor.encrypt(1, 2, FrameKind::Audio, &[1, 2, 3]),
            Err(FrameError::NoKey)
        );
        encryptor.set_key(&[9; KEY_LEN], 4).unwrap();
        assert_eq!(
            encryptor.encrypt(1, 2, FrameKind::VideoKey, &[0; 9]),
            Err(FrameError::Malformed),
            "a key frame must carry its clear header"
        );
        let sealed = encryptor.encrypt(1, 2, FrameKind::Audio, &[7; 30]).unwrap();
        assert_eq!(sealed[0], 7, "the Opus TOC byte stays readable");
        assert_ne!(&sealed[1..30], &[7; 29][..]);
        assert_eq!(sealed[sealed.len() - 2..], [12, 4]);

        let mut decryptor = Decryptor::default();
        assert_eq!(
            decryptor.decrypt(FrameKind::Audio, &sealed),
            Err(FrameError::NoKey)
        );
        decryptor.set_key(&[8; KEY_LEN], 4).unwrap();
        assert_eq!(
            decryptor.decrypt(FrameKind::Audio, &sealed),
            Err(FrameError::Rejected),
            "a different key never opens it"
        );
        decryptor.set_key(&[9; KEY_LEN], 4).unwrap();
        let mut tampered = sealed.clone();
        tampered[5] ^= 1;
        assert_eq!(
            decryptor.decrypt(FrameKind::Audio, &tampered),
            Err(FrameError::Rejected)
        );
        tampered = sealed.clone();
        tampered[0] ^= 1;
        assert_eq!(
            decryptor.decrypt(FrameKind::Audio, &tampered),
            Err(FrameError::Rejected),
            "the clear header is authenticated"
        );
        assert_eq!(
            decryptor.decrypt(FrameKind::Audio, &sealed).unwrap(),
            vec![7; 30]
        );
        for short in 0..=(1 + TAG_LEN + IV_LEN + TRAILER_LEN - 1) {
            assert_eq!(
                decryptor.decrypt(FrameKind::Audio, &sealed[..short]),
                Err(FrameError::Malformed)
            );
        }
    }

    #[test]
    fn every_frame_gets_a_fresh_iv_per_stream() {
        let mut encryptor = Encryptor::default();
        encryptor.set_key(&[1; KEY_LEN], 0).unwrap();
        let mut seen = std::collections::HashSet::new();
        for step in 0..500_u32 {
            for ssrc in [10_u32, 11] {
                let sealed = encryptor
                    .encrypt(ssrc, step / 3, FrameKind::VideoDelta, &[0; 20])
                    .unwrap();
                assert!(seen.insert(sealed[sealed.len() - 14..sealed.len() - 2].to_vec()));
            }
        }
    }
}
