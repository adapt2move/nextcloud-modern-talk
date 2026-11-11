//! Autonomous normal-call owner. Signaling polling, policy/HTTP writes and media
//! have separate bounded lanes; none is driven by GPUI rendering or chat polling.
mod activity;
mod moderation;
mod outgoing;
mod speaking;
mod status;

use super::{
    call_policy::{Bundle, Sources},
    transport::NetworkRuntime,
};
use crate::media::engine::{
    Error as EngineError, IceServer, PeerConnectionState, Role, RtcConfiguration,
};
use crate::media::{
    MediaService,
    engine::{Event as EngineEvent, NativePeer},
    service::Session,
};
use nextcloud_talk_api::{
    self as api,
    meeting::{
        MeetingReader, MeetingStop, MeetingWriter, Observation, ObservationError, Topology,
        WireResolver,
    },
    models::ParticipantFlags,
    signaling::{
        DescriptionKind, Event as SignalEvent, Negotiation, PeerSession, Signal, SignalBatch,
        StreamKind,
    },
};
use status::StatusSnapshot;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use talk_core::{
    calls::{
        self, CallSession, Command, DevicePermissions, DeviceState, MediaCounters, MediaWishes,
        NativeSupport, Phase, Snapshot,
    },
    ids::CallScope,
    lifecycle::Cancellation,
    policy::{Observed, RecordingState},
};
use tokio::sync::{mpsc, watch};
mod e2ee;
const OBSERVATIONS: [Observation; 6] = [
    Observation::Account,
    Observation::Capabilities,
    Observation::Room,
    Observation::ConnectedParticipants,
    Observation::CallParticipants,
    Observation::SignalingSettings,
];
pub(crate) const SUPPORT: NativeSupport = NativeSupport {
    audio: true,
    video: true,
    screen: cfg!(any(target_os = "linux", target_os = "macos")),
    external_signaling: true,
    end_to_end_encryption: true,
};
pub(crate) const DEVICES: DevicePermissions = DevicePermissions {
    microphone: true,
    camera: true,
    screen_session: cfg!(any(target_os = "linux", target_os = "macos")),
};

enum Write {
    Subscribe(PeerSession, StreamKind),
    SendStreamOffer(PeerSession, StreamKind),
    RestartSubscription(PeerSession, StreamKind),
    Refresh(u64),
    Select(PeerSession, Negotiation, StreamKind),
    Remove(PeerSession, StreamKind),
    Signal(PeerSession, Negotiation, Signal, StreamKind),
    Flags(MediaWishes),
}
impl Write {
    /// A fixed label for the local diagnostic log.
    fn name(&self) -> &'static str {
        match self {
            Self::Subscribe(..) => "subscribe",
            Self::SendStreamOffer(..) => "stream_offer",
            Self::RestartSubscription(..) => "restart_subscription",
            Self::Refresh(_) => "refresh",
            Self::Select(..) => "select",
            Self::Remove(..) => "remove",
            Self::Signal(..) => "signal",
            Self::Flags(_) => "flags",
        }
    }
}
struct PolicyUpdate {
    bundle: Bundle,
    after: u64,
}
struct RecordingPending {
    start: bool,
    requested: Instant,
    accepted: bool,
}
struct Peer {
    stream: StreamKind,
    dimensions: Option<(u32, u32)>,
    last_frame: Option<Instant>,
    local: u128,
    status: StatusSnapshot,
    speaking: speaking::Speaking,
    session: PeerSession,
    negotiation: Option<Negotiation>,
    engine: NativePeer,
    remote: bool,
    candidates: Vec<api::signaling::IceCandidate>,
    connected: bool,
    role: Role,
    subscription_requested: bool,
    recovery: Option<Instant>,
    restart_pending: bool,
    restarts: u8,
    screen_announced: Vec<PeerSession>,
    share_epoch: u64,
}
fn report_ice_support(engine: &NativePeer) {
    let omitted = &engine.ice_support().unsupported;
    if !omitted.is_empty() {
        // Once per bounded peer construction; categories contain no addresses,
        // credentials or remote error text. The usable UDP plan stays intact.
        eprintln!("meeting ICE unsupported alternatives: {omitted:?}");
    }
}
pub(crate) struct LiveCall {
    scope: CallScope,
    session: Arc<Session>,
    control: mpsc::Sender<(Command, u64)>,
    stop: MeetingStop,
}
impl LiveCall {
    pub(crate) fn session_replaced(&self) -> bool {
        self.stop.session_replaced()
    }
    pub(crate) fn closed(&self) -> bool {
        !self.session.live() || self.stop.is_closed()
    }
    pub(crate) fn close(&self) {
        self.session.close();
        self.stop.close();
    }
    pub(crate) fn snapshot(&self) -> Result<Snapshot, api::ErrorKind> {
        self.session
            .snapshot
            .lock()
            .map(|s| s.clone())
            .map_err(|_| api::ErrorKind::Uncertain)
    }
    pub(crate) fn command(&self, command: Command) -> Result<(), api::ErrorKind> {
        if command.scope() != self.scope {
            return Err(api::ErrorKind::WrongScope);
        }
        if matches!(command, Command::StartScreenShare(_)) && !SUPPORT.screen {
            return Err(api::ErrorKind::Unsupported);
        }
        if matches!(command, Command::Observe(_)) {
            return Ok(());
        }
        if matches!(command, Command::StopScreenShare(_)) {
            self.session.stop_screen();
        }
        if matches!(command, Command::Stop(_)) {
            self.session.close();
            self.stop.close();
            return Ok(());
        }
        // Close publication synchronously on any change; stale callbacks cannot
        // extend the old grant while the owner rechecks policy/user intent.
        match &command {
            Command::Change { wishes, .. } => {
                self.session.narrow(*wishes);
                // A mute intent must survive even if continuity is lost before
                // the media actor can drain this command. Never broaden here.
                if let Ok(mut snapshot) = self.session.snapshot.lock() {
                    snapshot.wishes.microphone &= wishes.microphone;
                    snapshot.wishes.camera &= wishes.camera;
                    snapshot.wishes.screen &= wishes.screen;
                }
            }
            Command::Observe(_) | Command::StartScreenShare(_) | Command::StopScreenShare(_) => {}
            Command::Consent(_) | Command::Reconnect(_) => self.session.suspend(),
            _ => {}
        }
        let end_for_everyone = matches!(command, Command::EndEveryone(_));
        let result = self
            .control
            .try_send((command, self.session.screen_epoch()))
            .map_err(|_| api::ErrorKind::Busy);
        if end_for_everyone && result.is_ok() {
            self.session.suspend();
        }
        result
    }
    #[allow(
        clippy::too_many_arguments,
        reason = "Transfers distinct admitted call, transport lanes and cancellation owners once"
    )]
    pub(crate) fn start(
        service: Arc<MediaService>,
        mut call: CallSession,
        initial: Bundle,
        reader: MeetingReader,
        mut writer: MeetingWriter,
        stop: MeetingStop,
        network: NetworkRuntime,
        cancel: Cancellation,
        account_rejection: Option<super::AccountRejection>,
    ) -> Result<Self, api::ErrorKind> {
        let scope = call.scope();
        let grant = call
            .advance(
                &initial.inputs,
                initial.exists,
                SUPPORT,
                DEVICES,
                network.now(),
            )
            .map_err(|_| api::ErrorKind::PermissionDenied)?;
        call.joined(scope).map_err(|_| api::ErrorKind::Conflict)?;
        let snapshot = Snapshot {
            wishes: call.wishes(),
            call: scope,
            entry: call.entry(),
            participants: talk_core::bounded::Page::empty(),
            phase: Phase::Connecting,
            microphone: DeviceState::Off,
            local_speaking: false,
            camera: DeviceState::Off,
            screen: DeviceState::Unavailable,
            sharing: talk_core::calls::SharingState::Idle,
            peers: 0,
            recording: RecordingState::Unknown,
            recording_control: None,
            recording_pending: false,
            recording_uncertain: false,
            encrypted: false,
            hand_supported: false,
            hand_raised: false,
            reactions: talk_core::bounded::Page::empty(),
            latest_reaction: None,
            invitees_loaded: false,
            invitees: talk_core::bounded::Page::empty(),
            ring_status: None,
            mute_status: None,
            can_end_everyone: false,
            end_pending: false,
            end_failed: false,
            counters: MediaCounters::default(),
            revision: 0,
        };
        let session = service
            .admit(snapshot, cancel.clone())
            .map_err(|_| api::ErrorKind::Busy)?;
        session.grant(grant, network.now());
        let (control_tx, control) = mpsc::channel(8);
        let (signals_tx, signals) = mpsc::channel(2);
        let (write_tx, write_rx) = mpsc::channel(64);
        let (policy_tx, policy) = mpsc::channel(2);
        let (hand_tx, hands) = watch::channel(None);
        let (reaction_tx, reactions) = mpsc::channel(4);
        let (encryption_tx, encryption_rx) = mpsc::channel(64);
        let (ring_tx, ring_requests) = mpsc::channel(1);
        let (ring_result_tx, ring_results) = mpsc::channel(2);
        let moderation_writer = writer
            .take_moderation_writer()
            .ok_or(api::ErrorKind::Uncertain)?;
        tokio::spawn(moderation::run(
            moderation_writer,
            ring_requests,
            ring_result_tx,
            session.clone(),
            network.clone(),
            cancel.clone(),
        ));
        let activity_writer = writer
            .take_activity_writer()
            .ok_or(api::ErrorKind::Uncertain)?;
        tokio::spawn(outgoing::run(
            activity_writer,
            hands,
            reactions,
            encryption_rx,
            session.clone(),
            network.clone(),
            cancel.clone(),
        ));
        let own = writer.own_session()?;
        let topology = writer.topology();
        let wires = writer.wire_resolver();
        tokio::spawn(read_loop(
            reader,
            signals_tx,
            session.clone(),
            network.clone(),
            cancel.clone(),
        ));
        tokio::spawn(write_loop(
            writer,
            write_rx,
            policy_tx,
            session.clone(),
            network.clone(),
            cancel.clone(),
            account_rejection,
        ));
        tokio::spawn(media_loop(
            call,
            initial,
            own,
            control,
            signals,
            write_tx,
            policy,
            hand_tx,
            reaction_tx,
            ring_tx,
            ring_results,
            session.clone(),
            network,
            cancel,
            stop.clone(),
            topology,
            wires,
            encryption_tx,
        ));
        Ok(Self {
            scope,
            session,
            control: control_tx,
            stop,
        })
    }
}
impl Drop for LiveCall {
    fn drop(&mut self) {
        self.session.close();
        self.stop.close();
    }
}
fn context(
    scope: CallScope,
    network: &NetworkRuntime,
    cancel: &Cancellation,
    millis: u64,
) -> api::scope::OperationContext {
    let now = network.now();
    api::scope::OperationContext {
        scope: api::scope::SessionScope {
            account: api::scope::BindingId::from_key(scope.account.account.as_local()),
            runtime: api::scope::LeaseId::from_key(scope.account.runtime.as_local()),
        },
        deadline: api::scope::Moment {
            epoch: api::scope::ClockId::from_key(now.epoch.as_local()),
            millis: now.millis.saturating_add(millis),
        },
        cancellation: api::scope::Cancellation::from_signal(Cancel(cancel.clone())),
    }
}
struct Cancel(Cancellation);
impl api::scope::CancellationSignal for Cancel {
    fn cancel(&self) {
        self.0.cancel();
    }
    fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }
}
async fn read_loop(
    mut reader: MeetingReader,
    events: mpsc::Sender<SignalBatch>,
    session: Arc<Session>,
    network: NetworkRuntime,
    cancel: Cancellation,
) {
    while session.live() {
        let result = reader
            .poll(&context(session.scope, &network, &cancel, 35_000))
            .await;
        match result {
            Ok(batch) => {
                if events.try_send(batch).is_err() {
                    break;
                }
            }
            Err(_error) => {
                #[cfg(feature = "backend-test")]
                eprintln!("normal read stopped: {:?}", _error.kind);
                break;
            }
        }
    }
    session.close();
}
pub(super) struct PolicyLoadFailure {
    pub(super) kind: api::ErrorKind,
    evidence: calls::PolicyEvidence,
}
/// A whole refresh in one call; the live writer runs the same reads one at a
/// time between its signaling writes.
#[cfg(test)]
pub(super) async fn refresh(
    writer: &mut MeetingWriter,
    scope: CallScope,
    network: &NetworkRuntime,
    cancel: &Cancellation,
    account_rejection: Option<&super::AccountRejection>,
) -> Result<Bundle, PolicyLoadFailure> {
    let mut sources = begin_refresh(writer, scope)?;
    for observation in OBSERVATIONS {
        observe(
            writer,
            &mut sources,
            observation,
            scope,
            network,
            cancel,
            account_rejection,
        )
        .await?;
    }
    finish_refresh(sources)
}
fn begin_refresh(writer: &MeetingWriter, scope: CallScope) -> Result<Sources, PolicyLoadFailure> {
    let own = writer.own_session().map_err(|kind| PolicyLoadFailure {
        kind,
        evidence: calls::PolicyEvidence::Participants,
    })?;
    Ok(Sources::new(scope, own))
}
fn finish_refresh(sources: Sources) -> Result<Bundle, PolicyLoadFailure> {
    sources.finish(true).map_err(|evidence| PolicyLoadFailure {
        kind: api::ErrorKind::Unsupported,
        evidence,
    })
}
/// One of the six reads a policy refresh consists of.
async fn observe(
    writer: &mut MeetingWriter,
    sources: &mut Sources,
    observation: Observation,
    scope: CallScope,
    network: &NetworkRuntime,
    cancel: &Cancellation,
    account_rejection: Option<&super::AccountRejection>,
) -> Result<(), PolicyLoadFailure> {
    let value = writer
        .observe(&context(scope, network, cancel, 3000), observation)
        .await
        .map_err(|e| {
            let kind = e.kind();
            if matches!(observation, Observation::Account)
                && (kind == api::ErrorKind::ReauthenticationRequired
                    || e == ObservationError::CanonicalIdentityMismatch)
            {
                // Only the authoritative user probe can confirm rejection
                // or canonical identity loss; other WrongScope errors cannot.
                // Close local admission immediately; durable suppression is
                // owned by identity and must not depend on another UI poll.
                if let Some(report) = account_rejection {
                    report(scope.account);
                }
                cancel.cancel();
            }
            #[cfg(feature = "backend-test")]
            {
                let source = match observation {
                    Observation::Account => "account",
                    Observation::Capabilities => "capabilities",
                    Observation::Room => "room",
                    Observation::ConnectedParticipants => "participants",
                    Observation::CallParticipants => "call participants",
                    Observation::SignalingSettings => "signaling settings",
                };
                eprintln!("normal observation stopped: {source} {kind:?}");
            }
            PolicyLoadFailure {
                kind,
                evidence: super::call_policy::evidence(observation),
            }
        })?;
    sources.add(observation, value);
    Ok(())
}
/// What a failed policy refresh does to a live call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RefreshFailure {
    /// A short network hiccup while the last qualified refresh still holds: keep
    /// the call as it is and try again.
    RetryQuietly,
    /// The last refresh is too old to cover the gap: show the policy gate, keep trying.
    RetryAndGate,
    /// A rejection, or too many consecutive failures: stop and show the gate.
    Stop,
}
/// The policy lease is valid 13 s from the request start (see `talk_core::policy`).
/// Staying quiet for 10 s therefore never outlasts the evidence it relies on.
const QUIET_REFRESH_WINDOW: Duration = Duration::from_secs(10);
const MAX_REFRESH_FAILURES: u8 = 40;
const REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const REFRESH_RETRY: Duration = Duration::from_millis(250);
/// A burst of membership events asks for one extra refresh, not one each.
const REFRESH_HINT_SPACING: Duration = Duration::from_millis(500);
/// Failures that say nothing about the call: the network, a slow server, or
/// all local request slots in use by other work at that moment.
fn transient_refresh_error(kind: api::ErrorKind) -> bool {
    matches!(
        kind,
        api::ErrorKind::Transport
            | api::ErrorKind::Deadline
            | api::ErrorKind::ServerUnavailable
            | api::ErrorKind::RateLimited
            | api::ErrorKind::Busy
    )
}
fn refresh_failure(transient: bool, failures: u8, since_good: Duration) -> RefreshFailure {
    if !transient || failures >= MAX_REFRESH_FAILURES {
        RefreshFailure::Stop
    } else if since_good < QUIET_REFRESH_WINDOW {
        RefreshFailure::RetryQuietly
    } else {
        RefreshFailure::RetryAndGate
    }
}
async fn write_loop(
    mut writer: MeetingWriter,
    mut commands: mpsc::Receiver<Write>,
    policy: mpsc::Sender<PolicyUpdate>,
    session: Arc<Session>,
    network: NetworkRuntime,
    cancel: Cancellation,
    account_rejection: Option<super::AccountRejection>,
) {
    let scope = session.scope;
    let mut refresh_at = Instant::now();
    let mut requested = 0;
    let mut refresh_failures = 0u8;
    let mut last_good = Instant::now();
    // A refresh in progress: its reads so far, the next one, when it started
    // and the request sequence it answers.
    let mut running: Option<(Sources, usize, Instant, u64)> = None;
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    while session.live() {
        // Signaling writes go out between the reads of a refresh instead of
        // waiting for all six: candidates and device flags are not held back.
        if running.is_none() {
            tick.tick().await;
        }
        for _ in 0..16 {
            let Ok(command) = commands.try_recv() else {
                break;
            };
            let name = command.name();
            let result = match command {
                Write::Refresh(after) => {
                    requested = requested.max(after);
                    refresh_at = Instant::now();
                    Ok(())
                }
                Write::RestartSubscription(peer, stream) => {
                    writer
                        .restart_subscription(
                            &context(scope, &network, &cancel, 3000),
                            &peer,
                            stream,
                        )
                        .await
                }
                Write::SendStreamOffer(peer, stream) => {
                    writer
                        .send_stream_offer(&context(scope, &network, &cancel, 3000), &peer, stream)
                        .await
                }
                Write::Subscribe(peer, stream) => {
                    writer
                        .request_stream_offer(
                            &context(scope, &network, &cancel, 3000),
                            &peer,
                            stream,
                        )
                        .await
                }
                Write::Select(peer, sid, stream) => writer.select_stream(peer, sid, stream),
                Write::Remove(peer, stream) => {
                    writer.remove_stream(&peer, stream);
                    Ok(())
                }
                Write::Signal(peer, sid, signal, stream) => writer
                    .send_stream(
                        &context(scope, &network, &cancel, 3000),
                        &peer,
                        &sid,
                        signal,
                        stream,
                    )
                    .await
                    .map(|_| ()),
                Write::Flags(w) => {
                    writer
                        .flags(
                            &context(scope, &network, &cancel, 3000),
                            ParticipantFlags::new(w.microphone, w.camera),
                        )
                        .await
                }
            };
            if let Err(_error) = result {
                crate::media::note_call_failure(Some((name, super::transport::kind(_error.kind))));
                crate::media::note_call_stop("signaling write failed");
                #[cfg(feature = "backend-test")]
                eprintln!("normal write stopped: {:?}", _error.kind);
                session.close();
                break;
            }
        }
        if running.is_none() && Instant::now() >= refresh_at {
            match begin_refresh(&writer, scope) {
                Ok(sources) => running = Some((sources, 0, Instant::now(), requested)),
                Err(error) => {
                    crate::media::note_call_failure(Some((
                        error.evidence.name(),
                        super::transport::kind(error.kind),
                    )));
                    crate::media::note_call_stop(error.evidence.name());
                    break;
                }
            }
        }
        let Some((mut sources, next, refresh_started, after)) = running.take() else {
            continue;
        };
        let step = observe(
            &mut writer,
            &mut sources,
            OBSERVATIONS[next],
            scope,
            &network,
            &cancel,
            account_rejection.as_ref(),
        )
        .await;
        let result = match step {
            Ok(()) if next + 1 < OBSERVATIONS.len() => {
                running = Some((sources, next + 1, refresh_started, after));
                continue;
            }
            Ok(()) => finish_refresh(sources),
            Err(error) => Err(error),
        };
        match result {
            Ok(bundle) => {
                refresh_failures = 0;
                last_good = refresh_started;
                crate::media::note_call_failure(None);
                session.set_control_degraded(false);
                if policy.try_send(PolicyUpdate { bundle, after }).is_err() {
                    break;
                }
                // Measured from the start: a slow round must not also delay the
                // next one, or a single failed request outlasts the lease. A
                // refresh asked for meanwhile follows at once.
                if requested == after && refresh_at <= refresh_started {
                    refresh_at = (refresh_started + REFRESH_INTERVAL)
                        .max(Instant::now() + Duration::from_secs(1));
                }
            }
            Err(error) => {
                #[cfg(feature = "backend-test")]
                eprintln!(
                    "normal policy refresh stopped: {:?} {}",
                    error.kind,
                    error.evidence.name()
                );
                session.set_control_degraded(true);
                crate::media::note_call_failure(Some((
                    error.evidence.name(),
                    super::transport::kind(error.kind),
                )));
                let transient = transient_refresh_error(error.kind);
                let action = refresh_failure(transient, refresh_failures, last_good.elapsed());
                if action != RefreshFailure::RetryQuietly
                    && let Ok(mut snapshot) = session.snapshot.lock()
                {
                    snapshot.phase = Phase::Waiting(calls::Gate::PolicyUnavailable(error.evidence));
                }
                if action == RefreshFailure::Stop {
                    crate::media::note_call_stop(error.evidence.name());
                    break;
                }
                refresh_failures += 1;
                // No success or sliding grant is emitted on failure.
                // Existing publication still expires at its original lease.
                refresh_at = Instant::now() + REFRESH_RETRY;
            }
        }
    }
    session.close();
    let _ = writer.leave(&context(scope, &network, &cancel, 3000)).await;
}
/// Peer connection settings. With call end-to-end encryption the connection gets
/// the key holders for `peer` (a connection never exists without them then).
fn configuration(
    bundle: &Bundle,
    e2ee: Option<&mut e2ee::Call>,
    peer: &PeerSession,
) -> RtcConfiguration {
    RtcConfiguration {
        e2ee: e2ee.map(|call| call.keys(peer)),
        ice_servers: bundle
            .settings
            .ice_servers
            .iter()
            .map(|s| IceServer {
                urls: s.urls.clone(),
                username: s.username.to_string(),
                password: s.credential.to_string(),
            })
            .collect(),
        ..Default::default()
    }
}
/// Whether this call must be end-to-end encrypted (Talk's `isEnabled`: advertised
/// and not explicitly disabled). Admission only reaches the media owner with a
/// known policy; a later unreadable refresh neither starts nor stops encryption.
fn requires_e2ee(bundle: &Bundle) -> bool {
    matches!(
        bundle.inputs.host_configuration,
        Observed::Known { value, .. } if value.encryption_required
    )
}
/// One address a peer offers that this engine cannot use (unresolvable,
/// unsupported or surplus) is skipped, as browsers do. Ending the call for it
/// removed everyone because of one candidate; the remaining candidates or the
/// bounded connection recovery decide whether the peer connects.
fn remote_candidate(applied: Result<(), EngineError>) -> Result<(), &'static str> {
    match applied {
        Err(EngineError::Closed) => Err("peer engine closed"),
        _ => Ok(()),
    }
}
fn sid() -> Result<Negotiation, ()> {
    let mut value = [0; 16];
    getrandom::fill(&mut value).map_err(|_| ())?;
    Negotiation::parse(&value.iter().map(|b| format!("{b:02x}")).collect::<String>())
        .map_err(|_| ())
}
fn send(tx: &mpsc::Sender<Write>, value: Write) -> Result<(), ()> {
    tx.try_send(value).map_err(|_| ())
}
fn camera_guards(engine: &mut NativePeer, session: &Arc<Session>) {
    let source = Arc::downgrade(session);
    engine.set_publish_guard(Arc::new(move || {
        source.upgrade().is_some_and(|s| s.publication().camera)
    }));
    let source = Arc::downgrade(session);
    engine.set_audio_guard(Arc::new(move || {
        source.upgrade().is_some_and(|s| s.publication().microphone)
    }));
}
fn reconcile_participants(
    participants: &mut activity::Participants,
    bundle: &Bundle,
    invitees: &[(u64, u128)],
) -> Result<(), ()> {
    let sessions: Vec<_> = bundle
        .peers
        .iter()
        .map(|peer| peer.session.clone())
        .collect();
    let reserved: Vec<_> = invitees.iter().map(|(_, local)| *local).collect();
    participants.reconcile(&sessions, &reserved, || {
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).map_err(|_| ())?;
        Ok(u128::from_le_bytes(bytes))
    })
}

#[allow(clippy::too_many_arguments)]
async fn reconcile(
    peers: &mut Vec<Peer>,
    bundle: &Bundle,
    own: &PeerSession,
    tx: &mpsc::Sender<Write>,
    session: &Arc<Session>,
    topology: Topology,
    sfu_members: &[PeerSession],
    participants: &activity::Participants,
    mut e2ee: Option<&mut e2ee::Call>,
) -> Result<(), ()> {
    let wishes = session.publication();
    let local_media = wishes.microphone || wishes.camera;
    let mut i = 0;
    while i < peers.len() {
        if peers[i].role != Role::Publisher
            && (!bundle.peers.iter().any(|p| p.session == peers[i].session)
                || (topology == Topology::Sfu
                    && (!sfu_members.contains(&peers[i].session)
                        || (peers[i].stream == StreamKind::Camera
                            && !bundle
                                .peers
                                .iter()
                                .any(|p| p.session == peers[i].session && p.publish_media)))))
        {
            let mut p = peers.remove(i);
            p.engine.close();
            send(tx, Write::Remove(p.session, p.stream))?;
        } else {
            i += 1;
        }
    }
    if topology == Topology::Sfu
        && local_media
        && !peers
            .iter()
            .any(|p| p.role == Role::Publisher && p.stream == StreamKind::Camera)
    {
        let mut config = configuration(bundle, e2ee.as_deref_mut(), own);
        config.role = Role::Publisher;
        let mut engine = NativePeer::new(config).await.map_err(|_| ())?;
        camera_guards(&mut engine, session);
        report_ice_support(&engine);
        let negotiation = sid()?;
        send(
            tx,
            Write::Select(own.clone(), negotiation.clone(), StreamKind::Camera),
        )?;
        let offer = engine.offer().await.map_err(|_| ())?;
        send(
            tx,
            Write::Signal(
                own.clone(),
                negotiation.clone(),
                Signal::Description(offer),
                StreamKind::Camera,
            ),
        )?;
        peers.push(Peer {
            screen_announced: Vec::with_capacity(16),
            share_epoch: 0,
            recovery: None,
            restart_pending: false,
            restarts: 0,
            stream: StreamKind::Camera,
            dimensions: None,
            last_frame: None,
            local: 0,
            status: StatusSnapshot::default(),
            speaking: speaking::Speaking::default(),
            session: own.clone(),
            negotiation: Some(negotiation),
            engine,
            remote: false,
            candidates: Vec::with_capacity(64),
            connected: false,
            role: Role::Publisher,
            subscription_requested: false,
        });
    }
    for incoming in &bundle.peers {
        if let Some(peer) = peers
            .iter_mut()
            .find(|p| p.session == incoming.session && p.stream == StreamKind::Camera)
        {
            if peer.role == Role::Subscriber
                && !peer.subscription_requested
                && incoming.publish_media
                && (incoming.microphone || incoming.camera)
            {
                send(
                    tx,
                    Write::Subscribe(incoming.session.clone(), StreamKind::Camera),
                )?;
                peer.subscription_requested = true;
            }
            continue;
        }
        if topology == Topology::Sfu
            && (!sfu_members.contains(&incoming.session) || !incoming.publish_media)
        {
            continue;
        }
        let role = if topology == Topology::Sfu {
            Role::Subscriber
        } else {
            Role::Direct
        };
        let mut config = configuration(bundle, e2ee.as_deref_mut(), &incoming.session);
        config.role = role;
        let mut engine = NativePeer::new(config).await.map_err(|_| ())?;
        camera_guards(&mut engine, session);
        report_ice_support(&engine);
        let offer = topology == Topology::Direct
            && local_media
            && (own.local_should_offer(&incoming.session) || !incoming.publish_media);
        let subscription_requested =
            topology == Topology::Sfu && (incoming.microphone || incoming.camera);
        let negotiation = if offer {
            let n = sid()?;
            send(
                tx,
                Write::Select(incoming.session.clone(), n.clone(), StreamKind::Camera),
            )?;
            let sdp = engine.offer().await.map_err(|_| ())?;
            send(
                tx,
                Write::Signal(
                    incoming.session.clone(),
                    n.clone(),
                    Signal::Description(sdp),
                    StreamKind::Camera,
                ),
            )?;
            Some(n)
        } else {
            if subscription_requested {
                send(
                    tx,
                    Write::Subscribe(incoming.session.clone(), StreamKind::Camera),
                )?;
            }
            None
        };
        let local = participants
            .0
            .iter()
            .find(|person| person.session == incoming.session)
            .ok_or(())?
            .local;
        peers.push(Peer {
            screen_announced: Vec::with_capacity(16),
            share_epoch: 0,
            recovery: None,
            restart_pending: false,
            restarts: 0,
            stream: StreamKind::Camera,
            dimensions: None,
            last_frame: None,
            local,
            status: StatusSnapshot::default(),
            speaking: speaking::Speaking::default(),
            session: incoming.session.clone(),
            negotiation,
            engine,
            remote: false,
            candidates: Vec::with_capacity(64),
            connected: false,
            role,
            subscription_requested,
        });
    }
    Ok(())
}
#[allow(clippy::too_many_arguments)]
async fn media_loop(
    mut call: CallSession,
    mut bundle: Bundle,
    own: PeerSession,
    mut control: mpsc::Receiver<(Command, u64)>,
    mut signals: mpsc::Receiver<SignalBatch>,
    writes: mpsc::Sender<Write>,
    mut policies: mpsc::Receiver<PolicyUpdate>,
    hand_tx: watch::Sender<Option<outgoing::Message>>,
    reaction_tx: mpsc::Sender<outgoing::Message>,
    ring_tx: mpsc::Sender<moderation::Request>,
    mut ring_results: mpsc::Receiver<moderation::Result>,
    session: Arc<Session>,
    network: NetworkRuntime,
    cancel: Cancellation,
    stop: MeetingStop,
    topology: Topology,
    wires: WireResolver,
    encryption_tx: mpsc::Sender<(PeerSession, serde_json::Value)>,
) {
    let scope = call.scope();
    let started = Instant::now();
    let mut peers: Vec<Peer> = Vec::with_capacity(17);
    let mut participants = activity::Participants::default();
    let mut sfu_members = Vec::with_capacity(100);
    // Opaque identities live only for this call generation and are retired as
    // soon as the authoritative attendee leaves the bounded projection.
    let mut invitee_ids: Vec<(u64, u128)> = Vec::with_capacity(100);
    let mut transport_ready = true;
    let mut pending_screen_stops = Vec::with_capacity(4);
    let mut retired_screens = Vec::with_capacity(32);
    let mut latest_screen = None;
    let mut next_screen_frame = Instant::now();
    let mut required_policy = 0u64;
    let mut received_policy = 0u64;
    let mut refresh_hint = false;
    let mut hinted = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_millis(10));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_flags = call.wishes();
    let mut camera_offer_pending = !last_flags.microphone && !last_flags.camera;
    let mut local_speaking = speaking::Speaking::default();
    let mut hand_raised = false;
    let mut hand_timestamp = 0u64;
    let mut announced_refresh = 0u64;
    let mut hand_announcements = activity::HandAnnouncements::default();
    let mut reaction_sequence = 0u64;
    let mut latest_reaction: Option<calls::CallReaction> = None;
    let mut last_reaction: Option<Instant> = None;
    let mut ring_status: Option<calls::CallRingStatus> = None;
    let mut mute_status: Option<calls::CallRingStatus> = None;
    let mut end_pending = false;
    let mut end_failed = false;
    let mut recording_pending: Option<RecordingPending> = None;
    let mut recording_uncertain = false;
    let mut refreshes = 1u64;
    let mut stage = "initial peer reconciliation";
    let result: Result<(), ()> = async {
        // Required call encryption is part of the call from its first frame. An
        // owner that cannot start it ends the call; it never falls back to plain media.
        let mut e2ee = if requires_e2ee(&bundle) {
            stage = "call encryption keys";
            let own_wire = wires.own().map_err(|_| ())?;
            let resolve = wires.clone();
            Some(
                e2ee::Call::start(&own_wire, Arc::new(move |peer| resolve.peer(peer)), encryption_tx)
                    .map_err(|_| ())?,
            )
        } else {
            None
        };
        reconcile_participants(&mut participants, &bundle, &invitee_ids)?;
        while session.live() && !stop.is_closed() && !cancel.is_cancelled() {
            tick.tick().await;
            let mut refresh_needed = false;
            while let Ok((command, screen_epoch)) = control.try_recv() {
                match command {
                    Command::Change { wishes, .. } if !end_pending => {
                        call.set_wishes(scope, wishes).map_err(|_| ())?;
                    }
                    Command::StartScreenShare(_) => {
                        if !SUPPORT.screen { continue; }
                        if !session.start_screen(screen_epoch) { continue; }
                        let mut wishes = call.wishes(); wishes.screen = true;
                        call.set_wishes(scope, wishes).map_err(|_| ())?;
                    }
                    Command::StopScreenShare(_) => {
                        let mut wishes = call.wishes(); wishes.screen = false;
                        call.set_wishes(scope, wishes).map_err(|_| ())?;
                        // Already retired synchronously by command admission.
                    }
                    Command::MuteParticipant { target, .. } if transport_ready && received_policy >= required_policy && bundle.mute_supported
                        && call.phase() == Phase::Active && !end_pending
                        && !ring_status.is_some_and(|status| status.outcome == calls::RingOutcome::Pending)
                        && !mute_status.is_some_and(|status| status.outcome == calls::RingOutcome::Pending) => {
                        if let Some(person) = participants.0.iter().find(|person| person.local == target)
                            && bundle.peers.iter().any(|source| source.session == person.session && source.microphone) {
                            let outcome = if ring_tx.try_send(moderation::Request::Mute { target, peer: person.session.clone() }).is_ok() {
                                calls::RingOutcome::Pending
                            } else { calls::RingOutcome::Failed };
                            mute_status = Some(calls::CallRingStatus { target, outcome });
                        }
                    }
                    Command::Ring { target, .. } if transport_ready && received_policy >= required_policy && bundle.ring_supported
                        && call.phase() == Phase::Active && !end_pending
                        && !mute_status.is_some_and(|status| status.outcome == calls::RingOutcome::Pending)
                        && !ring_status.is_some_and(|status| status.outcome == calls::RingOutcome::Pending) => {
                        if let Some((attendee, _)) = invitee_ids.iter().find(|(_, local)| *local == target)
                            && bundle.invitees.iter().any(|person| person.attendee == *attendee && person.ring_eligible) {
                            let outcome = if ring_tx.try_send(moderation::Request::Ring { target, attendee: *attendee }).is_ok() {
                                calls::RingOutcome::Pending
                            } else { calls::RingOutcome::Failed };
                            ring_status = Some(calls::CallRingStatus { target, outcome });
                        }
                    }
                    Command::SetRecording { start, .. } if transport_ready && received_policy >= required_policy
                        && call.phase() == Phase::Active && !end_pending && !recording_uncertain
                        && recording_pending.is_none() && bundle.recording_control == Some(start)
                        && !ring_status.is_some_and(|status| status.outcome == calls::RingOutcome::Pending)
                        && !mute_status.is_some_and(|status| status.outcome == calls::RingOutcome::Pending) => {
                        if ring_tx.try_send(moderation::Request::Recording { start }).is_ok() {
                            recording_pending = Some(RecordingPending { start, requested: Instant::now(), accepted: false });
                        } else {
                            recording_uncertain = true;
                        }
                    }
                    Command::EndEveryone(_) => {
                        // Confirmation always stops local publication. A stale or
                        // refused moderator action cannot silently restore it.
                        call.set_wishes(scope, MediaWishes::default()).map_err(|_| ())?;
                        session.suspend();
                        if transport_ready && received_policy >= required_policy && bundle.end_supported
                            && call.phase() == Phase::Active && !end_pending
                            && ring_tx.try_send(moderation::Request::EndEveryone).is_ok() {
                            end_pending = true;
                            end_failed = false;
                        } else if !end_pending {
                            end_failed = true;
                        }
                    }
                    Command::Consent(_) => { call.consent(scope).map_err(|_| ())?; }
                    Command::SetHand { raised, .. } if transport_ready && received_policy >= required_policy && bundle.hand_supported && call.phase() == Phase::Active => {
                        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                            .map_or(0, |time| u64::try_from(time.as_millis()).unwrap_or(u64::MAX));
                        if let Some(next) = hand_timestamp.checked_add(1) {
                            hand_timestamp = now.max(next);
                            hand_raised = raised;
                            let peers: Vec<_> = bundle.peers.iter().map(|p| p.session.clone()).collect();
                            hand_announcements.changed(&peers);
                            let message = outgoing::Message { peers,
                                activity: api::signaling::activity::Activity::RaisedHand { state: raised, timestamp: hand_timestamp }, created: Instant::now() };
                            let _ = hand_tx.send(Some(message));
                        }
                    }
                    Command::React { emoji, .. } if transport_ready && received_policy >= required_policy && call.phase() == Phase::Active
                        && bundle.reactions.as_slice().contains(&emoji)
                        && last_reaction.is_none_or(|at| at.elapsed() >= Duration::from_secs(2)) => {
                        let message = outgoing::Message { peers: bundle.peers.iter().map(|p| p.session.clone()).collect(),
                            activity: api::signaling::activity::Activity::Reaction(emoji.as_str().to_owned()), created: Instant::now() };
                        if reaction_tx.try_send(message).is_ok() {
                            last_reaction = Some(Instant::now());
                            if let Some(next) = reaction_sequence.checked_add(1) {
                                reaction_sequence = next;
                                latest_reaction = Some(calls::CallReaction { sequence: next, participant: None, emoji });
                            }
                        }
                    }
                    Command::Reconnect(_) => {
                        // A new explicit call generation owns a replacement
                        // publisher/subscriber set. Never reuse retired SIDs.
                        call.disconnected(scope).map_err(|_| ())?;
                        return Err(());
                    }
                    Command::Stop(_) => { session.close(); break; }
                    _ => {}
                }
            }
            while let Ok(result) = ring_results.try_recv() {
                match result {
                    moderation::Result::Ring { target, accepted } => {
                        if ring_status.is_some_and(|status| status.target == target && status.outcome == calls::RingOutcome::Pending) {
                            ring_status = Some(calls::CallRingStatus { target,
                                outcome: if accepted { calls::RingOutcome::Requested } else { calls::RingOutcome::Failed } });
                        }
                    }
                    moderation::Result::Mute { target, accepted } => {
                        if mute_status.is_some_and(|status| status.target == target && status.outcome == calls::RingOutcome::Pending) {
                            mute_status = Some(calls::CallRingStatus { target,
                                outcome: if accepted { calls::RingOutcome::Requested } else { calls::RingOutcome::Failed } });
                        }
                    }
                    moderation::Result::EndEveryone { accepted } if end_pending => {
                        end_pending = false;
                        if accepted { session.close(); stop.close(); }
                        else { end_failed = true; }
                    }
                    moderation::Result::EndEveryone { .. } => {}
                    moderation::Result::Recording { start, accepted } => {
                        if let Some(pending) = recording_pending.as_mut().filter(|pending| pending.start == start) {
                            if accepted {
                                pending.accepted = true;
                            } else {
                                recording_pending = None;
                                recording_uncertain = true;
                            }
                            refresh_hint = true;
                        }
                    }
                }
            }
            stage = "remote signaling";
            while let Ok(batch) = signals.try_recv() {
                for event in batch.into_events() {
                    match event {
                        SignalEvent::Signal { from, negotiation, signal, stream } => {
                            if stream == StreamKind::Screen && (retired_screens.len() >= 32 || retired_screens.iter().any(|(peer, sid)| peer == &from && sid == &negotiation)) { continue; }
                            if stream == StreamKind::Screen && !peers.iter().any(|p| p.session == from && p.stream == stream) {
                                // REST membership admits the sender, never the offer itself.
                                if !bundle.peers.iter().any(|p| p.session == from)
                                    || peers.iter().any(|p| p.stream == StreamKind::Screen) { continue; }
                                if !matches!(&signal, Signal::Description(s) if s.kind() == DescriptionKind::Offer) { continue; }
                                let mut config = configuration(&bundle, e2ee.as_mut(), &from);
                                config.role = Role::Subscriber;
                                let Ok(mut engine) = NativePeer::new(config).await else { continue; };
                                engine.screen_profile();
                                let mut identity = [0; 16]; getrandom::fill(&mut identity).map_err(|_| ())?;
                                peers.push(Peer { screen_announced: Vec::with_capacity(16), share_epoch: 0, recovery: None, restart_pending: false, restarts: 0, stream, dimensions: None, last_frame: None, local: u128::from_le_bytes(identity),
                                    status: StatusSnapshot::default(), speaking: speaking::Speaking::default(), session: from.clone(), negotiation: None,
                                    engine, remote: false, candidates: Vec::with_capacity(64), connected: false,
                                    role: Role::Subscriber, subscription_requested: true });
                            }
                            let Some(peer) = peers.iter_mut().find(|p| p.session == from && p.stream == stream) else { continue; };
                            if peer.negotiation.as_ref().is_some_and(|n| *n != negotiation) { continue; }
                            if peer.negotiation.is_none() {
                                if !matches!(&signal, Signal::Description(s) if s.kind() == DescriptionKind::Offer) { continue; }
                                peer.negotiation = Some(negotiation.clone());
                                send(&writes, Write::Select(from.clone(), negotiation.clone(), stream))?;
                            }
                            let applied: Result<(), &'static str> = async {
                            match signal {
                                Signal::StopScreen | Signal::SendOffer => return Ok(()),
                                Signal::Description(sdp) => {
                                    if let Some(answer) = peer.engine.remote(&sdp).await.map_err(|error| match error {
                                        EngineError::Unsupported => "remote description unsupported",
                                        EngineError::Malformed => "remote description malformed",
                                        _ => "remote description rejected",
                                    })? {
                                        send(&writes, Write::Signal(from, negotiation, Signal::Description(answer), stream)).map_err(|_| "signaling queue full")?;
                                    }
                                    peer.remote = true;
                                    for candidate in peer.candidates.drain(..) {
                                        remote_candidate(peer.engine.candidate(&candidate).await)?;
                                    }
                                }
                                Signal::Candidate(candidate) => {
                                    if peer.remote {
                                        remote_candidate(peer.engine.candidate(&candidate).await)?;
                                    } else if peer.candidates.len() < 64 {
                                        peer.candidates.push(candidate);
                                    } else { return Err("remote candidates before description overflowed"); }
                                }
                            }
                            Ok(()) }.await;
                            if let Err(reason) = applied {
                                if stream == StreamKind::Screen { peer.engine.close(); }
                                else { stage = reason; return Err(()); }
                            }

                        }
                        SignalEvent::ScreenStopped { from, negotiation } => {
                            if let Some(index) = peers.iter().position(|p| p.session == from && p.stream == StreamKind::Screen && negotiation.as_ref().is_none_or(|n| p.negotiation.as_ref() == Some(n))) {
                                let mut peer = peers.remove(index);
                                #[cfg(feature = "backend-test")]
                                eprintln!("screen stop evidence connected={} rtp={} keyframes={} feedback={} decoded={}", peer.connected, peer.engine.video_packets(), peer.engine.video_keyframes(), peer.engine.video_feedback_requests(), peer.engine.counters().video_received);
                                peer.engine.close();
                                if let Some(sid) = peer.negotiation && retired_screens.len() < 32 { retired_screens.push((peer.session.clone(), sid)); }
                                send(&writes, Write::Remove(peer.session, peer.stream))?;
                            }
                        }
                        SignalEvent::Membership(members) => {
                            if topology != Topology::Direct { stage = "membership event on a routed call"; return Err(()); }
                            if !members.iter().any(|m| m.session == own && m.in_call.is_some_and(|f| f & 1 != 0)) { stage = "membership without own call session"; return Err(()); }
                            let mut index = 0;
                            while index < peers.len() {
                                if !members.iter().any(|m| m.session == peers[index].session && m.in_call.is_some_and(|f| f & 1 != 0)) {
                                    let mut peer = peers.remove(index);
                                    peer.engine.close();
                                    send(&writes, Write::Remove(peer.session, peer.stream))?;
                                } else { index += 1; }
                            }
                            refresh_hint = true;
                        }
                        SignalEvent::SfuMembership(members) => {
                            if topology != Topology::Sfu { stage = "routed membership on a direct call"; return Err(()); }
                            // Routing facts can retire an absent route, but may
                            // add no media peer without fresh REST membership.
                            sfu_members = members;
                            refresh_hint = true;
                        }
                        SignalEvent::TransportInterrupted => {
                            transport_ready = false;
                            for peer in &mut peers {
                                if !peer.connected && peer.recovery.is_none() {
                                    peer.recovery = Some(Instant::now());
                                    peer.restart_pending = peer.role != Role::Direct || peer.engine.is_offerer();
                                }
                            }
                            session.stop_screen();
                            latest_reaction = None;
                            call.disconnected(scope).map_err(|_| ())?;
                            refresh_needed = true;
                        }
                        SignalEvent::TransportResumed => {
                            transport_ready = true;
                            refresh_needed = true;
                        }
                        SignalEvent::SubscriptionRejected { from, stream } => {
                            if topology != Topology::Sfu { stage = "subscription rejection on a direct call"; return Err(()); }
                            if let Some(peer) = peers.iter_mut().find(|p| p.session == from && p.stream == stream && p.role == Role::Subscriber) {
                                peer.connected = false;
                                peer.recovery.get_or_insert_with(Instant::now);
                                peer.restart_pending = true;
                            }
                        }
                        SignalEvent::ForceMuteBy { from } => {
                            // Stopping publication is safe even before a policy refresh;
                            // an unknown sender cannot create a participant or authority.
                            if bundle.peers.iter().any(|peer| peer.session == from) {
                                let mut wishes = call.wishes();
                                wishes.microphone = false;
                                call.set_wishes(scope, wishes).map_err(|_| ())?;
                                session.suspend();
                                refresh_needed = true;
                            }
                        }
                        SignalEvent::ForceMute => {
                            let mut wishes = call.wishes();
                            wishes.microphone = false;
                            call.set_wishes(scope, wishes).map_err(|_| ())?;
                            refresh_needed = true;
                        }
                        // Membership reconciliation owns peer identity; unsolicited activity
                        // cannot create a peer or change media authority.
                        SignalEvent::Activity { from, activity } => {
                            if let Some(person) = participants.0.iter_mut().find(|person| person.session == from) {
                                match activity {
                                    api::signaling::activity::Activity::RaisedHand { state, timestamp } => person.hand.update(state, timestamp),
                                    api::signaling::activity::Activity::Reaction(value) => {
                                        if let (Ok(emoji), Some(next)) = (talk_core::bounded::Text::new(&value), reaction_sequence.checked_add(1)) {
                                            reaction_sequence = next;
                                            latest_reaction = Some(calls::CallReaction { sequence: next, participant: Some(person.local), emoji });
                                        }
                                    }
                                }
                            }
                        }
                        SignalEvent::Encryption { from, wire, payload } => {
                            if let Some(call) = e2ee.as_mut() {
                                call.message(Instant::now(), &from, &wire, &payload);
                            }
                        }
                        SignalEvent::PolicyChanged | SignalEvent::Hint { .. } => { refresh_hint = true; }
                    }
                }
            }
            if refresh_needed {
                session.suspend();
                required_policy = required_policy.checked_add(1).ok_or(())?;
                send(&writes, Write::Refresh(required_policy))?;
                refresh_hint = false;
            }
            // Somebody joined, left or changed a device: something may have
            // changed, nothing is known to be withdrawn. Read the policy again
            // at once, but keep publishing on the current grant, which still
            // ends at its own lease. Only a known restriction or a lost control
            // connection (above) suspends until fresh evidence arrives.
            if refresh_hint && hinted.elapsed() >= REFRESH_HINT_SPACING {
                send(&writes, Write::Refresh(required_policy))?;
                refresh_hint = false;
                hinted = Instant::now();
            }
            while let Ok(next) = policies.try_recv() {
                // The request sequence is assigned before the first HTTP
                // observation, not at completion. A delayed earlier refresh
                // cannot restore a grant after a newer invalidation/reconnect.
                if next.after >= required_policy {
                    refreshes = refreshes.saturating_add(1);
                    received_policy = next.after;
                    bundle = next.bundle;
                    // A call that became encrypted while we were not keyed cannot
                    // continue; a new admission starts the exchange from the beginning.
                    if e2ee.is_none() && requires_e2ee(&bundle) { stage = "call became encrypted"; return Err(()); }
                    stage = "participant reconciliation";
                    reconcile_participants(&mut participants, &bundle, &invitee_ids)?;
                }
            }
            let policy_ready = transport_ready && received_policy >= required_policy;
            if let Some(call) = e2ee.as_mut() {
                let members: Vec<PeerSession> = bundle.peers.iter().map(|p| p.session.clone()).collect();
                call.sync(Instant::now(), &members);
                call.poll(Instant::now());
            }
            if !call.wishes().microphone && !call.wishes().camera {
                camera_offer_pending = true;
            }
            let recording = moderation::current_recording(
                match bundle.inputs.room {
                    Observed::Known { value, lease } => Observed::Known { value: value.recording, lease },
                    Observed::Unknown => Observed::Unknown,
                },
                scope, network.now(), policy_ready,
            );
            if let Some(pending) = recording_pending.as_ref() {
                match moderation::recording_resolution(
                    pending.start, pending.accepted, recording != RecordingState::Unknown, recording,
                    pending.requested.elapsed() >= Duration::from_secs(20),
                ) {
                    moderation::RecordingResolution::Pending => {}
                    moderation::RecordingResolution::Confirmed => { recording_pending = None; }
                    moderation::RecordingResolution::Uncertain => {
                        recording_pending = None;
                        recording_uncertain = true;
                    }
                }
            }
            if policy_ready && bundle.hand_supported && announced_refresh != refreshes {
                announced_refresh = refreshes;
                let peers: Vec<_> = bundle.peers.iter().map(|p| p.session.clone()).collect();
                if let Some((activity, targets)) = hand_announcements.refresh(&peers, hand_raised, hand_timestamp) {
                    let _ = hand_tx.send(Some(outgoing::Message { peers: targets, activity, created: Instant::now() }));
                }
            }
            let clear_hand = !policy_ready && { hand_tx.borrow().is_some() };
            if clear_hand { let _ = hand_tx.send(None); }
            stage = "policy grant";
            let video_limit = if policy_ready {
                let grant = call.advance(&bundle.inputs, bundle.exists, SUPPORT, DEVICES, network.now()).map_err(|gate| { stage = gate.diagnostic(); })?;
                let limit = usize::from(grant.remote_video_limit());
                session.grant(grant, network.now());
                // A receive-only HPB rejoin must advertise explicit unmute
                // before a new publisher offer. The writer preserves FIFO.
                let publication = session.publication();
                if publication != last_flags {
                    send(&writes, Write::Flags(publication))?;
                    last_flags = publication;
                }
                stage = "peer setup";
                reconcile(&mut peers, &bundle, &own, &writes,
                    &session,
                    topology, &sfu_members, &participants, e2ee.as_mut()).await?;
                limit
            } else {
                session.suspend();
                0
            };
            // Remove vanished SFU routes immediately even while REST refresh is
            // pending. Any later restoration needs a new call admission.
            if topology == Topology::Sfu {
                let mut index = 0;
                while index < peers.len() {
                    if peers[index].role == Role::Subscriber && !sfu_members.contains(&peers[index].session) {
                        let mut peer = peers.remove(index);
                        peer.engine.close();
                        send(&writes, Write::Remove(peer.session, peer.stream))?;
                    } else { index += 1; }
                }
            }
            stage = "media exchange";
            let wishes = session.publication();
            let audio = session.take_audio();
            if policy_ready && wishes.microphone {
                if let Some(samples) = &audio {
                    local_speaking.observe(samples, Instant::now());
                }
            } else {
                local_speaking = speaking::Speaking::default();
            }
            let video = session.take_video();
            if let Some(frame) = session.take_screen() { latest_screen = Some(frame); }
            if !session.screen_capture_active() || latest_screen.as_ref().is_some_and(|(epoch, _)| *epoch != session.screen_epoch()) { latest_screen = None; }
            // A compositor may deliver only one static frame, before ICE is
            // connected. Retain one current-generation backing and encode it at
            // most 10 fps, so late subscribers/keyframe recovery can see it too.
            let screen = if wishes.screen && Instant::now() >= next_screen_frame {
                next_screen_frame = Instant::now() + Duration::from_millis(100);
                latest_screen.as_ref()
            } else { None };
            if let Some((share_epoch, _)) = screen && wishes.screen {
                let share_epoch = *share_epoch;
                let destinations: Vec<_> = if topology == Topology::Sfu { vec![own.clone()] }
                    else { bundle.peers.iter().map(|p| p.session.clone()).collect() };
                for destination in destinations {
                    if peers.iter().any(|p| p.stream == StreamKind::Screen && p.session == destination && p.role != Role::Subscriber) { continue; }
                    if peers.iter().filter(|p| p.stream == StreamKind::Screen && p.role != Role::Subscriber).count() >= 4 { break; }
                    let mut config = configuration(&bundle, e2ee.as_mut(), &destination);
                    config.role = if topology == Topology::Sfu { Role::Publisher } else { Role::Direct };
                    let role = config.role;
                    let mut engine = NativePeer::new(config).await.map_err(|_| ())?; engine.screen_profile();
                    let source = Arc::downgrade(&session);
                    engine.set_publish_guard(Arc::new(move || source.upgrade().is_some_and(|s| s.screen_epoch() == share_epoch && s.publication().screen)));
                    let negotiation = sid()?;
                    send(&writes, Write::Select(destination.clone(), negotiation.clone(), StreamKind::Screen))?;
                    let sdp = engine.offer().await.map_err(|_| ())?;
                    send(&writes, Write::Signal(destination.clone(), negotiation.clone(), Signal::Description(sdp), StreamKind::Screen))?;
                    peers.push(Peer { screen_announced: Vec::with_capacity(16), share_epoch, recovery: None, restart_pending: false, restarts: 0, stream: StreamKind::Screen, dimensions: None, last_frame: None, local: 0,
                        status: StatusSnapshot::default(), speaking: speaking::Speaking::default(), session: destination, negotiation: Some(negotiation),
                        engine, remote: false, candidates: Vec::with_capacity(64), connected: false, role, subscription_requested: false });
                }
            }
            let mut index = 0;
            while index < peers.len() {
                if (!call.wishes().screen || !session.screen_capture_active() || peers[index].share_epoch != session.screen_epoch()) && peers[index].stream == StreamKind::Screen && peers[index].role != Role::Subscriber {
                    let mut peer = peers.remove(index);
                    if let Some(n) = peer.negotiation {
                        if transport_ready {
                            send(&writes, Write::Signal(peer.session.clone(), n, Signal::StopScreen, StreamKind::Screen))?;
                            send(&writes, Write::Remove(peer.session.clone(), StreamKind::Screen))?;
                        } else if pending_screen_stops.len() < 4 { pending_screen_stops.push((peer.session.clone(), n)); }
                        else { return Err(()); }
                    }
                    peer.engine.close();
                } else { index += 1; }
            }
            if transport_ready && policy_ready {
                for (peer, negotiation) in pending_screen_stops.drain(..) {
                    send(&writes, Write::Signal(peer.clone(), negotiation, Signal::StopScreen, StreamKind::Screen))?;
                    send(&writes, Write::Remove(peer, StreamKind::Screen))?;
                }
            }
            let mut totals = MediaCounters::default();
            let mut any_connected = false;
            let mut mix = [0i32; 480];
            let mut mixed = false;
            let mut receive_index = 0;
            let receiving_screen = peers.iter().any(|p| p.stream == StreamKind::Screen && p.role == Role::Subscriber);
            let camera_limit = video_limit.saturating_sub(usize::from(receiving_screen)).min(4);
            for peer in &mut peers {
                if !peer.engine.is_open() && peer.stream == StreamKind::Screen { continue; }
                if let Some(since) = peer.recovery {
                    if since.elapsed() > Duration::from_secs(15) {
                        if peer.stream == StreamKind::Screen {
                            if peer.role != Role::Subscriber { session.fail_screen(calls::ShareFailure::TimedOut); }
                            peer.engine.close(); continue;
                        }
                        stage = "peer connection not recovered in 15 s";
                        return Err(());
                    }
                    if peer.restart_pending && policy_ready && peer.restarts < 3
                        && since.elapsed() >= Duration::from_millis(500u64 << (2 * peer.restarts)) {
                        peer.restarts += 1; peer.restart_pending = false;
                        if peer.role == Role::Subscriber {
                            let mut config = configuration(&bundle, e2ee.as_mut(), &peer.session); config.role = Role::Subscriber;
                            peer.engine.close();
                            peer.engine = NativePeer::new(config).await.map_err(|_| { stage = "subscriber restart"; })?;
                            if peer.stream == StreamKind::Screen { peer.engine.screen_profile(); }
                            peer.negotiation = None; peer.remote = false; peer.candidates.clear();
                            send(&writes, Write::RestartSubscription(peer.session.clone(), peer.stream))?;
                        } else if let Some(negotiation) = &peer.negotiation {
                            let offer = peer.engine.restart().await.map_err(|_| { stage = "ice restart offer"; })?;
                            send(&writes, Write::Signal(peer.session.clone(), negotiation.clone(), Signal::Description(offer), peer.stream))?;
                        }
                    }
                }
                let receives = peer.role != Role::Publisher && !(peer.stream == StreamKind::Screen && peer.role == Role::Direct);
                let publishes = peer.role != Role::Subscriber;
                peer.engine.receive_video(receives && if peer.stream == StreamKind::Screen { video_limit > 0 } else { receive_index < camera_limit });
                if receives && peer.stream == StreamKind::Camera { receive_index += 1; }
                if peer.stream == StreamKind::Camera && topology == Topology::Direct && policy_ready && peer.negotiation.is_none()
                    && (wishes.microphone || wishes.camera)
                    && bundle.peers.iter().find(|p| p.session == peer.session)
                        .is_some_and(|p| own.local_should_offer(&p.session) || !p.publish_media)
                {
                    let negotiation = sid()?;
                    send(&writes, Write::Select(peer.session.clone(), negotiation.clone(), peer.stream))?;
                    let sdp = peer.engine.offer().await.map_err(|_| { stage = "direct offer"; })?;
                    send(&writes, Write::Signal(peer.session.clone(), negotiation.clone(), Signal::Description(sdp), peer.stream))?;
                    peer.negotiation = Some(negotiation);
                }
                peer.engine.publish(publishes && peer.stream == StreamKind::Camera && wishes.microphone,
                    publishes && if peer.stream == StreamKind::Screen { wishes.screen } else { wishes.camera }).map_err(|_| { stage = "publication switch"; })?;
                loop {
                    let event = match peer.engine.event().await {
                        Ok(Some(event)) => event,
                        Ok(None) => break,
                        Err(error) if peer.stream == StreamKind::Screen => {
                            #[cfg(feature = "backend-test")]
                            eprintln!("screen media stopped: {error:?}");
                            let _ = error;
                            if peer.role != Role::Subscriber { session.fail_screen(calls::ShareFailure::Unavailable); }
                            peer.engine.close(); break;
                        }
                        Err(error) => {
                            #[cfg(feature = "backend-test")]
                            eprintln!("camera engine stopped: role={:?} error={error:?}", peer.role);
                            stage = match (peer.role, error) {
                                (Role::Publisher, EngineError::Closed) => "publisher engine closed",
                                (Role::Publisher, EngineError::Overloaded) => "publisher engine overloaded",
                                (Role::Publisher, _) => "publisher engine failed",
                                (_, EngineError::Closed) => "peer engine closed",
                                (_, EngineError::Overloaded) => "peer engine overloaded",
                                _ => "peer engine failed",
                            };
                            return Err(());
                        }
                    };
                    match event {
                        EngineEvent::Candidate(candidate) => {
                            // Drop late candidates; recover with fresh ICE/SDP
                            // after renewed control/policy, never a stale replay.
                            if !transport_ready {
                                if !peer.connected {
                                    peer.recovery.get_or_insert_with(Instant::now);
                                    peer.restart_pending = peer.role != Role::Direct || peer.engine.is_offerer();
                                }
                                continue;
                            }
                            if let Some(negotiation) = &peer.negotiation {
                                send(&writes, Write::Signal(peer.session.clone(), negotiation.clone(), Signal::Candidate(candidate), peer.stream))?;
                            }
                        }
                        EngineEvent::Connection(state) => {
                            peer.connected = state == PeerConnectionState::Connected;
                            if matches!(state, PeerConnectionState::Failed | PeerConnectionState::Disconnected) && peer.recovery.is_none() {
                                    peer.recovery = Some(Instant::now());
                                    peer.restart_pending = peer.role != Role::Direct || peer.engine.is_offerer();
                            }
                            if state == PeerConnectionState::Connected { peer.recovery = None; peer.restart_pending = false; }
                            if state == PeerConnectionState::Closed {
                                if peer.stream == StreamKind::Screen { peer.engine.close(); break; }
                                stage = "peer connection closed";
                                return Err(());
                            }
                        }
                    }
                }
                if !peer.engine.is_open() && peer.stream == StreamKind::Screen { continue; }
                if publishes && peer.connected {
                    // Web peers do not request a publisher that joined receive-only.
                    // Once publication is connected, use HPB's existing sendoffer
                    // handshake, as for a newly shared screen. A status message on
                    // SCTP cannot reach a subscriber that has not been created yet.
                    if peer.role == Role::Publisher && peer.stream == StreamKind::Camera
                        && topology == Topology::Sfu && policy_ready && camera_offer_pending
                        && (wishes.microphone || wishes.camera) {
                        for target in bundle.peers.iter().filter(|target| sfu_members.contains(&target.session)).take(16) {
                            send(&writes, Write::SendStreamOffer(target.session.clone(), StreamKind::Camera))?;
                        }
                        camera_offer_pending = false;
                    }
                    if peer.stream == StreamKind::Screen && topology == Topology::Sfu {
                        for target in &bundle.peers {
                            if sfu_members.contains(&target.session) && !peer.screen_announced.contains(&target.session) && peer.screen_announced.len() < 16 {
                                send(&writes, Write::SendStreamOffer(target.session.clone(), StreamKind::Screen))?;
                                peer.screen_announced.push(target.session.clone());
                            }
                        }
                    }
                    if peer.stream == StreamKind::Camera && let Some(samples) = &audio { peer.engine.audio(samples).await.map_err(|_| { stage = "audio send"; })?; }
                    if let Some(frame) = if peer.stream == StreamKind::Screen { screen.map(|(_, frame)| frame) } else { video.as_ref() } {
                        if peer.engine.video(frame.clone(), started.elapsed().as_micros() as i64).is_err() {
                            if peer.stream == StreamKind::Screen { session.fail_screen(calls::ShareFailure::Unavailable); peer.engine.close(); continue; }
                            stage = "video send";
                            return Err(());
                        }
                        if peer.stream == StreamKind::Screen && peer.engine.counters().video_transmitted > 0 {
                            session.screen_flowing(peer.share_epoch);
                        }
                    }
                    if peer.status.attempt(Instant::now(), wishes, peer.role == Role::Publisher && policy_ready) {
                        let audio: &[u8] = if wishes.microphone { br#"{"type":"audioOn"}"# } else { br#"{"type":"audioOff"}"# };
                        let video: &[u8] = if wishes.camera { br#"{"type":"videoOn"}"# } else { br#"{"type":"videoOff"}"# };
                        if peer.engine.send_status(audio).await.is_ok()
                            && peer.engine.send_status(video).await.is_ok() {
                            peer.status.accepted(Instant::now(), wishes);
                        }
                    }
                }
                if receives {
                    if let Some(frame) = peer.engine.take_audio() && frame.data.len() == 480 {
                        peer.speaking.observe(&frame.data, Instant::now());
                        for (output, input) in mix.iter_mut().zip(frame.data.iter()) { *output += i32::from(*input); }
                        mixed = true;
                    }
                    while let Some(frame) = peer.engine.take_video() {
                        peer.dimensions = Some((frame.width, frame.height)); peer.last_frame = Some(Instant::now());
                        session.show_video(peer.local, frame);
                    }
                }
                any_connected |= peer.connected;
                let c = peer.engine.counters();
                totals.audio_sent += c.audio_sent;
                totals.audio_received += c.audio_received;
                totals.video_sent += c.video_sent;
                totals.video_received += c.video_received;
                totals.data_received += c.data_received;
                totals.offers_sent += c.offers_sent;
                totals.answers_sent += c.answers_sent;
            }
            totals.policy_refreshes = refreshes;
            let mut retired = 0;
            while retired < peers.len() {
                if peers[retired].stream == StreamKind::Screen && !peers[retired].engine.is_open() {
                    let peer = peers.remove(retired);
                    if peer.role != Role::Subscriber && session.screen_capture_active() { session.fail_screen(calls::ShareFailure::Unavailable); }
                    if let Some(sid) = peer.negotiation {
                        if peer.role == Role::Subscriber && retired_screens.len() < 32 { retired_screens.push((peer.session.clone(), sid)); }
                        else if peer.role != Role::Subscriber {
                            if transport_ready { send(&writes, Write::Signal(peer.session.clone(), sid, Signal::StopScreen, StreamKind::Screen))?; }
                            else if pending_screen_stops.len() < 4 { pending_screen_stops.push((peer.session.clone(), sid)); continue; }
                            else { return Err(()); }
                        }
                    }
                    send(&writes, Write::Remove(peer.session, peer.stream))?;
                } else { retired += 1; }
            }
            let mut participant_snapshot = talk_core::bounded::Page::empty();
            let screen = peers.iter().find(|peer| peer.stream == StreamKind::Screen && peer.role == Role::Subscriber);
            for person in participants.0.iter().take(if screen.is_some() { 15 } else { 16 }) {
                if let Some(source) = bundle.peers.iter().find(|p| p.session == person.session) {
                    let speaking = policy_ready && peers.iter().find(|peer| peer.role != Role::Publisher && peer.session == person.session)
                        .is_some_and(|peer| peer.speaking.active(Instant::now()));
                    participant_snapshot.try_push(calls::Participant {
                        local: person.local, participant: person.local, kind: calls::StreamKind::Camera,
                        dimensions: peers.iter().find(|peer| peer.session == person.session && peer.stream == StreamKind::Camera).and_then(|peer| peer.dimensions),
                        flowing: peers.iter().find(|peer| peer.session == person.session && peer.stream == StreamKind::Camera).is_some_and(|peer| peer.last_frame.is_some_and(|at| at.elapsed() < Duration::from_secs(3))),
                        name: source.name.clone(), microphone: source.microphone, camera: source.camera, raised_hand: person.hand.raised(), speaking,
                        can_mute: policy_ready && bundle.mute_supported && source.microphone && !end_pending
                            && !ring_status.is_some_and(|status| status.outcome == calls::RingOutcome::Pending)
                            && !mute_status.is_some_and(|status| status.outcome == calls::RingOutcome::Pending),
                    }).map_err(|_| ())?;
                }
            }
            if let Some(peer) = screen
                && let Some(source) = bundle.peers.iter().find(|source| source.session == peer.session)
                && let Some(person) = participants.0.iter().find(|person| person.session == peer.session) {
                participant_snapshot.try_push(calls::Participant {
                    local: peer.local, participant: person.local, name: source.name.clone(),
                    kind: calls::StreamKind::Screen, dimensions: peer.dimensions,
                    flowing: peer.last_frame.is_some_and(|at| at.elapsed() < Duration::from_secs(3)),
                    microphone: source.microphone, camera: source.camera,
                    raised_hand: person.hand.raised(), speaking: false, can_mute: false,
                }).map_err(|_| ())?;
            }
            let mut invitees = talk_core::bounded::Page::empty();
            if policy_ready {
                invitee_ids.retain(|(id, _)| bundle.invitees.iter().any(|person| person.attendee == *id));
                for person in &bundle.invitees {
                    let local = if let Some((_, local)) = invitee_ids.iter().find(|(id, _)| *id == person.attendee) {
                        *local
                    } else {
                        let mut bytes = [0u8; 16];
                        if getrandom::fill(&mut bytes).is_err() { continue; }
                        let local = u128::from_le_bytes(bytes);
                        if local == 0 || invitee_ids.iter().any(|(_, existing)| *existing == local)
                            || participants.0.iter().any(|person| person.local == local) { continue; }
                        invitee_ids.push((person.attendee, local));
                        local
                    };
                    let _ = invitees.try_push(calls::CallInvitee { local, ring_allowed: bundle.ring_supported && person.ring_eligible && !end_pending
                        && !mute_status.is_some_and(|status| status.outcome == calls::RingOutcome::Pending)
                        && !ring_status.is_some_and(|status| status.outcome == calls::RingOutcome::Pending), name: person.name.clone() });
                }
            } else {
                invitee_ids.clear();
            }
            session.retain_video(&peers.iter().filter(|p| p.role != Role::Publisher && p.stream == StreamKind::Camera).take(camera_limit)
                .chain(peers.iter().filter(|p| p.role == Role::Subscriber && p.stream == StreamKind::Screen).take(usize::from(video_limit > 0)))
                .map(|p| p.local).collect::<Vec<_>>());
            if mixed {
                let mut output = [0i16; 480];
                for (out, input) in output.iter_mut().zip(mix) { *out = input.clamp(i16::MIN as i32, i16::MAX as i32) as i16; }
                session.play_audio(&output);
            }
            if policy_ready && wishes != last_flags {
                send(&writes, Write::Flags(wishes))?;
                last_flags = wishes;
            }
            let recovering = peers.iter().any(|p| p.stream == StreamKind::Camera && p.recovery.is_some());
            if policy_ready && any_connected && !recovering { call.connected(scope, network.now()).map_err(|_| { stage = "connected without a fresh grant"; })?; }
            if let Ok(mut snapshot) = session.snapshot.lock() {
                let (mic, camera) = session.actual();
                let (mic_failed, camera_failed) = session.failed();
                if snapshot.sharing == calls::SharingState::Idle {
                    snapshot.screen = if SUPPORT.screen && matches!(bundle.inputs.room, Observed::Known { value, .. } if value.permissions.publish_screen) { DeviceState::Off } else { DeviceState::Unavailable };
                }
                snapshot.phase = if session.control_degraded() && matches!(snapshot.phase, Phase::Waiting(calls::Gate::PolicyUnavailable(_))) {
                    snapshot.phase
                } else if recovering || !transport_ready || session.control_degraded() { Phase::Reconnecting } else { call.phase() };
                snapshot.wishes = call.wishes();
                snapshot.microphone = if mic_failed { DeviceState::Unavailable } else if mic { DeviceState::Active } else if wishes.microphone { DeviceState::Requested } else { DeviceState::Off };
                snapshot.local_speaking = policy_ready && mic && local_speaking.active(Instant::now());
                snapshot.camera = if camera_failed { DeviceState::Unavailable } else if camera { DeviceState::Active } else if wishes.camera { DeviceState::Requested } else { DeviceState::Off };
                snapshot.peers = participants.0.len() as u16;
                snapshot.participants = participant_snapshot;
                snapshot.hand_supported = policy_ready && bundle.hand_supported && call.phase() == Phase::Active;
                snapshot.hand_raised = hand_raised;
                snapshot.reactions = if policy_ready && call.phase() == Phase::Active { bundle.reactions.clone() } else { talk_core::bounded::Page::empty() };
                snapshot.latest_reaction = latest_reaction.clone();
                snapshot.invitees_loaded = policy_ready;
                snapshot.invitees = invitees;
                snapshot.ring_status = ring_status;
                snapshot.mute_status = mute_status;
                snapshot.can_end_everyone = policy_ready && bundle.end_supported && call.phase() == Phase::Active && !end_failed;
                snapshot.end_pending = end_pending;
                snapshot.end_failed = end_failed;
                snapshot.counters = totals;
                snapshot.recording = recording;
                snapshot.recording_control = if recording != RecordingState::Unknown && call.phase() == Phase::Active && !end_pending {
                    bundle.recording_control
                } else { None };
                snapshot.recording_pending = recording_pending.is_some();
                snapshot.recording_uncertain = recording_uncertain;
                snapshot.encrypted = e2ee.is_some();
                snapshot.revision = snapshot.revision.saturating_add(1);
            }
        }
        Ok(())
    }.await;
    if result.is_err() {
        // The gate shown for a stopped owner is generic; keep the step that
        // stopped it for the local diagnostic log. A fixed label, no payload.
        crate::media::note_call_stop(stage);
        #[cfg(feature = "backend-test")]
        eprintln!("normal actor stopped: {stage}");
    }
    // Publish a policy rejection before closing the latch. Composition may
    // retire this owner as soon as it observes closed(); peer disposal below
    // can await native work and must not hide the reason from that observer.
    if let Phase::Waiting(_) = call.phase()
        && let Ok(mut snapshot) = session.snapshot.lock()
    {
        snapshot.phase = closed_phase(call.phase(), snapshot.phase, result.is_ok());
    }
    session.close();
    for mut peer in peers {
        #[cfg(feature = "backend-test")]
        if peer.stream == StreamKind::Screen {
            eprintln!(
                "screen evidence role={:?} connected={} rtp={} decoded={}",
                peer.role,
                peer.connected,
                peer.engine.video_packets(),
                peer.engine.counters().video_received
            );
        }
        peer.engine.close();
    }
    stop.close();
    if let Ok(mut snapshot) = session.snapshot.lock() {
        // The policy writer can close the session while the media loop exits
        // normally. Preserve its concrete failure instead of reporting "ended"
        // or replacing a required encryption/consent gate with generic staleness.
        snapshot.phase = closed_phase(call.phase(), snapshot.phase, result.is_ok());
        snapshot.microphone = DeviceState::Off;
        snapshot.camera = DeviceState::Off;
        snapshot.screen = DeviceState::Off;
        snapshot.revision = snapshot.revision.saturating_add(1);
    }
}

fn closed_phase(owner: Phase, observed: Phase, completed: bool) -> Phase {
    match owner {
        // The live owner has closed. Consent cannot be transferred into its
        // replacement; explicit rejoin creates a fresh, unchecked consent gate.
        Phase::Waiting(calls::Gate::RecordingConsent) => {
            Phase::Waiting(calls::Gate::RecordingConsentRejoin)
        }
        Phase::Waiting(gate)
            if !matches!(gate, calls::Gate::PolicyStale | calls::Gate::PolicyUnknown) =>
        {
            owner
        }
        _ if matches!(observed, Phase::Waiting(calls::Gate::PolicyUnavailable(_))) => observed,
        Phase::Waiting(_) => owner,
        _ if completed => Phase::Ended,
        Phase::Reconnecting => Phase::Reconnecting,
        _ => Phase::Waiting(calls::Gate::PolicyStale),
    }
}
#[cfg(test)]
mod policy_gate_tests {
    use super::*;
    #[test]
    fn a_short_network_blip_keeps_the_call_but_old_evidence_or_rejection_does_not() {
        let fresh = Duration::from_secs(4);
        let old = QUIET_REFRESH_WINDOW;
        assert_eq!(
            refresh_failure(true, 0, fresh),
            RefreshFailure::RetryQuietly
        );
        assert_eq!(
            refresh_failure(true, 3, fresh),
            RefreshFailure::RetryQuietly
        );
        assert_eq!(refresh_failure(true, 3, old), RefreshFailure::RetryAndGate);
        assert_eq!(refresh_failure(false, 0, fresh), RefreshFailure::Stop);
        assert_eq!(
            refresh_failure(true, MAX_REFRESH_FAILURES, fresh),
            RefreshFailure::Stop
        );
        // The quiet window must stay inside the lease the evidence relies on.
        assert!(
            QUIET_REFRESH_WINDOW.as_millis()
                < u128::from(
                    talk_core::policy::MAX_SOURCE_AGE_MS - talk_core::policy::SOURCE_ALLOWANCE_MS
                )
        );
    }
    #[test]
    fn one_unusable_remote_candidate_does_not_end_the_call() {
        for error in [
            EngineError::Engine,
            EngineError::Malformed,
            EngineError::Overloaded,
            EngineError::Unsupported,
        ] {
            assert_eq!(remote_candidate(Err(error)), Ok(()));
        }
        assert_eq!(remote_candidate(Ok(())), Ok(()));
        assert!(remote_candidate(Err(EngineError::Closed)).is_err());
    }
    #[test]
    fn shutdown_preserves_policy_evidence_and_new_restrictive_decisions() {
        let missing = Phase::Waiting(calls::Gate::PolicyUnavailable(
            calls::PolicyEvidence::SignalingSettings,
        ));
        assert_eq!(closed_phase(Phase::Active, missing, true), missing);
        assert_eq!(
            closed_phase(Phase::Waiting(calls::Gate::PolicyStale), missing, false),
            missing
        );
        for gate in [
            calls::Gate::RequiredEncryption,
            calls::Gate::ClassifiedRecording,
        ] {
            let denied = Phase::Waiting(gate);
            assert_eq!(closed_phase(denied, missing, false), denied);
        }
        assert_eq!(
            closed_phase(
                Phase::Waiting(calls::Gate::RecordingConsent),
                missing,
                false
            ),
            Phase::Waiting(calls::Gate::RecordingConsentRejoin)
        );
        assert_eq!(
            closed_phase(Phase::Active, Phase::Active, true),
            Phase::Ended
        );
    }
}
