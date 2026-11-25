//! Runs on the existing selected-device owner. No separate native-device thread.
use super::*;
use talk_adapters::media::{CaptureHandle, MediaService};
#[derive(Default)]
pub(super) struct Calls {
    scope: Option<talk_core::ids::CallScope>,
    audio: [Option<(audio::Capture, Arc<Signal>)>; 2],
    camera: Option<(camera::Capture, Arc<Signal>)>,
    frames: frames::FramePool,
    speaker_failed: bool,
    /// The device each kind last resolved to, to tell a new device from a failure.
    resolved: [Option<DeviceId>; 3],
    screen: Option<(u64, screen::Capture)>,
}
impl Calls {
    pub(super) fn stop(&mut self) -> Result<(), DeviceError> {
        for slot in &mut self.audio {
            if let Some((_, signal)) = slot.as_ref() {
                signal.revoke();
            }
            *slot = None;
        }
        if let Some((mut camera, signal)) = self.camera.take() {
            signal.revoke();
            camera.stop()?;
        }
        self.screen = None;
        self.frames.clear_released();
        self.scope = None;
        self.speaker_failed = false;
        self.resolved = [None, None, None];
        Ok(())
    }
    pub(super) fn active(&self) -> bool {
        self.scope.is_some()
    }
    /// A running camera is polled twice as often, so a new frame reaches the
    /// preview and the encoder within half a display frame.
    pub(super) fn camera_active(&self) -> bool {
        self.camera.is_some()
    }
    pub(super) fn tick(
        &mut self,
        service: &MediaService,
        inventory: &Inventory,
        preferences: &DevicePreferences,
    ) -> Result<(), DeviceError> {
        let handle = service.capture();
        let scope = handle.as_ref().and_then(CaptureHandle::scope);
        if scope != self.scope {
            self.stop()?;
            self.scope = scope;
        }
        let Some(handle) = handle else {
            return Ok(());
        };
        // Another device is not a device failure, whether the user chose it or
        // the system default moved (headphones unplugged): the stream follows
        // the selection, and a medium blocked by an earlier failure is tried
        // again. A specifically chosen device that disappears still stops.
        let mut changed = [false; 3];
        for (index, kind) in KINDS.into_iter().enumerate() {
            let current = inventory
                .effective(selection(preferences, index), kind)
                .ok();
            // A route can disappear and return with the same persistent ID.
            // Lift its failure latch only on the transition back to availability.
            changed[index] = current.is_some() && self.resolved[index] != current;
            self.resolved[index] = current;
        }
        if changed[0] || changed[2] {
            handle.device_retry(changed[0], changed[2]);
        }
        if changed[1] {
            self.speaker_failed = false;
        }
        let wishes = handle.requested();
        let screen_generation = handle.screen_generation();
        if self.screen.as_ref().is_some_and(|(generation, capture)| {
            Some(*generation) != screen_generation || capture.finished()
        }) {
            self.screen = None;
        }
        if let Some(generation) = screen_generation
            && self.screen.is_none()
        {
            match screen::Capture::start(handle.clone(), generation) {
                Ok(capture) => self.screen = Some((generation, capture)),
                Err(()) => handle.screen_state(
                    generation,
                    talk_core::calls::SharingState::Failed(
                        talk_core::calls::ShareFailure::Unavailable,
                    ),
                ),
            }
        }
        for (index, requested) in [wishes.microphone, handle.receive_allowed()]
            .into_iter()
            .enumerate()
        {
            let resolved = inventory.effective(selection(preferences, index), KINDS[index]);
            let restart = self.audio[index]
                .as_ref()
                .is_some_and(|(c, _)| c.epoch() != handle.audio_epoch(index == 0));
            if let Some((capture, signal)) = &self.audio[index] {
                let failure = if !requested || restart || changed[index] {
                    None
                } else {
                    resolved
                        .as_ref()
                        .err()
                        .copied()
                        .or_else(|| stream_failure(signal))
                };
                if !requested
                    || restart
                    || resolved.as_ref() != Ok(capture.id())
                    || failure.is_some()
                {
                    signal.revoke();
                    self.audio[index] = None;
                    if let Some(error) = failure {
                        note_failure(
                            if index == 0 {
                                "call_microphone"
                            } else {
                                "call_speaker"
                            },
                            error,
                        );
                        if index == 0 {
                            handle.device_failed(true, false);
                        } else {
                            self.speaker_failed = true;
                        }
                    }
                }
            }
            if requested
                && (index != 0 || handle.requested().microphone)
                && self.audio[index].is_none()
                && !(index == 1 && self.speaker_failed)
            {
                let signal = Arc::new(Signal::new());
                let result = resolved.and_then(|id| {
                    inventory
                        .audio_backend
                        .as_ref()
                        .ok_or(DeviceError::Unavailable)?
                        .call(&id, index == 0, signal.clone(), handle.clone())
                });
                match result {
                    Ok(c) => self.audio[index] = Some((c, signal)),
                    // The grant expired while opening the device. The next
                    // grant will select a fresh stream and epoch.
                    Err(DeviceError::Stale) => signal.revoke(),
                    Err(error) => {
                        note_failure(
                            if index == 0 {
                                "call_microphone"
                            } else {
                                "call_speaker"
                            },
                            error,
                        );
                        signal.revoke();
                        if index == 0 {
                            handle.device_failed(true, false);
                        } else {
                            self.speaker_failed = true;
                        }
                    }
                }
            }
        }
        let resolved = inventory.effective(&preferences.camera, DeviceKind::Camera);
        let switched = changed[2]
            && self
                .camera
                .as_ref()
                .is_some_and(|(c, _)| resolved.as_ref().is_ok_and(|id| id != c.id()));
        if self.camera.as_ref().is_some_and(|(c, s)| {
            !wishes.camera
                || resolved.as_ref() != Ok(c.id())
                || s.error.load(Ordering::Acquire) != 0
                || s.stalled()
        }) {
            if let Some((mut c, s)) = self.camera.take() {
                s.revoke();
                c.stop()?;
            }
            self.frames.clear_released();
            if wishes.camera && !switched {
                handle.device_failed(false, true);
            }
        }
        if handle.requested().camera && self.camera.is_none() {
            let signal = Arc::new(Signal::new());
            match resolved.and_then(|id| inventory.start_camera(&id, signal.clone())) {
                Ok(c) => self.camera = Some((c, signal)),
                Err(error) => {
                    note_failure("call_camera", error);
                    signal.revoke();
                    handle.device_failed(false, true);
                }
            }
        }
        if let Some((camera, signal)) = &mut self.camera {
            match camera.poll(&mut self.frames, VideoSurfaceId(0)) {
                Ok(Some(frame)) => {
                    handle.camera_rgba(frame.width(), frame.height(), frame.rgba());
                }
                Ok(None) => {}
                Err(e) => {
                    note_failure("call_camera", e);
                    signal.fail(e);
                    handle.device_failed(false, true);
                }
            }
        }
        Ok(())
    }
}
impl Drop for Calls {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn stream_failure(signal: &Signal) -> Option<DeviceError> {
    decode_error(signal.error.load(Ordering::Acquire)).or_else(|| {
        if !signal.valid() {
            Some(DeviceError::StreamInvalidated)
        } else if signal.stalled() {
            Some(DeviceError::StreamStalled)
        } else if signal.last_activity.load(Ordering::Relaxed) == 0
            && signal.born.elapsed() >= Duration::from_secs(8)
        {
            Some(DeviceError::StreamStartTimeout)
        } else {
            None
        }
    })
}

fn note_failure(kind: &'static str, error: DeviceError) {
    let reason = match error {
        DeviceError::PermissionDenied => "permission_denied",
        DeviceError::Removed => "removed",
        DeviceError::StreamStartTimeout => "callback_start_timeout",
        DeviceError::StreamInvalidated => "stream_invalidated",
        DeviceError::StreamStalled => "callback_stalled",
        DeviceError::Busy => "busy",
        DeviceError::Unavailable => "unavailable",
        DeviceError::InvalidSelection => "invalid_selection",
        DeviceError::Stale => "stale",
        DeviceError::CaptureFailed => "capture_failed",
    };
    crate::diagnostics::note(crate::diagnostics::Note::info(kind, "failed").detail(reason));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_first_callback_and_stalled_stream_have_distinct_causes() {
        let mut signal = Signal::new();
        assert_eq!(stream_failure(&signal), None);
        signal.born -= Duration::from_secs(9);
        assert_eq!(
            stream_failure(&signal),
            Some(DeviceError::StreamStartTimeout)
        );
        signal.last_activity.store(1, Ordering::Relaxed);
        assert_eq!(stream_failure(&signal), Some(DeviceError::StreamStalled));
        signal.touch();
        assert_eq!(stream_failure(&signal), None);
        signal.fail(audio::map_error(cpal::Error::new(
            cpal::ErrorKind::StreamInvalidated,
        )));
        assert_eq!(
            stream_failure(&signal),
            Some(DeviceError::StreamInvalidated)
        );
    }
}
