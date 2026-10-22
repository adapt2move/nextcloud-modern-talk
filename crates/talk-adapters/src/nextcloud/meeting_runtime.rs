//! Autonomous normal-call owner. Signaling polling, policy/HTTP writes and media
//! have separate bounded lanes; none is driven by GPUI rendering or chat polling.
mod status;

use super::{
    call_policy::{Bundle, Sources},
    transport::NetworkRuntime,
};
use crate::media::engine::{IceServer, PeerConnectionState, Role, RtcConfiguration};
use crate::media::{
    MediaService,
    engine::{Event as EngineEvent, NativePeer},
    service::Session,
};
use nextcloud_talk_api::{
    self as api,
    meeting::{MeetingReader, MeetingStop, MeetingWriter, Observation, ObservationError, Topology},
    models::ParticipantFlags,
    signaling::{
        DescriptionKind, Event as SignalEvent, Negotiation, PeerSession, Signal, SignalBatch,
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
use tokio::sync::mpsc;
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
    screen: false,
    external_signaling: true,
    end_to_end_encryption: false,
};
pub(crate) const DEVICES: DevicePermissions = DevicePermissions {
    microphone: true,
    camera: true,
    screen_session: false,
};

enum Write {
    Subscribe(PeerSession),
    Refresh(u64),
    Select(PeerSession, Negotiation),
    Remove(PeerSession),
    Signal(PeerSession, Negotiation, Signal),
    Flags(MediaWishes),
}
struct PolicyUpdate {
    bundle: Bundle,
    after: u64,
}
struct Peer {
    local: u128,
    status: StatusSnapshot,
    session: PeerSession,
    negotiation: Option<Negotiation>,
    engine: NativePeer,
    remote: bool,
    candidates: Vec<api::signaling::IceCandidate>,
    connected: bool,
    role: Role,
    subscription_requested: bool,
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
    control: mpsc::Sender<Command>,
    stop: MeetingStop,
}
impl LiveCall {
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
        if matches!(command, Command::Observe(_)) {
            return Ok(());
        }
        if matches!(command, Command::Stop(_)) {
            self.session.close();
            self.stop.close();
            return Ok(());
        }
        // Close publication synchronously on any change; stale callbacks cannot
        // extend the old grant while the owner rechecks policy/user intent.
        if !matches!(command, Command::Observe(_)) {
            self.session.suspend();
        }
        self.control
            .try_send(command)
            .map_err(|_| api::ErrorKind::Busy)
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
        writer: MeetingWriter,
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
            call: scope,
            entry: call.entry(),
            participants: talk_core::bounded::Page::empty(),
            phase: Phase::Connecting,
            microphone: DeviceState::Off,
            camera: DeviceState::Off,
            screen: DeviceState::Unavailable,
            peers: 0,
            recording: RecordingState::Unknown,
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
        let own = writer.own_session()?;
        let topology = writer.topology();
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
            session.clone(),
            network,
            cancel,
            stop.clone(),
            topology,
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
pub(super) async fn refresh(
    writer: &mut MeetingWriter,
    scope: CallScope,
    network: &NetworkRuntime,
    cancel: &Cancellation,
    account_rejection: Option<&super::AccountRejection>,
) -> Result<Bundle, api::ErrorKind> {
    let mut sources = Sources::new(scope, writer.own_session()?);
    for observation in OBSERVATIONS {
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
                kind
            })?;
        sources.add(observation, value);
    }
    sources.finish(true)
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
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    while session.live() {
        tick.tick().await;
        for _ in 0..16 {
            let Ok(command) = commands.try_recv() else {
                break;
            };
            let result = match command {
                Write::Refresh(after) => {
                    requested = requested.max(after);
                    refresh_at = Instant::now();
                    Ok(())
                }
                Write::Subscribe(peer) => {
                    writer
                        .request_offer(&context(scope, &network, &cancel, 3000), &peer)
                        .await
                }
                Write::Select(peer, sid) => writer.select(peer, sid),
                Write::Remove(peer) => {
                    writer.remove(&peer);
                    Ok(())
                }
                Write::Signal(peer, sid, signal) => writer
                    .send(
                        &context(scope, &network, &cancel, 3000),
                        &peer,
                        &sid,
                        signal,
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
                #[cfg(feature = "backend-test")]
                eprintln!("normal write stopped: {:?}", _error.kind);
                session.close();
                break;
            }
        }
        if Instant::now() >= refresh_at {
            match refresh(
                &mut writer,
                scope,
                &network,
                &cancel,
                account_rejection.as_ref(),
            )
            .await
            {
                Ok(bundle) => {
                    if policy
                        .try_send(PolicyUpdate {
                            bundle,
                            after: requested,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                Err(_error) => {
                    #[cfg(feature = "backend-test")]
                    eprintln!("normal refresh stopped: {:?}", _error);
                    break;
                }
            }
            refresh_at = Instant::now() + Duration::from_secs(5);
        }
    }
    session.close();
    let _ = writer.leave(&context(scope, &network, &cancel, 3000)).await;
}
fn configuration(bundle: &Bundle) -> RtcConfiguration {
    RtcConfiguration {
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
fn sid() -> Result<Negotiation, ()> {
    let mut value = [0; 16];
    getrandom::fill(&mut value).map_err(|_| ())?;
    Negotiation::parse(&value.iter().map(|b| format!("{b:02x}")).collect::<String>())
        .map_err(|_| ())
}
fn send(tx: &mpsc::Sender<Write>, value: Write) -> Result<(), ()> {
    tx.try_send(value).map_err(|_| ())
}
async fn reconcile(
    peers: &mut Vec<Peer>,
    bundle: &Bundle,
    own: &PeerSession,
    tx: &mpsc::Sender<Write>,
    local_media: bool,
    topology: Topology,
    sfu_members: &[PeerSession],
) -> Result<(), ()> {
    let mut i = 0;
    while i < peers.len() {
        if peers[i].role != Role::Publisher
            && (!bundle.peers.iter().any(|p| p.session == peers[i].session)
                || (topology == Topology::Sfu
                    && (!sfu_members.contains(&peers[i].session)
                        || !bundle
                            .peers
                            .iter()
                            .any(|p| p.session == peers[i].session && p.publish_media))))
        {
            let mut p = peers.remove(i);
            p.engine.close();
            send(tx, Write::Remove(p.session))?;
        } else {
            i += 1;
        }
    }
    if topology == Topology::Sfu && local_media && !peers.iter().any(|p| p.role == Role::Publisher)
    {
        let mut config = configuration(bundle);
        config.role = Role::Publisher;
        let mut engine = NativePeer::new(config).await.map_err(|_| ())?;
        report_ice_support(&engine);
        let negotiation = sid()?;
        send(tx, Write::Select(own.clone(), negotiation.clone()))?;
        let offer = engine.offer().await.map_err(|_| ())?;
        send(
            tx,
            Write::Signal(own.clone(), negotiation.clone(), Signal::Description(offer)),
        )?;
        peers.push(Peer {
            local: 0,
            status: StatusSnapshot::default(),
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
        if let Some(peer) = peers.iter_mut().find(|p| p.session == incoming.session) {
            if peer.role == Role::Subscriber
                && !peer.subscription_requested
                && incoming.publish_media
                && (incoming.microphone || incoming.camera)
            {
                send(tx, Write::Subscribe(incoming.session.clone()))?;
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
        let mut config = configuration(bundle);
        config.role = role;
        let mut engine = NativePeer::new(config).await.map_err(|_| ())?;
        report_ice_support(&engine);
        let offer = topology == Topology::Direct
            && local_media
            && (own.local_should_offer(&incoming.session) || !incoming.publish_media);
        let subscription_requested =
            topology == Topology::Sfu && (incoming.microphone || incoming.camera);
        let negotiation = if offer {
            let n = sid()?;
            send(tx, Write::Select(incoming.session.clone(), n.clone()))?;
            let sdp = engine.offer().await.map_err(|_| ())?;
            send(
                tx,
                Write::Signal(
                    incoming.session.clone(),
                    n.clone(),
                    Signal::Description(sdp),
                ),
            )?;
            Some(n)
        } else {
            if subscription_requested {
                send(tx, Write::Subscribe(incoming.session.clone()))?;
            }
            None
        };
        let mut identity = [0; 16];
        getrandom::fill(&mut identity).map_err(|_| ())?;
        peers.push(Peer {
            local: u128::from_le_bytes(identity),
            status: StatusSnapshot::default(),
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
    mut control: mpsc::Receiver<Command>,
    mut signals: mpsc::Receiver<SignalBatch>,
    writes: mpsc::Sender<Write>,
    mut policies: mpsc::Receiver<PolicyUpdate>,
    session: Arc<Session>,
    network: NetworkRuntime,
    cancel: Cancellation,
    stop: MeetingStop,
    topology: Topology,
) {
    let scope = call.scope();
    let started = Instant::now();
    let mut peers: Vec<Peer> = Vec::with_capacity(17);
    let mut sfu_members = Vec::with_capacity(100);
    let mut transport_ready = true;
    let mut required_policy = 0u64;
    let mut received_policy = 0u64;
    let mut tick = tokio::time::interval(Duration::from_millis(10));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_flags = call.wishes();
    let mut refreshes = 1u64;
    let mut stage = "initial peer reconciliation";
    let result: Result<(), ()> = async {
        while session.live() && !stop.is_closed() && !cancel.is_cancelled() {
            tick.tick().await;
            let mut refresh_needed = false;
            while let Ok(command) = control.try_recv() {
                match command {
                    Command::Change { wishes, .. } => {
                        call.set_wishes(scope, wishes).map_err(|_| ())?;
                    }
                    Command::Consent(_) => { call.consent(scope).map_err(|_| ())?; }
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
            stage = "remote signaling";
            while let Ok(batch) = signals.try_recv() {
                for event in batch.into_events() {
                    match event {
                        SignalEvent::Signal { from, negotiation, signal } => {
                            let Some(peer) = peers.iter_mut().find(|p| p.session == from) else { continue; };
                            if peer.negotiation.as_ref().is_some_and(|n| *n != negotiation) { continue; }
                            if peer.negotiation.is_none() {
                                if !matches!(&signal, Signal::Description(s) if s.kind() == DescriptionKind::Offer) { continue; }
                                peer.negotiation = Some(negotiation.clone());
                                send(&writes, Write::Select(from.clone(), negotiation.clone()))?;
                            }
                            match signal {
                                Signal::Description(sdp) => {
                                    if let Some(answer) = peer.engine.remote(&sdp).await.map_err(|_| ())? {
                                        send(&writes, Write::Signal(from, negotiation, Signal::Description(answer)))?;
                                    }
                                    peer.remote = true;
                                    for candidate in peer.candidates.drain(..) {
                                        peer.engine.candidate(&candidate).await.map_err(|_| ())?;
                                    }
                                }
                                Signal::Candidate(candidate) => {
                                    if peer.remote {
                                        peer.engine.candidate(&candidate).await.map_err(|_| ())?;
                                    } else if peer.candidates.len() < 64 {
                                        peer.candidates.push(candidate);
                                    } else { return Err(()); }
                                }
                            }
                        }
                        SignalEvent::Membership(members) => {
                            if topology != Topology::Direct { return Err(()); }
                            if !members.iter().any(|m| m.session == own && m.in_call.is_some_and(|f| f & 1 != 0)) { return Err(()); }
                            let mut index = 0;
                            while index < peers.len() {
                                if !members.iter().any(|m| m.session == peers[index].session && m.in_call.is_some_and(|f| f & 1 != 0)) {
                                    let mut peer = peers.remove(index);
                                    peer.engine.close();
                                    send(&writes, Write::Remove(peer.session))?;
                                } else { index += 1; }
                            }
                            refresh_needed = true;
                        }
                        SignalEvent::SfuMembership(members) => {
                            if topology != Topology::Sfu { return Err(()); }
                            // Routing facts can retire an absent route, but may
                            // add no media peer without fresh REST membership.
                            sfu_members = members;
                            refresh_needed = true;
                        }
                        SignalEvent::TransportInterrupted => {
                            transport_ready = false;
                            call.disconnected(scope).map_err(|_| ())?;
                            refresh_needed = true;
                        }
                        SignalEvent::TransportResumed => {
                            transport_ready = true;
                            refresh_needed = true;
                        }
                        SignalEvent::ForceMute => {
                            let mut wishes = call.wishes();
                            wishes.microphone = false;
                            call.set_wishes(scope, wishes).map_err(|_| ())?;
                            refresh_needed = true;
                        }
                        SignalEvent::PolicyChanged | SignalEvent::Hint { .. } => { refresh_needed = true; }
                    }
                }
            }
            if refresh_needed {
                session.suspend();
                required_policy = required_policy.checked_add(1).ok_or(())?;
                send(&writes, Write::Refresh(required_policy))?;
            }
            while let Ok(next) = policies.try_recv() {
                // The request sequence is assigned before the first HTTP
                // observation, not at completion. A delayed earlier refresh
                // cannot restore a grant after a newer invalidation/reconnect.
                if next.after >= required_policy {
                    refreshes = refreshes.saturating_add(1);
                    received_policy = next.after;
                    bundle = next.bundle;
                }
            }
            let policy_ready = transport_ready && received_policy >= required_policy;
            stage = "policy grant";
            let video_limit = if policy_ready {
                let grant = call.advance(&bundle.inputs, bundle.exists, SUPPORT, DEVICES, network.now()).map_err(|_| ())?;
                let limit = usize::from(grant.remote_video_limit());
                session.grant(grant, network.now());
                reconcile(&mut peers, &bundle, &own, &writes,
                    session.publication().microphone || session.publication().camera,
                    topology, &sfu_members).await?;
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
                        send(&writes, Write::Remove(peer.session))?;
                    } else { index += 1; }
                }
            }
            stage = "media exchange";
            let wishes = session.publication();
            let audio = session.take_audio();
            let video = session.take_video();
            let mut totals = MediaCounters::default();
            let mut any_connected = false;
            let mut mix = [0i32; 480];
            let mut mixed = false;
            let mut receive_index = 0;
            for peer in &mut peers {
                let receives = peer.role != Role::Publisher;
                let publishes = peer.role != Role::Subscriber;
                peer.engine.receive_video(receives && receive_index < video_limit);
                if receives { receive_index += 1; }
                if topology == Topology::Direct && policy_ready && peer.negotiation.is_none()
                    && (wishes.microphone || wishes.camera)
                    && bundle.peers.iter().find(|p| p.session == peer.session)
                        .is_some_and(|p| own.local_should_offer(&p.session) || !p.publish_media)
                {
                    let negotiation = sid()?;
                    send(&writes, Write::Select(peer.session.clone(), negotiation.clone()))?;
                    let sdp = peer.engine.offer().await.map_err(|_| ())?;
                    send(&writes, Write::Signal(peer.session.clone(), negotiation.clone(), Signal::Description(sdp)))?;
                    peer.negotiation = Some(negotiation);
                }
                peer.engine.publish(publishes && wishes.microphone, publishes && wishes.camera).map_err(|_| ())?;
                while let Some(event) = peer.engine.event().await.map_err(|error| {
                    #[cfg(feature = "backend-test")]
                    eprintln!("normal media event stopped: {:?} {error:?}", peer.role);
                    let _ = error;
                })? {
                    match event {
                        EngineEvent::Candidate(candidate) => {
                            // An incomplete exchange is never replayed across
                            // a lost transport. Established media can resume.
                            if !transport_ready { return Err(()); }
                            if let Some(negotiation) = &peer.negotiation {
                                send(&writes, Write::Signal(peer.session.clone(), negotiation.clone(), Signal::Candidate(candidate)))?;
                            }
                        }
                        EngineEvent::Connection(state) => {
                            peer.connected = state == PeerConnectionState::Connected;
                            if matches!(state, PeerConnectionState::Failed | PeerConnectionState::Disconnected | PeerConnectionState::Closed) {
                                call.disconnected(scope).map_err(|_| ())?;
                                session.suspend();
                                return Err(());
                            }
                        }
                    }
                }
                if publishes && peer.connected {
                    if let Some(samples) = &audio { peer.engine.audio(samples).await.map_err(|_| ())?; }
                    if let Some(frame) = &video {
                        peer.engine.video(frame.clone(), started.elapsed().as_micros() as i64).map_err(|_| ())?;
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
                        for (output, input) in mix.iter_mut().zip(frame.data.iter()) { *output += i32::from(*input); }
                        mixed = true;
                    }
                    while let Some(frame) = peer.engine.take_video() { session.show_video(peer.local, frame); }
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
            let mut participants = talk_core::bounded::Page::empty();
            for peer in peers.iter().filter(|p| p.role != Role::Publisher) {
                if let Some(source) = bundle.peers.iter().find(|p| p.session == peer.session) {
                    participants.try_push(calls::Participant {
                        local: peer.local, name: source.name.clone(), microphone: source.microphone, camera: source.camera,
                    }).map_err(|_| ())?;
                }
            }
            session.retain_video(&peers.iter().filter(|p| p.role != Role::Publisher).take(video_limit).map(|p| p.local).collect::<Vec<_>>());
            if mixed {
                let mut output = [0i16; 480];
                for (out, input) in output.iter_mut().zip(mix) { *out = input.clamp(i16::MIN as i32, i16::MAX as i32) as i16; }
                session.play_audio(&output);
            }
            if policy_ready && wishes != last_flags {
                send(&writes, Write::Flags(wishes))?;
                last_flags = wishes;
            }
            if policy_ready && any_connected { call.connected(scope, network.now()).map_err(|_| ())?; }
            if let Ok(mut snapshot) = session.snapshot.lock() {
                let (mic, camera) = session.actual();
                let (mic_failed, camera_failed) = session.failed();
                snapshot.phase = call.phase();
                snapshot.microphone = if mic_failed { DeviceState::Unavailable } else if mic { DeviceState::Active } else if wishes.microphone { DeviceState::Requested } else { DeviceState::Off };
                snapshot.camera = if camera_failed { DeviceState::Unavailable } else if camera { DeviceState::Active } else if wishes.camera { DeviceState::Requested } else { DeviceState::Off };
                snapshot.peers = peers.iter().filter(|p| p.role != Role::Publisher).count() as u16;
                snapshot.participants = participants;
                snapshot.counters = totals;
                snapshot.recording = match bundle.inputs.room { Observed::Known { value, .. } => value.recording, _ => RecordingState::Unknown };
                snapshot.revision = snapshot.revision.saturating_add(1);
            }
        }
        Ok(())
    }.await;
    #[cfg(feature = "backend-test")]
    if result.is_err() {
        eprintln!("normal actor stopped: {stage}");
    }
    session.close();
    for mut peer in peers {
        peer.engine.close();
    }
    stop.close();
    if let Ok(mut snapshot) = session.snapshot.lock() {
        snapshot.phase = if result.is_ok() {
            Phase::Ended
        } else if call.phase() == Phase::Reconnecting {
            Phase::Reconnecting
        } else {
            Phase::Waiting(calls::Gate::PolicyStale)
        };
        snapshot.microphone = DeviceState::Off;
        snapshot.camera = DeviceState::Off;
        snapshot.screen = DeviceState::Off;
        snapshot.revision = snapshot.revision.saturating_add(1);
    }
}
