//! Opt-in migration acceptance probes. These do not select a shipping backend.
//! Keep encrypted-frame acceptance executable: rtpvp8pay parses compressed VP8
//! partitions, whereas Talk leaves only the 10/3-byte codec prefix readable.
use super::{codecs::Codec, *};
use gstreamer::{self as gst, prelude::*};
use gstreamer_app::AppSrc;
use gstreamer_webrtc as wr;

struct Peer {
    pipeline: gst::Pipeline,
    rtc: gst::Element,
    candidates: Receiver<(u32, String)>,
    failed: Arc<AtomicBool>,
}
impl Peer {
    fn new() -> Self {
        gst::init().unwrap();
        let pipeline = gst::parse::launch(concat!(
            "webrtcbin name=rtc bundle-policy=max-bundle latency=30 ",
            "appsrc name=audio is-live=true format=time block=false do-timestamp=true ",
            "caps=audio/x-raw,format=S16LE,layout=interleaved,rate=48000,channels=1 ! ",
            "opusenc frame-size=10 ! rtpopuspay pt=111 ! ",
            "application/x-rtp,media=audio,encoding-name=OPUS,payload=111,clock-rate=48000 ! rtc. ",
            "appsrc name=video is-live=true format=time block=false do-timestamp=true ",
            "caps=video/x-raw,format=RGBA,width=320,height=240,framerate=30/1 ! ",
            "videoconvert ! vp8enc deadline=1 cpu-used=8 threads=2 keyframe-max-dist=30 ! ",
            "rtpvp8pay pt=96 ! application/x-rtp,media=video,encoding-name=VP8,payload=96,clock-rate=90000 ! rtc."
        )).unwrap().downcast::<gst::Pipeline>().unwrap();
        let rtc = pipeline.by_name("rtc").unwrap();
        let (send, candidates) = mpsc::sync_channel(128);
        let failed = Arc::new(AtomicBool::new(false));
        let flag = failed.clone();
        rtc.connect("on-ice-candidate", false, move |values| {
            let item = (
                values[1].get::<u32>().unwrap(),
                values[2].get::<String>().unwrap(),
            );
            if send.try_send(item).is_err() {
                flag.store(true, Ordering::Release);
            }
            None
        });
        let weak = pipeline.downgrade();
        let flag = failed.clone();
        rtc.connect_pad_added(move |_, pad| {
            if pad.direction() != gst::PadDirection::Src { return; }
            let Some(pipeline) = weak.upgrade() else { return; };
            let result = (|| {
                let caps = pad.current_caps()?;
                let video = caps.structure(0)?.get::<String>("media").ok()? == "video";
                let description = if video {
                    "queue max-size-buffers=64 max-size-bytes=131072 max-size-time=100000000 leaky=downstream ! rtpvp8depay ! vp8dec ! videoconvert ! video/x-raw,format=RGBA ! appsink name=remote_video sync=false async=false max-buffers=2 drop=true enable-last-sample=false"
                } else {
                    "queue max-size-buffers=5 max-size-bytes=20480 max-size-time=100000000 leaky=downstream ! rtpopusdepay ! opusdec ! audioconvert ! audioresample ! audio/x-raw,format=S16LE,rate=48000,channels=1 ! audiobuffersplit output-buffer-duration=1/100 strict-buffer-size=true ! appsink name=remote_audio sync=false async=false max-buffers=3 drop=true enable-last-sample=false"
                };
                let bin = gst::parse::bin_from_description(description, true).ok()?;
                pipeline.add(&bin).ok()?;
                pad.link(&bin.static_pad("sink")?).ok()?;
                bin.sync_state_with_parent().ok()?;
                Some(())
            })();
            if result.is_none() { flag.store(true, Ordering::Release); }
        });
        for name in ["audio", "video"] {
            let src = pipeline
                .by_name(name)
                .unwrap()
                .downcast::<AppSrc>()
                .unwrap();
            src.set_max_buffers(3);
            src.set_max_bytes(if name == "audio" { 2880 } else { 320 * 240 * 8 });
            src.set_leaky_type(gstreamer_app::AppLeakyType::Downstream);
        }
        pipeline.set_state(gst::State::Playing).unwrap();
        Self {
            pipeline,
            rtc,
            candidates,
            failed,
        }
    }
    async fn description(&self, action: &str, field: &'static str) -> wr::WebRTCSessionDescription {
        self.description_options(action, field, None).await
    }
    async fn description_options(
        &self,
        action: &str,
        field: &'static str,
        options: Option<gst::Structure>,
    ) -> wr::WebRTCSessionDescription {
        let (send, receive) = tokio::sync::oneshot::channel();
        let promise = gst::Promise::with_change_func(move |reply| {
            let description = reply
                .ok()
                .flatten()
                .and_then(|s| s.get::<wr::WebRTCSessionDescription>(field).ok());
            let _ = send.send(description);
        });
        self.rtc.emit_by_name::<()>(action, &[&options, &promise]);
        tokio::time::timeout(Duration::from_secs(3), receive)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    }
    async fn set(&self, action: &str, description: &wr::WebRTCSessionDescription) {
        let (send, receive) = tokio::sync::oneshot::channel();
        let promise = gst::Promise::with_change_func(move |reply| {
            let success = reply.is_ok_and(|s| s.is_none_or(|s| !s.has_field("error")));
            let _ = send.send(success);
        });
        self.rtc
            .emit_by_name::<()>(action, &[description, &promise]);
        assert!(
            tokio::time::timeout(Duration::from_secs(3), receive)
                .await
                .unwrap()
                .unwrap()
        );
    }
    fn pump_ice(&self, other: &Self) {
        while let Ok((line, candidate)) = self.candidates.try_recv() {
            other
                .rtc
                .emit_by_name::<()>("add-ice-candidate", &[&line, &candidate]);
        }
        assert!(
            !self.failed.load(Ordering::Acquire),
            "bounded callback or receive branch failed"
        );
        if let Some(error) = self
            .pipeline
            .bus()
            .unwrap()
            .pop_filtered(&[gst::MessageType::Error])
        {
            // Opt-in local assertion only; public diagnostics never expose native errors.
            panic!("pipeline failed: {error:?}");
        }
    }
    fn push(&self, name: &str, bytes: Vec<u8>) {
        self.pipeline
            .by_name(name)
            .unwrap()
            .downcast::<AppSrc>()
            .unwrap()
            .push_buffer(gst::Buffer::from_mut_slice(bytes))
            .unwrap();
    }
    fn drain(&self, name: &str) -> u64 {
        use gstreamer_app::AppSink;
        let Some(sink) = self.pipeline.by_name(name) else {
            return 0;
        };
        let sink = sink.downcast::<AppSink>().unwrap();
        let mut count = 0;
        while let Some(sample) = sink.try_pull_sample(gst::ClockTime::ZERO) {
            let buffer = sample.buffer().unwrap();
            let data = buffer.map_readable().unwrap();
            if name == "remote_audio" {
                assert_eq!(data.len(), 960);
                if data
                    .as_slice()
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .any(|b| i16::from_le_bytes(*b).unsigned_abs() > 500)
                {
                    count += 1;
                }
            } else {
                assert_eq!(data.len(), 320 * 240 * 4);
                count += 1;
            }
        }
        count
    }
}
impl Drop for Peer {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

#[tokio::test]
#[ignore = "migration probe: requires webrtcbin, libnice, DTLS/SRTP and local ICE"]
async fn webrtcbin_sdp_ice_and_connected_codecs_exchange_fresh_av() {
    let a = Peer::new();
    let b = Peer::new();
    let offer = a.description("create-offer", "offer").await;
    a.set("set-local-description", &offer).await;
    b.set("set-remote-description", &offer).await;
    let answer = b.description("create-answer", "answer").await;
    b.set("set-local-description", &answer).await;
    a.set("set-remote-description", &answer).await;
    let start = Instant::now();
    let mut counts = [[0_u64; 2]; 2];
    let mut at_three_seconds = None;
    let mut tick = tokio::time::interval(Duration::from_millis(10));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    for frame in 0..800_u32 {
        tick.tick().await;
        a.pump_ice(&b);
        b.pump_ice(&a);
        for (index, peer) in [&a, &b].into_iter().enumerate() {
            peer.push(
                "audio",
                (0..480)
                    .flat_map(|n| {
                        ((((frame * 480 + n) as f64 * 0.06).sin() * 6000.0) as i16).to_le_bytes()
                    })
                    .collect(),
            );
            if frame % 3 == 0 {
                peer.push("video", [frame as u8, 80, 160, 255].repeat(320 * 240));
            }
            counts[index][0] += peer.drain("remote_audio");
            counts[index][1] += peer.drain("remote_video");
        }
        if at_three_seconds.is_none() && start.elapsed() >= Duration::from_secs(3) {
            at_three_seconds = Some(counts);
        }
    }
    let before = at_three_seconds.unwrap();
    for index in 0..2 {
        assert!(
            counts[index][0] > before[index][0] + 100,
            "fresh audio: {counts:?}"
        );
        assert!(
            counts[index][1] > before[index][1] + 20,
            "fresh video: {counts:?}"
        );
    }
}

/// This is an acceptance gate, not an expected-failure test. Until it passes,
/// do not put Talk E2EE upstream of rtpvp8pay in the production call pipeline.
#[test]
#[ignore = "migration acceptance gate: sealed Talk VP8 must survive the native payloader"]
fn talk_e2ee_vp8_survives_gstreamer_payloader() {
    sealed_vp8_payloader("rtpvp8pay")
}

#[test]
#[ignore = "migration probe: alternative rsrtp payloader, not a shipping backend"]
fn talk_e2ee_vp8_survives_alternative_rust_payloader() {
    gst::init().unwrap();
    gstrsrtp::plugin_register_static().unwrap();
    sealed_vp8_payloader("rtpvp8pay2")
}

fn sealed_vp8_payloader(payloader: &str) {
    use crate::media::e2ee::frame::{Decryptor, Encryptor, FrameKind};
    let encoder = Codec::video_encoder(320, 240, false).unwrap();
    let mut encryptor = Encryptor::default();
    let mut decryptor = Decryptor::default();
    encryptor.set_key(&[5; 32], 2).unwrap();
    decryptor.set_key(&[5; 32], 2).unwrap();
    // Each stream keeps its keyframe/depayloader state across delta frames.
    let make = || {
        Codec::new(&format!(
        "appsrc name=input is-live=true format=time caps=video/x-vp8,width=320,height=240,framerate=30/1 ! {payloader} ! rtpvp8depay ! appsink name=output sync=false async=false max-buffers=1 drop=true enable-last-sample=false"
    ), 2, 1 << 20).unwrap()
    };
    let plain_pipe = make();
    let sealed_pipe = make();
    for frame in 0..12_u32 {
        encoder
            .push(
                [frame as u8 * 20, 90, 170, 255].repeat(320 * 240),
                Some(Duration::from_millis(33 * u64::from(frame))),
                Some(Duration::from_millis(33)),
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let (plain, duration) = loop {
            if let Some(encoded) = encoder.encoded(1 << 20).unwrap() {
                break encoded;
            }
            assert!(Instant::now() < deadline, "encoder timed out");
            std::thread::sleep(Duration::from_millis(5));
        };
        let kind = if plain[0] & 1 == 0 {
            FrameKind::VideoKey
        } else {
            FrameKind::VideoDelta
        };
        let sealed = encryptor
            .encrypt(0x1234, frame * 3000, kind, &plain)
            .unwrap();
        assert_eq!(
            decryptor.decrypt(kind, &sealed).unwrap(),
            plain,
            "valid Talk frame"
        );
        // Control: the same native payloader accepts the original codec frame.
        for (label, target, expected) in [
            ("plain", &plain_pipe, plain),
            ("sealed", &sealed_pipe, sealed),
        ] {
            target
                .push(
                    expected.clone(),
                    Some(Duration::from_millis(33 * u64::from(frame))),
                    Some(duration),
                )
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(1);
            let roundtrip = loop {
                match target.encoded(1 << 20) {
                    Ok(Some((data, _))) => break data,
                    Ok(None) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    _ => panic!(
                        "{label} VP8 frame {frame} rejected by {payloader}; encryption must precede RTP fragmentation"
                    ),
                }
            };
            assert_eq!(
                roundtrip, expected,
                "{label} frame {frame} preserved byte for byte"
            );
        }
    }
}

/// The production peer currently restarts ICE without replacing its call scope.
/// A replacement engine must actually rotate both credentials, not acknowledge
/// an option while returning the old SDP credentials.
#[tokio::test]
#[ignore = "migration acceptance gate: pinned webrtcbin must implement ICE restart"]
async fn webrtcbin_ice_restart_rotates_credentials() {
    let a = Peer::new();
    let b = Peer::new();
    let offer = a.description("create-offer", "offer").await;
    a.set("set-local-description", &offer).await;
    b.set("set-remote-description", &offer).await;
    let answer = b.description("create-answer", "answer").await;
    b.set("set-local-description", &answer).await;
    a.set("set-remote-description", &answer).await;
    let options = gst::Structure::builder("application/x-gst-webrtc-offer-options")
        .field("ice-restart", true)
        .build();
    let restart = a
        .description_options("create-offer", "offer", Some(options))
        .await;
    let credentials = |description: &wr::WebRTCSessionDescription| {
        let sdp = description.sdp().as_text().unwrap();
        sdp.lines()
            .filter(|line| line.starts_with("a=ice-ufrag:") || line.starts_with("a=ice-pwd:"))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let previous = credentials(&offer);
    let next = credentials(&restart);
    assert!(!previous.is_empty() && previous.len() == next.len());
    // Never include the credential strings in assertion output.
    assert!(
        previous.iter().zip(&next).all(|(old, new)| old != new),
        "webrtcbin returned unchanged ICE credentials for ice-restart=true"
    );
}
