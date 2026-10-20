use super::{DeviceError, Signal, VideoSurfaceId};
use std::sync::Arc;

pub(super) const WIDTH: u32 = 640;
pub(super) const HEIGHT: u32 = 480;
pub(super) const FRAME_BYTES: usize = WIDTH as usize * HEIGHT as usize * 4;

/// Owned transient RGBA8 pixels, tightly packed. This has no native capture or GPU
/// handle. Revocation makes rgba() empty, even if a consumer retains its Arc. A
/// copied GPUI image must also be retired by composition when the surface stops.
pub struct VideoFrame {
    surface: VideoSurfaceId,
    sequence: u64,
    pixels: Vec<u8>,
    signal: Arc<Signal>,
}
impl VideoFrame {
    pub fn surface(&self) -> VideoSurfaceId {
        self.surface
    }
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    pub fn width(&self) -> u32 {
        WIDTH
    }
    pub fn height(&self) -> u32 {
        HEIGHT
    }
    pub fn rgba(&self) -> &[u8] {
        if self.signal.valid() {
            &self.pixels
        } else {
            &[]
        }
    }
}

/// Allocate three frames once, then reuse only exclusively owned slots. Old UI
/// leases remain charged against this pool across stop/restart; a slow consumer
/// cannot cause an extra pool, unbounded allocations, or overwrite a live lease.
#[derive(Default)]
pub(super) struct FramePool {
    slots: Vec<Arc<VideoFrame>>,
    sequence: u64,
}
impl FramePool {
    pub(super) fn clear_released(&mut self) {
        for slot in &mut self.slots {
            if let Some(frame) = Arc::get_mut(slot) {
                frame.pixels.fill(0);
            }
        }
    }
    pub(super) fn copy(
        &mut self,
        source: &[u8],
        stride: usize,
        surface: VideoSurfaceId,
        signal: Arc<Signal>,
    ) -> Result<Option<Arc<VideoFrame>>, DeviceError> {
        let row = WIDTH as usize * 4;
        if stride < row
            || stride > row + 4096
            || source.len() < stride * HEIGHT as usize
            || source.len() > FRAME_BYTES + 4096 * HEIGHT as usize
        {
            return Err(DeviceError::CaptureFailed);
        }
        if !signal.valid() {
            return Ok(None);
        }
        if self.slots.is_empty() {
            for _ in 0..3 {
                self.slots.push(Arc::new(VideoFrame {
                    surface,
                    sequence: 0,
                    pixels: vec![0; FRAME_BYTES],
                    signal: signal.clone(),
                }));
            }
        }
        let Some(slot) = self.slots.iter_mut().find(|f| Arc::strong_count(f) == 1) else {
            return Ok(None);
        };
        let Some(frame) = Arc::get_mut(slot) else {
            return Ok(None);
        };
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or(DeviceError::CaptureFailed)?;
        frame.surface = surface;
        frame.sequence = self.sequence;
        frame.signal = signal;
        for (input, output) in source
            .chunks(stride)
            .zip(frame.pixels.chunks_mut(row))
            .take(HEIGHT as usize)
        {
            output.copy_from_slice(&input[..row]);
        }
        Ok(Some(slot.clone()))
    }
}
