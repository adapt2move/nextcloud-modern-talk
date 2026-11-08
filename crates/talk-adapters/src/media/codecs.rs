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
    pub(super) fn video_encoder(width: u32, height: u32, screen: bool) -> Result<Self, Error> {
        let fps = if screen { 10 } else { 30 };
        let bitrate = if screen { 2_000_000 } else { 800_000 };
        let mut result = Self::new(
            &format!(
                "appsrc name=input is-live=true format=time block=false caps=video/x-raw,format=RGBA,width={width},height={height},framerate={fps}/1 ! videoconvert ! video/x-raw,format=I420 ! vp8enc deadline=1 cpu-used=8 threads=2 target-bitrate={bitrate} lag-in-frames=0 keyframe-max-dist={fps} ! appsink name=output sync=false max-buffers=2 drop=true enable-last-sample=false"
            ),
            2,
            u64::from(width) * u64::from(height) * 8,
        )?;
        result.dimensions = Some((width, height));
        Ok(result)
    }
    pub(super) fn decoder(
        video: bool,
        payload: u8,
        screen: bool,
        decrypt: Option<std::sync::Arc<std::sync::Mutex<crate::media::e2ee::frame::Decryptor>>>,
    ) -> Result<Self, Error> {
        let (media, name, clock, tail) = if video && screen {
            (
                "video",
                "VP8",
                90_000,
                "rtpvp8depay name=depay ! vp8dec threads=2 ! videoconvert ! videoscale ! video/x-raw,format=RGBA,width=[2,1920],height=[2,1080],pixel-aspect-ratio=1/1",
            )
        } else if video {
            (
                "video",
                "VP8",
                90_000,
                "rtpvp8depay name=depay ! vp8dec threads=2 ! videoconvert ! videoscale ! video/x-raw,format=RGBA,width=[2,640],height=[2,360],pixel-aspect-ratio=1/1",
            )
        } else {
            (
                "audio",
                "OPUS",
                48_000,
                "rtpopusdepay name=depay ! opusdec plc=true ! audioconvert ! audioresample ! audio/x-raw,format=S16LE,layout=interleaved,rate=48000,channels=1",
            )
        };
        let latency = if video && screen { 100 } else { 30 };
        // A static screen may supply one complete frame. Sink preroll must not
        // hold that frame waiting for further input before samples can be read.
        let result = Self::new(
            &format!(
                "appsrc name=input is-live=true format=time do-timestamp=true block=false caps=application/x-rtp,media={media},encoding-name={name},clock-rate={clock},payload={payload} ! rtpjitterbuffer latency={latency} drop-on-latency=true max-dropout-time=1000 max-misorder-time=100 ! {tail} ! appsink name=output sync=false async=false max-buffers={} drop=true enable-last-sample=false",
                if video { 2 } else { 3 }
            ),
            if video && screen {
                1024
            } else if video {
                64
            } else {
                5
            },
            if video && screen {
                1_048_576
            } else if video {
                131_072
            } else {
                20_480
            },
        )?;
        if let Some(decryptor) = decrypt {
            result.protect_incoming(video, decryptor)?;
        }
        if screen {
            // Janus can deliver an entire screen keyframe in a burst. Keep one
            // admitted compressed frame, not just the camera's small RTP batch;
            // dropping its beginning makes every subsequent delta undecodable.
            result
                .source
                .set_max_time(gst::ClockTime::from_mseconds(100));
        }
        Ok(result)
    }
    /// Open every reassembled frame between the depayloader and the decoder.
    /// A frame that cannot be opened is dropped here and never reaches the
    /// decoder: media without a key (or from a participant that does not encrypt)
    /// is not shown.
    fn protect_incoming(
        &self,
        video: bool,
        decryptor: std::sync::Arc<std::sync::Mutex<crate::media::e2ee::frame::Decryptor>>,
    ) -> Result<(), Error> {
        use crate::media::e2ee::frame::FrameKind;
        let pad = self
            .pipeline
            .by_name("depay")
            .and_then(|element| element.static_pad("src"))
            .ok_or(Error::Engine)?;
        pad.add_probe(gst::PadProbeType::BUFFER, move |_, info| {
            let Some(gst::PadProbeData::Buffer(buffer)) = &info.data else {
                return gst::PadProbeReturn::Drop;
            };
            let opened = {
                let Ok(map) = buffer.map_readable() else {
                    return gst::PadProbeReturn::Drop;
                };
                let data = map.as_slice();
                let kind = if !video {
                    FrameKind::Audio
                } else if data.first().is_some_and(|byte| byte & 1 == 0) {
                    FrameKind::VideoKey
                } else {
                    FrameKind::VideoDelta
                };
                decryptor
                    .lock()
                    .ok()
                    .and_then(|mut decryptor| decryptor.decrypt(kind, data).ok())
            };
            let Some(plain) = opened else {
                return gst::PadProbeReturn::Drop;
            };
            let mut replacement = gst::Buffer::from_mut_slice(plain);
            if let Some(target) = replacement.get_mut()
                && buffer
                    .copy_into(
                        target,
                        gst::BufferCopyFlags::FLAGS | gst::BufferCopyFlags::TIMESTAMPS,
                        ..,
                    )
                    .is_err()
            {
                return gst::PadProbeReturn::Drop;
            }
            info.data = Some(gst::PadProbeData::Buffer(replacement));
            gst::PadProbeReturn::Ok
        })
        .ok_or(Error::Engine)?;
        Ok(())
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
            || width > 1920
            || height > 1080
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

/// The path every real call uses, without a network: encode, seal, packetize, then
/// feed the RTP into the decoder pipeline whose probe opens the frames.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::e2ee::frame::{Decryptor, Encryptor, FrameKind, KEY_LEN};
    use rtc::{
        rtp::{
            codec::opus::OpusPayloader, codec::vp8::Vp8Payloader, header::Header, packet::Packet,
            packetizer::Payloader,
        },
        shared::marshal::Marshal,
    };
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    /// Wrap payload chunks into marshalled RTP packets.
    macro_rules! rtp {
        ($payloads:expr, $payload_type:expr, $sequence:expr, $timestamp:expr) => {{
            let payloads = $payloads;
            let last = payloads.len().saturating_sub(1);
            payloads
                .into_iter()
                .enumerate()
                .map(|(index, payload)| {
                    $sequence = $sequence.wrapping_add(1);
                    Packet {
                        header: Header {
                            version: 2,
                            marker: index == last,
                            payload_type: $payload_type,
                            sequence_number: $sequence,
                            timestamp: $timestamp,
                            ssrc: 0x1234,
                            ..Default::default()
                        },
                        payload,
                    }
                    .marshal()
                    .unwrap()
                    .to_vec()
                })
                .collect::<Vec<Vec<u8>>>()
        }};
    }
    fn holders(
        send: Option<[u8; KEY_LEN]>,
        receive: Option<[u8; KEY_LEN]>,
    ) -> (Arc<Mutex<Encryptor>>, Arc<Mutex<Decryptor>>) {
        let (mut encryptor, mut decryptor) = (Encryptor::default(), Decryptor::default());
        if let Some(key) = send {
            encryptor.set_key(&key, 2).unwrap();
        }
        if let Some(key) = receive {
            decryptor.set_key(&key, 2).unwrap();
        }
        (
            Arc::new(Mutex::new(encryptor)),
            Arc::new(Mutex::new(decryptor)),
        )
    }
    fn wait<T>(mut poll: impl FnMut() -> Option<T>) -> Option<T> {
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(3) {
            if let Some(value) = poll() {
                return Some(value);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        None
    }

    /// Opus frames through the sealed path; the decoded PCM must be audible.
    fn audio(send: Option<[u8; KEY_LEN]>, receive: Option<[u8; KEY_LEN]>) -> bool {
        let (encryptor, decryptor) = holders(send, receive);
        let encoder = Codec::audio_encoder().unwrap();
        let decoder = Codec::decoder(false, 111, false, Some(decryptor)).unwrap();
        let mut sequence = 0_u16;
        for frame in 0..40_u32 {
            let pcm: Vec<u8> = (0..480)
                .flat_map(|n| {
                    ((((frame * 480 + n) as f64 * 0.06).sin() * 6000.0) as i16).to_le_bytes()
                })
                .collect();
            encoder
                .push(
                    pcm,
                    Some(Duration::from_millis(10 * u64::from(frame))),
                    Some(Duration::from_millis(10)),
                )
                .unwrap();
            let Some((opus, _)) = wait(|| encoder.encoded(4096).unwrap()) else {
                continue;
            };
            let Ok(sealed) =
                encryptor
                    .lock()
                    .unwrap()
                    .encrypt(0x1234, frame * 480, FrameKind::Audio, &opus)
            else {
                continue;
            };
            let payloads = OpusPayloader.payload(1200, &sealed.into()).unwrap();
            for packet in rtp!(payloads, 111, sequence, frame * 480) {
                decoder.push(packet, None, None).unwrap();
            }
        }
        wait(|| {
            decoder.sample().unwrap().and_then(|sample| {
                let map = sample.buffer()?.map_readable().ok()?;
                map.as_slice()
                    .chunks_exact(2)
                    .any(|b| i16::from_le_bytes([b[0], b[1]]).unsigned_abs() > 500)
                    .then_some(())
            })
        })
        .is_some()
    }

    #[test]
    fn sealed_opus_is_audible_only_with_the_right_key() {
        gst::init().unwrap();
        let key = [5_u8; KEY_LEN];
        assert!(audio(Some(key), Some(key)), "the right key plays");
        assert!(
            !audio(Some(key), Some([6; KEY_LEN])),
            "a wrong key plays nothing"
        );
        assert!(!audio(Some(key), None), "no key plays nothing");
        assert!(
            !audio(None, Some(key)),
            "unsealed audio does not pass a strict receiver"
        );
    }

    fn video(send: Option<[u8; KEY_LEN]>, receive: Option<[u8; KEY_LEN]>) -> Option<(u32, u32)> {
        let (encryptor, decryptor) = holders(send, receive);
        let encoder = Codec::video_encoder(320, 240, false).unwrap();
        let decoder = Codec::decoder(true, 96, false, Some(decryptor)).unwrap();
        let mut sequence = 0_u16;
        let mut shown = None;
        for frame in 0..8_u32 {
            let mut rgba = vec![0_u8; 320 * 240 * 4];
            for pixel in rgba.as_chunks_mut::<4>().0 {
                pixel.copy_from_slice(&[(frame * 20) as u8, 90, 170, 255]);
            }
            encoder
                .push(
                    rgba,
                    Some(Duration::from_millis(33 * u64::from(frame))),
                    Some(Duration::from_millis(33)),
                )
                .unwrap();
            let Some((vp8, _)) = wait(|| encoder.encoded(1 << 20).unwrap()) else {
                continue;
            };
            let kind = if vp8[0] & 1 == 0 {
                FrameKind::VideoKey
            } else {
                FrameKind::VideoDelta
            };
            let Ok(sealed) = encryptor
                .lock()
                .unwrap()
                .encrypt(0x1234, frame * 3000, kind, &vp8)
            else {
                continue;
            };
            let payloads = Vp8Payloader::default()
                .payload(1200, &sealed.into())
                .unwrap();
            for packet in rtp!(payloads, 96, sequence, frame * 3000) {
                decoder.push(packet, None, None).unwrap();
            }
            // The jitter buffer plays out in real time.
            std::thread::sleep(Duration::from_millis(33));
            shown = shown.or(decoder
                .video()
                .unwrap()
                .map(|frame| (frame.width, frame.height)));
        }
        shown.or(wait(|| decoder.video().unwrap()).map(|frame| (frame.width, frame.height)))
    }

    #[test]
    fn sealed_vp8_decodes_only_with_the_right_key() {
        gst::init().unwrap();
        let key = [5_u8; KEY_LEN];
        assert_eq!(
            video(Some(key), Some(key)),
            Some((320, 240)),
            "the right key decodes"
        );
        assert_eq!(
            video(Some(key), Some([6; KEY_LEN])),
            None,
            "a wrong key decodes nothing"
        );
        assert_eq!(video(Some(key), None), None, "no key decodes nothing");
        assert_eq!(
            video(None, Some(key)),
            None,
            "unsealed video does not pass a strict receiver"
        );
    }
}
