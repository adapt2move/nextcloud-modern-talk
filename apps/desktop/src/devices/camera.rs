//! GStreamer's established native camera plugins, through safe Rust APIs only.
//! Exact raw capture caps bound our negotiated frames; appsink and our pixel pool
//! are bounded separately. Native plugin/driver allocations remain their boundary.
use super::frames::{FRAME_BYTES, FramePool, HEIGHT, VideoFrame, WIDTH};
use super::{
    DeviceError, DeviceId, DeviceInfo, DeviceKind, Signal, VideoSurfaceId, decode_error, safe_label,
};
use gstreamer as gst;
use gstreamer_video::prelude::*;
use std::sync::{Arc, atomic::Ordering};
use std::time::{Duration, Instant};

pub(super) struct Device {
    pub info: DeviceInfo,
    native: Option<gst::Device>,
    path: Option<String>,
}

pub(super) fn enumerate() -> Result<Vec<Device>, DeviceError> {
    gst::init().map_err(|_| DeviceError::Unavailable)?;
    let monitor = gst::DeviceMonitor::new();
    monitor
        .add_filter(Some("Video/Source"), None)
        .ok_or(DeviceError::Unavailable)?;
    // DeviceMonitor's bus is otherwise unbounded. This is a short probe; periodic
    // fresh probes also cover AVFoundation versions without hotplug callbacks.
    monitor.bus().set_flushing(true);
    if monitor.start().is_err() {
        #[cfg(target_os = "linux")]
        {
            let mut result = Vec::new();
            append_linux_metadata(&mut result);
            return Ok(result);
        }
        #[cfg(not(target_os = "linux"))]
        return Err(DeviceError::Unavailable);
    }
    let devices = monitor.devices();
    monitor.stop();
    let mut result = Vec::with_capacity(24);
    for native in devices.into_iter().take(128) {
        if result.len() == 24 {
            break;
        }
        let Some(properties) = native.properties() else {
            continue;
        };
        let identity = if let Ok(id) = properties.get::<String>("avf.unique_id") {
            Some(("avf", id))
        } else if let Ok(path) = properties.get::<String>("device.path") {
            if properties.get::<String>("device.api").ok().as_deref() == Some("mediafoundation") {
                Some(("mf", path))
            } else {
                linux_identity(&path).map(|id| ("v4l2", id))
            }
        } else if let Ok(path) = properties.get::<String>("device-path") {
            Some(("mf", path))
        } else {
            None
        };
        let Some((namespace, identity)) = identity else {
            continue;
        };
        let Ok(id) = opaque_id(namespace, &identity) else {
            continue;
        };
        if result.iter().any(|d: &Device| d.info.id == id) {
            continue;
        }
        let label = safe_label(native.display_name().as_str());
        // There is no cross-platform camera-default API. GStreamer's first native
        // camera is the System Default policy, resolved and shown explicitly. A
        // saved Specific choice always resolves its native ID instead of an index.
        let info = DeviceInfo {
            id,
            label,
            kind: DeviceKind::Camera,
            is_default: result.is_empty(),
        };
        let path = properties
            .get::<String>("device.path")
            .ok()
            .filter(|_| namespace == "v4l2");
        result.push(Device {
            info,
            native: Some(native),
            path,
        });
    }
    #[cfg(target_os = "linux")]
    append_linux_metadata(&mut result);
    Ok(result)
}

fn opaque_id(namespace: &str, value: &str) -> Result<DeviceId, DeviceError> {
    if value.is_empty() || value.len() > 4096 {
        return Err(DeviceError::InvalidSelection);
    }
    let mut hash = gst::glib::Checksum::new(gst::glib::ChecksumType::Sha256)
        .ok_or(DeviceError::Unavailable)?;
    hash.update(value.as_bytes());
    let digest = hash.string().ok_or(DeviceError::Unavailable)?;
    DeviceId::new(&format!("camera:{namespace}:{digest}"))
}
#[cfg(target_os = "linux")]
fn linux_identity(path: &str) -> Option<String> {
    use std::path::Path;
    let target = std::fs::canonicalize(path).ok()?;
    // Prefer udev's serial-based stable identity; never persist /dev/videoN.
    // The by-path fallback is the OS's stable connector identity for devices
    // without serial numbers. It cannot distinguish identical replacements.
    for directory in ["/dev/v4l/by-id", "/dev/v4l/by-path"] {
        if let Ok(entries) = std::fs::read_dir(directory) {
            for entry in entries.take(128).flatten() {
                if std::fs::canonicalize(entry.path()).ok().as_ref() == Some(&target) {
                    return entry.path().to_str().map(str::to_owned);
                }
            }
        }
    }
    // Built-in/virtual devices may have no udev link. The canonical sysfs device
    // identifies the bus/driver, with video index removed (which is enumeration
    // order, not identity). Include its per-device stream index for multi-node HW.
    let node = Path::new(path).file_name()?.to_str()?;
    let class = Path::new("/sys/class/video4linux").join(node);
    let stream = std::fs::read_to_string(class.join("index")).ok()?;
    if let Ok(device) = std::fs::canonicalize(class.join("device")) {
        return Some(format!("{}:{}", device.to_str()?, stream.trim()));
    }
    // OS virtual cameras have no physical parent. Their configured OS node and
    // name are their identity, not a claim about a physical serial number.
    let virtual_device = std::fs::canonicalize(&class).ok()?;
    if !virtual_device.starts_with("/sys/devices/virtual/video4linux") {
        return None;
    }
    let name = std::fs::read_to_string(class.join("name")).ok()?;
    Some(format!(
        "virtual:{}:{}:{}",
        virtual_device.to_str()?,
        name.trim(),
        stream.trim()
    ))
}
#[cfg(not(target_os = "linux"))]
fn linux_identity(_path: &str) -> Option<String> {
    None
}

#[cfg(target_os = "linux")]
fn append_linux_metadata(result: &mut Vec<Device>) {
    // V4L2's native probe omits nodes it cannot open. Keep sysfs-visible devices
    // in inventory so an explicit start can report the real EACCES, rather than
    // misrepresenting permission refusal as a physically removed camera.
    let Ok(entries) = std::fs::read_dir("/sys/class/video4linux") else {
        return;
    };
    for entry in entries.take(128).flatten() {
        if result.len() >= 24 {
            break;
        }
        let Some(node) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !node.starts_with("video") {
            continue;
        }
        let path = format!("/dev/{node}");
        let Some(identity) = linux_identity(&path) else {
            continue;
        };
        let Ok(id) = opaque_id("v4l2", &identity) else {
            continue;
        };
        if result.iter().any(|d| d.info.id == id) {
            continue;
        }
        let Ok(name) = std::fs::read_to_string(entry.path().join("name")) else {
            continue;
        };
        result.push(Device {
            info: DeviceInfo {
                id,
                label: safe_label(&name),
                kind: DeviceKind::Camera,
                is_default: result.is_empty(),
            },
            native: None,
            path: Some(path),
        });
    }
}

pub(super) struct Capture {
    id: DeviceId,
    pipeline: gst::Pipeline,
    sink: gstreamer_app::AppSink,
    signal: Arc<Signal>,
    last_frame: Instant,
    stopped: bool,
}
impl Capture {
    pub(super) fn id(&self) -> &DeviceId {
        &self.id
    }
    pub(super) fn start(device: &Device, signal: Arc<Signal>) -> Result<Self, DeviceError> {
        #[cfg(target_os = "linux")]
        if let Some(path) = &device.path {
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .map_err(|error| match error.kind() {
                    std::io::ErrorKind::PermissionDenied => DeviceError::PermissionDenied,
                    std::io::ErrorKind::NotFound => DeviceError::Removed,
                    std::io::ErrorKind::ResourceBusy => DeviceError::Busy,
                    _ => DeviceError::CaptureFailed,
                })?;
        }
        let source = match &device.native {
            Some(native) => native
                .create_element(None)
                .map_err(|_| DeviceError::Removed)?,
            None => gst::ElementFactory::make("v4l2src")
                .property(
                    "device",
                    device.path.as_deref().ok_or(DeviceError::Removed)?,
                )
                .build()
                .map_err(|_| DeviceError::Unavailable)?,
        };
        let pipeline = gst::Pipeline::new();
        let input_caps = gst::Caps::builder("video/x-raw")
            .field("width", WIDTH as i32)
            .field("height", HEIGHT as i32)
            .field(
                "framerate",
                gst::FractionRange::new(gst::Fraction::new(1, 1), gst::Fraction::new(30, 1)),
            )
            .build();
        let output_caps = gst::Caps::builder("video/x-raw")
            .field("format", "RGBA")
            .field("width", WIDTH as i32)
            .field("height", HEIGHT as i32)
            .field("framerate", gst::Fraction::new(15, 1))
            .build();
        let filter = gst::ElementFactory::make("capsfilter")
            .property("caps", &input_caps)
            .build()
            .map_err(|_| DeviceError::Unavailable)?;
        let convert = gst::ElementFactory::make("videoconvert")
            .build()
            .map_err(|_| DeviceError::Unavailable)?;
        let rate = gst::ElementFactory::make("videorate")
            .property("drop-only", true)
            .build()
            .map_err(|_| DeviceError::Unavailable)?;
        let sink = gstreamer_app::AppSink::builder()
            .caps(&output_caps)
            .max_buffers(1)
            .max_bytes(FRAME_BYTES as u64)
            .drop(true)
            .sync(false)
            .enable_last_sample(false)
            .wait_on_eos(false)
            .build();
        let elements = [&source, &filter, &convert, &rate, sink.upcast_ref()];
        pipeline
            .add_many(elements)
            .map_err(|_| DeviceError::CaptureFailed)?;
        gst::Element::link_many(elements).map_err(|_| DeviceError::CaptureFailed)?;
        let bus = pipeline.bus().ok_or(DeviceError::Unavailable)?;
        let errors = signal.clone();
        bus.set_sync_handler(move |_, message| {
            match message.view() {
                gst::MessageView::Error(error) => errors.fail(map_error(error.error())),
                gst::MessageView::Eos(_) => errors.fail(DeviceError::Removed),
                _ => {}
            }
            // Never accumulate a media event queue. Retain only a typed error.
            gst::BusSyncReply::Drop
        });
        let capture = Self {
            id: device.info.id.clone(),
            pipeline,
            sink,
            signal,
            last_frame: Instant::now() - Duration::from_secs(1),
            stopped: false,
        };
        if !capture.signal.valid() {
            return Err(DeviceError::Stale);
        }
        capture
            .pipeline
            .set_state(gst::State::Playing)
            .map_err(|_| {
                decode_error(capture.signal.error.load(Ordering::Acquire))
                    .unwrap_or(DeviceError::CaptureFailed)
            })?;
        Ok(capture)
    }
    pub(super) fn poll(
        &mut self,
        frames: &mut FramePool,
        surface: VideoSurfaceId,
    ) -> Result<Option<Arc<VideoFrame>>, DeviceError> {
        if !self.signal.valid() || self.last_frame.elapsed() < Duration::from_millis(66) {
            return Ok(None);
        }
        let Some(sample) = self.sink.try_pull_sample(gst::ClockTime::ZERO) else {
            return Ok(None);
        };
        let caps = sample.caps().ok_or(DeviceError::CaptureFailed)?;
        let info =
            gstreamer_video::VideoInfo::from_caps(caps).map_err(|_| DeviceError::CaptureFailed)?;
        if info.width() != WIDTH
            || info.height() != HEIGHT
            || info.format() != gstreamer_video::VideoFormat::Rgba
        {
            return Err(DeviceError::CaptureFailed);
        }
        let buffer = sample.buffer().ok_or(DeviceError::CaptureFailed)?;
        if buffer.size() > FRAME_BYTES + 4096 * HEIGHT as usize {
            return Err(DeviceError::CaptureFailed);
        }
        let mapped = gstreamer_video::VideoFrameRef::from_buffer_ref_readable(buffer, &info)
            .map_err(|_| DeviceError::CaptureFailed)?;
        let stride = usize::try_from(
            *mapped
                .plane_stride()
                .first()
                .ok_or(DeviceError::CaptureFailed)?,
        )
        .map_err(|_| DeviceError::CaptureFailed)?;
        let pixels = mapped
            .plane_data(0)
            .map_err(|_| DeviceError::CaptureFailed)?;
        self.last_frame = Instant::now();
        self.signal.touch();
        self.signal.observed.store(true, Ordering::Release);
        frames.copy(pixels, stride, surface, self.signal.clone())
    }
    pub(super) fn stop(&mut self) -> Result<(), DeviceError> {
        if self.stopped {
            return Ok(());
        }
        self.pipeline
            .set_state(gst::State::Null)
            .map_err(|_| DeviceError::CaptureFailed)?;
        let (result, current, _) = self.pipeline.state(gst::ClockTime::SECOND);
        result.map_err(|_| DeviceError::CaptureFailed)?;
        if current != gst::State::Null {
            return Err(DeviceError::CaptureFailed);
        }
        self.stopped = true;
        Ok(())
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
fn map_error(error: gst::glib::Error) -> DeviceError {
    if error.matches(gst::ResourceError::NotAuthorized) {
        DeviceError::PermissionDenied
    } else if error.matches(gst::ResourceError::Busy) {
        DeviceError::Busy
    } else if error.matches(gst::ResourceError::NotFound) {
        DeviceError::Removed
    } else {
        DeviceError::CaptureFailed
    }
}
