//! Explicit local MP4/WebM demux and codec branches. No URI handlers,
//! decodebin/autoplugging, HTML, playlists or external resources are admitted.
use super::{Error, Frame, Shared};
use gstreamer::{self as gst, prelude::*};
use gstreamer_app::{AppSink, AppSrc};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use talk_core::protocol::content::{MAX_VIDEO_BYTES, VideoBytes};

const MAX_WIDTH: u32 = 1920;
const MAX_HEIGHT: u32 = 1080;
const MAX_FRAME_BYTES: usize = MAX_WIDTH as usize * MAX_HEIGHT as usize * 4;
const CHUNK: usize = 64 * 1024;

#[derive(Clone, Copy)]
pub(super) enum Container {
    Mp4,
    Webm,
}
impl Container {
    pub(super) fn detect(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.get(..4) == Some(b"\x1aE\xdf\xa3") {
            return Ok(Self::Webm);
        }
        if bytes.get(4..8) == Some(b"ftyp") && bytes.len() >= 16 {
            return Ok(Self::Mp4);
        }
        Err(Error::Unsupported)
    }
}

#[derive(Default)]
struct Signals {
    error: Mutex<Option<Error>>,
    eos: AtomicBool,
    clock_lost: AtomicBool,
    latency: AtomicBool,
    discovered: AtomicBool,
    video: AtomicBool,
    audio: AtomicBool,
    pads: AtomicUsize,
}
impl Signals {
    fn fail(&self, error: Error) {
        if let Ok(mut current) = self.error.lock() {
            current.get_or_insert(error);
        }
    }
}

/// A GstMemory wrapper only clones the bounded original's Arc. Each chunk is a
/// range into the same backing, not an additional copy of the whole attachment.
struct Chunk {
    bytes: VideoBytes,
    start: usize,
    end: usize,
}
impl AsRef<[u8]> for Chunk {
    fn as_ref(&self) -> &[u8] {
        self.bytes
            .as_slice()
            .get(self.start..self.end)
            .unwrap_or_default()
    }
}

pub(super) struct Pipeline {
    pipeline: gst::Pipeline,
    sink: Arc<Mutex<Option<AppSink>>>,
    signals: Arc<Signals>,
    stopped: bool,
}

pub(super) struct Observation {
    pub position_ms: Option<u64>,
    pub duration_ms: Option<u64>,
    /// A current state alone can still have an asynchronous transition pending.
    pub settled_state: Option<gst::State>,
}
impl Pipeline {
    pub(super) fn new(
        bytes: VideoBytes,
        container: Container,
        shared: Arc<Shared>,
    ) -> Result<Self, Error> {
        gst::init().map_err(|_| Error::Unsupported)?;
        let pipeline = gst::Pipeline::new();
        let signals = Arc::new(Signals::default());
        let sink = Arc::new(Mutex::new(None));
        let lifetime = shared.clone();
        let source = source(bytes, shared, signals.clone())?;
        let demux = element(match container {
            Container::Mp4 => "qtdemux",
            Container::Webm => "matroskademux",
        })?;
        pipeline
            .add_many([source.upcast_ref(), &demux])
            .map_err(|_| Error::Decoder)?;
        source.link(&demux).map_err(|_| Error::Decoder)?;
        let weak = pipeline.downgrade();
        let events = signals.clone();
        let output = sink.clone();
        demux.connect_pad_added(move |_, pad| {
            if lifetime.stop.load(Ordering::Acquire) {
                return;
            }
            if events.pads.fetch_add(1, Ordering::Relaxed) >= 8 {
                events.fail(Error::TooLarge);
                return;
            }
            let Some(pipeline) = weak.upgrade() else {
                return;
            };
            let result = add_track(&pipeline, pad, container, &events, &output);
            if let Err(error) = result {
                events.fail(error);
            }
        });
        let events = signals.clone();
        demux.connect_no_more_pads(move |_| {
            if !events.video.load(Ordering::Acquire) {
                events.fail(Error::Unsupported);
            }
            events.discovered.store(true, Ordering::Release);
        });
        let bus = pipeline.bus().ok_or(Error::Decoder)?;
        let events = signals.clone();
        bus.set_sync_handler(move |_, message| {
            match message.view() {
                gst::MessageView::Error(error) => {
                    let raw = error.error();
                    let kind = if raw.matches(gst::CoreError::MissingPlugin)
                        || raw.matches(gst::StreamError::CodecNotFound)
                        || raw.matches(gst::StreamError::WrongType)
                    {
                        Error::Unsupported
                    } else if error
                        .src()
                        .is_some_and(|source| source.name() == "content-audio-output")
                    {
                        Error::AudioUnavailable
                    } else {
                        Error::Decoder
                    };
                    events.fail(kind);
                }
                gst::MessageView::Eos(_) => events.eos.store(true, Ordering::Release),
                gst::MessageView::ClockLost(_) => events.clock_lost.store(true, Ordering::Release),
                gst::MessageView::Latency(_) => events.latency.store(true, Ordering::Release),
                _ => {}
            }
            // No unbounded native event queue. State/position are sampled by the
            // sole worker. Native producers can hold locks here: retain only
            // coalesced signals, never perform state changes in this callback.
            gst::BusSyncReply::Drop
        });
        Ok(Self {
            pipeline,
            sink,
            signals,
            stopped: false,
        })
    }
    pub(super) fn error(&self) -> Option<Error> {
        self.signals.error.lock().ok().and_then(|error| *error)
    }
    pub(super) fn eos(&self) -> bool {
        self.signals.eos.load(Ordering::Acquire)
    }
    pub(super) fn clock_lost(&self) -> bool {
        self.signals.clock_lost.swap(false, Ordering::AcqRel)
    }
    pub(super) fn update_latency(&self) -> Result<(), Error> {
        if self.signals.latency.swap(false, Ordering::AcqRel) {
            self.pipeline
                .recalculate_latency()
                .map_err(|_| Error::ControlFailed)?;
        }
        Ok(())
    }
    pub(super) fn set_playing(&self, playing: bool) -> Result<(), Error> {
        self.pipeline
            .set_state(if playing {
                gst::State::Playing
            } else {
                gst::State::Paused
            })
            .map(|_| ())
            .map_err(|_| Error::ControlFailed)
    }
    pub(super) fn seek(&self, position_ms: u64) -> Result<(), Error> {
        self.pipeline
            .seek_simple(
                gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT,
                gst::ClockTime::from_mseconds(position_ms),
            )
            .map_err(|_| Error::ControlFailed)?;
        self.signals.eos.store(false, Ordering::Release);
        Ok(())
    }
    pub(super) fn frame(&self, preroll: bool) -> Result<Option<Frame>, Error> {
        let sink = self
            .sink
            .lock()
            .map_err(|_| Error::WorkerUnavailable)?
            .clone();
        let Some(sink) = sink else {
            return Ok(None);
        };
        let sample = if preroll {
            sink.try_pull_preroll(gst::ClockTime::ZERO)
        } else {
            sink.try_pull_sample(gst::ClockTime::ZERO)
        };
        sample.map(|sample| frame(&sample)).transpose()
    }
    pub(super) fn observation(&self) -> Observation {
        let position = self
            .pipeline
            .query_position::<gst::ClockTime>()
            .map(|time| time.mseconds());
        let duration = self
            .pipeline
            .query_duration::<gst::ClockTime>()
            .map(|time| time.mseconds());
        let (result, state, pending) = self.pipeline.state(gst::ClockTime::ZERO);
        Observation {
            position_ms: position,
            duration_ms: duration,
            settled_state: (self.signals.discovered.load(Ordering::Acquire)
                && result == Ok(gst::StateChangeSuccess::Success)
                && pending == gst::State::VoidPending)
                .then_some(state),
        }
    }
    pub(super) fn stop(&mut self) {
        if self.stopped {
            return;
        }
        // This native call may stall. Shared's independently observed deadline
        // remains live and capacity stays occupied until NULL is observed.
        let _ = self.pipeline.set_state(gst::State::Null);
        loop {
            let (_, current, pending) = self.pipeline.state(gst::ClockTime::from_mseconds(100));
            if current == gst::State::Null && pending == gst::State::VoidPending {
                self.stopped = true;
                break;
            }
            std::thread::park_timeout(std::time::Duration::from_millis(100));
        }
        if let Ok(mut sink) = self.sink.lock() {
            *sink = None;
        }
    }
}
impl Drop for Pipeline {
    fn drop(&mut self) {
        self.stop();
    }
}

fn source(bytes: VideoBytes, shared: Arc<Shared>, signals: Arc<Signals>) -> Result<AppSrc, Error> {
    let length = bytes.as_slice().len();
    let cursor = Arc::new(Mutex::new(0usize));
    let seek_cursor = cursor.clone();
    let seek_shared = shared.clone();
    let source = AppSrc::builder()
        .format(gst::Format::Bytes)
        .stream_type(gstreamer_app::AppStreamType::RandomAccess)
        .property("blocksize", CHUNK as u32)
        .size(i64::try_from(length).map_err(|_| Error::TooLarge)?)
        .max_buffers(1)
        .max_bytes(MAX_VIDEO_BYTES as u64)
        .block(false)
        .callbacks(
            gstreamer_app::AppSrcCallbacks::builder()
                .need_data(move |source, requested| {
                    if shared.stop.load(Ordering::Acquire) {
                        return;
                    }
                    let Ok(mut offset) = cursor.lock() else {
                        signals.fail(Error::WorkerUnavailable);
                        return;
                    };
                    if *offset >= length {
                        let _ = source.end_of_stream();
                        return;
                    }
                    // Pull mode must answer the exact requested range. Large
                    // moov/keyframe reads use one zero-copy range of the same
                    // reserved original, never a new large byte allocation.
                    // Unknown-size hints retain the normal 64 KiB block size.
                    let amount = match read_amount(requested, *offset, length) {
                        Ok(amount) => amount,
                        Err(error) => {
                            signals.fail(error);
                            let _ = source.end_of_stream();
                            return;
                        }
                    };
                    let end = *offset + amount;
                    let mut buffer = gst::Buffer::new();
                    let Some(writable) = buffer.get_mut() else {
                        signals.fail(Error::WorkerUnavailable);
                        return;
                    };
                    writable.set_offset(*offset as u64);
                    writable.set_offset_end(end as u64);
                    writable.append_memory(gst::Memory::from_slice(Chunk {
                        bytes: bytes.clone(),
                        start: *offset,
                        end,
                    }));
                    *offset = end;
                    if let Err(error) = source.push_buffer(buffer)
                        && error != gst::FlowError::Flushing
                    {
                        signals.fail(Error::Decoder);
                    }
                })
                .seek_data(move |_, position| {
                    if seek_shared.stop.load(Ordering::Acquire) {
                        return false;
                    }
                    let Ok(position) = usize::try_from(position) else {
                        return false;
                    };
                    if position > length {
                        return false;
                    }
                    if let Ok(mut cursor) = seek_cursor.lock() {
                        *cursor = position;
                        true
                    } else {
                        false
                    }
                })
                .build(),
        )
        .build();
    Ok(source)
}

fn read_amount(requested: u32, offset: usize, length: usize) -> Result<usize, Error> {
    if length > MAX_VIDEO_BYTES {
        return Err(Error::TooLarge);
    }
    let remaining = length.checked_sub(offset).ok_or(Error::Malformed)?;
    let requested = if requested == u32::MAX {
        CHUNK
    } else {
        usize::try_from(requested).map_err(|_| Error::TooLarge)?
    };
    if requested == 0 {
        return Err(Error::Malformed);
    }
    if requested > MAX_VIDEO_BYTES {
        return Err(Error::TooLarge);
    }
    Ok(requested.min(remaining))
}

fn element(factory: &str) -> Result<gst::Element, Error> {
    gst::ElementFactory::make(factory)
        .build()
        .map_err(|_| Error::Unsupported)
}

fn add_track(
    pipeline: &gst::Pipeline,
    pad: &gst::Pad,
    container: Container,
    signals: &Arc<Signals>,
    output: &Arc<Mutex<Option<AppSink>>>,
) -> Result<(), Error> {
    let caps = pad.current_caps().ok_or(Error::Malformed)?;
    let structure = caps.structure(0).ok_or(Error::Malformed)?;
    let name = structure.name().as_str();
    if name.starts_with("video/") {
        if signals.video.swap(true, Ordering::AcqRel) {
            return Err(Error::Unsupported);
        }
        validate_dimensions(structure)?;
        let (parser, decoder) = video_codec(container, name)?;
        let queue = queue(4 * 1024 * 1024, 64)?;
        let mut chain = vec![queue];
        if let Some(parser) = parser {
            let parser = element(parser)?;
            let src = parser.static_pad("src").ok_or(Error::Decoder)?;
            let events = signals.clone();
            src.add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, move |_, info| {
                if let Some(gst::PadProbeData::Event(event)) = &info.data
                    && let gst::EventView::Caps(caps) = event.view()
                    && let Some(structure) = caps.caps().structure(0)
                    && let Err(error) = validate_dimensions(structure)
                {
                    events.fail(error);
                    return gst::PadProbeReturn::Drop;
                }
                gst::PadProbeReturn::Ok
            });
            chain.push(parser);
        }
        chain.push(element(decoder)?);
        chain.push(element("videoconvert")?);
        chain.push(element("videoscale")?);
        chain.push(
            gst::ElementFactory::make("videorate")
                .property("drop-only", true)
                .property("max-rate", 30i32)
                .build()
                .map_err(|_| Error::Unsupported)?,
        );
        let caps = gst::Caps::builder("video/x-raw")
            .field("format", "BGRA")
            .field("width", gst::IntRange::new(1, MAX_WIDTH as i32))
            .field("height", gst::IntRange::new(1, MAX_HEIGHT as i32))
            .field(
                "framerate",
                gst::FractionRange::new(gst::Fraction::new(1, 1), gst::Fraction::new(30, 1)),
            )
            .field("pixel-aspect-ratio", gst::Fraction::new(1, 1))
            .build();
        let sink = AppSink::builder()
            .caps(&caps)
            .max_buffers(1)
            .max_bytes(MAX_FRAME_BYTES as u64)
            // One frame already bounds this queue. A second bound by media
            // time makes GstAppSink's drop accounting spin across a flushing
            // seek on GStreamer 1.28/macOS, holding its mutex even against a
            // zero-timeout pull. Keep the independent frame/byte limits.
            .drop(true)
            .sync(true)
            .enable_last_sample(false)
            .wait_on_eos(false)
            .build();
        chain.push(sink.clone().upcast());
        install(pipeline, pad, &chain)?;
        *output.lock().map_err(|_| Error::WorkerUnavailable)? = Some(sink);
    } else if name.starts_with("audio/") {
        if signals.audio.swap(true, Ordering::AcqRel) {
            return Err(Error::Unsupported);
        }
        let (parser, decoder) = audio_codec(container, structure)?;
        if structure
            .get::<i32>("channels")
            .is_ok_and(|channels| !(1..=2).contains(&channels))
            || structure
                .get::<i32>("rate")
                .is_ok_and(|rate| !(1..=192_000).contains(&rate))
        {
            return Err(Error::Unsupported);
        }
        let mut chain = vec![queue(1024 * 1024, 128)?];
        if let Some(parser) = parser {
            chain.push(element(parser)?);
        }
        chain.push(element(decoder)?);
        chain.push(element("audioconvert")?);
        chain.push(element("audioresample")?);
        let caps = gst::Caps::builder("audio/x-raw")
            .field("format", "S16LE")
            .field("rate", 48_000i32)
            .field("channels", gst::IntRange::new(1, 2))
            .field("layout", "interleaved")
            .build();
        chain.push(
            gst::ElementFactory::make("capsfilter")
                .property("caps", &caps)
                .build()
                .map_err(|_| Error::Unsupported)?,
        );
        let output = gst::ElementFactory::make(audio_output())
            .name("content-audio-output")
            .property("sync", true)
            .property("buffer-time", 100_000i64)
            .property("latency-time", 20_000i64)
            .property("enable-last-sample", false)
            .build()
            .map_err(|_| Error::AudioUnavailable)?;
        chain.push(output);
        install(pipeline, pad, &chain)?;
    }
    Ok(())
}

fn video_codec(
    container: Container,
    name: &str,
) -> Result<(Option<&'static str>, &'static str), Error> {
    match (container, name) {
        (Container::Webm, "video/x-vp8") => Ok((None, "vp8dec")),
        (Container::Webm, "video/x-vp9") => Ok((None, "vp9dec")),
        #[cfg(target_os = "macos")]
        (Container::Mp4, "video/x-h264") => Ok((Some("h264parse"), "vtdec")),
        #[cfg(target_os = "macos")]
        (Container::Mp4, "video/x-h265") => Ok((Some("h265parse"), "vtdec")),
        _ => Err(Error::Unsupported),
    }
}
fn audio_codec(
    container: Container,
    structure: &gst::StructureRef,
) -> Result<(Option<&'static str>, &'static str), Error> {
    match (container, structure.name().as_str()) {
        (Container::Webm, "audio/x-opus") => Ok((None, "opusdec")),
        #[cfg(target_os = "macos")]
        (Container::Mp4, "audio/mpeg") if structure.get::<i32>("mpegversion") == Ok(4) => {
            Ok((Some("aacparse"), "atdec"))
        }
        _ => Err(Error::Unsupported),
    }
}
fn audio_output() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "osxaudiosink"
    }
    #[cfg(target_os = "linux")]
    {
        "pulsesink"
    }
    #[cfg(target_os = "windows")]
    {
        "wasapisink"
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        "unconfigured-audio-output"
    }
}
fn validate_dimensions(structure: &gst::StructureRef) -> Result<(), Error> {
    let width = structure
        .get::<i32>("width")
        .map_err(|_| Error::Malformed)?;
    let height = structure
        .get::<i32>("height")
        .map_err(|_| Error::Malformed)?;
    if width <= 0 || height <= 0 {
        return Err(Error::Malformed);
    }
    if width > MAX_WIDTH as i32 || height > MAX_HEIGHT as i32 {
        return Err(Error::TooLarge);
    }
    Ok(())
}
fn queue(bytes: u32, buffers: u32) -> Result<gst::Element, Error> {
    gst::ElementFactory::make("queue")
        .property("max-size-bytes", bytes)
        .property("max-size-buffers", buffers)
        // Encoded track queues must cover native decoder reorder latency while
        // both sinks preroll. A 200ms audio queue deadlocked a valid H264/AAC
        // file whose VideoToolbox decoder reported 333ms. Bytes/count limits
        // remain independent; this is not an unbounded preroll exemption.
        .property("max-size-time", 2_000_000_000u64)
        .build()
        .map_err(|_| Error::Unsupported)
}
fn install(pipeline: &gst::Pipeline, pad: &gst::Pad, chain: &[gst::Element]) -> Result<(), Error> {
    pipeline.add_many(chain).map_err(|_| Error::Decoder)?;
    gst::Element::link_many(chain).map_err(|_| Error::Decoder)?;
    let sink = chain
        .first()
        .and_then(|element| element.static_pad("sink"))
        .ok_or(Error::Decoder)?;
    pad.link(&sink).map_err(|_| Error::Decoder)?;
    for element in chain {
        element
            .sync_state_with_parent()
            .map_err(|_| Error::Decoder)?;
    }
    Ok(())
}
fn frame(sample: &gst::Sample) -> Result<Frame, Error> {
    let info = gstreamer_video::VideoInfo::from_caps(sample.caps().ok_or(Error::Malformed)?)
        .map_err(|_| Error::Malformed)?;
    let (width, height) = (info.width(), info.height());
    if width == 0
        || height == 0
        || width > MAX_WIDTH
        || height > MAX_HEIGHT
        || info.format() != gstreamer_video::VideoFormat::Bgra
    {
        return Err(Error::TooLarge);
    }
    let row = width as usize * 4;
    let stride = usize::try_from(info.stride()[0]).map_err(|_| Error::Malformed)?;
    if stride < row {
        return Err(Error::Malformed);
    }
    let buffer = sample.buffer().ok_or(Error::Malformed)?;
    let map = buffer.map_readable().map_err(|_| Error::Malformed)?;
    let length = row
        .checked_mul(height as usize)
        .filter(|size| *size <= MAX_FRAME_BYTES)
        .ok_or(Error::TooLarge)?;
    let mut bgra = Vec::with_capacity(length);
    for y in 0..height as usize {
        let start = info.offset()[0]
            .checked_add(y.checked_mul(stride).ok_or(Error::Malformed)?)
            .ok_or(Error::Malformed)?;
        let end = start.checked_add(row).ok_or(Error::Malformed)?;
        bgra.extend_from_slice(map.as_slice().get(start..end).ok_or(Error::Malformed)?);
    }
    Ok(Frame {
        width,
        height,
        bgra,
        sequence: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::super::{
        Phase, Player, run_pipeline,
        tests::{settle, synthetic_webm, until},
    };
    use super::*;
    use std::{
        thread,
        time::{Duration, Instant},
    };

    fn spawn(pipeline: Pipeline, shared: Arc<Shared>) -> Player {
        let state = shared.clone();
        let worker = thread::spawn(move || run_pipeline(pipeline, &state, Instant::now()));
        Player {
            shared,
            worker: Some(worker),
        }
    }

    #[test]
    fn pending_track_preroll_defers_play_and_latency_messages_remain_bounded() {
        let shared = Arc::new(Shared::new());
        let pipeline = Pipeline::new(synthetic_webm(), Container::Webm, shared.clone()).unwrap();
        pipeline.set_playing(false).unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        while pipeline.observation().settled_state != Some(gst::State::Paused) {
            assert!(pipeline.error().is_none());
            assert!(Instant::now() < deadline, "initial native preroll");
            thread::sleep(Duration::from_millis(10));
        }

        let sink = pipeline.sink.lock().unwrap().clone().unwrap();
        let latency_events = Arc::new(AtomicUsize::new(0));
        let received = latency_events.clone();
        sink.static_pad("sink").unwrap().add_probe(
            gst::PadProbeType::EVENT_UPSTREAM,
            move |_, info| {
                if let Some(gst::PadProbeData::Event(event)) = &info.data
                    && matches!(event.view(), gst::EventView::Latency(_))
                {
                    received.fetch_add(1, Ordering::Relaxed);
                }
                gst::PadProbeReturn::Ok
            },
        );
        for _ in 0..1_000 {
            pipeline
                .pipeline
                .post_message(gst::message::Latency::new())
                .unwrap();
        }
        assert!(!pipeline.pipeline.bus().unwrap().have_pending());
        pipeline.update_latency().unwrap();
        assert_eq!(latency_events.load(Ordering::Relaxed), 1);
        pipeline.update_latency().unwrap();
        assert_eq!(latency_events.load(Ordering::Relaxed), 1);

        // A newly installed track can leave current=PAUSED, pending=PAUSED.
        // Its first buffer is controlled here, independent of codec/device speed.
        let source = AppSrc::builder()
            .format(gst::Format::Time)
            .max_buffers(1)
            .max_bytes(1)
            .block(false)
            .build();
        let waiting = gst::ElementFactory::make("fakesink")
            .property("sync", false)
            .property("enable-last-sample", false)
            .build()
            .unwrap();
        pipeline
            .pipeline
            .add_many([source.upcast_ref(), &waiting])
            .unwrap();
        source.link(&waiting).unwrap();
        source.sync_state_with_parent().unwrap();
        waiting.sync_state_with_parent().unwrap();
        let native = pipeline.pipeline.clone();
        let (_, current, pending) = native.state(gst::ClockTime::ZERO);
        assert_eq!((current, pending), (gst::State::Paused, gst::State::Paused));
        assert_eq!(pipeline.observation().settled_state, None);

        let mut player = spawn(pipeline, shared);
        player.set_playing(true).unwrap();
        let complete_after = player.shared.completed_iterations.load(Ordering::Acquire) + 2;
        until(&player, |snapshot| {
            snapshot.phase == Phase::Starting
                && player.shared.completed_iterations.load(Ordering::Acquire) >= complete_after
                && player.shared.frame.lock().unwrap().is_some()
        });
        assert_eq!(native.state(gst::ClockTime::ZERO).2, gst::State::Paused);
        source
            .push_buffer(gst::Buffer::with_size(1).unwrap())
            .unwrap();
        until(&player, |snapshot| {
            snapshot.phase == Phase::Playing && snapshot.position_ms > 100
        });
        player.stop();
        until(&player, |snapshot| snapshot.phase == Phase::Stopped);
        settle(&mut player);
    }

    #[test]
    fn lost_clock_cannot_leave_a_playing_owner_frozen() {
        let shared = Arc::new(Shared::new());
        let pipeline = Pipeline::new(synthetic_webm(), Container::Webm, shared.clone()).unwrap();
        let native = pipeline.pipeline.clone();
        // A dedicated clock models a retired device clock. Never modify the
        // process-wide system clock or depend on a physical output route change.
        let retired = gst::glib::Object::builder::<gst::SystemClock>()
            .property("name", "retired-content-clock")
            .build();
        let time = retired.internal_time();
        retired.set_calibration(time, time, 1, 1_000_000_000);
        assert_ne!(
            retired.upcast_ref::<gst::Clock>(),
            &gst::SystemClock::obtain()
        );
        native.use_clock(Some(&retired));
        let signals = pipeline.signals.clone();
        let mut player = spawn(pipeline, shared);
        until(&player, |snapshot| snapshot.phase == Phase::Paused);
        player.set_playing(true).unwrap();
        until(&player, |snapshot| snapshot.phase == Phase::Playing);
        assert_eq!(native.clock().as_ref(), Some(retired.upcast_ref()));
        assert_eq!(player.snapshot().position_ms, 0);

        // Restore normal selection policy, then deliver the same native event
        // as a disappearing audio clock. Re-selection needs a state cycle.
        native.auto_clock();
        native
            .post_message(
                gst::message::ClockLost::builder(retired.upcast_ref())
                    .src(&native)
                    .build(),
            )
            .unwrap();
        until(&player, |snapshot| {
            snapshot.phase == Phase::Playing && snapshot.position_ms > 100
        });
        assert_ne!(native.clock().as_ref(), Some(retired.upcast_ref()));

        player.set_playing(false).unwrap();
        until(&player, |snapshot| snapshot.phase == Phase::Paused);
        native
            .post_message(gst::message::ClockLost::new(&native.clock().unwrap()))
            .unwrap();
        let complete_after = player.shared.completed_iterations.load(Ordering::Acquire) + 2;
        until(&player, |snapshot| {
            snapshot.phase == Phase::Paused
                && player.shared.completed_iterations.load(Ordering::Acquire) >= complete_after
                && !signals.clock_lost.load(Ordering::Acquire)
        });
        assert!(!player.shared.playing.load(Ordering::Acquire));
        assert_eq!(native.current_state(), gst::State::Paused);
        player.stop();
        // NULL may already have flushed the bus; either result must preserve
        // the irreversible Stop and confirmed worker exit.
        let _ = native.post_message(gst::message::ClockLost::new(&retired.upcast()));
        until(&player, |snapshot| snapshot.phase == Phase::Stopped);
        settle(&mut player);
        assert_eq!(native.current_state(), gst::State::Null);
    }

    #[test]
    fn compressed_metadata_is_rejected_before_decoder_creation() {
        gst::init().unwrap();
        for (width, height, expected) in [
            (1920, 1080, Ok(())),
            (1921, 1080, Err(Error::TooLarge)),
            (1920, 1081, Err(Error::TooLarge)),
            (i32::MAX, i32::MAX, Err(Error::TooLarge)),
            (0, 10, Err(Error::Malformed)),
            (-1, 10, Err(Error::Malformed)),
        ] {
            let caps = gst::Structure::builder("video/x-vp8")
                .field("width", width)
                .field("height", height)
                .build();
            assert_eq!(validate_dimensions(&caps), expected);
        }
        let missing = gst::Structure::builder("video/x-h264").build();
        assert_eq!(validate_dimensions(&missing), Err(Error::Malformed));
        assert_eq!(
            video_codec(Container::Webm, "video/x-h264"),
            Err(Error::Unsupported)
        );
        assert_eq!(
            video_codec(Container::Mp4, "video/x-raw"),
            Err(Error::Unsupported)
        );
        let audio = gst::Structure::builder("audio/mpeg")
            .field("mpegversion", 1i32)
            .build();
        assert_eq!(audio_codec(Container::Mp4, &audio), Err(Error::Unsupported));
    }

    #[test]
    fn large_demux_reads_share_the_original_and_ranges_stay_finite() {
        gst::init().unwrap();
        assert_eq!(read_amount(128 * 1024, 0, 256 * 1024), Ok(128 * 1024));
        assert_eq!(read_amount(u32::MAX, 0, 256 * 1024), Ok(CHUNK));
        assert_eq!(read_amount(MAX_VIDEO_BYTES as u32, 9, 10), Ok(1));
        assert_eq!(read_amount(1, 11, 10), Err(Error::Malformed));
        assert_eq!(read_amount(0, 0, 10), Err(Error::Malformed));
        assert_eq!(
            read_amount(MAX_VIDEO_BYTES as u32 + 1, 0, 10),
            Err(Error::TooLarge)
        );
        assert_eq!(read_amount(1, 0, MAX_VIDEO_BYTES + 1), Err(Error::TooLarge));
        let bytes = VideoBytes::new(vec![31; 256 * 1024]).unwrap();
        let original = bytes.as_slice().as_ptr();
        let memory = gst::Memory::from_slice(Chunk {
            bytes: bytes.clone(),
            start: 1,
            end: 128 * 1024,
        });
        let mapped = memory.map_readable().unwrap();
        assert_eq!(mapped.as_ptr(), original.wrapping_add(1));
        assert_eq!(mapped.len(), 128 * 1024 - 1);
    }
}
