//! Private, versioned native IPC. No wire text is Debug/displayed.
use std::fmt;
use std::io::{self, Read, Write};

pub const RECORD_MAX: usize = 8192;
pub const HEADER: usize = 88;
pub const PAYLOAD_MAX: usize = RECORD_MAX - HEADER;
pub const SDP_MAX: usize = 65536;
pub const CHUNK_MAX: usize = PAYLOAD_MAX - 9;
pub const CANDIDATE_MAX: usize = 2048;

/// Random local wire identities, bound by the supervisor to exactly one owner.
/// Account, host, runtime and call each occupy 16 bytes; none is a server token.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct Scope(pub [u8; 64]);

impl fmt::Debug for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MediaScope([local identities])")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum Kind {
    Create = 1,
    Offer = 2,
    Description = 3,
    Candidate = 4,
    Stop = 5,
    Stats = 6,
    DiagnosticStall = 7,
    Ready = 100,
    LocalDescription = 101,
    LocalCandidate = 102,
    Connected = 103,
    Counters = 104,
    Drained = 105,
    Failed = 106,
}

impl Kind {
    fn parse(value: u16) -> io::Result<Self> {
        Ok(match value {
            1 => Self::Create,
            2 => Self::Offer,
            3 => Self::Description,
            4 => Self::Candidate,
            5 => Self::Stop,
            6 => Self::Stats,
            7 => Self::DiagnosticStall,
            100 => Self::Ready,
            101 => Self::LocalDescription,
            102 => Self::LocalCandidate,
            103 => Self::Connected,
            104 => Self::Counters,
            105 => Self::Drained,
            106 => Self::Failed,
            _ => return Err(invalid()),
        })
    }
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid native media record")
}

pub struct Record {
    pub kind: Kind,
    pub scope: Scope,
    pub operation: u64,
    pub serial: u64,
    payload: Box<[u8]>,
}

impl fmt::Debug for Record {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MediaRecord")
            .field("kind", &self.kind)
            .field("operation", &self.operation)
            .field("serial", &self.serial)
            .field("bytes", &self.payload.len())
            .finish()
    }
}

impl Record {
    pub fn new(
        kind: Kind,
        scope: Scope,
        operation: u64,
        serial: u64,
        payload: &[u8],
    ) -> io::Result<Self> {
        if payload.len() > PAYLOAD_MAX || operation == 0 || serial == 0 {
            return Err(invalid());
        }
        Ok(Self {
            kind,
            scope,
            operation,
            serial,
            payload: payload.into(),
        })
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    pub fn read(reader: &mut impl Read) -> io::Result<Self> {
        let mut size = [0; 4];
        reader.read_exact(&mut size)?;
        let size = u32::from_be_bytes(size) as usize;
        // Check the untrusted size before allocating or reading the body.
        if !(HEADER..=RECORD_MAX).contains(&size) {
            return Err(invalid());
        }
        let mut bytes = [0; RECORD_MAX];
        reader.read_exact(&mut bytes[..size])?;
        if &bytes[..4] != b"NMTM" || bytes[4..6] != [0, 2] {
            return Err(invalid());
        }
        let kind = Kind::parse(u16::from_be_bytes([bytes[6], bytes[7]]))?;
        let mut scope = [0; 64];
        scope.copy_from_slice(&bytes[8..72]);
        let operation = u64::from_be_bytes(bytes[72..80].try_into().map_err(|_| invalid())?);
        let serial = u64::from_be_bytes(bytes[80..88].try_into().map_err(|_| invalid())?);
        Self::new(kind, Scope(scope), operation, serial, &bytes[HEADER..size])
    }

    pub fn write(&self, writer: &mut impl Write) -> io::Result<()> {
        let mut bytes = [0; RECORD_MAX + 4];
        let len = HEADER + self.payload.len();
        bytes[..4].copy_from_slice(&(len as u32).to_be_bytes());
        bytes[4..8].copy_from_slice(b"NMTM");
        bytes[8..10].copy_from_slice(&2u16.to_be_bytes());
        bytes[10..12].copy_from_slice(&(self.kind as u16).to_be_bytes());
        bytes[12..76].copy_from_slice(&self.scope.0);
        bytes[76..84].copy_from_slice(&self.operation.to_be_bytes());
        bytes[84..92].copy_from_slice(&self.serial.to_be_bytes());
        bytes[92..len + 4].copy_from_slice(&self.payload);
        writer.write_all(&bytes[..len + 4])
    }
}

pub use nextcloud_talk_api::signaling::DescriptionKind;
pub struct SessionDescription(nextcloud_talk_api::signaling::SessionDescription);
impl fmt::Debug for SessionDescription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl SessionDescription {
    pub fn validate(text: &str) -> io::Result<()> {
        nextcloud_talk_api::signaling::SessionDescription::validate(text)
    }
    pub fn new(kind: DescriptionKind, text: &str) -> io::Result<Self> {
        nextcloud_talk_api::signaling::SessionDescription::new(kind, text).map(Self)
    }
    pub fn kind(&self) -> DescriptionKind {
        self.0.kind()
    }
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
    pub fn chunks(&self) -> impl Iterator<Item = (usize, &[u8])> {
        self.as_str()
            .as_bytes()
            .chunks(CHUNK_MAX)
            .enumerate()
            .map(|(i, b)| (i * CHUNK_MAX, b))
    }
}
impl From<nextcloud_talk_api::signaling::SessionDescription> for SessionDescription {
    fn from(value: nextcloud_talk_api::signaling::SessionDescription) -> Self {
        Self(value)
    }
}
impl From<SessionDescription> for nextcloud_talk_api::signaling::SessionDescription {
    fn from(value: SessionDescription) -> Self {
        value.0
    }
}

/// One in-flight SDP, strict offsets and correlation; no unbounded fragment map.
pub struct DescriptionAssembler {
    storage: Option<Box<[u8; SDP_MAX]>>,
    received: usize,
    total: usize,
    kind: DescriptionKind,
    operation: u64,
}

impl Default for DescriptionAssembler {
    fn default() -> Self {
        Self {
            storage: None,
            received: 0,
            total: 0,
            kind: DescriptionKind::Offer,
            operation: 0,
        }
    }
}

impl DescriptionAssembler {
    pub fn push(&mut self, record: &Record) -> io::Result<Option<SessionDescription>> {
        let p = record.payload();
        if p.len() <= 9 {
            return Err(invalid());
        }
        let total = u32::from_be_bytes(p[..4].try_into().map_err(|_| invalid())?) as usize;
        let offset = u32::from_be_bytes(p[4..8].try_into().map_err(|_| invalid())?) as usize;
        let kind = match p[8] {
            1 => DescriptionKind::Offer,
            2 => DescriptionKind::Answer,
            _ => return Err(invalid()),
        };
        if total == 0
            || total > SDP_MAX
            || offset
                .checked_add(p.len() - 9)
                .is_none_or(|end| end > total)
        {
            return Err(invalid());
        }
        if self.storage.is_none() {
            if offset != 0 {
                return Err(invalid());
            }
            self.storage = Some(Box::new([0; SDP_MAX]));
            self.total = total;
            self.received = 0;
            self.operation = record.operation;
            self.kind = kind;
        }
        if offset != self.received
            || total != self.total
            || record.operation != self.operation
            || kind != self.kind
        {
            return Err(invalid());
        }
        let storage = self.storage.as_mut().ok_or_else(invalid)?;
        storage[offset..offset + p.len() - 9].copy_from_slice(&p[9..]);
        self.received += p.len() - 9;
        if self.received != self.total {
            return Ok(None);
        }
        let text = std::str::from_utf8(&storage[..self.total]).map_err(|_| invalid())?;
        let result = SessionDescription::new(kind, text)?;
        self.storage = None;
        Ok(Some(result))
    }
}

pub struct IceCandidate(nextcloud_talk_api::signaling::IceCandidate);
impl fmt::Debug for IceCandidate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl IceCandidate {
    pub fn validate(mid: &str, index: u8, candidate: &str) -> io::Result<()> {
        nextcloud_talk_api::signaling::IceCandidate::validate(mid, index, candidate)
    }
    pub fn new(mid: &str, index: u8, candidate: &str) -> io::Result<Self> {
        nextcloud_talk_api::signaling::IceCandidate::new(mid, index, candidate).map(Self)
    }
    pub fn mid(&self) -> &str {
        self.0.mid()
    }
    pub fn index(&self) -> u8 {
        self.0.index()
    }
    pub fn candidate(&self) -> &str {
        self.0.candidate()
    }
    pub fn encode(&self) -> Box<[u8]> {
        let mut p = [0; CANDIDATE_MAX + 66];
        p[0] = self.mid().len() as u8;
        p[1] = self.index();
        p[2..2 + self.mid().len()].copy_from_slice(self.mid().as_bytes());
        p[2 + self.mid().len()..2 + self.mid().len() + self.candidate().len()]
            .copy_from_slice(self.candidate().as_bytes());
        p[..2 + self.mid().len() + self.candidate().len()].into()
    }
    pub fn decode(p: &[u8]) -> io::Result<Self> {
        if p.len() < 3 || p.len() <= 2 + p[0] as usize {
            return Err(invalid());
        }
        let mid_end = 2 + p[0] as usize;
        Self::new(
            std::str::from_utf8(&p[2..mid_end]).map_err(|_| invalid())?,
            p[1],
            std::str::from_utf8(&p[mid_end..]).map_err(|_| invalid())?,
        )
    }
}

impl From<nextcloud_talk_api::signaling::IceCandidate> for IceCandidate {
    fn from(value: nextcloud_talk_api::signaling::IceCandidate) -> Self {
        Self(value)
    }
}
impl From<IceCandidate> for nextcloud_talk_api::signaling::IceCandidate {
    fn from(value: IceCandidate) -> Self {
        value.0
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn framing_rejects_before_body_read_and_handles_fragmentation() {
        let mut bad = &u32::MAX.to_be_bytes()[..];
        assert_eq!(
            Record::read(&mut bad).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        let r = Record::new(Kind::Create, Scope([9; 64]), 1, 1, &[3]).unwrap();
        let mut bytes = Vec::new();
        r.write(&mut bytes).unwrap();
        let decoded = Record::read(&mut &bytes[..]).unwrap();
        assert_eq!(decoded.scope, r.scope);
        assert_eq!(decoded.payload(), &[3]);
        bytes[8] = 1;
        assert!(Record::read(&mut &bytes[..]).is_err());
    }
    #[test]
    fn sdp_chunk_reordering_cannot_replace_live_description() {
        let mut a = DescriptionAssembler::default();
        let mut p = [0u8; 14];
        p[..4].copy_from_slice(&10u32.to_be_bytes());
        p[8] = 1;
        p[9..].copy_from_slice(b"v=0\r\n");
        let r = Record::new(Kind::Description, Scope([1; 64]), 2, 1, &p).unwrap();
        assert!(a.push(&r).unwrap().is_none());
        assert!(a.push(&r).is_err());
        let wrong_op = Record::new(Kind::Description, Scope([1; 64]), 3, 2, &p).unwrap();
        assert!(a.push(&wrong_op).is_err());
    }
    #[test]
    fn signals_do_not_leak_in_debug() {
        let c =
            IceCandidate::new("video", 0, "candidate:CANARY 1 UDP 1 127.0.0.1 9 typ host").unwrap();
        assert!(!format!("{c:?}").contains("CANARY"));
        assert_eq!(
            IceCandidate::decode(&c.encode()).unwrap().candidate(),
            c.candidate()
        );
        assert!(IceCandidate::new("v", 24, c.candidate()).is_err());
    }
}
