//! Device-rate adaptation stays in GStreamer. CPAL callbacks never wait for a
//! streaming thread; late blocks are bounded and discarded under overload.
use super::super::DeviceError;
use gstreamer::{self as gst, prelude::*};
use gstreamer_app::{AppSink, AppSrc};

pub(super) struct Converter {
    pipeline: gst::Pipeline,
    source: AppSrc,
    sink: AppSink,
    input_rate: u32,
    output_rate: u32,
    frames: u64,
    failed: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl Converter {
    pub(super) fn new(input_rate: u32, output_rate: u32) -> Result<Self, DeviceError> {
        if ![input_rate, output_rate]
            .iter()
            .all(|r| (8_000..=192_000).contains(r))
        {
            return Err(DeviceError::Unavailable);
        }
        gst::init().map_err(|_| DeviceError::Unavailable)?;
        let pipeline = gst::parse::launch(&format!(
            "appsrc name=input is-live=true format=time block=false caps=audio/x-raw,format=S16LE,layout=interleaved,channels=1,rate={input_rate} ! audioresample ! audio/x-raw,rate={output_rate} ! audiobuffersplit output-buffer-duration=1/100 strict-buffer-size=true ! appsink name=output sync=false async=false max-buffers=12 drop=true enable-last-sample=false"
        )).map_err(|_| DeviceError::Unavailable)?.downcast::<gst::Pipeline>()
            .map_err(|_| DeviceError::Unavailable)?;
        let source = pipeline
            .by_name("input")
            .and_then(|e| e.downcast::<AppSrc>().ok())
            .ok_or(DeviceError::Unavailable)?;
        let sink = pipeline
            .by_name("output")
            .and_then(|e| e.downcast::<AppSink>().ok())
            .ok_or(DeviceError::Unavailable)?;
        source.set_max_buffers(12);
        source.set_max_bytes(u64::from(input_rate) * 2 * 120 / 1000);
        source.set_max_time(gst::ClockTime::from_mseconds(120));
        source.set_leaky_type(gstreamer_app::AppLeakyType::Downstream);
        let failed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let errors = failed.clone();
        pipeline
            .bus()
            .ok_or(DeviceError::Unavailable)?
            .set_sync_handler(move |_, message| {
                if matches!(message.view(), gst::MessageView::Error(_)) {
                    errors.store(true, std::sync::atomic::Ordering::Release);
                }
                gst::BusSyncReply::Drop
            });
        let result = Self {
            pipeline,
            source,
            sink,
            input_rate,
            output_rate,
            frames: 0,
            failed,
        };
        result
            .pipeline
            .set_state(gst::State::Playing)
            .map_err(|_| DeviceError::CaptureFailed)?;
        Ok(result)
    }
    pub(super) fn push(&mut self, pcm: &[i16]) -> Result<(), DeviceError> {
        if pcm.len() > 19_200 {
            return Err(DeviceError::CaptureFailed);
        }
        let time = |frames: u64| {
            u64::try_from(u128::from(frames) * 1_000_000_000 / u128::from(self.input_rate))
                .ok()
                .filter(|ns| *ns < u64::MAX)
                .map(gst::ClockTime::from_nseconds)
                .ok_or(DeviceError::CaptureFailed)
        };
        let mut buffer = gst::Buffer::from_mut_slice(
            pcm.iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<_>>(),
        );
        let pts = time(self.frames)?;
        self.frames = self
            .frames
            .checked_add(pcm.len() as u64)
            .ok_or(DeviceError::CaptureFailed)?;
        let end = time(self.frames)?;
        let writable = buffer.get_mut().ok_or(DeviceError::CaptureFailed)?;
        writable.set_pts(pts);
        writable.set_duration(end - pts);
        self.source
            .push_buffer(buffer)
            .map_err(|_| DeviceError::CaptureFailed)?;
        Ok(())
    }
    pub(super) fn pull(&self) -> Result<Option<Vec<i16>>, DeviceError> {
        if self.failed.load(std::sync::atomic::Ordering::Acquire) {
            return Err(DeviceError::CaptureFailed);
        }
        let Some(sample) = self.sink.try_pull_sample(gst::ClockTime::ZERO) else {
            return Ok(None);
        };
        let mapped = sample
            .buffer()
            .ok_or(DeviceError::CaptureFailed)?
            .map_readable()
            .map_err(|_| DeviceError::CaptureFailed)?;
        if mapped.len() % 2 != 0 || mapped.len() > (self.output_rate as usize / 100 + 1) * 2 {
            return Err(DeviceError::CaptureFailed);
        }
        Ok(Some(
            mapped
                .as_slice()
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| i16::from_le_bytes(*b))
                .collect(),
        ))
    }
}
impl Drop for Converter {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn bluetooth_and_desktop_rates_convert_in_both_directions_without_drift() {
        for native in [8_000, 16_000, 24_000, 32_000, 44_100, 48_000, 96_000] {
            for (input, output) in [(native, 48_000), (48_000, native)] {
                let mut converter = Converter::new(input, output).unwrap();
                let mut decoded = Vec::new();
                for packet in 0..30 {
                    let pcm: Vec<_> = (packet * input / 100..(packet + 1) * input / 100)
                        .map(|n| {
                            ((f64::from(n) * 440.0 * std::f64::consts::TAU / f64::from(input))
                                .sin()
                                * 8_000.) as i16
                        })
                        .collect();
                    converter.push(&pcm).unwrap();
                    std::thread::sleep(Duration::from_millis(10));
                    while let Some(block) = converter.pull().unwrap() {
                        if output == 48_000 {
                            assert_eq!(block.len(), 480);
                        }
                        decoded.extend(block);
                    }
                }
                converter.source.end_of_stream().unwrap();
                let until = Instant::now() + Duration::from_secs(2);
                while Instant::now() < until && !converter.sink.is_eos() {
                    while let Some(block) = converter.pull().unwrap() {
                        decoded.extend(block);
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                assert!(converter.sink.is_eos());
                let expected = output as usize * 30 / 100;
                assert!(
                    decoded.len().abs_diff(expected) <= output as usize / 100 + 1,
                    "{input}->{output}: got {}, expected {expected}",
                    decoded.len()
                );
                let crossings = decoded
                    .windows(2)
                    .filter(|pair| pair[0] < 0 && pair[1] >= 0)
                    .count();
                assert!(
                    (126..=134).contains(&crossings),
                    "{input}->{output}: {crossings} cycles"
                );
            }
        }
    }

    #[test]
    fn dropping_the_converter_stops_its_pipeline_and_flushes_buffered_pcm() {
        let mut converter = Converter::new(16_000, 48_000).unwrap();
        converter.push(&[1000; 160]).unwrap();
        let pipeline = converter.pipeline.clone();
        let source = converter.source.clone();
        drop(converter);
        assert_eq!(pipeline.current_state(), gst::State::Null);
        assert_eq!(source.current_level_buffers(), 0);
        assert!(
            source
                .push_buffer(gst::Buffer::from_mut_slice(vec![0; 320]))
                .is_err()
        );
    }

    #[test]
    fn native_pcm_queues_remain_bounded_when_a_callback_stops_draining() {
        let mut converter = Converter::new(16_000, 48_000).unwrap();
        for _ in 0..200 {
            converter.push(&[1000; 160]).unwrap();
        }
        std::thread::sleep(Duration::from_millis(100));
        assert!(converter.source.current_level_buffers() <= 12);
        let mut blocks = 0;
        while converter.pull().unwrap().is_some() {
            blocks += 1;
        }
        assert!(blocks <= 12);
        assert!(converter.push(&vec![0; 19_201]).is_err());
    }
}
