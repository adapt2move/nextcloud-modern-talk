//! Rust WebRTC 0.21 runtime plus safe, maintained GStreamer codecs. The call
//! owner supplies admitted raw capture; this module never opens devices.
#![forbid(unsafe_code)]

#[path = "codecs.rs"]
mod codecs;
#[path = "ice.rs"]
mod ice;
use codecs::Codec;
use futures_util::FutureExt;
pub use ice::{CandidateKind, IceEvidence, IcePolicy, IceSupport, SelectedPair};
use nextcloud_talk_api::signaling as protocol;
use rtc::{
    media::Sample,
    peer_connection::configuration::setting_engine::SctpMaxMessageSize,
    rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication,
    rtp_transceiver::rtp_sender::{
        RTCRtpCodec, RTCRtpCodecParameters, RTCRtpCodingParameters, RTCRtpEncodingParameters,
        RtpCodecKind,
    },
    shared::marshal::Marshal,
};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    time::{Duration, Instant},
};
use webrtc::{
    data_channel::{DataChannel, DataChannelEvent, RTCDataChannelState},
    media_stream::{
        MediaStreamTrack,
        track_local::static_sample::TrackLocalStaticSample,
        track_remote::{TrackRemote, TrackRemoteEvent},
    },
    peer_connection::{
        MediaEngine, PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler,
        RTCBundlePolicy, RTCConfigurationBuilder, RTCIceCandidateInit, RTCIceServer,
        RTCIceTransportPolicy, RTCPeerConnectionIceEvent, RTCPeerConnectionState, RTCSdpType,
        RTCSessionDescription, Registry, SettingEngineBuilder, register_default_interceptors,
    },
    rtp_transceiver::{RTCRtpTransceiverDirection, RTCRtpTransceiverInit, RtpSender},
};

const EVENTS: usize = 136;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Closed,
    Engine,
    Malformed,
    Overloaded,
    Unsupported,
}
#[derive(Default)]
pub struct IceServer {
    pub urls: Vec<String>,
    pub username: String,
    pub password: String,
}
#[derive(Default)]
pub struct RtcConfiguration {
    pub ice_servers: Vec<IceServer>,
    pub ice_policy: IcePolicy,
    pub role: Role,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Role {
    #[default]
    Direct,
    Publisher,
    Subscriber,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeerConnectionState {
    New,
    Connecting,
    Connected,
    Disconnected,
    Failed,
    Closed,
}
pub enum Event {
    Candidate(protocol::IceCandidate),
    Connection(PeerConnectionState),
}
/// Owned packed RGBA, independent of either transport or codec dependency.
#[derive(Clone)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
pub struct AudioFrame {
    pub data: [i16; 480],
}
enum CallbackEvent {
    Public(Event),
    Track(Arc<dyn TrackRemote>),
    Channel(Arc<dyn DataChannel>),
}
#[derive(Clone)]
struct Callback {
    open: Arc<AtomicBool>,
    mid: Arc<Mutex<String>>,
    events: SyncSender<CallbackEvent>,
}
impl Callback {
    fn send(&self, event: CallbackEvent) {
        if self.open.load(Ordering::Acquire) && self.events.try_send(event).is_err() {
            self.open.store(false, Ordering::Release);
        }
    }
}
#[async_trait::async_trait]
impl PeerConnectionEventHandler for Callback {
    async fn on_ice_candidate(&self, event: RTCPeerConnectionIceEvent) {
        if !self.open.load(Ordering::Acquire) {
            return;
        }
        let parsed = event.candidate.to_json().ok().and_then(|c| {
            // rtc 0.21.1 to_json leaves sdpMid empty. Our single bundled
            // transport belongs to the first media section of the local SDP.
            let mid = self.mid.lock().ok()?;
            protocol::IceCandidate::new(&mid, 0, &c.candidate).ok()
        });
        if let Some(c) = parsed {
            self.send(CallbackEvent::Public(Event::Candidate(c)));
        } else {
            self.open.store(false, Ordering::Release);
        }
    }
    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        let state = match state {
            RTCPeerConnectionState::New => PeerConnectionState::New,
            RTCPeerConnectionState::Connecting => PeerConnectionState::Connecting,
            RTCPeerConnectionState::Connected => PeerConnectionState::Connected,
            RTCPeerConnectionState::Disconnected => PeerConnectionState::Disconnected,
            RTCPeerConnectionState::Failed => PeerConnectionState::Failed,
            RTCPeerConnectionState::Closed => PeerConnectionState::Closed,
            _ => return,
        };
        self.send(CallbackEvent::Public(Event::Connection(state)));
    }
    async fn on_track(&self, track: Arc<dyn TrackRemote>) {
        self.send(CallbackEvent::Track(track));
    }
    async fn on_data_channel(&self, channel: Arc<dyn DataChannel>) {
        self.send(CallbackEvent::Channel(channel));
    }
}
#[derive(Default)]
pub struct Counters {
    pub offers_sent: u64,
    pub answers_sent: u64,
    pub audio_sent: u64,
    pub video_sent: u64,
    pub audio_received: u64,
    pub audio_nonzero: u64,
    pub video_received: u64,
    pub data_received: u64,
}
struct Outgoing {
    track: Arc<TrackLocalStaticSample>,
    sender: Arc<dyn RtpSender>,
    ssrc: u32,
    encoder: Option<Codec>,
    enabled: bool,
    clock: Duration,
}
impl Outgoing {
    async fn new(
        pc: &dyn PeerConnection,
        kind: RtpCodecKind,
        codec: RTCRtpCodec,
        role: Role,
    ) -> Result<Self, Error> {
        let mut id = [0; 4];
        getrandom::fill(&mut id).map_err(|_| Error::Engine)?;
        let ssrc = u32::from_le_bytes(id).max(1);
        let track = Arc::new(
            TrackLocalStaticSample::new(
                Instant::now(),
                MediaStreamTrack::new(
                    "nmt".into(),
                    kind.to_string(),
                    kind.to_string(),
                    kind,
                    vec![RTCRtpEncodingParameters {
                        rtp_coding_parameters: RTCRtpCodingParameters {
                            ssrc: Some(ssrc),
                            ..Default::default()
                        },
                        codec,
                        ..Default::default()
                    }],
                ),
            )
            .map_err(|_| Error::Engine)?,
        );
        let sender = if role == Role::Publisher {
            pc.add_transceiver_from_track(
                track.clone(),
                Some(RTCRtpTransceiverInit {
                    direction: RTCRtpTransceiverDirection::Sendonly,
                    ..Default::default()
                }),
            )
            .await
            .map_err(|_| Error::Engine)?
            .sender()
            .await
            .map_err(|_| Error::Engine)?
            .ok_or(Error::Engine)?
        } else {
            pc.add_track(track.clone())
                .await
                .map_err(|_| Error::Engine)?
        };
        Ok(Self {
            track,
            sender,
            ssrc,
            encoder: None,
            enabled: false,
            clock: Duration::ZERO,
        })
    }
    fn publish(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.encoder = None;
        }
    }
    async fn flush(&self, max: usize) -> Result<(), Error> {
        if !self.enabled {
            return Ok(());
        }
        let Some(encoder) = &self.encoder else {
            return Ok(());
        };
        // Only the owning call task can write, and each tick drains at most the
        // bounded encoder output queue. No send task can outlive publication.
        for _ in 0..3 {
            let Some((data, duration)) = encoder.encoded(max)? else {
                break;
            };
            let parameters = self
                .sender
                .get_parameters()
                .await
                .map_err(|_| Error::Engine)?;
            let payload = parameters
                .rtp_parameters
                .codecs
                .first()
                .ok_or(Error::Engine)?
                .payload_type;
            let sample = Sample {
                data: data.into(),
                duration,
                ..Sample::new(Instant::now())
            };
            tokio::time::timeout(
                Duration::from_millis(50),
                self.track.write_sample(self.ssrc, payload, &sample, &[]),
            )
            .await
            .map_err(|_| Error::Engine)?
            .map_err(|_| Error::Engine)?;
        }
        Ok(())
    }
}
struct Incoming {
    track: Arc<dyn TrackRemote>,
    decoder: Option<Codec>,
    ssrc: u32,
    video: bool,
    pli: Instant,
}
impl Incoming {
    async fn pump(&mut self, enabled: bool) -> Result<(), Error> {
        if !enabled {
            self.decoder = None;
        }
        for _ in 0..64 {
            let Some(event) = self.track.poll().now_or_never() else {
                break;
            };
            match event {
                Some(TrackRemoteEvent::OnRtpPacket(packet)) if enabled => {
                    if packet.payload.len() > 4096 {
                        return Err(Error::Overloaded);
                    }
                    if self.decoder.is_none() {
                        self.decoder =
                            Some(Codec::decoder(self.video, packet.header.payload_type)?);
                    }
                    let bytes = packet.marshal().map_err(|_| Error::Malformed)?;
                    self.decoder
                        .as_ref()
                        .ok_or(Error::Engine)?
                        .push(bytes.to_vec(), None, None)?;
                }
                None
                | Some(
                    TrackRemoteEvent::OnError
                    | TrackRemoteEvent::OnEnding
                    | TrackRemoteEvent::OnEnded,
                ) => return Err(Error::Closed),
                _ => {}
            }
        }
        if self.video && enabled && self.pli.elapsed() >= Duration::from_secs(2) {
            tokio::time::timeout(
                Duration::from_millis(50),
                self.track.write_rtcp(vec![Box::new(PictureLossIndication {
                    sender_ssrc: 0,
                    media_ssrc: self.ssrc,
                })]),
            )
            .await
            .map_err(|_| Error::Engine)?
            .map_err(|_| Error::Engine)?;
            self.pli = Instant::now();
        }
        Ok(())
    }
}
struct Channel {
    inner: Arc<dyn DataChannel>,
    label: String,
}
pub struct NativePeer {
    pc: Option<Box<dyn PeerConnection>>,
    open: Arc<AtomicBool>,
    mid: Arc<Mutex<String>>,
    events: Receiver<CallbackEvent>,
    audio: Option<Outgoing>,
    video: Option<Outgoing>,
    remote_audio: Option<Incoming>,
    remote_video: Option<Incoming>,
    receive_video: bool,
    channels: Vec<Channel>,
    pcm: VecDeque<[i16; 480]>,
    counters: Counters,
    remote_descriptions: u8,
    remote_candidates: u16,
    local_candidates: u16,
    role: Role,
    ice_support: IceSupport,
    ice_evidence: IceEvidence,
}
impl NativePeer {
    pub fn ice_support(&self) -> &IceSupport {
        &self.ice_support
    }
    /// Counts observed gathered candidates and queries the engine's *selected*
    /// pair. An allocated relay candidate alone is not evidence of relay use.
    #[allow(
        dead_code,
        reason = "Queried by the native ICE qualification executable"
    )]
    pub async fn ice_evidence(&self) -> Result<IceEvidence, Error> {
        self.check()?;
        let mut evidence = self.ice_evidence;
        let pc = self.pc()?;
        let mut dtls = pc.sctp().await.map(|s| s.transport());
        if dtls.is_none()
            && let Some(audio) = &self.audio
        {
            dtls = audio.sender.transport().await.map_err(|_| Error::Engine)?;
        }
        if dtls.is_none()
            && let Some(receiver) = pc.get_receivers().await.first()
        {
            dtls = receiver.transport().await.map_err(|_| Error::Engine)?;
        }
        if let Some(dtls) = dtls {
            let pair = dtls
                .ice_transport()
                .get_selected_candidate_pair()
                .await
                .map_err(|_| Error::Engine)?;
            if let Some(pair) = pair {
                use webrtc::peer_connection::{RTCIceCandidateType as T, RTCIceProtocol};
                let kind = |candidate| match candidate {
                    T::Host => CandidateKind::Host,
                    T::Srflx => CandidateKind::ServerReflexive,
                    T::Prflx => CandidateKind::PeerReflexive,
                    T::Relay => CandidateKind::Relay,
                    _ => CandidateKind::Unknown,
                };
                evidence.selected = Some(SelectedPair {
                    local: kind(pair.local().typ),
                    remote: kind(pair.remote().typ),
                    udp: pair.local().protocol == RTCIceProtocol::Udp
                        && pair.remote().protocol == RTCIceProtocol::Udp,
                });
            }
        }
        Ok(evidence)
    }
    /// Allocates transport/tracks, never devices. Publication begins disabled.
    pub async fn new(config: RtcConfiguration) -> Result<Self, Error> {
        if config.ice_servers.len() > 8
            || config.ice_servers.iter().any(|s| {
                s.urls.len() > 4
                    || s.urls.iter().any(|u| u.len() > 2048)
                    || s.username.len() > 1024
                    || s.password.len() > 2048
            })
        {
            return Err(Error::Malformed);
        }
        let role = config.role;
        let (ice_servers, ice_support) = ice::plan(config.ice_servers, config.ice_policy)?;
        let configuration = RTCConfigurationBuilder::default()
            .with_bundle_policy(RTCBundlePolicy::MaxBundle)
            .with_ice_transport_policy(match config.ice_policy {
                IcePolicy::All => RTCIceTransportPolicy::All,
                IcePolicy::Relay => RTCIceTransportPolicy::Relay,
            })
            .with_ice_servers(
                ice_servers
                    .into_iter()
                    .map(|s| RTCIceServer {
                        urls: s.urls,
                        username: s.username,
                        credential: s.password,
                    })
                    .collect(),
            )
            .build();
        let audio_codec = RTCRtpCodec {
            mime_type: "audio/opus".into(),
            clock_rate: 48_000,
            channels: 2,
            sdp_fmtp_line: "minptime=10;useinbandfec=1".into(),
            ..Default::default()
        };
        let video_codec = RTCRtpCodec {
            mime_type: "video/VP8".into(),
            clock_rate: 90_000,
            ..Default::default()
        };
        let mut media = MediaEngine::default();
        for (kind, codec, payload_type) in [
            (RtpCodecKind::Audio, audio_codec.clone(), 111),
            (RtpCodecKind::Video, video_codec.clone(), 96),
        ] {
            media
                .register_codec(
                    RTCRtpCodecParameters {
                        rtp_codec: codec,
                        payload_type,
                    },
                    kind,
                )
                .map_err(|_| Error::Engine)?;
        }
        let registry = register_default_interceptors(Registry::new(), &mut media)
            .map_err(|_| Error::Engine)?;
        let open = Arc::new(AtomicBool::new(true));
        let (sender, events) = mpsc::sync_channel(EVENTS);
        let mid = Arc::new(Mutex::new("0".to_owned()));
        let settings = SettingEngineBuilder::new()
            .with_sctp_max_message_size(SctpMaxMessageSize::Bounded(4096))
            .with_sctp_max_receive_buffer_size(65_536)
            .build();
        // In 0.21.1 the pooled driver's Drop has a durable shutdown latch. The
        // default nonpooled driver's Drop detaches; explicitly select the pool so
        // synchronous call revocation also terminates transport on every path.
        let pc = tokio::time::timeout(
            Duration::from_secs(3),
            PeerConnectionBuilder::new()
                .with_configuration(configuration)
                .with_media_engine(media)
                .with_interceptor_registry(registry)
                .with_setting_engine(settings)
                .with_data_channel_send_buffer_limit(8192)
                .with_dedicated_reactor_pool_size(2)
                .with_udp_addrs(vec!["0.0.0.0:0"])
                .with_handler(Arc::new(Callback {
                    open: open.clone(),
                    mid: mid.clone(),
                    events: sender,
                }))
                .build(),
        )
        .await
        .map_err(|_| Error::Engine)?
        .map_err(|_| Error::Engine)?;
        let (audio, video) = if role == Role::Subscriber {
            (None, None)
        } else {
            (
                Some(Outgoing::new(&pc, RtpCodecKind::Audio, audio_codec, role).await?),
                Some(Outgoing::new(&pc, RtpCodecKind::Video, video_codec, role).await?),
            )
        };
        let mut peer = Self {
            pc: Some(Box::new(pc)),
            open,
            mid,
            events,
            audio,
            video,
            remote_audio: None,
            remote_video: None,
            receive_video: true,
            channels: Vec::with_capacity(2),
            pcm: VecDeque::with_capacity(2),
            counters: Counters::default(),
            remote_descriptions: 0,
            remote_candidates: 0,
            local_candidates: 0,
            role,
            ice_support,
            ice_evidence: IceEvidence::default(),
        };
        if role != Role::Direct {
            // Talk 25 webrtc.js creates "status" for every SFU publisher and
            // subscriber before SDP. Received Janus relays use a separate
            // channel rather than the remote browser's original DCEP label.
            let channel = peer
                .pc()?
                .create_data_channel("status", None)
                .await
                .map_err(|_| Error::Engine)?;
            peer.add_channel(channel).await?;
        }
        Ok(peer)
    }
    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }
    fn check(&self) -> Result<(), Error> {
        if self.is_open() {
            Ok(())
        } else {
            Err(Error::Closed)
        }
    }
    fn pc(&self) -> Result<&dyn PeerConnection, Error> {
        self.pc.as_deref().ok_or(Error::Closed)
    }
    pub fn publish(&mut self, microphone: bool, camera: bool) -> Result<(), Error> {
        self.check()?;
        if self.role == Role::Subscriber {
            return if microphone || camera {
                Err(Error::Unsupported)
            } else {
                Ok(())
            };
        }
        self.audio
            .as_mut()
            .ok_or(Error::Closed)?
            .publish(microphone);
        self.video.as_mut().ok_or(Error::Closed)?.publish(camera);
        Ok(())
    }
    pub async fn offer(&mut self) -> Result<protocol::SessionDescription, Error> {
        self.check()?;
        if self.role == Role::Subscriber {
            return Err(Error::Unsupported);
        }
        if self.channels.is_empty()
            || (self.role == Role::Publisher
                && !self.channels.iter().any(|c| c.label == "simplewebrtc"))
        {
            let channel = self
                .pc()?
                .create_data_channel("simplewebrtc", None)
                .await
                .map_err(|_| Error::Engine)?;
            self.add_channel(channel).await?;
        }
        let description =
            tokio::time::timeout(Duration::from_secs(3), self.pc()?.create_offer(None))
                .await
                .map_err(|_| Error::Engine)?
                .map_err(|_| Error::Engine)?;
        let result = self.local(description).await?;
        self.counters.offers_sent += 1;
        Ok(result)
    }
    async fn local(
        &mut self,
        description: RTCSessionDescription,
    ) -> Result<protocol::SessionDescription, Error> {
        self.check()?;
        let kind = match description.sdp_type {
            RTCSdpType::Offer => protocol::DescriptionKind::Offer,
            RTCSdpType::Answer => protocol::DescriptionKind::Answer,
            _ => return Err(Error::Malformed),
        };
        let mid = description
            .sdp
            .lines()
            .find_map(|line| line.strip_prefix("a=mid:"))
            .ok_or(Error::Malformed)?;
        *self.mid.lock().map_err(|_| Error::Engine)? = mid.to_owned();
        let result = protocol::SessionDescription::new(kind, &description.sdp)
            .map_err(|_| Error::Malformed)?;
        tokio::time::timeout(
            Duration::from_secs(3),
            self.pc()?.set_local_description(description),
        )
        .await
        .map_err(|_| Error::Engine)?
        .map_err(|_| Error::Engine)?;
        self.check()?;
        Ok(result)
    }
    pub async fn remote(
        &mut self,
        description: &protocol::SessionDescription,
    ) -> Result<Option<protocol::SessionDescription>, Error> {
        self.check()?;
        if self.remote_descriptions >= 8 {
            return Err(Error::Unsupported);
        }
        self.remote_descriptions += 1;
        let sdp = description.as_str();
        if sdp.lines().filter(|l| l.starts_with("m=")).count() > 3
            || sdp.lines().filter(|l| l.starts_with("m=audio ")).count() > 1
            || sdp.lines().filter(|l| l.starts_with("m=video ")).count() > 1
        {
            return Err(Error::Unsupported);
        }
        let offer = description.kind() == protocol::DescriptionKind::Offer;
        if (self.role == Role::Publisher && offer) || (self.role == Role::Subscriber && !offer) {
            return Err(Error::Unsupported);
        }
        let native = if offer {
            RTCSessionDescription::offer(sdp.into())
        } else {
            RTCSessionDescription::answer(sdp.into())
        }
        .map_err(|_| Error::Malformed)?;
        tokio::time::timeout(
            Duration::from_secs(3),
            self.pc()?.set_remote_description(native),
        )
        .await
        .map_err(|_| Error::Engine)?
        .map_err(|_| Error::Engine)?;
        self.check()?;
        if offer {
            if self.role == Role::Subscriber {
                for transceiver in self.pc()?.get_transceivers().await {
                    transceiver
                        .set_direction(RTCRtpTransceiverDirection::Recvonly)
                        .await
                        .map_err(|_| Error::Engine)?;
                }
            }
            let answer =
                tokio::time::timeout(Duration::from_secs(3), self.pc()?.create_answer(None))
                    .await
                    .map_err(|_| Error::Engine)?
                    .map_err(|_| Error::Engine)?;
            let result = self.local(answer).await?;
            self.counters.answers_sent += 1;
            Ok(Some(result))
        } else {
            Ok(None)
        }
    }
    pub async fn candidate(&mut self, candidate: &protocol::IceCandidate) -> Result<(), Error> {
        self.check()?;
        if self.remote_candidates >= 128 {
            return Err(Error::Overloaded);
        }
        self.remote_candidates += 1;
        let native = RTCIceCandidateInit {
            candidate: candidate.candidate().into(),
            sdp_mid: Some(candidate.mid().into()),
            sdp_mline_index: Some(u16::from(candidate.index())),
            ..Default::default()
        };
        tokio::time::timeout(Duration::from_secs(3), self.pc()?.add_ice_candidate(native))
            .await
            .map_err(|_| Error::Engine)?
            .map_err(|_| Error::Engine)?;
        self.check()
    }
    pub async fn event(&mut self) -> Result<Option<Event>, Error> {
        let result = self.poll().await;
        if result.is_err() {
            self.close();
        }
        result
    }
    async fn poll(&mut self) -> Result<Option<Event>, Error> {
        self.check()?;
        if let Some(audio) = &self.audio {
            audio.flush(4096).await?;
        }
        if let Some(video) = &self.video {
            video.flush(1_048_576).await?;
        }
        if let Some(audio) = &mut self.remote_audio {
            audio.pump(true).await?;
        }
        if let Some(video) = &mut self.remote_video {
            video.pump(self.receive_video).await?;
        }
        for channel in &self.channels {
            for _ in 0..16 {
                let Some(Some(event)) = channel.inner.poll().now_or_never() else {
                    break;
                };
                if let DataChannelEvent::OnMessage(message) = event {
                    if !message.is_string || message.data.len() > 4096 {
                        #[cfg(feature = "backend-test")]
                        eprintln!(
                            "media data rejected: role={:?} text={} over_limit={}",
                            self.role,
                            message.is_string,
                            message.data.len() > 4096
                        );
                        return Err(Error::Malformed);
                    }
                    self.counters.data_received = self.counters.data_received.saturating_add(1);
                }
            }
        }
        while let Ok(event) = self.events.try_recv() {
            match event {
                CallbackEvent::Public(Event::Candidate(c)) => {
                    if self.local_candidates >= 128 {
                        return Err(Error::Overloaded);
                    }
                    self.local_candidates += 1;
                    let kind = c.candidate().split_ascii_whitespace().collect::<Vec<_>>();
                    match kind.windows(2).find(|w| w[0] == "typ").map(|w| w[1]) {
                        Some("host") => self.ice_evidence.host_candidates += 1,
                        Some("srflx") => self.ice_evidence.server_reflexive_candidates += 1,
                        Some("relay") => self.ice_evidence.relay_candidates += 1,
                        _ => {}
                    }
                    return Ok(Some(Event::Candidate(c)));
                }
                CallbackEvent::Public(event) => return Ok(Some(event)),
                CallbackEvent::Channel(c) => self.add_channel(c).await?,
                CallbackEvent::Track(track) => {
                    if self.role == Role::Publisher {
                        return Err(Error::Unsupported);
                    }
                    let ssrcs = track.ssrcs().await;
                    if ssrcs.len() != 1 {
                        return Err(Error::Unsupported);
                    }
                    let ssrc = *ssrcs.first().ok_or(Error::Malformed)?;
                    let codec = track.codec(ssrc).await.ok_or(Error::Unsupported)?;
                    let video = match codec.mime_type.to_ascii_lowercase().as_str() {
                        "audio/opus" => false,
                        "video/vp8" => true,
                        _ => return Err(Error::Unsupported),
                    };
                    let target = if video {
                        &mut self.remote_video
                    } else {
                        &mut self.remote_audio
                    };
                    if target.is_some() {
                        return Err(Error::Unsupported);
                    }
                    *target = Some(Incoming {
                        track,
                        decoder: None,
                        ssrc,
                        video,
                        pli: Instant::now() - Duration::from_secs(2),
                    });
                }
            }
        }
        Ok(None)
    }
    async fn add_channel(&mut self, inner: Arc<dyn DataChannel>) -> Result<(), Error> {
        let label = inner.label().await.map_err(|_| Error::Engine)?;
        // Janus 1.3.0 videoroom relays data with the publisher ID as its label.
        // HPB 2.1.1 assigns ID 1 to the ordinary audio/video publisher. This is
        // channel metadata only; the admitted subscription owns the identity.
        let known = matches!(label.as_str(), "status" | "simplewebrtc")
            || (self.role == Role::Subscriber && label == "1");
        if self.channels.len() >= 2 || !known || self.channels.iter().any(|c| c.label == label) {
            #[cfg(feature = "backend-test")]
            eprintln!(
                "media channel rejected: role={:?} count={} known={} duplicate={}",
                self.role,
                self.channels.len(),
                known,
                self.channels.iter().any(|c| c.label == label)
            );
            return Err(Error::Unsupported);
        }
        self.channels.push(Channel { inner, label });
        Ok(())
    }
    pub async fn send_status(&mut self, message: &[u8]) -> Result<(), Error> {
        self.check()?;
        if message.len() > 4096 {
            return Err(Error::Malformed);
        }
        let text = std::str::from_utf8(message).map_err(|_| Error::Malformed)?;
        // Direct Talk peers create status remotely; SFU peers create it locally
        // before SDP. simplewebrtc remains the separate hark channel.
        let c = self
            .channels
            .iter()
            .find(|c| c.label == "status")
            .ok_or(Error::Engine)?;
        if c.inner.ready_state().await.map_err(|_| Error::Engine)? != RTCDataChannelState::Open {
            return Err(Error::Engine);
        }
        tokio::time::timeout(Duration::from_millis(50), c.inner.try_send_text(text))
            .await
            .map_err(|_| Error::Overloaded)?
            .map_err(|_| Error::Overloaded)
    }
    pub async fn audio(&mut self, samples: &[i16]) -> Result<(), Error> {
        self.check()?;
        if samples.len() != 480 {
            return Err(Error::Malformed);
        }
        let audio = self.audio.as_mut().ok_or(Error::Closed)?;
        if !audio.enabled {
            return Err(Error::Closed);
        }
        if audio.encoder.is_none() {
            audio.encoder = Some(Codec::audio_encoder()?);
        }
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        audio.encoder.as_ref().ok_or(Error::Engine)?.push(
            bytes,
            Some(audio.clock),
            Some(Duration::from_millis(10)),
        )?;
        audio.clock += Duration::from_millis(10);
        self.counters.audio_sent = self.counters.audio_sent.saturating_add(1);
        Ok(())
    }
    pub fn video(&mut self, frame: VideoFrame, timestamp_us: i64) -> Result<(), Error> {
        self.check()?;
        if frame.width == 0
            || frame.height == 0
            || frame.width > 1280
            || frame.height > 720
            || frame.rgba.len() != frame.width as usize * frame.height as usize * 4
            || timestamp_us < 0
        {
            return Err(Error::Malformed);
        }
        let video = self.video.as_mut().ok_or(Error::Closed)?;
        if !video.enabled {
            return Err(Error::Closed);
        }
        if video.encoder.as_ref().and_then(|c| c.dimensions) != Some((frame.width, frame.height)) {
            video.encoder = Some(Codec::video_encoder(frame.width, frame.height)?);
        }
        video.encoder.as_ref().ok_or(Error::Engine)?.push(
            frame.rgba,
            Some(Duration::from_micros(timestamp_us as u64)),
            Some(Duration::from_nanos(33_333_333)),
        )?;
        self.counters.video_sent = self.counters.video_sent.saturating_add(1);
        Ok(())
    }
    pub fn take_audio(&mut self) -> Option<AudioFrame> {
        if !self.is_open() {
            return None;
        }
        if self.pcm.is_empty() {
            let result = self.remote_audio.as_ref()?.decoder.as_ref()?.sample();
            let sample = match result {
                Ok(Some(s)) => s,
                Ok(None) => return None,
                Err(_) => {
                    self.close();
                    return None;
                }
            };
            let Some(buffer) = sample.buffer() else {
                self.close();
                return None;
            };
            let Ok(data) = buffer.map_readable() else {
                self.close();
                return None;
            };
            // Opus 10/20 ms, mono PCM: at most two 480-sample output blocks.
            if data.is_empty() || data.len() > 1920 || !data.len().is_multiple_of(960) {
                self.close();
                return None;
            }
            for block in data.as_chunks::<960>().0 {
                let mut pcm = [0; 480];
                for (out, bytes) in pcm.iter_mut().zip(block.as_chunks::<2>().0) {
                    *out = i16::from_le_bytes([bytes[0], bytes[1]]);
                }
                self.pcm.push_back(pcm);
            }
        }
        let data = self.pcm.pop_front()?;
        self.counters.audio_received = self.counters.audio_received.saturating_add(1);
        if data.iter().any(|s| s.unsigned_abs() > 100) {
            self.counters.audio_nonzero = self.counters.audio_nonzero.saturating_add(1);
        }
        Some(AudioFrame { data })
    }
    pub fn take_video(&mut self) -> Option<VideoFrame> {
        if !self.is_open() {
            return None;
        }
        match self.remote_video.as_ref()?.decoder.as_ref()?.video() {
            Ok(Some(frame)) => {
                self.counters.video_received = self.counters.video_received.saturating_add(1);
                Some(frame)
            }
            Ok(None) => None,
            Err(_) => {
                self.close();
                None
            }
        }
    }
    pub fn receive_video(&mut self, enabled: bool) {
        self.receive_video = enabled;
        if !enabled && let Some(video) = &mut self.remote_video {
            video.decoder = None;
        }
    }
    pub fn counters(&mut self) -> &Counters {
        &self.counters
    }
    pub fn close(&mut self) {
        self.open.store(false, Ordering::Release);
        self.audio = None;
        self.video = None;
        self.remote_audio = None;
        self.remote_video = None;
        self.channels.clear();
        self.pcm.clear();
        // Pooled PeerConnection Drop sets the driver's irreversible close latch.
        self.pc = None;
        while self.events.try_recv().is_ok() {}
    }
}
impl Drop for NativePeer {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(color: u8) -> VideoFrame {
        let mut rgba = vec![0; 320 * 240 * 4];
        for p in rgba.as_chunks_mut::<4>().0 {
            p.copy_from_slice(&[color, 90, 170, 255]);
        }
        VideoFrame {
            width: 320,
            height: 240,
            rgba,
        }
    }

    #[tokio::test]
    async fn two_peers_exchange_av_data_and_revoke_publication() {
        let mut a = NativePeer::new(RtcConfiguration::default()).await.unwrap();
        let mut b = NativePeer::new(RtcConfiguration::default()).await.unwrap();
        assert_eq!(a.audio(&[0; 480]).await, Err(Error::Closed));
        assert_eq!(a.video(image(40), 0), Err(Error::Closed));
        // Browser/Talk owns this channel in normal calls. This peer reproduces
        // that topology, exercising SCTP and both accepted channel labels.
        let status = b
            .pc()
            .unwrap()
            .create_data_channel("status", None)
            .await
            .unwrap();
        b.add_channel(status).await.unwrap();
        let offer = a.offer().await.unwrap();
        let answer = b.remote(&offer).await.unwrap().unwrap();
        a.remote(&answer).await.unwrap();
        a.publish(true, true).unwrap();
        b.publish(true, true).unwrap();
        let start = Instant::now();
        let mut connected = [false; 2];
        let mut status_sent = [false; 2];
        let mut tick = tokio::time::interval(Duration::from_millis(10));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut frame = 0usize;
        while start.elapsed() < Duration::from_secs(12) {
            tick.tick().await;
            while let Some(e) = a.event().await.expect("offerer event") {
                match e {
                    Event::Candidate(c) => b.candidate(&c).await.unwrap(),
                    Event::Connection(s) => connected[0] = s == PeerConnectionState::Connected,
                }
            }
            while let Some(e) = b.event().await.expect("answerer event") {
                match e {
                    Event::Candidate(c) => a.candidate(&c).await.unwrap(),
                    Event::Connection(s) => connected[1] = s == PeerConnectionState::Connected,
                }
            }
            if connected.iter().all(|c| *c) {
                for (index, peer) in [&mut a, &mut b].into_iter().enumerate() {
                    let mut pcm = [0; 480];
                    for (n, p) in pcm.iter_mut().enumerate() {
                        *p = (((frame * 480 + n) as f64 * 0.06).sin() * 4000.0) as i16;
                    }
                    peer.audio(&pcm).await.unwrap();
                    if frame.is_multiple_of(3) {
                        peer.video(
                            image((frame % 200) as u8),
                            start.elapsed().as_micros() as i64,
                        )
                        .unwrap();
                    }
                    while peer.take_audio().is_some() {}
                    while let Some(video) = peer.take_video() {
                        assert!(video.rgba.iter().any(|v| *v != 0));
                    }
                    if !status_sent[index]
                        && peer.send_status(br#"{"type":"audioOn"}"#).await.is_ok()
                    {
                        status_sent[index] = true;
                    }
                }
            }
            frame += 1;
            if [&a, &b].iter().all(|p| {
                p.counters.audio_nonzero > 100
                    && p.counters.video_received > 10
                    && p.counters.data_received > 0
            }) {
                break;
            }
        }
        for (name, peer) in [("offerer", &mut a), ("answerer", &mut b)] {
            let c = peer.counters();
            eprintln!(
                "{name}: audio={} nonzero={} video={} data={}",
                c.audio_received, c.audio_nonzero, c.video_received, c.data_received
            );
            assert!(
                c.audio_nonzero > 100 && c.video_received > 10 && c.data_received > 0,
                "{name} bidirectional media/data"
            );
            peer.publish(false, false).unwrap();
            assert!(peer.audio.as_ref().unwrap().encoder.is_none());
            assert!(peer.video.as_ref().unwrap().encoder.is_none());
            assert_eq!(peer.audio(&[100; 480]).await, Err(Error::Closed));
            assert_eq!(peer.video(image(120), 0), Err(Error::Closed));
            peer.close();
            assert!(!peer.is_open());
            assert_eq!(peer.publish(true, true), Err(Error::Closed));
            assert!(peer.take_audio().is_none());
            assert!(peer.take_video().is_none());
        }
    }
}
