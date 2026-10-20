//! Maintained libwebrtc binding. The call owner supplies already admitted raw
//! capture; this module never opens a microphone, camera or screen by itself.
//! Native encode/decode/DTLS/SRTP/ICE/SCTP belong to the maintained engine.
#![forbid(unsafe_code)]

use futures_util::{FutureExt, StreamExt};
use libwebrtc::{
    MediaType,
    audio_frame::AudioFrame,
    audio_source::{AudioSourceOptions, native::NativeAudioSource},
    audio_stream::native::{NativeAudioStream, NativeAudioStreamOptions},
    data_channel::{DataChannel, DataChannelInit},
    ice_candidate::IceCandidate,
    media_stream_track::MediaStreamTrack,
    peer_connection::{
        AnswerOptions, OfferOptions, PeerConnection, PeerConnectionState, TrackEvent,
    },
    peer_connection_factory::{
        PeerConnectionFactory, RtcConfiguration, native::PeerConnectionFactoryExt,
    },
    session_description::{SdpType, SessionDescription},
    video_frame::{BoxVideoFrame, I420Buffer, VideoBuffer, VideoFrame, VideoRotation},
    video_source::{VideoResolution, native::NativeVideoSource},
    video_stream::native::{NativeVideoStream, NativeVideoStreamOptions},
};
use nextcloud_talk_api::signaling as protocol;
use std::{
    borrow::Cow,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    time::Duration,
};

// ICE gathering may finish before setLocalDescription returns. Admit the
// existing per-peer candidate budget plus eight lifecycle/track events.
const EVENTS: usize = 136;
pub const CAMERA_WIDTH: u32 = 640;
pub const CAMERA_HEIGHT: u32 = 480;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Closed,
    Engine,
    Malformed,
    Overloaded,
    Unsupported,
}

pub enum Event {
    Candidate(protocol::IceCandidate),
    Connection(PeerConnectionState),
    Track(TrackEvent),
    Channel(DataChannel),
}

/// Every callback has the exact owner's irreversible latch. Queue overflow
/// closes admission; media frames never enter this signaling queue.
#[derive(Clone)]
struct Callback {
    open: Arc<AtomicBool>,
    events: SyncSender<Event>,
}
impl Callback {
    fn send(&self, event: Event) {
        if self.open.load(Ordering::Acquire) && self.events.try_send(event).is_err() {
            self.open.store(false, Ordering::Release);
            #[cfg(feature = "backend-test")]
            eprintln!("native callback queue overflow");
        }
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

pub struct NativePeer {
    pc: PeerConnection,
    _factory: PeerConnectionFactory,
    open: Arc<AtomicBool>,
    events: Receiver<Event>,
    audio_source: NativeAudioSource,
    video_source: NativeVideoSource,
    audio_track: MediaStreamTrack,
    video_track: MediaStreamTrack,
    audio: Option<NativeAudioStream>,
    video: Option<NativeVideoStream>,
    remote_video: Option<libwebrtc::video_track::RtcVideoTrack>,
    receive_video: bool,
    channels: Vec<DataChannel>,
    data_received: Arc<AtomicU64>,
    counters: Counters,
    remote_descriptions: u8,
    remote_candidates: u16,
    local_candidates: u16,
}

impl NativePeer {
    /// Construction allocates tracks but does not capture. A production caller
    /// must bind lifetime/policy before feeding frames or signaling this peer.
    pub fn new(config: RtcConfiguration) -> Result<Self, Error> {
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
        let factory = PeerConnectionFactory::default();
        let pc = factory
            .create_peer_connection(config)
            .map_err(|_| Error::Engine)?;
        let open = Arc::new(AtomicBool::new(true));
        let (sender, events) = mpsc::sync_channel(EVENTS);
        let callback = Callback {
            open: open.clone(),
            events: sender,
        };
        let ice = callback.clone();
        pc.on_ice_candidate(Some(Box::new(move |candidate| {
            if !ice.open.load(Ordering::Acquire) {
                return;
            }
            let parsed = u8::try_from(candidate.sdp_mline_index())
                .ok()
                .and_then(|index| {
                    protocol::IceCandidate::new(&candidate.sdp_mid(), index, &candidate.candidate())
                        .ok()
                });
            match parsed {
                Some(candidate) => ice.send(Event::Candidate(candidate)),
                None => ice.open.store(false, Ordering::Release),
            }
        })));
        let connection = callback.clone();
        pc.on_connection_state_change(Some(Box::new(move |state| {
            connection.send(Event::Connection(state))
        })));
        let tracks = callback.clone();
        pc.on_track(Some(Box::new(move |track| {
            tracks.send(Event::Track(track))
        })));
        pc.on_data_channel(Some(Box::new(move |channel| {
            callback.send(Event::Channel(channel))
        })));
        let audio_source = NativeAudioSource::new(
            AudioSourceOptions {
                echo_cancellation: false,
                noise_suppression: false,
                auto_gain_control: false,
            },
            48_000,
            1,
            0,
        );
        let video_source = NativeVideoSource::new(
            VideoResolution {
                width: CAMERA_WIDTH,
                height: CAMERA_HEIGHT,
            },
            false,
        );
        let audio_track: MediaStreamTrack = factory
            .create_audio_track("audio", audio_source.clone())
            .into();
        let video_track: MediaStreamTrack = factory
            .create_video_track("video", video_source.clone())
            .into();
        audio_track.set_enabled(false);
        video_track.set_enabled(false);
        for (track, kind, codec) in [
            (audio_track.clone(), MediaType::Audio, "audio/opus"),
            (video_track.clone(), MediaType::Video, "video/VP8"),
        ] {
            let id = track.id();
            pc.add_track(track, &["nmt"]).map_err(|_| Error::Engine)?;
            let transceiver = pc
                .transceivers()
                .into_iter()
                .find(|t| t.sender().track().is_some_and(|track| track.id() == id))
                .ok_or(Error::Engine)?;
            let codecs = factory
                .get_rtp_sender_capabilities(kind)
                .codecs
                .into_iter()
                .filter(|c| c.mime_type.eq_ignore_ascii_case(codec))
                .collect();
            transceiver
                .set_codec_preferences(codecs)
                .map_err(|_| Error::Engine)?;
        }
        Ok(Self {
            pc,
            _factory: factory,
            open,
            events,
            audio_source,
            video_source,
            audio_track,
            video_track,
            audio: None,
            video: None,
            remote_video: None,
            receive_video: true,
            channels: Vec::with_capacity(2),
            data_received: Arc::new(AtomicU64::new(0)),
            counters: Counters::default(),
            remote_descriptions: 0,
            remote_candidates: 0,
            local_candidates: 0,
        })
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
    /// No implicit toggles on connect/reconnect. Caller has already checked the
    /// current user's request, current policy and physical-device result.
    pub fn publish(&mut self, microphone: bool, camera: bool) -> Result<(), Error> {
        self.check()?;
        if (self.audio_track.enabled() != microphone && !self.audio_track.set_enabled(microphone))
            || (self.video_track.enabled() != camera && !self.video_track.set_enabled(camera))
        {
            self.close();
            return Err(Error::Engine);
        }
        if !microphone {
            self.audio_source.clear_buffer();
        }
        Ok(())
    }
    pub async fn offer(&mut self) -> Result<protocol::SessionDescription, Error> {
        self.check()?;
        if self.channels.is_empty() {
            let channel = self
                .pc
                .create_data_channel("simplewebrtc", DataChannelInit::default())
                .map_err(|_| Error::Engine)?;
            self.add_channel(channel)?;
        }
        let description = tokio::time::timeout(
            Duration::from_secs(3),
            self.pc.create_offer(OfferOptions {
                offer_to_receive_audio: true,
                offer_to_receive_video: true,
                ice_restart: false,
            }),
        )
        .await
        .map_err(|_| Error::Engine)?
        .map_err(|_| Error::Engine)?;
        let result = self.local(description).await?;
        self.counters.offers_sent += 1;
        Ok(result)
    }
    async fn local(
        &mut self,
        description: SessionDescription,
    ) -> Result<protocol::SessionDescription, Error> {
        self.check()?;
        let kind = match description.sdp_type() {
            SdpType::Offer => protocol::DescriptionKind::Offer,
            SdpType::Answer => protocol::DescriptionKind::Answer,
            _ => return Err(Error::Malformed),
        };
        let result = protocol::SessionDescription::new(kind, &description.to_string())
            .map_err(|_| Error::Malformed)?;
        tokio::time::timeout(
            Duration::from_secs(3),
            self.pc.set_local_description(description),
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
        // Bound native topology before parsing. Codec internals remain the
        // maintained library boundary; this is not a custom allocator claim.
        if sdp.lines().filter(|l| l.starts_with("m=")).count() > 3
            || sdp.lines().filter(|l| l.starts_with("m=audio ")).count() > 1
            || sdp.lines().filter(|l| l.starts_with("m=video ")).count() > 1
        {
            return Err(Error::Unsupported);
        }
        let offer = description.kind() == protocol::DescriptionKind::Offer;
        let native = SessionDescription::parse(
            sdp,
            if offer {
                SdpType::Offer
            } else {
                SdpType::Answer
            },
        )
        .map_err(|_| Error::Malformed)?;
        tokio::time::timeout(
            Duration::from_secs(3),
            self.pc.set_remote_description(native),
        )
        .await
        .map_err(|_| Error::Engine)?
        .map_err(|_| Error::Engine)?;
        self.check()?;
        if offer {
            let answer = tokio::time::timeout(
                Duration::from_secs(3),
                self.pc.create_answer(AnswerOptions::default()),
            )
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
        let native = IceCandidate::parse(
            candidate.mid(),
            i32::from(candidate.index()),
            candidate.candidate(),
        )
        .map_err(|_| Error::Malformed)?;
        tokio::time::timeout(Duration::from_secs(3), self.pc.add_ice_candidate(native))
            .await
            .map_err(|_| Error::Engine)?
            .map_err(|_| Error::Engine)?;
        self.check()
    }
    pub fn event(&mut self) -> Result<Option<Event>, Error> {
        self.check()?;
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Track(event) => match event.track {
                    MediaStreamTrack::Audio(track) if self.audio.is_none() => {
                        self.audio = Some(NativeAudioStream::with_options(
                            track,
                            48_000,
                            1,
                            NativeAudioStreamOptions {
                                queue_size_frames: Some(3),
                            },
                        ));
                    }
                    MediaStreamTrack::Video(track) if self.remote_video.is_none() => {
                        self.remote_video = Some(track.clone());
                        if self.receive_video {
                            self.video = Some(NativeVideoStream::with_options(
                                track,
                                NativeVideoStreamOptions {
                                    queue_size_frames: Some(2),
                                },
                            ));
                        }
                    }
                    _ => {
                        self.close();
                        return Err(Error::Unsupported);
                    }
                },
                Event::Channel(channel) => self.add_channel(channel)?,
                Event::Candidate(candidate) => {
                    if self.local_candidates >= 128 {
                        self.close();
                        return Err(Error::Overloaded);
                    }
                    self.local_candidates += 1;
                    return Ok(Some(Event::Candidate(candidate)));
                }
                other => return Ok(Some(other)),
            }
        }
        Ok(None)
    }
    fn add_channel(&mut self, channel: DataChannel) -> Result<(), Error> {
        if self.channels.len() >= 2
            || !matches!(channel.label().as_str(), "status" | "simplewebrtc")
        {
            channel.close();
            self.close();
            return Err(Error::Unsupported);
        }
        let open = self.open.clone();
        let received = self.data_received.clone();
        channel.on_message(Some(Box::new(move |data| {
            if !open.load(Ordering::Acquire) {
                return;
            }
            if data.binary || data.data.len() > 4096 {
                open.store(false, Ordering::Release);
                return;
            }
            // Status cannot remotely unmute the local user. Content is not retained.
            received.fetch_add(1, Ordering::Relaxed);
        })));
        self.channels.push(channel);
        Ok(())
    }
    pub fn send_status(&self, message: &[u8]) -> Result<(), Error> {
        self.check()?;
        if message.len() > 4096 {
            return Err(Error::Malformed);
        }
        let channel = self
            .channels
            .iter()
            .find(|c| c.label() == "status")
            .ok_or(Error::Engine)?;
        if channel.buffered_amount() > 8192 {
            return Err(Error::Overloaded);
        }
        channel.send(message, false).map_err(|_| Error::Engine)
    }
    pub async fn audio(&mut self, samples: &[i16]) -> Result<(), Error> {
        self.check()?;
        if samples.len() != 480 {
            return Err(Error::Malformed);
        }
        if !self.audio_track.enabled() {
            return Err(Error::Closed);
        }
        let frame = AudioFrame {
            data: Cow::Borrowed(samples),
            sample_rate: 48_000,
            num_channels: 1,
            samples_per_channel: 480,
        };
        tokio::time::timeout(
            Duration::from_millis(50),
            self.audio_source.capture_frame(&frame),
        )
        .await
        .map_err(|_| Error::Engine)?
        .map_err(|_| Error::Engine)?;
        self.counters.audio_sent = self.counters.audio_sent.saturating_add(1);
        self.check()
    }
    pub fn video(&mut self, buffer: I420Buffer, timestamp_us: i64) -> Result<(), Error> {
        self.check()?;
        if buffer.width() > 1280 || buffer.height() > 720 {
            return Err(Error::Malformed);
        }
        if !self.video_track.enabled() {
            return Err(Error::Closed);
        }
        let mut frame = VideoFrame::new(VideoRotation::VideoRotation0, buffer);
        frame.timestamp_us = timestamp_us;
        // The maintained source returns false when adaptation drops a frame
        // (including before any sender exists). It is not a capture failure.
        if self.video_source.capture_frame(&frame) {
            self.counters.video_sent = self.counters.video_sent.saturating_add(1);
        }
        Ok(())
    }
    pub fn take_audio(&mut self) -> Option<AudioFrame<'static>> {
        if !self.is_open() {
            return None;
        }
        let frame = self.audio.as_mut()?.next().now_or_never()??;
        if frame.data.len() > 960 || frame.sample_rate != 48_000 || frame.num_channels != 1 {
            self.close();
            return None;
        }
        self.counters.audio_received = self.counters.audio_received.saturating_add(1);
        if frame.data.iter().any(|s| s.unsigned_abs() > 100) {
            self.counters.audio_nonzero = self.counters.audio_nonzero.saturating_add(1);
        }
        Some(frame)
    }
    pub fn take_video(&mut self) -> Option<BoxVideoFrame> {
        if !self.is_open() {
            return None;
        }
        let frame = self.video.as_mut()?.next().now_or_never()??;
        if frame.buffer.width() > 1920 || frame.buffer.height() > 1080 {
            self.close();
            return None;
        }
        self.counters.video_received = self.counters.video_received.saturating_add(1);
        Some(frame)
    }
    /// Local decode subscription. Internal P2P signaling has no SFU unsubscribe;
    /// this bounds application sinks and does not claim reduced wire traffic.
    pub fn receive_video(&mut self, enabled: bool) {
        if self.receive_video == enabled {
            return;
        }
        self.receive_video = enabled;
        self.video = None;
        if enabled
            && self.is_open()
            && let Some(track) = &self.remote_video
        {
            self.video = Some(NativeVideoStream::with_options(
                track.clone(),
                NativeVideoStreamOptions {
                    queue_size_frames: Some(2),
                },
            ));
        }
    }
    pub fn counters(&mut self) -> &Counters {
        self.counters.data_received = self.data_received.load(Ordering::Relaxed);
        &self.counters
    }
    pub fn close(&mut self) {
        self.open.store(false, Ordering::Release);
        self.audio_track.set_enabled(false);
        self.video_track.set_enabled(false);
        self.audio_source.clear_buffer();
        for channel in self.channels.drain(..) {
            channel.on_message(None);
            channel.on_state_change(None);
            channel.close();
        }
        self.audio = None;
        self.video = None;
        self.remote_video = None;
        self.pc.on_ice_candidate(None);
        self.pc.on_track(None);
        self.pc.on_connection_state_change(None);
        self.pc.on_data_channel(None);
        self.pc.close();
        while self.events.try_recv().is_ok() {}
    }
}
impl Drop for NativePeer {
    fn drop(&mut self) {
        self.close();
    }
}
