//! CPAL owns CoreAudio/WASAPI/ALSA/PulseAudio callbacks and native destruction.
//! Samples are reduced in the input callback, never copied, retained or uploaded.
use super::{DeviceError, DeviceId, DeviceInfo, DeviceKind, Signal, safe_label};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};
use std::sync::{Arc, atomic::Ordering};
use std::time::Duration;

pub(super) const TONE_DURATION: Duration = Duration::from_secs(4);
const MAX_AUDIO_PER_KIND: usize = 20;

pub(super) struct Backend {
    host: cpal::Host,
}
impl Backend {
    // One host per runtime: repeated hotplug refresh must not create unbounded
    // native connections or retry CPAL's potentially stalled host initializer.
    pub(super) fn new() -> Self {
        Self {
            host: cpal::default_host(),
        }
    }
    pub(super) fn microphone(
        &self,
        id: &DeviceId,
        signal: Arc<Signal>,
    ) -> Result<Capture, DeviceError> {
        Capture::microphone(&self.host, id, signal)
    }
    pub(super) fn tone(&self, id: &DeviceId, signal: Arc<Signal>) -> Result<Capture, DeviceError> {
        Capture::tone(&self.host, id, signal)
    }
    pub(super) fn enumerate(&self, kind: DeviceKind) -> Result<Vec<DeviceInfo>, DeviceError> {
        let host = &self.host;
        let (devices, default) = match kind {
            DeviceKind::Microphone => (
                host.input_devices(),
                host.default_input_device().and_then(|d| d.id().ok()),
            ),
            DeviceKind::Speaker => (
                host.output_devices(),
                host.default_output_device().and_then(|d| d.id().ok()),
            ),
            DeviceKind::Camera => return Err(DeviceError::InvalidSelection),
        };
        let mut result = Vec::with_capacity(MAX_AUDIO_PER_KIND);
        {
            for device in devices.map_err(map_error)?.take(MAX_AUDIO_PER_KIND) {
                let Ok(native_id) = device.id() else {
                    continue;
                };
                let Ok(id) = DeviceId::new(&format!("audio:{native_id}")) else {
                    continue;
                };
                if result
                    .iter()
                    .any(|d: &DeviceInfo| d.kind == kind && d.id == id)
                {
                    continue;
                }
                let label = device
                    .description()
                    .map(|d| safe_label(d.name()))
                    .unwrap_or_else(|_| safe_label("Audio device"));
                result.push(DeviceInfo {
                    id,
                    label,
                    kind,
                    is_default: default.as_ref() == Some(&native_id),
                });
            }
        }
        Ok(result)
    }
}

pub(super) struct Capture {
    // Keep Stream alive. Drop synchronously joins/disposes the CPAL stream owner;
    // the runtime publishes its acknowledgement only after dropping this value.
    _stream: cpal::Stream,
    id: DeviceId,
}
impl Capture {
    pub(super) fn id(&self) -> &DeviceId {
        &self.id
    }
    fn microphone(
        host: &cpal::Host,
        id: &DeviceId,
        signal: Arc<Signal>,
    ) -> Result<Self, DeviceError> {
        let device = selected_device(host, id)?;
        let config = device.default_input_config().map_err(map_error)?;
        validate_config(&config)?;
        macro_rules! build {
            ($t:ty) => {
                input::<$t>(&device, config.config(), signal)
            };
        }
        let stream = match config.sample_format() {
            SampleFormat::F32 => build!(f32),
            SampleFormat::F64 => build!(f64),
            SampleFormat::I8 => build!(i8),
            SampleFormat::I16 => build!(i16),
            SampleFormat::I24 => build!(cpal::I24),
            SampleFormat::I32 => build!(i32),
            SampleFormat::I64 => build!(i64),
            SampleFormat::U8 => build!(u8),
            SampleFormat::U16 => build!(u16),
            SampleFormat::U24 => build!(cpal::U24),
            SampleFormat::U32 => build!(u32),
            SampleFormat::U64 => build!(u64),
            _ => return Err(DeviceError::CaptureFailed),
        }?;
        stream.play().map_err(map_error)?;
        Ok(Self {
            _stream: stream,
            id: id.clone(),
        })
    }
    fn tone(host: &cpal::Host, id: &DeviceId, signal: Arc<Signal>) -> Result<Self, DeviceError> {
        let device = selected_device(host, id)?;
        let config = device.default_output_config().map_err(map_error)?;
        validate_config(&config)?;
        macro_rules! build {
            ($t:ty) => {
                output::<$t>(&device, config.config(), signal)
            };
        }
        let stream = match config.sample_format() {
            SampleFormat::F32 => build!(f32),
            SampleFormat::F64 => build!(f64),
            SampleFormat::I8 => build!(i8),
            SampleFormat::I16 => build!(i16),
            SampleFormat::I24 => build!(cpal::I24),
            SampleFormat::I32 => build!(i32),
            SampleFormat::I64 => build!(i64),
            SampleFormat::U8 => build!(u8),
            SampleFormat::U16 => build!(u16),
            SampleFormat::U24 => build!(cpal::U24),
            SampleFormat::U32 => build!(u32),
            SampleFormat::U64 => build!(u64),
            _ => return Err(DeviceError::CaptureFailed),
        }?;
        stream.play().map_err(map_error)?;
        Ok(Self {
            _stream: stream,
            id: id.clone(),
        })
    }
}
fn selected_device(host: &cpal::Host, id: &DeviceId) -> Result<cpal::Device, DeviceError> {
    let native_id: cpal::DeviceId = id
        .as_str()
        .strip_prefix("audio:")
        .ok_or(DeviceError::InvalidSelection)?
        .parse()
        .map_err(|_| DeviceError::InvalidSelection)?;
    // The persisted CPAL host is part of its opaque ID. Never treat a display name
    // or ordinal as an identity, including after the Linux default host changes.
    if native_id.host() != host.id() {
        return Err(DeviceError::Removed);
    }
    host.device_by_id(&native_id).ok_or(DeviceError::Removed)
}
fn validate_config(config: &cpal::SupportedStreamConfig) -> Result<(), DeviceError> {
    if !(1..=8).contains(&config.channels()) || !(8_000..=192_000).contains(&config.sample_rate()) {
        return Err(DeviceError::CaptureFailed);
    }
    Ok(())
}
fn input<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    signal: Arc<Signal>,
) -> Result<cpal::Stream, DeviceError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    // Do not enable CPAL's PulseAudio timeout: that delegates stream creation to
    // an unjoined native-init thread. Keep creation on our one owner instead, so
    // a stalled open remains Starting/Stopping rather than falsely acknowledging
    // disposal while a late thread can still open the microphone.
    let errors = signal.clone();
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                if !signal.valid() || data.is_empty() {
                    return;
                }
                // Bound callback work as well as storage. This is an actual RMS level,
                // not a timer animation. No audio leaves this borrowed callback buffer.
                let count = data.len().min(8192);
                let energy = data.iter().take(count).fold(0.0f64, |sum, sample| {
                    let value = f32::from_sample(*sample);
                    sum + if value.is_finite() {
                        f64::from(value.clamp(-1.0, 1.0)).powi(2)
                    } else {
                        0.0
                    }
                });
                let rms = (energy / count as f64).sqrt();
                // Logarithmic -60..0 dBFS mapped to 0..1000, silence exactly zero.
                let level = if rms <= 0.001 {
                    0
                } else {
                    ((20.0 * rms.log10() + 60.0) / 60.0 * 1000.0).clamp(0.0, 1000.0) as u16
                };
                signal.level.store(level, Ordering::Relaxed);
                signal.touch();
                signal.observed.store(true, Ordering::Release);
            },
            move |error| {
                if error.kind() != cpal::ErrorKind::Xrun {
                    errors.fail(map_error(error));
                }
            },
            None,
        )
        .map_err(map_error)
}
fn output<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    signal: Arc<Signal>,
) -> Result<cpal::Stream, DeviceError>
where
    T: SizedSample + FromSample<f32>,
{
    let errors = signal.clone();
    let sample_rate = config.sample_rate;
    let rate = sample_rate as f32;
    let channels = usize::from(config.channels);
    let limit = (rate * TONE_DURATION.as_secs_f32()) as u64;
    let mut position = 0u64;
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                let valid = signal.valid();
                let mut energy = 0.0f64;
                let mut count = 0usize;
                for frame in data.chunks_mut(channels) {
                    let value = if valid && position < limit {
                        let source = (position * 16_000 / u64::from(sample_rate)) as usize;
                        let offset = 44 + source * 2;
                        let data = include_bytes!("../../assets/sounds/call-ringtone.wav");
                        data.get(offset..offset + 2).map_or(0.0, |bytes| {
                            f32::from(i16::from_le_bytes([bytes[0], bytes[1]])) / 32768.0
                        })
                    } else {
                        0.0
                    };
                    energy += f64::from(value).powi(2);
                    count += 1;
                    position = position.saturating_add(1);
                    for sample in frame {
                        *sample = T::from_sample(value);
                    }
                }
                if valid && count != 0 {
                    let rms = (energy / count as f64).sqrt();
                    let level = if rms <= 0.001 {
                        0
                    } else {
                        ((20.0 * rms.log10() + 60.0) / 60.0 * 1000.0).clamp(0.0, 1000.0) as u16
                    };
                    signal.level.store(level, Ordering::Relaxed);
                    signal.touch();
                    signal.observed.store(true, Ordering::Release);
                }
            },
            move |error| {
                if error.kind() != cpal::ErrorKind::Xrun {
                    errors.fail(map_error(error));
                }
            },
            None,
        )
        .map_err(map_error)
}
pub(super) fn map_error(error: cpal::Error) -> DeviceError {
    match error.kind() {
        cpal::ErrorKind::PermissionDenied => DeviceError::PermissionDenied,
        cpal::ErrorKind::DeviceBusy => DeviceError::Busy,
        cpal::ErrorKind::DeviceNotAvailable
        | cpal::ErrorKind::DeviceChanged
        | cpal::ErrorKind::StreamInvalidated => DeviceError::Removed,
        cpal::ErrorKind::HostUnavailable => DeviceError::Unavailable,
        _ => DeviceError::CaptureFailed,
    }
}

impl Backend {
    pub(super) fn call(
        &self,
        id: &DeviceId,
        input: bool,
        signal: Arc<Signal>,
        sink: talk_adapters::media::CaptureHandle,
    ) -> Result<Capture, DeviceError> {
        let device = selected_device(&self.host, id)?;
        let choose = |c: &cpal::SupportedStreamConfigRange| {
            c.channels() <= 2
                && c.channels() > 0
                && c.min_sample_rate() <= 48_000
                && c.max_sample_rate() >= 48_000
        };
        let config = if input {
            device
                .supported_input_configs()
                .map_err(map_error)?
                .take(64)
                .find(choose)
        } else {
            device
                .supported_output_configs()
                .map_err(map_error)?
                .take(64)
                .find(choose)
        }
        .ok_or(DeviceError::Unavailable)?
        .with_sample_rate(48_000);
        macro_rules! build {
            ($t:ty) => {
                call_stream::<$t>(&device, config.config(), input, signal, sink)
            };
        }
        let stream = match config.sample_format() {
            SampleFormat::F32 => build!(f32),
            SampleFormat::F64 => build!(f64),
            SampleFormat::I16 => build!(i16),
            SampleFormat::I32 => build!(i32),
            SampleFormat::U16 => build!(u16),
            _ => return Err(DeviceError::Unavailable),
        }?;
        stream.play().map_err(map_error)?;
        Ok(Capture {
            _stream: stream,
            id: id.clone(),
        })
    }
}
fn call_stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    input: bool,
    signal: Arc<Signal>,
    sink: talk_adapters::media::CaptureHandle,
) -> Result<cpal::Stream, DeviceError>
where
    T: SizedSample + FromSample<f32>,
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels);
    let errors = signal.clone();
    if input {
        let mut frame = [0i16; 480];
        let mut used = 0;
        device
            .build_input_stream(
                config,
                move |data: &[T], _| {
                    if !signal.valid() || !sink.requested().microphone {
                        frame.fill(0);
                        used = 0;
                        return;
                    }
                    if data.len() > 19_200 {
                        signal.fail(DeviceError::CaptureFailed);
                        sink.device_failed(true, false);
                        return;
                    }
                    for samples in data.chunks_exact(channels) {
                        let sample = samples
                            .iter()
                            .map(|s| f32::from_sample(*s))
                            .filter(|s| s.is_finite())
                            .sum::<f32>()
                            / channels as f32;
                        frame[used] = (sample.clamp(-1., 1.) * 32767.) as i16;
                        used += 1;
                        if used == 480 {
                            sink.microphone(&frame);
                            used = 0;
                        }
                    }
                    signal.touch();
                    signal.observed.store(true, Ordering::Release);
                },
                move |e| {
                    if e.kind() != cpal::ErrorKind::Xrun {
                        errors.fail(map_error(e));
                    }
                },
                None,
            )
            .map_err(map_error)
    } else {
        let mut frame = [0i16; 480];
        let mut position = 480;
        let mut ring_position = 0u64;
        device
            .build_output_stream(
                config,
                move |data: &mut [T], _| {
                    for samples in data.chunks_mut(channels) {
                        if position == 480 {
                            if !sink.output_audio(&mut frame) && sink.outgoing_wait() {
                                for sample in &mut frame {
                                    *sample = ringback_sample(ring_position);
                                    ring_position = ring_position.saturating_add(1);
                                }
                            }
                            position = 0;
                        }
                        let value = if signal.valid() && sink.receive_allowed() {
                            frame[position] as f32 / 32768.
                        } else {
                            0.
                        };
                        for sample in samples {
                            *sample = T::from_sample(value);
                        }
                        position += 1;
                    }
                    signal.touch();
                    signal.observed.store(true, Ordering::Release);
                },
                move |e| {
                    if e.kind() != cpal::ErrorKind::Xrun {
                        errors.fail(map_error(e));
                    }
                },
                None,
            )
            .map_err(map_error)
    }
}

// Outgoing local wait feedback only. Incoming notifications stay with the OS
// notification owner and its Focus/DND policy. PCM metadata is validated once.
fn ringback_sample(position: u64) -> i16 {
    const BYTES: &[u8] = include_bytes!("../../assets/sounds/call-ringback.wav");
    static DATA: std::sync::OnceLock<Option<(usize, usize)>> = std::sync::OnceLock::new();
    let Some((start, length)) = *DATA.get_or_init(|| {
        if BYTES.get(..4) != Some(b"RIFF") || BYTES.get(8..12) != Some(b"WAVE") {
            return None;
        }
        let mut offset = 12usize;
        let mut format = false;
        let mut data = None;
        for _ in 0..32 {
            let header = BYTES.get(offset..offset.checked_add(8)?)?;
            let size = u32::from_le_bytes(header[4..8].try_into().ok()?) as usize;
            let begin = offset + 8;
            let end = begin.checked_add(size)?;
            let chunk = BYTES.get(begin..end)?;
            match &header[..4] {
                b"fmt " => {
                    format = chunk.len() >= 16
                        && u16::from_le_bytes(chunk[0..2].try_into().ok()?) == 1
                        && u16::from_le_bytes(chunk[2..4].try_into().ok()?) == 2
                        && u32::from_le_bytes(chunk[4..8].try_into().ok()?) == 48000
                        && u16::from_le_bytes(chunk[12..14].try_into().ok()?) == 4
                        && u16::from_le_bytes(chunk[14..16].try_into().ok()?) == 16;
                }
                b"data" => {
                    if size > 48_000 * 4 * 8 || !size.is_multiple_of(4) {
                        return None;
                    }
                    data = Some((begin, size));
                }
                _ => {}
            }
            offset = end.checked_add(size % 2)?;
            if offset >= BYTES.len() {
                break;
            }
        }
        if format { data } else { None }
    }) else {
        return 0;
    };
    if position >= 60 * 48_000 {
        return 0;
    }
    let sample = (position % (6 * 48_000)) as usize;
    let offset = sample * 4;
    if offset + 4 > length {
        return 0;
    }
    let left = i16::from_le_bytes([BYTES[start + offset], BYTES[start + offset + 1]]);
    let right = i16::from_le_bytes([BYTES[start + offset + 2], BYTES[start + offset + 3]]);
    ((i32::from(left) + i32::from(right)) / 2) as i16
}

#[cfg(test)]
mod call_sound_tests {
    use super::ringback_sample;
    #[test]
    fn shipped_stereo_48k_ringback_has_audio_then_gap_and_finite_end() {
        assert!((0..4 * 48_000).step_by(17).any(|n| ringback_sample(n) != 0));
        assert!(
            (4 * 48_000..6 * 48_000)
                .step_by(113)
                .all(|n| ringback_sample(n) == 0)
        );
        assert_eq!(
            ringback_sample(10_000),
            ringback_sample(6 * 48_000 + 10_000)
        );
        assert!(
            (60 * 48_000..62 * 48_000)
                .step_by(113)
                .all(|n| ringback_sample(n) == 0)
        );
    }
}
