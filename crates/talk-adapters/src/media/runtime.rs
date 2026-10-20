//! Adapter-private synthetic media owner; no credentials, server IDs or UI DTOs.
//! Startup is blocking on a bounded owner worker. All subsequent admission and
//! pumping is nonblocking. Production authority/device admission is absent.
use super::supervisor::{DiagnosticSession, Error as WorkerError, Exit, StopHandle};
use super::wire::{
    self, DescriptionAssembler, IceCandidate, Kind, Record, Scope, SessionDescription,
};
use std::path::Path;
use std::time::Duration;
use talk_core::ids::{AccountId, CallGeneration, CallScope, HostId, RuntimeGeneration};
use talk_core::lifecycle::Cancellation;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    WrongScope,
    Cancelled,
    Busy,
    Stopped,
    Unsupported,
    Malformed,
    Worker(WorkerError),
}

/// This explicit fixture intent is never a device/policy capability or a grant
/// to join Talk. There is deliberately no constructor for production capture.
#[derive(Clone, Copy, Debug)]
pub enum SyntheticIntent {
    ObserveOnly,
    Audio,
    Video,
    AudioVideo,
}
impl SyntheticIntent {
    fn bits(self) -> u8 {
        match self {
            Self::ObserveOnly => 0,
            Self::Audio => 1,
            Self::Video => 2,
            Self::AudioVideo => 3,
        }
    }
}

pub struct SyntheticConfig {
    call: CallScope,
    wire: Scope,
    cancellation: Cancellation,
    intent: SyntheticIntent,
}
impl SyntheticConfig {
    pub fn new(
        call: CallScope,
        wire: Scope,
        cancellation: Cancellation,
        intent: SyntheticIntent,
    ) -> Result<Self, Error> {
        let id = |offset| {
            u128::from_be_bytes(
                wire.0[offset..offset + 16]
                    .try_into()
                    .expect("fixed scope width"),
            )
        };
        if !call.is_consistent()
            || call.account.account != AccountId::from_local(id(0))
            || call.conversation.host != HostId::from_local(id(16))
            || call.account.runtime != RuntimeGeneration::from_local(id(32))
            || call.call != CallGeneration::from_local(id(48))
        {
            return Err(Error::WrongScope);
        }
        if cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        Ok(Self {
            call,
            wire,
            cancellation,
            intent,
        })
    }
}

pub enum Intent {
    Offer,
    RemoteDescription(SessionDescription),
    RemoteCandidate(IceCandidate),
    Inspect,
}
/// A Busy result proves non-admission and returns the original bounded intent;
/// it is safe for its owning reactor to retain it, without replaying HTTP.
pub struct Rejected {
    pub reason: Error,
    pub intent: Intent,
}

pub enum Observation {
    Ready,
    Connected,
    LocalDescription(SessionDescription),
    LocalCandidate(IceCandidate),
    Counters([u64; 18]),
    Drained,
    Failed(u8),
}
/// Surfaces do not yet exist; frames/pointers never enter renderer snapshots.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceAvailability {
    Unavailable,
}

pub struct Control {
    call: CallScope,
    stop: StopHandle,
}
impl Control {
    pub fn close(&self, call: CallScope) -> Result<(), Error> {
        if call != self.call {
            return Err(Error::WrongScope);
        }
        self.stop.close();
        Ok(())
    }
}

struct PendingDescription {
    value: SessionDescription,
    offset: usize,
    operation: u64,
}
/// One helper owner. No additional event/command queue: the supervisor owns the
/// two16-record channels. This layer holds at most one64KiB outgoing description
/// and one64KiB incoming assembler (plus its at-most64KiB completed conversion).
/// Each advance submits at most4 chunks and consumes at most4 events.
pub struct SyntheticMedia {
    config: SyntheticConfig,
    session: DiagnosticSession,
    pending: Option<PendingDescription>,
    assembler: DescriptionAssembler,
    serial: u64,
    operation: u64,
    commands: u16,
    offers: u8,
    descriptions_in: u8,
    candidates_in: u8,
    descriptions_out: u8,
    candidates_out: u8,
    closed: bool,
}
impl SyntheticMedia {
    pub fn start(helper: &Path, config: SyntheticConfig) -> Result<Self, Error> {
        if config.cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let create = Record::new(Kind::Create, config.wire, 1, 1, &[config.intent.bits()])
            .map_err(|_| Error::Malformed)?;
        let session = DiagnosticSession::start(helper, create).map_err(Error::Worker)?;
        Ok(Self {
            config,
            session,
            pending: None,
            assembler: DescriptionAssembler::default(),
            serial: 1,
            operation: 1,
            commands: 1,
            offers: 0,
            descriptions_in: 0,
            candidates_in: 0,
            descriptions_out: 0,
            candidates_out: 0,
            closed: false,
        })
    }
    pub fn control(&self) -> Control {
        Control {
            call: self.config.call,
            stop: self.session.stop_handle(),
        }
    }
    pub fn surface(&self) -> SurfaceAvailability {
        SurfaceAvailability::Unavailable
    }
    pub fn outcome(&self) -> Exit {
        self.session.outcome()
    }
    pub fn close(&mut self) {
        self.closed = true;
        self.pending = None;
        self.assembler = DescriptionAssembler::default();
        self.session.revoke();
    }
    fn check(&mut self, call: CallScope) -> Result<(), Error> {
        if call != self.config.call {
            return Err(Error::WrongScope);
        }
        if self.config.cancellation.is_cancelled() {
            self.close();
            return Err(Error::Cancelled);
        }
        if self.closed || self.session.is_revoked() || self.session.outcome() != Exit::Running {
            self.close();
            return Err(Error::Stopped);
        }
        Ok(())
    }
    fn command(&mut self, kind: Kind, payload: &[u8], operation: u64) -> Result<(), Error> {
        if self.commands >= 512 {
            self.close();
            return Err(Error::Unsupported);
        }
        let record = Record::new(kind, self.config.wire, operation, self.serial + 1, payload)
            .map_err(|_| Error::Malformed)?;
        match self.session.submit(record) {
            Ok(()) => {
                self.serial += 1;
                self.commands += 1;
                self.operation = operation;
                Ok(())
            }
            Err(WorkerError::Busy) => Err(Error::Busy),
            Err(error) => {
                self.close();
                Err(Error::Worker(error))
            }
        }
    }
    pub fn submit(&mut self, call: CallScope, intent: Intent) -> Result<(), Rejected> {
        if let Err(reason) = self.check(call) {
            return Err(Rejected { reason, intent });
        }
        if self.pending.is_some() {
            return Err(Rejected {
                reason: Error::Busy,
                intent,
            });
        }
        let result = match &intent {
            Intent::Offer if self.offers < 2 => self.command(Kind::Offer, &[], self.operation + 1),
            Intent::RemoteDescription(_) if self.descriptions_in < 2 => Ok(()),
            Intent::RemoteCandidate(candidate) if self.candidates_in < 64 => {
                self.command(Kind::Candidate, &candidate.encode(), self.operation + 1)
            }
            Intent::Inspect => self.command(Kind::Stats, &[], self.operation + 1),
            _ => Err(Error::Unsupported),
        };
        if let Err(reason) = result {
            return Err(Rejected { reason, intent });
        }
        match intent {
            Intent::Offer => self.offers += 1,
            Intent::RemoteDescription(value) => {
                self.operation += 1;
                self.descriptions_in += 1;
                self.pending = Some(PendingDescription {
                    value,
                    offset: 0,
                    operation: self.operation,
                });
            }
            Intent::RemoteCandidate(_) => self.candidates_in += 1,
            Intent::Inspect => {}
        }
        Ok(())
    }
    fn pump(&mut self) -> Result<(), Error> {
        for _ in 0..4 {
            let Some(mut pending) = self.pending.take() else {
                return Ok(());
            };
            let bytes = pending.value.as_str().as_bytes();
            let end = (pending.offset + wire::CHUNK_MAX).min(bytes.len());
            let mut payload = [0; wire::PAYLOAD_MAX];
            payload[..4].copy_from_slice(&(bytes.len() as u32).to_be_bytes());
            payload[4..8].copy_from_slice(&(pending.offset as u32).to_be_bytes());
            payload[8] = pending.value.kind() as u8;
            payload[9..9 + end - pending.offset].copy_from_slice(&bytes[pending.offset..end]);
            match self.command(
                Kind::Description,
                &payload[..9 + end - pending.offset],
                pending.operation,
            ) {
                Ok(()) => {
                    pending.offset = end;
                    if end < bytes.len() {
                        self.pending = Some(pending);
                    }
                }
                Err(Error::Busy) => {
                    self.pending = Some(pending);
                    return Ok(());
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
    pub fn advance(&mut self, call: CallScope) -> Result<Option<Observation>, Error> {
        self.check(call)?;
        if let Err(error) = self.pump() {
            self.close();
            return Err(error);
        }
        for _ in 0..4 {
            let Some(event) = self.session.receive(Duration::ZERO) else {
                return Ok(None);
            };
            let observation = match event.kind {
                Kind::Ready => Some(Observation::Ready),
                Kind::Connected => Some(Observation::Connected),
                Kind::LocalDescription => match self.assembler.push(&event) {
                    Ok(Some(value)) if self.descriptions_out < 2 => {
                        self.descriptions_out += 1;
                        Some(Observation::LocalDescription(value))
                    }
                    Ok(None) => None,
                    _ => {
                        self.close();
                        return Err(Error::Malformed);
                    }
                },
                Kind::LocalCandidate if self.candidates_out < 64 => {
                    match IceCandidate::decode(event.payload()) {
                        Ok(value) => {
                            self.candidates_out += 1;
                            Some(Observation::LocalCandidate(value))
                        }
                        Err(_) => {
                            self.close();
                            return Err(Error::Malformed);
                        }
                    }
                }
                Kind::Counters if event.payload().len() == 144 => {
                    let mut values = [0; 18];
                    for (i, value) in values.iter_mut().enumerate() {
                        *value = u64::from_be_bytes(
                            event.payload()[i * 8..i * 8 + 8]
                                .try_into()
                                .expect("checked counters"),
                        );
                    }
                    Some(Observation::Counters(values))
                }
                Kind::Drained => {
                    self.close();
                    Some(Observation::Drained)
                }
                Kind::Failed if event.payload().len() == 1 => {
                    let reason = event.payload()[0];
                    self.close();
                    Some(Observation::Failed(reason))
                }
                _ => {
                    self.close();
                    return Err(Error::Malformed);
                }
            };
            if observation.is_some() {
                return Ok(observation);
            }
        }
        Ok(None)
    }
}
impl Drop for SyntheticMedia {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::ids::{AccountScope, ConversationId};
    #[test]
    fn config_binds_every_scope_before_worker_allocation() {
        let wire = Scope([3; 64]);
        let id = u128::from_be_bytes([3; 16]);
        let call = CallScope {
            account: AccountScope {
                account: AccountId::from_local(id),
                runtime: RuntimeGeneration::from_local(id),
            },
            conversation: ConversationId {
                account: AccountId::from_local(id),
                host: HostId::from_local(id),
                local: 4,
            },
            call: CallGeneration::from_local(id),
        };
        assert!(
            SyntheticConfig::new(
                call,
                wire,
                Cancellation::default(),
                SyntheticIntent::ObserveOnly
            )
            .is_ok()
        );
        for offset in [0, 16, 32, 48] {
            let mut stale = wire;
            stale.0[offset] ^= 1;
            assert!(matches!(
                SyntheticConfig::new(
                    call,
                    stale,
                    Cancellation::default(),
                    SyntheticIntent::AudioVideo
                ),
                Err(Error::WrongScope)
            ));
        }
        let cancellation = Cancellation::default();
        cancellation.cancel();
        assert!(matches!(
            SyntheticConfig::new(call, wire, cancellation, SyntheticIntent::AudioVideo),
            Err(Error::Cancelled)
        ));
    }
}
