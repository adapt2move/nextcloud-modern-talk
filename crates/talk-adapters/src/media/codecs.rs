//! Safe GStreamer codecs. No device elements belong in these pipelines.
//! Appsrc/appsink queues are finite, nonblocking and discard oldest media under
//! overload. Dropping a pipeline flushes queued media before a publication grant
//! can be reused; dependency-private codec allocations remain unqualified.
use super::{Error, VideoFrame};
use gstreamer::{self as gst, prelude::*};
use gstreamer_app::{AppSink, AppSrc};
use std::time::Duration;

pub(super) struct Codec {
    pipeline: gst::Pipeline,
    source: AppSrc,
    sink: AppSink,
    pub(super) dimensions: Option<(u32, u32)>,
}
impl Codec {
    fn new(description: &str, buffers: u64, bytes: u64) -> Result<Self, Error> {
        gst::init().map_err(|_| Error::Engine)?;
        let pipeline = gst::parse::launch(description)
            .map_err(|_| Error::Engine)?
            .downcast::<gst::Pipeline>()
            .map_err(|_| Error::Engine)?;
        let source = pipeline
            .by_name("input")
            .and_then(|e| e.downcast::<AppSrc>().ok())
            .ok_or(Error::Engine)?;
        let sink = pipeline
            .by_name("output")
            .and_then(|e| e.downcast::<AppSink>().ok())
            .ok_or(Error::Engine)?;
        source.set_max_buffers(buffers);
        source.set_max_bytes(bytes);
        source.set_max_time(gst::ClockTime::from_mseconds(40));
        source.set_leaky_type(gstreamer_app::AppLeakyType::Downstream);
        let result = Self {
            pipeline,
            source,
            sink,
            dimensions: None,
        };
        result
            .pipeline
            .set_state(gst::State::Playing)
            .map_err(|_| Error::Engine)?;
        Ok(result)
    }
    pub(super) fn audio_encoder() -> Result<Self, Error> {
        Self::new(
            concat!(
                "appsrc name=input is-live=true format=time block=false ",
                "caps=audio/x-raw,format=S16LE,layout=interleaved,rate=48000,channels=1 ! ",
                "opusenc frame-size=10 bitrate=32000 inband-fec=true ! ",
                "appsink name=output sync=false max-buffers=3 drop=true enable-last-sample=false"
            ),
            3,
            2880,
        )
    }
    pub(super) fn video_encoder(width: u32, height: u32) -> Result<Self, Error> {
        let mut result = Self::new(
            &format!(
                "appsrc name=input is-live=true format=time block=false caps=video/x-raw,format=RGBA,width={width},height={height},framerate=30/1 ! videoconvert ! video/x-raw,format=I420 ! vp8enc deadline=1 cpu-used=8 threads=2 target-bitrate=800000 lag-in-frames=0 keyframe-max-dist=30 ! appsink name=output sync=false max-buffers=2 drop=true enable-last-sample=false"
            ),
            2,
            u64::from(width) * u64::from(height) * 8,
        )?;
        result.dimensions = Some((width, height));
        Ok(result)
    }
    pub(super) fn decoder(video: bool, payload: u8) -> Result<Self, Error> {
        let (media, name, clock, tail) = if video {
            (
                "video",
                "VP8",
                90_000,
                "rtpvp8depay ! vp8dec threads=2 ! videoconvert ! videoscale ! video/x-raw,format=RGBA,width=[2,640],height=[2,360],pixel-aspect-ratio=1/1",
            )
        } else {
            (
                "audio",
                "OPUS",
                48_000,
                "rtpopusdepay ! opusdec plc=true ! audioconvert ! audioresample ! audio/x-raw,format=S16LE,layout=interleaved,rate=48000,channels=1",
            )
        };
        Self::new(
            &format!(
                "appsrc name=input is-live=true format=time do-timestamp=true block=false caps=application/x-rtp,media={media},encoding-name={name},clock-rate={clock},payload={payload} ! rtpjitterbuffer latency=30 drop-on-latency=true max-dropout-time=100 max-misorder-time=100 ! {tail} ! appsink name=output sync=false max-buffers={} drop=true enable-last-sample=false",
                if video { 2 } else { 3 }
            ),
            if video { 64 } else { 5 },
            if video { 131_072 } else { 20_480 },
        )
    }
    pub(super) fn push(
        &self,
        data: Vec<u8>,
        pts: Option<Duration>,
        duration: Option<Duration>,
    ) -> Result<(), Error> {
        self.check()?;
        let mut buffer = gst::Buffer::from_mut_slice(data);
        let b = buffer.get_mut().ok_or(Error::Engine)?;
        b.set_pts(pts.map(|v| {
            gst::ClockTime::from_nseconds(v.as_nanos().min(u128::from(u64::MAX - 1)) as u64)
        }));
        b.set_duration(duration.map(|v| gst::ClockTime::from_nseconds(v.as_nanos() as u64)));
        self.source.push_buffer(buffer).map_err(|_| Error::Engine)?;
        Ok(())
    }
    fn check(&self) -> Result<(), Error> {
        if let Some(bus) = self.pipeline.bus()
            && bus.pop_filtered(&[gst::MessageType::Error]).is_some()
        {
            return Err(Error::Engine);
        }
        Ok(())
    }
    pub(super) fn sample(&self) -> Result<Option<gst::Sample>, Error> {
        self.check()?;
        Ok(self.sink.try_pull_sample(gst::ClockTime::ZERO))
    }
    pub(super) fn encoded(&self, max: usize) -> Result<Option<(Vec<u8>, Duration)>, Error> {
        let Some(sample) = self.sample()? else {
            return Ok(None);
        };
        let buffer = sample.buffer().ok_or(Error::Engine)?;
        if buffer.size() > max {
            return Err(Error::Overloaded);
        }
        let data = buffer
            .map_readable()
            .map_err(|_| Error::Engine)?
            .as_slice()
            .to_vec();
        let duration = buffer
            .duration()
            .map(|d| Duration::from_nanos(d.nseconds()))
            .unwrap_or(if self.dimensions.is_some() {
                Duration::from_millis(33)
            } else {
                Duration::from_millis(10)
            });
        Ok(Some((data, duration)))
    }
    pub(super) fn video(&self) -> Result<Option<VideoFrame>, Error> {
        let Some(sample) = self.sample()? else {
            return Ok(None);
        };
        let info = gstreamer_video::VideoInfo::from_caps(sample.caps().ok_or(Error::Engine)?)
            .map_err(|_| Error::Malformed)?;
        let (width, height) = (info.width(), info.height());
        if width == 0
            || height == 0
            || width > 640
            || height > 360
            || info.format() != gstreamer_video::VideoFormat::Rgba
        {
            return Err(Error::Malformed);
        }
        let stride = usize::try_from(info.stride()[0]).map_err(|_| Error::Malformed)?;
        let row = width as usize * 4;
        let buffer = sample.buffer().ok_or(Error::Engine)?;
        let map = buffer.map_readable().map_err(|_| Error::Engine)?;
        if stride < row {
            return Err(Error::Malformed);
        }
        let mut rgba = Vec::with_capacity(row * height as usize);
        for y in 0..height as usize {
            let offset = info.offset()[0]
                .checked_add(y.checked_mul(stride).ok_or(Error::Malformed)?)
                .ok_or(Error::Malformed)?;
            rgba.extend_from_slice(
                map.as_slice()
                    .get(offset..offset.checked_add(row).ok_or(Error::Malformed)?)
                    .ok_or(Error::Malformed)?,
            );
        }
        Ok(Some(VideoFrame {
            width,
            height,
            rgba,
        }))
    }
}
impl Drop for Codec {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}
