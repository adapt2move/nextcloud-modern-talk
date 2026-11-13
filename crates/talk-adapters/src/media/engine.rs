//! Rust WebRTC 0.21 runtime plus safe, maintained GStreamer codecs. The call
//! owner supplies admitted raw capture; this module never opens devices.
#![forbid(unsafe_code)]

#[path = "codecs.rs"]
mod codecs;
#[cfg(test)]
#[path = "gstreamer_probe.rs"]
mod gstreamer_probe;
#[path = "ice.rs"]
mod ice;
#[path = "turn_proxy.rs"]
mod turn_proxy;
use codecs::Codec;
use futures_util::FutureExt;
pub use ice::{CandidateKind, IceEvidence, IcePolicy, IceSupport, SelectedPair};
use nextcloud_talk_api::signaling as protocol;
use rtc::{
    media::Sample,
    peer_connection::configuration::setting_engine::SctpMaxMessageSize,
    rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication,
    rtp_transceiver::rtp_sender::{
        RTCPFeedback, RTCRtpCodec, RTCRtpCodecParameters, RTCRtpCodingParameters,
        RTCRtpEncodingParameters, RtpCodecKind,
    },
    shared::marshal::Marshal,
};
use std::{
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
/// Fixed operation and category only; no SDP, addresses, codec payloads or
/// dependency error strings may reach call diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EventError {
    pub step: &'static str,
    pub error: Error,
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
    /// Call end-to-end encryption; absent for calls that are not protected this way.
    pub(crate) e2ee: Option<crate::media::e2ee::Keys>,
    /// Test-only trust anchors for a TURN server whose certificate the system does not know.
    #[cfg(test)]
    pub(crate) turn_tls: Option<std::sync::Arc<rustls::ClientConfig>>,
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
    pub video_transmitted: u64,
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
    async fn flush(
        &self,
        max: usize,
        guard: Option<&(dyn Fn() -> bool + Send + Sync)>,
        protect: Option<(
            &std::sync::Mutex<crate::media::e2ee::frame::Encryptor>,
            bool,
        )>,
    ) -> Result<u64, Error> {
        if !self.enabled {
            return Ok(0);
        }
        let Some(encoder) = &self.encoder else {
            return Ok(0);
        };
        // Only the owning call task can write, and each tick drains at most the
        // bounded encoder output queue. No send task can outlive publication.
        let mut sent = 0;
        for _ in 0..3 {
            if guard.is_some_and(|guard| !guard()) {
                break;
            }
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
            // With end-to-end encryption a frame that cannot be protected is
            // dropped; it is never sent in the clear.
            let data = match protect {
                Some((encryptor, video)) => {
                    use crate::media::e2ee::frame::FrameKind;
                    let kind = if !video {
                        FrameKind::Audio
                    } else if data.first().is_some_and(|byte| byte & 1 == 0) {
                        FrameKind::VideoKey
                    } else {
                        FrameKind::VideoDelta
                    };
                    let timestamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |elapsed| elapsed.as_micros() as u32);
                    let sealed = encryptor
                        .lock()
                        .map_err(|_| Error::Engine)?
                        .encrypt(self.ssrc, timestamp, kind, &data);
                    match sealed {
                        Ok(sealed) => sealed,
                        Err(_) => continue,
                    }
                }
                None => data,
            };
            let sample = Sample {
                data: data.into(),
                duration,
                ..Sample::new(Instant::now())
            };
            if guard.is_some_and(|guard| !guard()) {
                break;
            }
            tokio::time::timeout(
                Duration::from_millis(50),
                self.track.write_sample(self.ssrc, payload, &sample, &[]),
            )
            .await
            .map_err(|_| Error::Engine)?
            .map_err(|_| Error::Engine)?;
            sent += 1;
        }
        Ok(sent)
    }
}
struct Incoming {
    track: Arc<dyn TrackRemote>,
    decoder: Option<Codec>,
    ssrc: u32,
    video: bool,
    screen: bool,
    packets: u64,
    discarded: u64,
    keyframes: u64,
    feedback_ssrc: u32,
    feedback_requests: u64,
    pli: Instant,
}
/// RFC 7741 descriptor and VP8 keyframe geometry, before native decode can
/// allocate reference surfaces. Delta packets cannot establish a decoder.
fn vp8_geometry(payload: &[u8]) -> Result<Option<(u32, u32)>, Error> {
    let first = *payload.first().ok_or(Error::Malformed)?;
    let mut offset = 1;
    let mut byte = || {
        let value = payload.get(offset).copied().ok_or(Error::Malformed);
        offset += 1;
        value
    };
    if first & 0x80 != 0 {
        let extension = byte()?;
        if extension & 0x80 != 0 && byte()? & 0x80 != 0 {
            byte()?;
        }
        if extension & 0x40 != 0 {
            byte()?;
        }
        if extension & 0x30 != 0 {
            byte()?;
        }
    }
    if first & 0x10 == 0 || first & 7 != 0 {
        return Ok(None);
    }
    let data = payload.get(offset..).ok_or(Error::Malformed)?;
    if data.first().ok_or(Error::Malformed)? & 1 != 0 {
        return Ok(None);
    }
    if data.len() < 10 || data[3..6] != [0x9d, 0x01, 0x2a] {
        return Err(Error::Malformed);
    }
    let width = u32::from(u16::from_le_bytes([data[6], data[7]]) & 0x3fff);
    let height = u32::from(u16::from_le_bytes([data[8], data[9]]) & 0x3fff);
    if width == 0 || height == 0 || width > 1920 || height > 1080 {
        return Err(Error::Overloaded);
    }
    Ok(Some((width, height)))
}
impl Incoming {
    async fn pump(
        &mut self,
        enabled: bool,
        decrypt: Option<std::sync::Arc<std::sync::Mutex<crate::media::e2ee::frame::Decryptor>>>,
        step: &mut &'static str,
    ) -> Result<(), Error> {
        if !enabled {
            self.decoder = None;
        }
        for _ in 0..64 {
            let Some(event) = self.track.poll().now_or_never() else {
                break;
            };
            match event {
                Some(TrackRemoteEvent::OnRtpPacket(packet)) if enabled => {
                    *step = "RTP packet";
                    self.packets = self.packets.saturating_add(1);
                    if packet.payload.len() > 4096 {
                        return Err(Error::Overloaded);
                    }
                    // Padding-only probes carry no codec header. Once a decoder
                    // exists, still pass them to the jitter buffer so valid RTP
                    // sequence numbers are not mistaken for lost frame packets.
                    if packet.payload.is_empty() && self.decoder.is_none() {
                        continue;
                    }
                    self.ssrc = packet.header.ssrc;
                    if self.video && !packet.payload.is_empty() {
                        *step = "VP8 header";
                        let geometry = match vp8_geometry(&packet.payload) {
                            Ok(value) => value,
                            // An authenticated RTP packet need not contain a
                            // complete/valid codec header. Drop it before native
                            // decode; the bounded pump and periodic PLI recover
                            // on a valid keyframe without ending everyone's call.
                            Err(Error::Malformed) => {
                                self.discarded = self.discarded.saturating_add(1);
                                continue;
                            }
                            Err(error) => return Err(error),
                        };
                        if geometry.is_some() {
                            self.keyframes = self.keyframes.saturating_add(1);
                        } else if self.decoder.is_none() {
                            continue;
                        }
                    }
                    if self.decoder.is_none() {
                        *step = if self.video {
                            "video decoder start"
                        } else {
                            "audio decoder start"
                        };
                        self.decoder = Some(Codec::decoder(
                            self.video,
                            packet.header.payload_type,
                            self.screen,
                            decrypt.clone(),
                        )?);
                    }
                    *step = "RTP marshal";
                    let bytes = packet.marshal().map_err(|_| Error::Malformed)?;
                    *step = if self.video {
                        "video decoder input"
                    } else {
                        "audio decoder input"
                    };
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
                ) => {
                    *step = "remote track ended";
                    return Err(Error::Closed);
                }
                _ => {}
            }
        }
        if self.video && enabled && self.pli.elapsed() >= Duration::from_secs(2) {
            *step = "video feedback";
            tokio::time::timeout(
                Duration::from_millis(50),
                self.track.write_rtcp(vec![Box::new(PictureLossIndication {
                    sender_ssrc: self.feedback_ssrc,
                    media_ssrc: self.ssrc,
                })]),
            )
            .await
            .map_err(|_| Error::Engine)?
            .map_err(|_| Error::Engine)?;
            self.pli = Instant::now();
            self.feedback_requests = self.feedback_requests.saturating_add(1);
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
    publish_guard: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    audio_guard: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    screen: bool,
    channels: Vec<Channel>,
    counters: Counters,
    remote_descriptions: u8,
    remote_candidates: u16,
    local_candidates: u16,
    role: Role,
    e2ee: Option<crate::media::e2ee::Keys>,
    _bridges: Vec<turn_proxy::Bridge>,
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
        let (mut ice_servers, ice_support, relays) =
            ice::plan(config.ice_servers, config.ice_policy)?;
        // TURN over TCP/TLS is carried by loopback bridges the peer owns.
        let mut bridges = Vec::with_capacity(relays.len());
        if !relays.is_empty() {
            let tls = turn_proxy::system_tls().ok();
            #[cfg(test)]
            let tls = config.turn_tls.clone().or(tls);
            for relay in relays {
                let bridge = turn_proxy::Bridge::start(relay.target, tls.clone()).await?;
                ice_servers[relay.server].urls[relay.url] =
                    format!("turn:{}?transport=udp", bridge.address());
                bridges.push(bridge);
            }
        }
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
            // The local track codec also participates in SDP generation. Keep
            // its repair feedback aligned with the installed NACK/PLI handlers.
            rtcp_feedback: vec![
                RTCPFeedback {
                    typ: "nack".into(),
                    parameter: String::new(),
                },
                RTCPFeedback {
                    typ: "nack".into(),
                    parameter: "pli".into(),
                },
            ],
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
            publish_guard: None,
            audio_guard: None,
            screen: false,
            channels: Vec::with_capacity(2),
            counters: Counters::default(),
            remote_descriptions: 0,
            remote_candidates: 0,
            local_candidates: 0,
            role,
            e2ee: config.e2ee,
            _bridges: bridges,
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
    /// Rechecked after encoding and immediately before the RTP sample write.
    pub fn set_publish_guard(&mut self, guard: Arc<dyn Fn() -> bool + Send + Sync>) {
        self.publish_guard = Some(guard);
    }
    pub fn set_audio_guard(&mut self, guard: Arc<dyn Fn() -> bool + Send + Sync>) {
        self.audio_guard = Some(guard);
    }
    /// Screen reception retains text resolution within the 1080p allocation class.
    pub fn screen_profile(&mut self) {
        self.screen = true;
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
    pub fn is_offerer(&self) -> bool {
        self.counters.offers_sent > 0
    }
    pub async fn restart(&mut self) -> Result<protocol::SessionDescription, Error> {
        self.check()?;
        if self.role == Role::Subscriber {
            return Err(Error::Unsupported);
        }
        tokio::time::timeout(Duration::from_secs(3), self.pc()?.restart_ice())
            .await
            .map_err(|_| Error::Engine)?
            .map_err(|_| Error::Engine)?;
        self.offer().await
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
        #[cfg(feature = "backend-test")]
        if self.screen {
            eprintln!(
                "screen local SDP PLI={}",
                description
                    .sdp
                    .lines()
                    .any(|l| l.starts_with("a=rtcp-fb:") && l.ends_with("nack pli"))
            );
        }
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
    pub async fn event(&mut self) -> Result<Option<Event>, EventError> {
        let mut step = "peer state";
        let result = self.poll(&mut step).await;
        if result.is_err() {
            self.close();
        }
        result.map_err(|error| EventError { step, error })
    }
    async fn poll(&mut self, step: &mut &'static str) -> Result<Option<Event>, Error> {
        self.check()?;
        let send = self.e2ee.as_ref().map(|keys| keys.send.clone());
        let receive = self.e2ee.as_ref().map(|keys| keys.receive.clone());
        if let Some(audio) = &self.audio {
            *step = "audio send";
            audio
                .flush(
                    4096,
                    self.audio_guard.as_deref(),
                    send.as_deref().map(|send| (send, false)),
                )
                .await?;
        }
        if let Some(video) = &self.video {
            *step = "video send";
            self.counters.video_transmitted = self.counters.video_transmitted.saturating_add(
                video
                    .flush(
                        1_048_576,
                        self.publish_guard.as_deref(),
                        send.as_deref().map(|send| (send, true)),
                    )
                    .await?,
            );
        }
        if let Some(audio) = &mut self.remote_audio {
            *step = "audio receive";
            audio.pump(true, receive.clone(), step).await?;
        }
        if let Some(video) = &mut self.remote_video {
            *step = "video receive";
            video.pump(self.receive_video, receive, step).await?;
        }
        for channel in &self.channels {
            *step = "data message";
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
                    *step = "local candidates";
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
                CallbackEvent::Channel(c) => {
                    *step = "data channel";
                    self.add_channel(c).await?;
                }
                CallbackEvent::Track(track) => {
                    *step = "remote track role";
                    if self.role == Role::Publisher {
                        return Err(Error::Unsupported);
                    }
                    *step = "remote track streams";
                    let ssrcs = track.ssrcs().await;
                    if ssrcs.len() != 1 {
                        return Err(Error::Unsupported);
                    }
                    let ssrc = *ssrcs.first().ok_or(Error::Malformed)?;
                    *step = "remote track codec";
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
                    *step = "remote track replacement";
                    if target.is_some() {
                        return Err(Error::Unsupported);
                    }
                    *step = "receiver identity";
                    let mut feedback_id = [0; 4];
                    getrandom::fill(&mut feedback_id).map_err(|_| Error::Engine)?;
                    // RTCP receiver feedback needs its own nonzero identity.
                    // Janus must not see an anonymous sender or the remote RTP SSRC.
                    let mut feedback_ssrc = u32::from_le_bytes(feedback_id).max(1);
                    if feedback_ssrc == ssrc {
                        feedback_ssrc = feedback_ssrc.wrapping_add(1).max(1);
                    }
                    *target = Some(Incoming {
                        track,
                        decoder: None,
                        ssrc,
                        video,
                        screen: self.screen,
                        packets: 0,
                        discarded: 0,
                        keyframes: 0,
                        feedback_ssrc,
                        feedback_requests: 0,
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
            || (self.role == Role::Subscriber
                && (label == "1"
                    || (self.screen
                        && label.len() <= 20
                        && label.bytes().all(|b| b.is_ascii_digit()))));
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
            || frame.width > if self.screen { 1920 } else { 1280 }
            || frame.height > if self.screen { 1080 } else { 720 }
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
            video.encoder = Some(Codec::video_encoder(
                frame.width,
                frame.height,
                self.screen,
            )?);
        }
        video.encoder.as_ref().ok_or(Error::Engine)?.push(
            frame.rgba,
            Some(Duration::from_micros(timestamp_us as u64)),
            Some(if self.screen {
                Duration::from_millis(100)
            } else {
                Duration::from_nanos(33_333_333)
            }),
        )?;
        self.counters.video_sent = self.counters.video_sent.saturating_add(1);
        Ok(())
    }
    pub fn take_audio(&mut self) -> Option<AudioFrame> {
        if !self.is_open() {
            return None;
        }
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
        let Ok(bytes) = buffer.map_readable() else {
            self.close();
            return None;
        };
        // audiobuffersplit owns block formation after conversion to mono S16LE
        // at 48 kHz. Packet duration (including 40/60 ms) never changes this
        // device contract. Keep an assertion at the native boundary rather than
        // reintroducing a second PCM queue/splitter here.
        if bytes.len() != 960 {
            self.close();
            return None;
        }
        let data = std::array::from_fn(|index| {
            i16::from_le_bytes([bytes[index * 2], bytes[index * 2 + 1]])
        });
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
    #[cfg(feature = "backend-test")]
    pub fn video_packets(&self) -> u64 {
        self.remote_video.as_ref().map_or(0, |v| v.packets)
    }
    #[cfg(feature = "backend-test")]
    pub fn video_keyframes(&self) -> u64 {
        self.remote_video.as_ref().map_or(0, |v| v.keyframes)
    }
    #[cfg(feature = "backend-test")]
    pub fn video_feedback_requests(&self) -> u64 {
        self.remote_video
            .as_ref()
            .map_or(0, |v| v.feedback_requests)
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
    #[tokio::test]
    async fn event_failure_keeps_safe_context_and_closes_the_peer() {
        let mut peer = NativePeer::new(RtcConfiguration::default()).await.unwrap();
        let channel = peer
            .pc()
            .unwrap()
            .create_data_channel("CANARY_PRIVATE_LABEL", None)
            .await
            .unwrap();
        let (sender, events) = mpsc::sync_channel(EVENTS);
        peer.events = events;
        assert!(sender.try_send(CallbackEvent::Channel(channel)).is_ok());
        let error = peer
            .event()
            .await
            .err()
            .expect("unknown channel is rejected");
        assert_eq!(
            error,
            EventError {
                step: "data channel",
                error: Error::Unsupported
            }
        );
        assert!(!format!("{error:?}").contains("CANARY"));
        assert!(!peer.is_open());
        assert_eq!(
            peer.event().await.err(),
            Some(EventError {
                step: "peer state",
                error: Error::Closed
            })
        );
    }

    #[test]
    fn vp8_header_bounds_apply_before_native_decode_and_include_extensions() {
        let mut payload = vec![0x10, 0, 0, 0, 0x9d, 0x01, 0x2a, 0x80, 0x07, 0x38, 0x04];
        assert_eq!(vp8_geometry(&payload), Ok(Some((1920, 1080))));
        let mut extended = vec![0x90, 0xf0, 0x80, 1, 0, 0];
        extended.extend_from_slice(&payload[1..]);
        assert_eq!(vp8_geometry(&extended), Ok(Some((1920, 1080))));
        for length in 1..extended.len() {
            assert_eq!(vp8_geometry(&extended[..length]), Err(Error::Malformed));
        }
        payload[7] = 0x00;
        payload[8] = 0x0f;
        assert_eq!(vp8_geometry(&payload), Err(Error::Overloaded));
        payload[1] = 1;
        assert_eq!(vp8_geometry(&payload), Ok(None)); // Delta never creates a decoder.
        assert_eq!(vp8_geometry(&[0, 1]), Ok(None)); // Partition continuation.
        assert_eq!(vp8_geometry(&[]), Err(Error::Malformed));
    }

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
    async fn damaged_vp8_frames_do_not_end_audio_or_later_video() {
        let mut sender = NativePeer::new(RtcConfiguration::default()).await.unwrap();
        let mut receiver = NativePeer::new(RtcConfiguration::default()).await.unwrap();
        let offer = sender.offer().await.unwrap();
        let answer = receiver.remote(&offer).await.unwrap().unwrap();
        sender.remote(&answer).await.unwrap();
        sender.publish(true, true).unwrap();
        let mut connected = [false; 2];
        let mut damaged_sent = 0;
        let mut audio_at_damage = 0;
        let mut decoded = 0;
        let started = Instant::now();
        let mut tick = tokio::time::interval(Duration::from_millis(10));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut frame = 0u64;
        while started.elapsed() < Duration::from_secs(6) {
            tick.tick().await;
            while let Some(event) = sender.event().await.unwrap() {
                match event {
                    Event::Candidate(c) => receiver.candidate(&c).await.unwrap(),
                    Event::Connection(s) => connected[0] = s == PeerConnectionState::Connected,
                }
            }
            while let Some(event) = receiver.event().await.unwrap() {
                match event {
                    Event::Candidate(c) => sender.candidate(&c).await.unwrap(),
                    Event::Connection(s) => connected[1] = s == PeerConnectionState::Connected,
                }
            }
            if !connected.iter().all(|v| *v) {
                continue;
            }
            if damaged_sent == 0 || damaged_sent == 1 && decoded >= 3 {
                let video = sender.video.as_ref().unwrap();
                // Authenticated RTP can still carry an incomplete codec header.
                // Send it through the real payloader, SRTP and remote track.
                video
                    .track
                    .write_sample(
                        video.ssrc,
                        96,
                        &Sample {
                            data: vec![0].into(),
                            duration: Duration::from_millis(33),
                            ..Sample::new(Instant::now())
                        },
                        &[],
                    )
                    .await
                    .unwrap();
                damaged_sent += 1;
                audio_at_damage = receiver.counters.audio_nonzero;
            }
            let pcm: [i16; 480] = std::array::from_fn(|n| {
                (((frame * 480 + n as u64) as f64 * 0.06).sin() * 4000.0) as i16
            });
            sender.audio(&pcm).await.unwrap();
            if frame.is_multiple_of(3) {
                sender
                    .video(image(70), started.elapsed().as_micros() as i64)
                    .unwrap();
            }
            frame += 1;
            while receiver.take_audio().is_some() {}
            while receiver.take_video().is_some() {
                decoded += 1;
            }
            if decoded >= 6
                && receiver.counters.audio_nonzero > audio_at_damage + 40
                && receiver
                    .remote_video
                    .as_ref()
                    .is_some_and(|v| v.discarded == 2)
            {
                break;
            }
        }
        assert_eq!(
            damaged_sent, 2,
            "damage is sent before and during video playback"
        );
        assert_eq!(
            receiver.remote_video.as_ref().map(|v| v.discarded),
            Some(2),
            "both damaged frames must have reached the receiver"
        );
        assert!(receiver.is_open(), "a bad frame must not close the peer");
        assert!(
            decoded >= 6 && receiver.counters.audio_nonzero > audio_at_damage + 40,
            "valid media after damage must decode: video={decoded}, audio={}",
            receiver.counters.audio_nonzero
        );
    }

    /// Exercise the production transport, decryptor, splitter and device handoff.
    /// A 40/60 ms Opus packet used to close the peer in take_audio().
    #[tokio::test]
    async fn opus_packet_duration_changes_keep_audio_and_video_alive() {
        for encrypted in [false, true] {
            let key = [42; crate::media::e2ee::frame::KEY_LEN];
            let config = || {
                if encrypted {
                    keyed(Some(key), Some(key))
                } else {
                    RtcConfiguration::default()
                }
            };
            let mut sender = NativePeer::new(config()).await.unwrap();
            let mut receiver = NativePeer::new(config()).await.unwrap();
            let offer = sender.offer().await.unwrap();
            let answer = receiver.remote(&offer).await.unwrap().unwrap();
            sender.remote(&answer).await.unwrap();
            sender.publish(true, true).unwrap();
            let mut connected = [false; 2];
            let mut tick = tokio::time::interval(Duration::from_millis(10));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let started = Instant::now();
            let mut frame = 0_u64;
            for duration in [10, 20, 40, 60] {
                sender.audio.as_mut().unwrap().encoder =
                    Some(Codec::audio_encoder_duration(duration).unwrap());
                let audio_before = receiver.counters.audio_nonzero;
                let video_before = receiver.counters.video_received;
                let deadline = Instant::now() + Duration::from_secs(5);
                while Instant::now() < deadline {
                    tick.tick().await;
                    while let Some(event) = sender.event().await.unwrap() {
                        match event {
                            Event::Candidate(c) => receiver.candidate(&c).await.unwrap(),
                            Event::Connection(s) => {
                                connected[0] = s == PeerConnectionState::Connected
                            }
                        }
                    }
                    while let Some(event) = receiver.event().await.unwrap() {
                        match event {
                            Event::Candidate(c) => sender.candidate(&c).await.unwrap(),
                            Event::Connection(s) => {
                                connected[1] = s == PeerConnectionState::Connected
                            }
                        }
                    }
                    if !connected.iter().all(|c| *c) {
                        continue;
                    }
                    let pcm = std::array::from_fn::<_, 480, _>(|n| {
                        (((frame * 480 + n as u64) as f64 * 0.06).sin() * 6000.0) as i16
                    });
                    sender.audio(&pcm).await.unwrap();
                    if frame.is_multiple_of(3) {
                        sender
                            .video(
                                image((frame % 200) as u8),
                                started.elapsed().as_micros() as i64,
                            )
                            .unwrap();
                    }
                    frame += 1;
                    while receiver.take_audio().is_some() {}
                    while receiver.take_video().is_some() {}
                    assert!(
                        receiver.is_open(),
                        "Opus {duration} ms, encrypted={encrypted}"
                    );
                    if receiver.counters.audio_nonzero > audio_before + 30
                        && receiver.counters.video_received > video_before + 5
                    {
                        break;
                    }
                }
                assert!(
                    receiver.counters.audio_nonzero > audio_before + 30,
                    "fresh audible Opus {duration} ms, encrypted={encrypted}"
                );
                assert!(
                    receiver.counters.video_received > video_before + 5,
                    "fresh video during Opus {duration} ms, encrypted={encrypted}"
                );
            }
            receiver.close();
            assert!(receiver.take_audio().is_none());
            assert_eq!(receiver.publish(true, true), Err(Error::Closed));
        }
    }

    #[tokio::test]
    async fn one_static_screen_frame_decodes_without_following_capture_frames() {
        let mut sender = NativePeer::new(RtcConfiguration::default()).await.unwrap();
        let mut receiver = NativePeer::new(RtcConfiguration::default()).await.unwrap();
        sender.screen_profile();
        receiver.screen_profile();
        let offer = sender.offer().await.unwrap();
        let answer = receiver.remote(&offer).await.unwrap().unwrap();
        sender.remote(&answer).await.unwrap();
        sender.publish(false, true).unwrap();
        let mut connected = [false; 2];
        let mut sent = false;
        let mut decoded = None;
        let started = Instant::now();
        let mut tick = tokio::time::interval(Duration::from_millis(10));
        while started.elapsed() < Duration::from_secs(6) {
            tick.tick().await;
            while let Some(event) = sender.event().await.unwrap() {
                match event {
                    Event::Candidate(c) => receiver.candidate(&c).await.unwrap(),
                    Event::Connection(s) => connected[0] = s == PeerConnectionState::Connected,
                }
            }
            while let Some(event) = receiver.event().await.unwrap() {
                match event {
                    Event::Candidate(c) => sender.candidate(&c).await.unwrap(),
                    Event::Connection(s) => connected[1] = s == PeerConnectionState::Connected,
                }
            }
            if !sent && connected.iter().all(|v| *v) {
                sender
                    .video(image(40), started.elapsed().as_micros() as i64)
                    .unwrap();
                sent = true;
            }
            if let Some(frame) = receiver.take_video() {
                decoded = Some(frame);
                break;
            }
        }
        let frame =
            decoded.expect("one static keyframe must leave the decoder without future input");
        assert_eq!((frame.width, frame.height), (320, 240));
        assert_eq!(sender.counters().video_sent, 1);
        sender.close();
        receiver.close();
    }

    /// Connect two peers and let each send audio and video for a while; report
    /// what each decoded. Local ICE on a busy workstation (VPN and bridge
    /// interfaces) sometimes does not connect within the window, so connecting is
    /// retried with fresh peers; media that never flows is reported as failure.
    async fn converse(
        configs: impl Fn() -> (RtcConfiguration, RtcConfiguration),
        for_how_long: Duration,
    ) -> [(u64, u64); 2] {
        // Local ICE across many interfaces is unreliable when several of these
        // run at once, so they take turns.
        static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        let _turn = TURN.lock().await;
        'attempt: for _ in 0..6 {
            let (left, right) = configs();
            let mut a = NativePeer::new(left).await.unwrap();
            let mut b = NativePeer::new(right).await.unwrap();
            let offer = a.offer().await.unwrap();
            let answer = b.remote(&offer).await.unwrap().unwrap();
            a.remote(&answer).await.unwrap();
            a.publish(true, true).unwrap();
            b.publish(true, true).unwrap();
            let start = Instant::now();
            let mut connected = [false; 2];
            let mut connected_at = None;
            let mut tick = tokio::time::interval(Duration::from_millis(10));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut frame = 0usize;
            loop {
                tick.tick().await;
                while let Some(e) = a.event().await.unwrap() {
                    match e {
                        Event::Candidate(c) => b.candidate(&c).await.unwrap(),
                        Event::Connection(s) => connected[0] = s == PeerConnectionState::Connected,
                    }
                }
                while let Some(e) = b.event().await.unwrap() {
                    match e {
                        Event::Candidate(c) => a.candidate(&c).await.unwrap(),
                        Event::Connection(s) => connected[1] = s == PeerConnectionState::Connected,
                    }
                }
                if connected.iter().all(|c| *c) {
                    let since = *connected_at.get_or_insert_with(Instant::now);
                    if since.elapsed() >= for_how_long {
                        break;
                    }
                    for peer in [&mut a, &mut b] {
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
                        while peer.take_video().is_some() {}
                    }
                } else if start.elapsed() > Duration::from_secs(4) {
                    a.close();
                    b.close();
                    continue 'attempt;
                }
                frame += 1;
            }
            let result = [&a, &b].map(|p| (p.counters.audio_nonzero, p.counters.video_received));
            a.close();
            b.close();
            return result;
        }
        panic!("the two local peers never connected");
    }

    fn keyed(
        own: Option<[u8; crate::media::e2ee::frame::KEY_LEN]>,
        remote: Option<[u8; crate::media::e2ee::frame::KEY_LEN]>,
    ) -> RtcConfiguration {
        let keys = crate::media::e2ee::Keys::new();
        if let Some(key) = own {
            keys.send.lock().unwrap().set_key(&key, 3).unwrap();
        }
        if let Some(key) = remote {
            keys.receive.lock().unwrap().set_key(&key, 3).unwrap();
        }
        RtcConfiguration {
            e2ee: Some(keys),
            ..Default::default()
        }
    }

    #[tokio::test]
    #[ignore = "needs local ICE across every interface, which is unreliable on busy workstations; run with --ignored"]
    async fn end_to_end_encrypted_audio_and_video_reach_a_peer_that_holds_the_key() {
        let (key_a, key_b) = ([1_u8; 32], [2_u8; 32]);
        let [a, b] = converse(
            || {
                (
                    keyed(Some(key_a), Some(key_b)),
                    keyed(Some(key_b), Some(key_a)),
                )
            },
            Duration::from_secs(5),
        )
        .await;
        for (name, (audio, video)) in [("a", a), ("b", b)] {
            assert!(
                audio > 100 && video > 10,
                "{name} decodes audio and video: {a:?} {b:?}"
            );
        }
    }

    #[tokio::test]
    #[ignore = "needs local ICE across every interface, which is unreliable on busy workstations; run with --ignored"]
    async fn media_is_not_shown_without_the_senders_key_or_from_a_sender_without_encryption() {
        let key = [7_u8; 32];
        // Each side holds a different key for the other.
        let [a, b] = converse(
            || {
                (
                    keyed(Some(key), Some([9; 32])),
                    keyed(Some([8; 32]), Some([6; 32])),
                )
            },
            Duration::from_secs(4),
        )
        .await;
        assert_eq!((a, b), ((0, 0), (0, 0)), "a wrong key shows nothing");
        // One side does not encrypt at all.
        let [plain, strict] = converse(
            || (RtcConfiguration::default(), keyed(Some(key), Some(key))),
            Duration::from_secs(4),
        )
        .await;
        assert_eq!(strict, (0, 0), "unencrypted media is dropped, not shown");
        assert_eq!(
            plain,
            (0, 0),
            "an encrypted sender is unreadable without E2EE"
        );
    }

    #[tokio::test]
    #[ignore = "needs local ICE across every interface, which is unreliable on busy workstations; run with --ignored"]
    async fn a_sender_without_a_key_sends_nothing_rather_than_plain_frames() {
        let [_, b] = converse(
            || (keyed(None, Some([4; 32])), RtcConfiguration::default()),
            Duration::from_secs(4),
        )
        .await;
        assert_eq!(
            b,
            (0, 0),
            "no key, no media (not even to a peer without E2EE)"
        );
    }

    /// Media relayed by a real TURN server reached over TLS (`NMT_TURNS_URL`, with
    /// `NMT_TURN_USER`/`NMT_TURN_PASS`), the server presenting the certificate in
    /// `fixtures/turn-test-cert.der`. Needs a server the test cannot start itself.
    #[tokio::test]
    #[ignore = "needs a TURN server reachable over TLS; see docs/development.md"]
    async fn media_flows_through_a_real_turn_server_over_tls() {
        let (Ok(url), Ok(user), Ok(pass)) = (
            std::env::var("NMT_TURNS_URL"),
            std::env::var("NMT_TURN_USER"),
            std::env::var("NMT_TURN_PASS"),
        ) else {
            panic!("NMT_TURNS_URL, NMT_TURN_USER and NMT_TURN_PASS must be set");
        };
        let mut roots = rustls::RootCertStore::empty();
        roots
            .add(rustls::pki_types::CertificateDer::from(
                include_bytes!("fixtures/turn-test-cert.der").to_vec(),
            ))
            .unwrap();
        let tls = std::sync::Arc::new(
            rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
                rustls::crypto::aws_lc_rs::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth(),
        );
        let config = || RtcConfiguration {
            ice_servers: vec![IceServer {
                urls: vec![url.clone()],
                username: user.clone(),
                password: pass.clone(),
            }],
            ice_policy: IcePolicy::Relay,
            turn_tls: Some(tls.clone()),
            ..Default::default()
        };
        let [a, b] = converse(|| (config(), config()), Duration::from_secs(5)).await;
        assert!(
            a.0 > 100 && a.1 > 10 && b.0 > 100 && b.1 > 10,
            "relayed media decodes: {a:?} {b:?}"
        );
    }

    /// The control: the same harness shows media when nothing is encrypted, so
    /// the zero results above mean protection, not a broken harness.
    #[tokio::test]
    #[ignore = "needs local ICE across every interface, which is unreliable on busy workstations; run with --ignored"]
    async fn the_harness_shows_unprotected_media() {
        let [a, b] = converse(
            || (RtcConfiguration::default(), RtcConfiguration::default()),
            Duration::from_secs(4),
        )
        .await;
        assert!(
            a.0 > 100 && a.1 > 10 && b.0 > 100 && b.1 > 10,
            "{a:?} {b:?}"
        );
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
        let mut restart_at = None;
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
                if let Some((left, right)) = restart_at {
                    if a.counters.video_received > left + 10
                        && b.counters.video_received > right + 10
                    {
                        break;
                    }
                } else {
                    let restart = a.restart().await.unwrap();
                    let credentials = |sdp: &str| {
                        sdp.lines()
                            .find(|line| line.starts_with("a=ice-ufrag:"))
                            .map(str::to_owned)
                    };
                    assert!(credentials(restart.as_str()) != credentials(offer.as_str()));
                    let answer = b.remote(&restart).await.unwrap().unwrap();
                    a.remote(&answer).await.unwrap();
                    restart_at = Some((a.counters.video_received, b.counters.video_received));
                }
            }
        }
        assert!(
            restart_at.is_some_and(|(left, right)| a.counters.video_received > left + 10
                && b.counters.video_received > right + 10),
            "decoded video continues after ICE restart"
        );
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
