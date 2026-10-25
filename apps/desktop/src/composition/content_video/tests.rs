use super::*;
use gstreamer::{self as gst, prelude::*};

#[test]
fn controls_are_bounded_coalesced_and_stop_has_priority() {
    let shared = Shared::new();
    for position in 0..10_000 {
        *shared.seek.lock().unwrap() = Some(position);
        shared.playing.store(position % 2 == 0, Ordering::Release);
    }
    assert_eq!(shared.seek.lock().unwrap().take(), Some(9_999));
    assert!(!shared.playing.load(Ordering::Acquire));
    shared.frame.lock().unwrap().replace(Frame {
        width: 1,
        height: 1,
        bgra: vec![0; 4],
        sequence: 1,
    });
    shared.stop();
    shared.phase(Phase::Playing);
    assert_eq!(shared.snapshot().phase, Phase::Stopping);
    assert!(shared.frame.lock().unwrap().is_none());
    *shared.stop_at.lock().unwrap() = Some(Instant::now() - STOP_LIMIT);
    assert!(shared.snapshot().stop_unconfirmed);
    shared.phase(Phase::Stopped);
    assert!(!shared.snapshot().stop_unconfirmed);
}

#[test]
fn container_and_seek_admission_are_finite() {
    assert!(matches!(
        pipeline::Container::detect(b"\x1aE\xdf\xa3rest"),
        Ok(pipeline::Container::Webm)
    ));
    assert!(matches!(
        pipeline::Container::detect(b"\0\0\0\x18ftypisom\0\0\0\0"),
        Ok(pipeline::Container::Mp4)
    ));
    for value in [
        b"https://example.invalid/video.mp4".as_slice(),
        b"#EXTM3U",
        b"",
        b"<html>",
    ] {
        assert!(matches!(
            pipeline::Container::detect(value),
            Err(Error::Unsupported)
        ));
    }
    assert_eq!(clamp_seek(u64::MAX, Some(2_000)), 1_999);
    assert_eq!(clamp_seek(20, Some(0)), 0);
    assert_eq!(clamp_seek(u64::MAX, None), MAX_POSITION_MS);
}

#[test]
fn unconfirmed_worker_exit_cannot_authorize_replacement() {
    let shared = Arc::new(Shared::new());
    let (release, wait) = std::sync::mpsc::sync_channel::<()>(1);
    let state = shared.clone();
    let worker = thread::spawn(move || {
        wait.recv().unwrap();
        state.phase(Phase::Stopped);
    });
    let mut player = Player {
        shared,
        worker: Some(worker),
    };
    player.stop();
    *player.shared.stop_at.lock().unwrap() = Some(Instant::now() - STOP_LIMIT);
    assert!(player.snapshot().stop_unconfirmed);
    assert!(!player.is_stopped());
    assert!(!player.reap());
    assert_eq!(player.set_playing(true), Err(Error::ControlFailed));
    assert_eq!(player.seek_ms(2_000), Err(Error::ControlFailed));
    release.send(()).unwrap();
    until(&player, |snapshot| snapshot.phase == Phase::Stopped);
    // The terminal snapshot can precede thread return by a few instructions.
    let deadline = Instant::now() + Duration::from_secs(2);
    while !player.reap() {
        assert!(Instant::now() < deadline);
        thread::yield_now();
    }
    assert!(!player.snapshot().stop_unconfirmed);
    assert!(player.is_stopped());
}

pub(super) fn synthetic_webm() -> VideoBytes {
    gst::init().unwrap();
    // Entirely synthetic codec work: no audio device, camera, server or GUI.
    synthetic(
        "videotestsrc num-buffers=30 pattern=red ! video/x-raw,width=160,height=90,framerate=15/1 ! vp8enc deadline=1 keyframe-max-dist=5 ! webmmux ! filesink name=fixture sync=false",
    )
}

fn synthetic(description: &str) -> VideoBytes {
    static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    struct Fixture(std::path::PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let directory = std::env::temp_dir().join(format!(
        "nmt-video-fixture-{}-{}",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&directory).unwrap();
    let fixture = Fixture(directory);
    let path = fixture.0.join("synthetic-video");
    let pipeline = gst::parse::launch(description)
        .unwrap()
        .downcast::<gst::Pipeline>()
        .unwrap();
    let sink = pipeline.by_name("fixture").unwrap();
    sink.set_property("location", path.to_str().unwrap());
    let bus = pipeline.bus().unwrap();
    pipeline.set_state(gst::State::Playing).unwrap();
    let finished = bus
        .timed_pop_filtered(
            gst::ClockTime::from_seconds(5),
            &[gst::MessageType::Error, gst::MessageType::Eos],
        )
        .expect("finite synthetic encoder");
    pipeline.set_state(gst::State::Null).unwrap();
    assert!(
        matches!(finished.view(), gst::MessageView::Eos(_)),
        "synthetic encoder failed: {finished:?}"
    );
    let file = std::fs::File::open(&path).unwrap();
    use std::io::Read;
    let mut bytes = Vec::new();
    file.take(MAX_VIDEO_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .unwrap();
    VideoBytes::new(bytes).unwrap()
}

#[track_caller]
pub(super) fn until(player: &Player, condition: impl Fn(Snapshot) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let snapshot = player.snapshot();
        if condition(snapshot) {
            return;
        }
        assert!(!matches!(snapshot.phase, Phase::Failed(_)), "{snapshot:?}");
        assert!(Instant::now() < deadline, "playback deadline: {snapshot:?}");
        thread::sleep(Duration::from_millis(10));
    }
}

pub(super) fn settle(player: &mut Player) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !player.reap() {
        assert!(Instant::now() < deadline, "worker did not settle");
        thread::yield_now();
    }
}

#[test]
fn synthetic_webm_prerolls_without_autoplay_seeks_plays_ends_and_stops() {
    let bytes = synthetic_webm();
    let mut player = Player::start(bytes.clone()).unwrap();
    assert!(matches!(Player::start(bytes), Err(Error::Busy)));
    until(&player, |snapshot| snapshot.phase == Phase::Paused);
    let first = player.take_frame().expect("real decoded preroll");
    assert_eq!((first.width, first.height), (160, 90));
    assert_eq!(first.bgra.len(), 160 * 90 * 4);
    // Red BGRA, not RGBA or an authored thumbnail.
    assert!(
        first
            .bgra
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[2] > 180 && pixel[0] < 40)
    );
    assert_eq!(player.snapshot().position_ms, 0);
    assert!(
        player
            .snapshot()
            .duration_ms
            .is_some_and(|duration| (1_900..=2_100).contains(&duration)),
        "{:?}",
        player.snapshot()
    );
    player.seek_ms(1_000).unwrap();
    until(&player, |snapshot| {
        snapshot.phase == Phase::Paused && snapshot.position_ms >= 600
    });
    let seek = player.take_frame().expect("seek preroll frame");
    assert!(seek.sequence > first.sequence);
    player.set_playing(true).unwrap();
    until(&player, |snapshot| {
        snapshot.phase == Phase::Playing && snapshot.position_ms > 1_000
    });
    until(&player, |snapshot| snapshot.phase == Phase::Ended);
    player.set_playing(true).unwrap();
    until(&player, |snapshot| {
        snapshot.phase == Phase::Playing && (100..1_000).contains(&snapshot.position_ms)
    });
    player.set_playing(false).unwrap();
    until(&player, |snapshot| snapshot.phase == Phase::Paused);
    player.stop();
    assert!(player.take_frame().is_none());
    assert_eq!(player.seek_ms(0), Err(Error::ControlFailed));
    until(&player, |snapshot| snapshot.phase == Phase::Stopped);
    settle(&mut player);
    assert!(!OCCUPIED.load(Ordering::Acquire));

    // A malformed admitted container fails explicitly and relinquishes capacity
    // only after its native pipeline has returned to NULL.
    let mut invalid =
        Player::start(VideoBytes::new(b"\x1aE\xdf\xa3not a media document".to_vec()).unwrap())
            .unwrap();
    until(&invalid, |snapshot| {
        matches!(snapshot.phase, Phase::Failed(_))
    });
    assert!(invalid.take_frame().is_none());
    settle(&mut invalid);

    // A valid frame can span more than 100ms of media time. Frame count and
    // bytes bound the appsink; its drop path must still allow slow clips.
    let bytes = synthetic(
        "videotestsrc num-buffers=2 pattern=red ! video/x-raw,width=160,height=90,framerate=1/1 ! vp8enc deadline=1 keyframe-max-dist=1 ! webmmux ! filesink name=fixture sync=false",
    );
    let mut slow = Player::start(bytes).unwrap();
    until(&slow, |snapshot| snapshot.phase == Phase::Paused);
    assert!(slow.take_frame().is_some());
    slow.set_playing(true).unwrap();
    until(&slow, |snapshot| {
        snapshot.phase == Phase::Playing && snapshot.position_ms > 100
    });
    until(&slow, |snapshot| snapshot.phase == Phase::Ended);
    slow.stop();
    until(&slow, |snapshot| snapshot.phase == Phase::Stopped);
    settle(&mut slow);

    #[cfg(target_os = "macos")]
    {
        // Apple's native encoder/decoder, no GPL encoder or libav dependency.
        // Silence exercises the actual AAC/audio output branch without a tone.
        let bytes = synthetic(
            "mp4mux name=mux ! filesink name=fixture sync=false videotestsrc num-buffers=30 pattern=ball ! video/x-raw,width=160,height=96,framerate=15/1 ! vtenc_h264 realtime=true allow-frame-reordering=false max-keyframe-interval=5 ! h264parse ! mux.video_0 audiotestsrc num-buffers=200 samplesperbuffer=480 wave=silence ! audio/x-raw,rate=48000,channels=2 ! atenc ! aacparse ! mux.audio_0",
        );
        let mut mp4 = Player::start(bytes).unwrap();
        until(&mp4, |snapshot| snapshot.phase == Phase::Paused);
        let frame = mp4.take_frame().expect("native H264 preroll");
        assert_eq!((frame.width, frame.height), (160, 96));
        assert_eq!(mp4.snapshot().position_ms, 0);
        mp4.set_playing(true).unwrap();
        until(&mp4, |snapshot| {
            snapshot.phase == Phase::Playing && snapshot.position_ms > 100
        });
        mp4.set_playing(false).unwrap();
        until(&mp4, |snapshot| snapshot.phase == Phase::Paused);
        mp4.seek_ms(1_000).unwrap();
        until(&mp4, |snapshot| {
            snapshot.phase == Phase::Paused && snapshot.position_ms >= 600
        });
        let seek = mp4.take_frame().expect("native H264 seek preroll");
        assert!(seek.sequence > frame.sequence);
        assert_ne!(
            seek.bgra, frame.bgra,
            "seek must decode a different media frame"
        );
        mp4.set_playing(true).unwrap();
        until(&mp4, |snapshot| {
            snapshot.phase == Phase::Playing && snapshot.position_ms > 1_000
        });
        until(&mp4, |snapshot| snapshot.phase == Phase::Ended);
        mp4.set_playing(true).unwrap();
        until(&mp4, |snapshot| {
            snapshot.phase == Phase::Playing && (100..1_000).contains(&snapshot.position_ms)
        });
        mp4.stop();
        until(&mp4, |snapshot| snapshot.phase == Phase::Stopped);
        settle(&mut mp4);
    }
}
