//! One source selection per share generation. The worker owns the portal session
//! and PipeWire FD until the pipeline is Null; cancellation never selects again.
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};
use talk_adapters::media::CaptureHandle;
use talk_core::calls::{ShareFailure, SharingState};

pub(super) struct Capture {
    cancel: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Capture {
    pub(super) fn start(handle: CaptureHandle, generation: u64) -> Result<Self, ()> {
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let worker = std::thread::Builder::new()
            .name("screen-capture".into())
            .spawn(move || {
                let active = || {
                    !stop.load(Ordering::Acquire) && handle.screen_generation() == Some(generation)
                };
                // Isolate dependency assertions on malformed native replies.
                // A failed worker must latch failure, never reopen a picker.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    capture(&handle, generation, &active)
                }))
                .unwrap_or(Err(ShareFailure::Unavailable));
                if active()
                    && let Err(reason) = result
                {
                    handle.screen_state(generation, SharingState::Failed(reason));
                }
            })
            .map_err(|_| ())?;
        Ok(Self {
            cancel,
            worker: Some(worker),
        })
    }
    pub(super) fn finished(&self) -> bool {
        self.worker.as_ref().is_none_or(JoinHandle::is_finished)
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
#[cfg(target_os = "linux")]
fn capture(
    handle: &CaptureHandle,
    generation: u64,
    active: &impl Fn() -> bool,
) -> Result<(), ShareFailure> {
    use ashpd::desktop::{
        PersistMode,
        screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType},
    };
    use futures_lite::{StreamExt, future};
    use std::os::fd::AsRawFd;
    async_io::block_on(async {
        let connection = deadline(ashpd::zbus::Connection::session(), Duration::from_secs(3))
            .await?
            .map_err(|_| ShareFailure::Unavailable)?;
        let result = async {
        let portal = deadline(Screencast::with_connection(connection.clone()), Duration::from_secs(3))
            .await?
            .map_err(|_| ShareFailure::Unavailable)?;
        // The private connection owns even a late CreateSession completion:
        // closing it below retires sessions after creation/picker timeouts too.
        let session = deadline(
            portal.create_session(Default::default()),
            Duration::from_secs(3),
        )
        .await?
        .map_err(|_| ShareFailure::Unavailable)?;
        let result = future::or(async {
            while active() { async_io::Timer::after(Duration::from_millis(20)).await; }
            Err(ShareFailure::Cancelled)
        }, async {
            let mut closed = deadline(session.receive_closed(), Duration::from_secs(3)).await?
                .map_err(|_| ShareFailure::Unavailable)?;
            future::or(async { closed.next().await; Err(ShareFailure::SourceClosed) }, async {
                let sources = deadline(portal.available_source_types(), Duration::from_secs(3)).await?
                    .map_err(|_| ShareFailure::Unavailable)? & (SourceType::Monitor | SourceType::Window);
                if sources.is_empty() { return Err(ShareFailure::Unavailable); }
                deadline(portal.select_sources(&session, SelectSourcesOptions::default()
                    .set_sources(sources).set_multiple(false)
                    .set_cursor_mode(CursorMode::Embedded).set_persist_mode(PersistMode::DoNot)), Duration::from_secs(90))
                    .await?.map_err(|_| ShareFailure::PermissionDenied)?.response().map_err(|_| ShareFailure::Cancelled)?;
                let sources = deadline(portal.start(&session, None, Default::default()), Duration::from_secs(90)).await?
                    .map_err(|_| ShareFailure::PermissionDenied)?.response().map_err(|_| ShareFailure::Cancelled)?;
                if sources.streams().len() != 1 { return Err(ShareFailure::Unavailable); }
                let source = &sources.streams()[0];
                // Reject oversize source buffers before opening PipeWire. The
                // native pool is not made bounded by promising to downscale later.
                let (width, height) = source.size().ok_or(ShareFailure::Unavailable)?;
                if width <= 0 || height <= 0 || width > 1920 || height > 1080 || width % 2 != 0 || height % 2 != 0 {
                    return Err(ShareFailure::Unavailable);
                }
                let fd = deadline(portal.open_pipe_wire_remote(&session, Default::default()), Duration::from_secs(3)).await?
                    .map_err(|_| ShareFailure::Unavailable)?;
                if !active() { return Err(ShareFailure::Cancelled); }
                handle.screen_state(generation, SharingState::Starting);
                let mut pipeline = Pipeline::new(&format!("pipewiresrc fd={} path={} do-timestamp=true always-copy=true min-buffers=3 max-buffers=4 ! video/x-raw,width={width},height={height} ! videoconvert ! video/x-raw,format=RGBA ! appsink name=output sync=false async=false max-buffers=2 drop=true enable-last-sample=false", fd.as_raw_fd(), source.pipe_wire_node_id()))?;
                let started = std::time::Instant::now();
                let mut seen = false;
                while active() {
                    seen |= pipeline.frame(handle, generation)?;
                    if !seen && started.elapsed() > Duration::from_secs(10) { return Err(ShareFailure::SourceClosed); }
                    async_io::Timer::after(Duration::from_millis(10)).await;
                }
                // pipeline is disposed before fd and before Session.Close.
                Ok(())
            }).await
        }).await;
        let _ = deadline(session.close(), Duration::from_secs(2)).await;
        result
        }.await;
        let _ = deadline(connection.close(), Duration::from_secs(2)).await;
        result
    })
}
#[cfg(target_os = "linux")]
async fn deadline<T>(
    work: impl std::future::Future<Output = T>,
    duration: Duration,
) -> Result<T, ShareFailure> {
    futures_lite::future::or(
        async {
            async_io::Timer::after(duration).await;
            Err(ShareFailure::TimedOut)
        },
        async { Ok(work.await) },
    )
    .await
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn capture(_: &CaptureHandle, _: u64, _: &impl Fn() -> bool) -> Result<(), ShareFailure> {
    Err(ShareFailure::Unavailable)
}

#[cfg(target_os = "linux")]
struct Pipeline {
    pipeline: gstreamer::Pipeline,
    sink: gstreamer_app::AppSink,
    last_frame: Option<std::time::Instant>,
}
#[cfg(target_os = "linux")]
impl Pipeline {
    fn new(description: &str) -> Result<Self, ShareFailure> {
        use gstreamer::prelude::*;
        gstreamer::init().map_err(|_| ShareFailure::Unavailable)?;
        let pipeline = gstreamer::parse::launch(description)
            .map_err(|_| ShareFailure::Unavailable)?
            .downcast::<gstreamer::Pipeline>()
            .map_err(|_| ShareFailure::Unavailable)?;
        let sink = pipeline
            .by_name("output")
            .and_then(|e| e.downcast::<gstreamer_app::AppSink>().ok())
            .ok_or(ShareFailure::Unavailable)?;
        let result = Self {
            pipeline,
            sink,
            last_frame: None,
        };
        result
            .pipeline
            .set_state(gstreamer::State::Playing)
            .map_err(|_| ShareFailure::Unavailable)?;
        Ok(result)
    }
    fn frame(&mut self, handle: &CaptureHandle, generation: u64) -> Result<bool, ShareFailure> {
        use gstreamer_video::prelude::*;
        if self.pipeline.bus().is_some_and(|bus| {
            bus.pop_filtered(&[gstreamer::MessageType::Error, gstreamer::MessageType::Eos])
                .is_some()
        }) {
            return Err(ShareFailure::SourceClosed);
        }
        let Some(sample) = self.sink.try_pull_sample(gstreamer::ClockTime::ZERO) else {
            return Ok(false);
        };
        let now = std::time::Instant::now();
        if self
            .last_frame
            .is_some_and(|last| now.duration_since(last) < Duration::from_millis(100))
        {
            return Ok(false);
        }
        self.last_frame = Some(now);
        let info =
            gstreamer_video::VideoInfo::from_caps(sample.caps().ok_or(ShareFailure::Unavailable)?)
                .map_err(|_| ShareFailure::Unavailable)?;
        let (width, height) = (info.width(), info.height());
        if width == 0
            || height == 0
            || width > 1920
            || height > 1080
            || info.format() != gstreamer_video::VideoFormat::Rgba
        {
            return Err(ShareFailure::Unavailable);
        }
        let buffer = sample.buffer().ok_or(ShareFailure::Unavailable)?;
        let map = gstreamer_video::VideoFrameRef::from_buffer_ref_readable(buffer, &info)
            .map_err(|_| ShareFailure::Unavailable)?;
        let stride =
            usize::try_from(map.plane_stride()[0]).map_err(|_| ShareFailure::Unavailable)?;
        let data = map.plane_data(0).map_err(|_| ShareFailure::Unavailable)?;
        let row = width as usize * 4;
        if stride < row {
            return Err(ShareFailure::Unavailable);
        }
        let mut rgba = Vec::with_capacity(row * height as usize);
        for y in 0..height as usize {
            let start = y.checked_mul(stride).ok_or(ShareFailure::Unavailable)?;
            rgba.extend_from_slice(
                data.get(start..start.checked_add(row).ok_or(ShareFailure::Unavailable)?)
                    .ok_or(ShareFailure::Unavailable)?,
            );
        }
        handle.screen_rgba(generation, width, height, &rgba);
        Ok(true)
    }
}
#[cfg(target_os = "linux")]
impl Drop for Pipeline {
    fn drop(&mut self) {
        use gstreamer::prelude::*;
        let _ = self.pipeline.set_state(gstreamer::State::Null);
    }
}

#[cfg(target_os = "macos")]
fn capture(
    handle: &CaptureHandle,
    generation: u64,
    active: &impl Fn() -> bool,
) -> Result<(), ShareFailure> {
    use std::{
        io::Read,
        process::{Command, Stdio},
        sync::mpsc,
        time::Instant,
    };
    let executable = std::env::current_exe().map_err(|_| ShareFailure::Unavailable)?;
    let directory = executable
        .parent()
        .filter(|p| {
            p.file_name().is_some_and(|n| n == "MacOS")
                && p.parent()
                    .and_then(|p| p.file_name())
                    .is_some_and(|n| n == "Contents")
        })
        .ok_or(ShareFailure::Unavailable)?;
    let mut child = Command::new(directory.join("talk-native-helper"))
        .arg("screen-capture")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| ShareFailure::Unavailable)?;
    let Some(mut output) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(ShareFailure::Unavailable);
    };
    let (send, receive) = mpsc::sync_channel(1);
    let reader = std::thread::Builder::new()
        .name("screen-frames".into())
        .spawn(move || {
            loop {
                let mut header = [0; 8];
                if output.read_exact(&mut header).is_err() {
                    break;
                }
                let width = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
                let height = u32::from_be_bytes([header[4], header[5], header[6], header[7]]);
                if width == 0
                    || height == 0
                    || width > 1920
                    || height > 1080
                    || !width.is_multiple_of(2)
                    || !height.is_multiple_of(2)
                {
                    break;
                }
                let mut rgba = vec![0; (width * height * 4) as usize];
                if output.read_exact(&mut rgba).is_err() {
                    break;
                }
                for pixel in rgba.as_chunks_mut::<4>().0 {
                    pixel.swap(0, 2);
                }
                match send.try_send((width, height, rgba)) {
                    Ok(()) | Err(mpsc::TrySendError::Full(_)) => {}
                    Err(mpsc::TrySendError::Disconnected(_)) => break,
                }
            }
        });
    let reader = match reader {
        Ok(reader) => reader,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ShareFailure::Unavailable);
        }
    };
    let mut last = Instant::now();
    let mut seen = false;
    let result = loop {
        if !active() {
            break Ok(());
        }
        match receive.recv_timeout(Duration::from_millis(20)) {
            Ok((width, height, rgba)) => {
                seen = true;
                last = Instant::now();
                handle.screen_rgba(generation, width, height, &rgba);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break Err(ShareFailure::SourceClosed),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if !seen && last.elapsed() > Duration::from_secs(95) {
            break Err(ShareFailure::TimedOut);
        }
    };
    child.stdin.take();
    let _ = child.kill();
    let _ = child.wait();
    let _ = reader.join();
    result
}
