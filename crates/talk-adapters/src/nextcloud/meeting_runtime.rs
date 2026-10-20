//! Autonomous normal-call owner. Signaling polling, policy/HTTP writes and media
//! have separate bounded lanes; none is driven by GPUI rendering or chat polling.
use super::{
    call_policy::{Bundle, Sources},
    transport::NetworkRuntime,
};
use crate::media::{
    MediaService,
    engine::{Event as EngineEvent, NativePeer},
    service::Session,
};
use libwebrtc::{
    peer_connection::PeerConnectionState,
    peer_connection_factory::{IceServer, RtcConfiguration},
    video_frame::VideoBuffer,
};
use nextcloud_talk_api::{
    self as api,
    meeting::{MeetingReader, MeetingStop, MeetingWriter, Observation},
    models::ParticipantFlags,
    signaling::{
        DescriptionKind, Event as SignalEvent, Negotiation, PeerSession, Signal, SignalBatch,
    },
};
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
    external_signaling: false,
    end_to_end_encryption: false,
};
pub(crate) const DEVICES: DevicePermissions = DevicePermissions {
    microphone: true,
    camera: true,
    screen_session: false,
};

enum Write {
    Select(PeerSession, Negotiation),
    Remove(PeerSession),
    Signal(PeerSession, Negotiation, Signal),
    Flags(MediaWishes),
}
struct Peer {
    local: u128,
    status: Option<MediaWishes>,
    session: PeerSession,
    negotiation: Option<Negotiation>,
    engine: NativePeer,
    remote: bool,
    candidates: Vec<api::signaling::IceCandidate>,
    connected: bool,
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
            scope,
            session.clone(),
            network.clone(),
            cancel.clone(),
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
async fn refresh(
    writer: &mut MeetingWriter,
    scope: CallScope,
    network: &NetworkRuntime,
    cancel: &Cancellation,
) -> Result<Bundle, api::ErrorKind> {
    let mut sources = Sources::new(scope, writer.own_session()?);
    for observation in OBSERVATIONS {
        let value = writer
            .observe(&context(scope, network, cancel, 3000), observation)
            .await
            .map_err(|e| e.kind)?;
        sources.add(observation, value);
    }
    sources.finish(true)
}
async fn write_loop(
    mut writer: MeetingWriter,
    mut commands: mpsc::Receiver<Write>,
    policy: mpsc::Sender<Bundle>,
    scope: CallScope,
    session: Arc<Session>,
    network: NetworkRuntime,
    cancel: Cancellation,
) {
    let mut refresh_at = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    while session.live() {
        tick.tick().await;
        if Instant::now() >= refresh_at {
            match refresh(&mut writer, scope, &network, &cancel).await {
                Ok(bundle) => {
                    if policy.try_send(bundle).is_err() {
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
        for _ in 0..16 {
            let Ok(command) = commands.try_recv() else {
                break;
            };
            let result = match command {
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
    }
    session.close();
    let _ = writer.leave(&context(scope, &network, &cancel, 3000)).await;
}
fn configuration(bundle: &Bundle) -> RtcConfiguration {
    let mut config = RtcConfiguration::default();
    config.ice_servers = bundle
        .settings
        .ice_servers
        .iter()
        .map(|s| IceServer {
            urls: s.urls.clone(),
            username: s.username.to_string(),
            password: s.credential.to_string(),
        })
        .collect();
    config
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
) -> Result<(), ()> {
    let mut i = 0;
    while i < peers.len() {
        if !bundle.peers.iter().any(|p| p.session == peers[i].session) {
            let mut p = peers.remove(i);
            p.engine.close();
            send(tx, Write::Remove(p.session))?;
        } else {
            i += 1;
        }
    }
    for incoming in &bundle.peers {
        if peers.iter().any(|p| p.session == incoming.session) {
            continue;
        }
        let mut engine = NativePeer::new(configuration(bundle)).map_err(|_| ())?;
        let offer =
            local_media && (own.local_should_offer(&incoming.session) || !incoming.publish_media);
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
            None
        };
        let mut identity = [0; 16];
        getrandom::fill(&mut identity).map_err(|_| ())?;
        peers.push(Peer {
            local: u128::from_le_bytes(identity),
            status: None,
            session: incoming.session.clone(),
            negotiation,
            engine,
            remote: false,
            candidates: Vec::with_capacity(64),
            connected: false,
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
    mut policies: mpsc::Receiver<Bundle>,
    session: Arc<Session>,
    network: NetworkRuntime,
    cancel: Cancellation,
    stop: MeetingStop,
) {
    let scope = call.scope();
    let started = Instant::now();
    let mut peers = Vec::with_capacity(16);
    let mut tick = tokio::time::interval(Duration::from_millis(10));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_flags = call.wishes();
    let mut refreshes = 1u64;
    let mut stage = "initial peer reconciliation";
    let result:Result<(),()>=async {
      reconcile(&mut peers,&bundle,&own,&writes,session.publication().microphone||session.publication().camera).await?;
      while session.live()&&!stop.is_closed()&&!cancel.is_cancelled(){
        tick.tick().await;
        while let Ok(command)=control.try_recv(){match command {
            Command::Change{wishes,..}=>{call.set_wishes(scope,wishes).map_err(|_|())?;},
            Command::Consent(_)=>{call.consent(scope).map_err(|_|())?;},
            Command::Reconnect(_)=>{call.disconnected(scope).map_err(|_|())?;for mut p in peers.drain(..){p.engine.close();send(&writes,Write::Remove(p.session))?;}},
            Command::Stop(_)=>{session.close();break;},_=>{}
        }}
        if let Ok(next)=policies.try_recv(){refreshes=refreshes.saturating_add(1);bundle=next;}
        stage="policy grant";let grant=call.advance(&bundle.inputs,bundle.exists,SUPPORT,DEVICES,network.now());
        let video_limit=match grant{Ok(g)=>{let limit=usize::from(g.remote_video_limit());session.grant(g,network.now());limit},Err(_)=>{session.suspend();for p in &mut peers{p.engine.publish(false,false).map_err(|_|())?;}return Err(());}};
        reconcile(&mut peers,&bundle,&own,&writes,session.publication().microphone||session.publication().camera).await?;
        stage="remote signaling";while let Ok(batch)=signals.try_recv(){for event in batch.into_events(){match event{
            SignalEvent::Signal{from,negotiation,signal}=>{
                let Some(p)=peers.iter_mut().find(|p|p.session==from)else{continue;};
                if p.negotiation.as_ref().is_some_and(|n|*n!=negotiation){continue;}
                if p.negotiation.is_none(){
                    if !matches!(&signal,Signal::Description(s) if s.kind()==DescriptionKind::Offer){continue;}
                    p.negotiation=Some(negotiation.clone());send(&writes,Write::Select(from.clone(),negotiation.clone()))?;
                }
                match signal {
                    Signal::Description(sdp)=>{if let Some(answer)=p.engine.remote(&sdp).await.map_err(|_|())?{send(&writes,Write::Signal(from,negotiation,Signal::Description(answer)))?;}p.remote=true;for c in p.candidates.drain(..){p.engine.candidate(&c).await.map_err(|_|())?;}},
                    Signal::Candidate(candidate)=>{if p.remote{p.engine.candidate(&candidate).await.map_err(|_|())?;}else if p.candidates.len()<64{p.candidates.push(candidate);}else{return Err(());}}
                }
            },
            SignalEvent::Membership(members)=>{
                if !members.iter().any(|m|m.session==own&&m.in_call.is_some_and(|f|f&1!=0)){return Err(());}
                for p in &mut peers{if !members.iter().any(|m|m.session==p.session&&m.in_call.is_some_and(|f|f&1!=0)){p.engine.close();}}
                let mut i=0;while i<peers.len(){if !peers[i].engine.is_open(){let p=peers.remove(i);send(&writes,Write::Remove(p.session))?;}else{i+=1;}}
            },SignalEvent::Hint{..}=>{}
        }}}
        stage="media exchange";let wishes=session.publication();let audio=session.take_audio();let video=session.take_video();let mut totals=MediaCounters::default();let mut any_connected=false;let mut mix=[0i32;480];let mut mixed=false;
        for (index,p) in peers.iter_mut().enumerate(){
            p.engine.receive_video(index<video_limit);
            if p.negotiation.is_none()&&(wishes.microphone||wishes.camera)&&bundle.peers.iter().find(|source|source.session==p.session).is_some_and(|source|own.local_should_offer(&p.session)||!source.publish_media){
                let n=sid()?;send(&writes,Write::Select(p.session.clone(),n.clone()))?;let sdp=p.engine.offer().await.map_err(|_|())?;send(&writes,Write::Signal(p.session.clone(),n.clone(),Signal::Description(sdp)))?;p.negotiation=Some(n);
            }
            p.engine.publish(wishes.microphone,wishes.camera).map_err(|_e|{#[cfg(feature="backend-test")] eprintln!("normal publish: {_e:?}");})?;
            while let Some(event)=p.engine.event().map_err(|_e|{#[cfg(feature="backend-test")] eprintln!("normal event: {_e:?}");})?{match event{
                EngineEvent::Candidate(c)=>if let Some(n)=&p.negotiation{send(&writes,Write::Signal(p.session.clone(),n.clone(),Signal::Candidate(c)))?;},
                EngineEvent::Connection(state)=>{p.connected=state==PeerConnectionState::Connected;if matches!(state,PeerConnectionState::Failed|PeerConnectionState::Disconnected|PeerConnectionState::Closed){call.disconnected(scope).map_err(|_|())?;session.suspend();p.engine.close();return Err(());}},_=>{}
            }}
            if p.connected && let Some(samples)=&audio{p.engine.audio(samples).await.map_err(|_e|{#[cfg(feature="backend-test")] eprintln!("normal audio: {_e:?}");})?;}
            if p.connected && let Some(frame)=&video{
                let mut copy=libwebrtc::video_frame::I420Buffer::new(frame.width(),frame.height());let (sy,su,sv)=frame.data();let(dy,du,dv)=copy.data_mut();dy.copy_from_slice(sy);du.copy_from_slice(su);dv.copy_from_slice(sv);p.engine.video(copy,started.elapsed().as_micros() as i64).map_err(|_e|{#[cfg(feature="backend-test")] eprintln!("normal video: {_e:?}");})?;
            }
            if let Some(frame)=p.engine.take_audio() && frame.data.len()==480{for (out,input) in mix.iter_mut().zip(frame.data.iter()){*out+=i32::from(*input);}mixed=true;}
            while let Some(frame)=p.engine.take_video(){session.show_video(p.local,frame);}
            if p.connected && p.status!=Some(wishes){
                let audio=serde_json::json!({"type":if wishes.microphone{"audioOn"}else{"audioOff"}}).to_string();
                let video=serde_json::json!({"type":if wishes.camera{"videoOn"}else{"videoOff"}}).to_string();
                if p.engine.send_status(audio.as_bytes()).is_ok()&&p.engine.send_status(video.as_bytes()).is_ok(){p.status=Some(wishes);}
            }
            any_connected|=p.connected;
            let c=p.engine.counters();totals.audio_sent+=c.audio_sent;totals.audio_received+=c.audio_received;totals.video_sent+=c.video_sent;totals.video_received+=c.video_received;totals.data_received+=c.data_received;totals.offers_sent+=c.offers_sent;totals.answers_sent+=c.answers_sent;
        }
        totals.policy_refreshes=refreshes;
        let mut participants=talk_core::bounded::Page::empty();for p in &peers{if let Some(source)=bundle.peers.iter().find(|s|s.session==p.session){participants.try_push(calls::Participant{local:p.local,name:source.name.clone(),microphone:source.microphone,camera:source.camera}).map_err(|_|())?;}}
        session.retain_video(&peers.iter().take(video_limit).map(|p|p.local).collect::<Vec<_>>());
        if mixed{let mut output=[0i16;480];for (out,input) in output.iter_mut().zip(mix){*out=input.clamp(i16::MIN as i32,i16::MAX as i32) as i16;}session.play_audio(&output);}
        if wishes!=last_flags{send(&writes,Write::Flags(wishes))?;last_flags=wishes;}
        if any_connected{call.connected(scope,network.now()).map_err(|_|())?;}
        if let Ok(mut snapshot)=session.snapshot.lock(){let (mic,camera)=session.actual();let (mic_failed,camera_failed)=session.failed();snapshot.phase=call.phase();snapshot.microphone=if mic_failed{DeviceState::Unavailable}else if mic{DeviceState::Active}else if wishes.microphone{DeviceState::Requested}else{DeviceState::Off};snapshot.camera=if camera_failed{DeviceState::Unavailable}else if camera{DeviceState::Active}else if wishes.camera{DeviceState::Requested}else{DeviceState::Off};snapshot.peers=peers.len() as u16;snapshot.participants=participants;snapshot.counters=totals;snapshot.recording=match bundle.inputs.room{Observed::Known{value,..}=>value.recording,_=>RecordingState::Unknown};snapshot.revision=snapshot.revision.saturating_add(1);}
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
