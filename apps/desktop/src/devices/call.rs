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
    /// The saved choice per kind, to tell the user's new choice from a failure.
    chosen: [Option<DeviceSelection>; 3],
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
        self.chosen = [None, None, None];
        Ok(())
    }
    pub(super) fn active(&self) -> bool {
        self.scope.is_some()
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
        // The user choosing another device is not a device failure: the stream
        // moves to the chosen device, and a device blocked by an earlier failure
        // gets a fresh attempt. A system default that moves on its own keeps the
        // stricter path below: the stream stops instead of migrating silently.
        let mut changed = [false; 3];
        for (index, changed) in changed.iter_mut().enumerate() {
            let choice = selection(preferences, index);
            *changed = self.chosen[index]
                .as_ref()
                .is_some_and(|previous| previous != choice);
            if *changed || self.chosen[index].is_none() {
                self.chosen[index] = Some(choice.clone());
            }
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
            let invalid = self.audio[index].as_ref().is_some_and(|(c, s)| {
                !requested
                    || resolved.as_ref() != Ok(c.id())
                    || !s.valid()
                    || s.error.load(Ordering::Acquire) != 0
                    || s.stalled()
            });
            let switched = self.audio[index].as_ref().is_some_and(|(c, s)| {
                changed[index]
                    && s.valid()
                    && s.error.load(Ordering::Acquire) == 0
                    && !s.stalled()
                    && resolved.as_ref().is_ok_and(|id| id != c.id())
            });
            if invalid {
                if let Some((_, s)) = self.audio[index].as_ref() {
                    s.revoke();
                }
                self.audio[index] = None;
                if requested && !switched {
                    if index == 0 {
                        handle.device_failed(true, false);
                    } else {
                        self.speaker_failed = true;
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
                    Err(_) => {
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
        let switched = self.camera.as_ref().is_some_and(|(c, s)| {
            changed[2]
                && s.error.load(Ordering::Acquire) == 0
                && !s.stalled()
                && resolved.as_ref().is_ok_and(|id| id != c.id())
        });
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
                Err(_) => {
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
