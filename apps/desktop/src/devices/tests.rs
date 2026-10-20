use super::*;

fn wait_for(
    runtime: &DeviceRuntime,
    timeout: Duration,
    predicate: impl Fn(&DeviceSnapshot) -> bool,
) -> DeviceSnapshot {
    let start = Instant::now();
    loop {
        let snapshot = runtime.snapshot();
        if predicate(&snapshot) {
            return snapshot;
        }
        assert!(
            start.elapsed() < timeout,
            "device state timed out: {snapshot:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn shutdown(runtime: &DeviceRuntime) {
    runtime.request_shutdown();
    wait_for(runtime, Duration::from_secs(3), |s| {
        s.runtime == RuntimeState::Stopped
    });
}

#[test]
fn idle_runtime_has_no_permission_or_capture_side_effect() {
    let runtime = DeviceRuntime::start().unwrap();
    let initial = runtime.snapshot();
    std::thread::sleep(Duration::from_millis(80));
    assert_eq!(runtime.snapshot(), initial);
    assert_eq!(initial.microphone_permission, PermissionState::Unknown);
    assert_eq!(initial.microphone_level, None);
    assert_eq!(initial.speaker_level, None);
    assert_eq!(initial.session, None);
    shutdown(&runtime);
}

#[test]
fn missing_saved_identity_is_retained_without_selecting_a_different_default() {
    let saved = DeviceId::new("audio:missing-physical-device").unwrap();
    let inventory = Inventory {
        audio: vec![DeviceInfo {
            id: DeviceId::new("audio:different-device").unwrap(),
            label: safe_label("Other mic"),
            kind: DeviceKind::Microphone,
            is_default: true,
        }],
        ..Inventory::default()
    };
    let choice = DeviceSelection::Specific(saved.clone());
    assert_eq!(
        inventory.resolve(&choice, DeviceKind::Microphone),
        SelectionState::DefaultFallback {
            missing: saved,
            effective: None
        }
    );
    assert_eq!(
        inventory.effective(&choice, DeviceKind::Microphone),
        Err(DeviceError::Removed)
    );
}

#[test]
fn frame_pool_is_bounded_across_sessions_and_revokes_old_pixels() {
    let mut pool = frames::FramePool::default();
    let pixels = vec![31; frames::FRAME_BYTES];
    let signal = Arc::new(Signal::new());
    let a = pool
        .copy(&pixels, 2560, VideoSurfaceId(1), signal.clone())
        .unwrap()
        .unwrap();
    let b = pool
        .copy(&pixels, 2560, VideoSurfaceId(1), signal.clone())
        .unwrap()
        .unwrap();
    let c = pool
        .copy(&pixels, 2560, VideoSurfaceId(1), signal.clone())
        .unwrap()
        .unwrap();
    assert_eq!(a.rgba().len(), frames::FRAME_BYTES);
    assert!(
        pool.copy(&pixels, 2560, VideoSurfaceId(1), signal.clone())
            .unwrap()
            .is_none()
    );
    signal.revoke();
    assert!(a.rgba().is_empty());
    let next = Arc::new(Signal::new());
    assert!(
        pool.copy(&pixels, 2560, VideoSurfaceId(2), next.clone())
            .unwrap()
            .is_none()
    );
    drop(b);
    let reused = pool
        .copy(&pixels, 2560, VideoSurfaceId(2), next)
        .unwrap()
        .unwrap();
    assert_eq!(reused.rgba()[0], 31);
    assert!(reused.sequence() > c.sequence());
}

#[test]
fn invalid_frame_layout_is_rejected_before_output_allocation() {
    let mut pool = frames::FramePool::default();
    let signal = Arc::new(Signal::new());
    assert!(
        pool.copy(&[0; 4], 0, VideoSurfaceId(1), signal.clone())
            .is_err()
    );
    assert!(
        pool.copy(&[0; 4], usize::MAX, VideoSurfaceId(1), signal)
            .is_err()
    );
}

#[test]
fn display_names_are_bounded_and_remove_control_characters() {
    let label = safe_label(&format!("Camera\n\u{202e}{}", "ß".repeat(300)));
    assert!(label.as_str().len() <= 128);
    assert!(!label.as_str().contains(['\n', '\u{202e}']));
}

#[test]
fn permission_errors_are_typed_and_stale_callbacks_cannot_publish() {
    assert_eq!(
        audio::map_error(cpal::Error::new(cpal::ErrorKind::PermissionDenied)),
        DeviceError::PermissionDenied
    );
    let signal = Signal::new();
    signal.revoke();
    signal.fail(DeviceError::PermissionDenied);
    assert_eq!(signal.error.load(Ordering::Acquire), 0);
}

#[test]
fn unavailable_start_stop_ack_and_obsolete_session_are_honest() {
    let runtime = DeviceRuntime::start().unwrap();
    let saved = DevicePreferences {
        microphone: DeviceSelection::Specific(DeviceId::new("audio:missing-test-device").unwrap()),
        ..DevicePreferences::default()
    };
    runtime
        .submit(DeviceIntent::ApplySelection {
            request: RequestId(1),
            revision: Revision(1),
            values: saved.clone(),
        })
        .unwrap();
    runtime
        .submit(DeviceIntent::StartPreview {
            request: RequestId(2),
            session: PreviewSession(1),
            kind: DeviceKind::Microphone,
        })
        .unwrap();
    let failed = wait_for(&runtime, Duration::from_secs(8), |s| {
        matches!(
            s.microphone_preview,
            PreviewState::Failed {
                error: DeviceError::Removed,
                ..
            }
        )
    });
    assert_eq!(failed.microphone_level, None);
    assert_eq!(runtime.selected_plan().saved, saved);
    runtime
        .submit(DeviceIntent::StopPreview {
            session: PreviewSession(1),
        })
        .unwrap();
    wait_for(&runtime, Duration::from_secs(3), |s| s.session.is_none());
    assert_eq!(
        runtime.submit(DeviceIntent::StartPreview {
            request: RequestId(3),
            session: PreviewSession(1),
            kind: DeviceKind::Microphone
        }),
        Err(DeviceError::Stale)
    );
    shutdown(&runtime);
}

/// Explicitly opt in on an owned PulseAudio null-sink/monitor. This test exercises
/// actual native provider callbacks and OS routing; it is synthetic DEVICE input,
/// not a physical microphone, acoustic speaker, permission prompt or Mac result.
#[test]
#[ignore = "requires owned OS loopback named NMT Synthetic DEVICE"]
fn native_os_loopback_meter_tone_and_disposal() {
    let runtime = DeviceRuntime::start().unwrap();
    runtime
        .submit(DeviceIntent::Enumerate {
            request: RequestId(1),
        })
        .unwrap();
    let snapshot = wait_for(&runtime, Duration::from_secs(8), |s| {
        s.devices.as_slice().iter().any(|d| {
            d.label
                .as_str()
                .replace('_', " ")
                .contains("NMT Synthetic DEVICE")
        })
    });
    let selected = |kind| {
        snapshot
            .devices
            .as_slice()
            .iter()
            .find(|d| {
                d.kind == kind
                    && d.label
                        .as_str()
                        .replace('_', " ")
                        .contains("NMT Synthetic DEVICE")
            })
            .unwrap()
            .id
            .clone()
    };
    let values = DevicePreferences {
        microphone: DeviceSelection::Specific(selected(DeviceKind::Microphone)),
        speaker: DeviceSelection::Specific(selected(DeviceKind::Speaker)),
        camera: DeviceSelection::SystemDefault,
    };
    runtime
        .submit(DeviceIntent::ApplySelection {
            request: RequestId(2),
            revision: Revision(1),
            values,
        })
        .unwrap();
    runtime
        .submit(DeviceIntent::StartPreview {
            request: RequestId(3),
            session: PreviewSession(1),
            kind: DeviceKind::Microphone,
        })
        .unwrap();
    wait_for(&runtime, Duration::from_secs(5), |s| {
        matches!(s.microphone_preview, PreviewState::Live { .. })
    });
    runtime
        .submit(DeviceIntent::TestSpeaker {
            request: RequestId(4),
            session: PreviewSession(1),
        })
        .unwrap();
    let output = wait_for(&runtime, Duration::from_secs(3), |s| {
        s.speaker_level.is_some_and(|v| v > 50)
    });
    assert!(matches!(output.speaker_test, PreviewState::Live { .. }));
    let heard = wait_for(&runtime, Duration::from_secs(3), |s| {
        s.microphone_level.is_some_and(|v| v > 50)
    });
    println!(
        "SYNTHETIC DEVICE: OS-loopback real RMS {:?}, submitted speaker activity {:?}",
        heard.microphone_level, output.speaker_level
    );
    let stopped = wait_for(&runtime, Duration::from_secs(3), |s| {
        s.speaker_test == PreviewState::Stopped && s.speaker_level.is_none()
    });
    // A per-kind inventory refresh preserves all other kinds and does not restart output.
    runtime
        .submit(DeviceIntent::Refresh {
            request: RequestId(5),
            kind: DeviceKind::Microphone,
        })
        .unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(runtime.snapshot().speaker, stopped.speaker);
    assert_eq!(runtime.snapshot().camera, stopped.camera);
    assert_eq!(runtime.snapshot().speaker_test, PreviewState::Stopped);
    // Switching the microphone preference revokes the stream. Only a fresh intent
    // after the worker's disposal acknowledgement may start it again.
    let mut values = runtime.selected_plan().saved;
    values.microphone = DeviceSelection::SystemDefault;
    runtime
        .submit(DeviceIntent::ApplySelection {
            request: RequestId(6),
            revision: Revision(2),
            values,
        })
        .unwrap();
    wait_for(&runtime, Duration::from_secs(3), |s| {
        s.microphone_preview == PreviewState::Stopped && s.microphone_level.is_none()
    });
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(runtime.snapshot().microphone_preview, PreviewState::Stopped);
    runtime
        .submit(DeviceIntent::StartPreview {
            request: RequestId(7),
            session: PreviewSession(1),
            kind: DeviceKind::Microphone,
        })
        .unwrap();
    wait_for(&runtime, Duration::from_secs(5), |s| {
        matches!(s.microphone_preview, PreviewState::Live { .. })
    });
    runtime
        .submit(DeviceIntent::StopDevice {
            session: PreviewSession(1),
            kind: DeviceKind::Microphone,
        })
        .unwrap();
    wait_for(&runtime, Duration::from_secs(3), |s| {
        s.microphone_preview == PreviewState::Stopped && s.microphone_level.is_none()
    });
    runtime
        .submit(DeviceIntent::StopPreview {
            session: PreviewSession(1),
        })
        .unwrap();
    wait_for(&runtime, Duration::from_secs(3), |s| s.session.is_none());
    shutdown(&runtime);
}

#[test]
fn stopping_device_and_session_do_not_admit_a_replacement_before_acknowledgement() {
    let runtime = DeviceRuntime::start().unwrap();
    {
        // Hold admission while exercising the stop boundary, before the worker
        // can acknowledge disposal. No hardware or permission operation runs.
        let mut state = runtime.shared.lock();
        start_desired(&mut state, RequestId(1), PreviewSession(1), 0).unwrap();
        state.stop(0);
        assert!(matches!(
            state.snapshot.microphone_preview,
            PreviewState::Stopping { .. }
        ));
        assert_eq!(
            start_desired(&mut state, RequestId(2), PreviewSession(1), 0),
            Err(DeviceError::Busy)
        );
        state.stop_all = true;
        assert_eq!(
            start_desired(&mut state, RequestId(3), PreviewSession(2), 1),
            Err(DeviceError::Busy)
        );
    }
    runtime.shared.wake.notify_one();
    wait_for(&runtime, Duration::from_secs(3), |s| s.session.is_none());
    shutdown(&runtime);
}

#[test]
#[ignore = "requires an owned OS V4L2 loopback camera"]
fn native_os_camera_capture_and_revoke() {
    let runtime = DeviceRuntime::start().unwrap();
    runtime
        .submit(DeviceIntent::Enumerate {
            request: RequestId(1),
        })
        .unwrap();
    let snapshot = wait_for(&runtime, Duration::from_secs(8), |s| {
        s.devices.as_slice().iter().any(|d| {
            d.kind == DeviceKind::Camera && d.label.as_str().contains("NMT Synthetic DEVICE")
        })
    });
    let id = snapshot
        .devices
        .as_slice()
        .iter()
        .find(|d| d.kind == DeviceKind::Camera && d.label.as_str().contains("NMT Synthetic DEVICE"))
        .unwrap()
        .id
        .clone();
    runtime
        .submit(DeviceIntent::ApplySelection {
            request: RequestId(2),
            revision: Revision(1),
            values: DevicePreferences {
                camera: DeviceSelection::Specific(id),
                ..DevicePreferences::default()
            },
        })
        .unwrap();
    runtime
        .submit(DeviceIntent::StartPreview {
            request: RequestId(3),
            session: PreviewSession(1),
            kind: DeviceKind::Camera,
        })
        .unwrap();
    let live = wait_for(&runtime, Duration::from_secs(8), |s| {
        matches!(
            s.camera_preview,
            PreviewState::Live {
                surface: Some(_),
                ..
            }
        )
    });
    let PreviewState::Live {
        surface: Some(surface),
        ..
    } = live.camera_preview
    else {
        unreachable!()
    };
    let frame = runtime.latest_frame(surface, 0).unwrap();
    assert_eq!(frame.rgba().len(), frames::FRAME_BYTES);
    let first_sequence = frame.sequence();
    let started = Instant::now();
    while runtime.latest_frame(surface, first_sequence).is_none() {
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "camera did not deliver a later actual frame"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    println!(
        "SYNTHETIC DEVICE: native OS camera frame {} {}x{}",
        frame.sequence(),
        frame.width(),
        frame.height()
    );
    runtime
        .submit(DeviceIntent::StopPreview {
            session: PreviewSession(1),
        })
        .unwrap();
    assert!(frame.rgba().is_empty());
    wait_for(&runtime, Duration::from_secs(3), |s| s.session.is_none());
    assert!(runtime.latest_frame(surface, 0).is_none());
    shutdown(&runtime);
}

#[test]
#[ignore = "run as a user denied read/write permission on the owned V4L2 fixture"]
fn native_os_camera_permission_refusal() {
    let runtime = DeviceRuntime::start().unwrap();
    runtime
        .submit(DeviceIntent::Enumerate {
            request: RequestId(1),
        })
        .unwrap();
    let snapshot = wait_for(&runtime, Duration::from_secs(8), |s| {
        s.devices.as_slice().iter().any(|d| {
            d.kind == DeviceKind::Camera && d.label.as_str().contains("NMT Synthetic DEVICE")
        })
    });
    let id = snapshot
        .devices
        .as_slice()
        .iter()
        .find(|d| d.kind == DeviceKind::Camera && d.label.as_str().contains("NMT Synthetic DEVICE"))
        .unwrap()
        .id
        .clone();
    runtime
        .submit(DeviceIntent::ApplySelection {
            request: RequestId(2),
            revision: Revision(1),
            values: DevicePreferences {
                camera: DeviceSelection::Specific(id),
                ..DevicePreferences::default()
            },
        })
        .unwrap();
    runtime
        .submit(DeviceIntent::StartPreview {
            request: RequestId(3),
            session: PreviewSession(1),
            kind: DeviceKind::Camera,
        })
        .unwrap();
    let denied = wait_for(&runtime, Duration::from_secs(5), |s| {
        matches!(
            s.camera_preview,
            PreviewState::Failed {
                error: DeviceError::PermissionDenied,
                ..
            }
        )
    });
    assert_eq!(denied.camera_permission, PermissionState::Denied);
    assert!(runtime.shared.lock().frame.is_none());
    shutdown(&runtime);
}

#[test]
#[ignore = "requires the owned loopback and a runner removing it after READY_OS_REMOVE"]
fn native_os_audio_hotplug_removal() {
    let runtime = DeviceRuntime::start().unwrap();
    runtime
        .submit(DeviceIntent::Enumerate {
            request: RequestId(1),
        })
        .unwrap();
    let snapshot = wait_for(&runtime, Duration::from_secs(8), |s| {
        s.devices.as_slice().iter().any(|d| {
            d.kind == DeviceKind::Microphone
                && d.label
                    .as_str()
                    .replace('_', " ")
                    .contains("NMT Synthetic DEVICE")
        })
    });
    let id = snapshot
        .devices
        .as_slice()
        .iter()
        .find(|d| {
            d.kind == DeviceKind::Microphone
                && d.label
                    .as_str()
                    .replace('_', " ")
                    .contains("NMT Synthetic DEVICE")
        })
        .unwrap()
        .id
        .clone();
    runtime
        .submit(DeviceIntent::ApplySelection {
            request: RequestId(2),
            revision: Revision(1),
            values: DevicePreferences {
                microphone: DeviceSelection::Specific(id.clone()),
                ..DevicePreferences::default()
            },
        })
        .unwrap();
    runtime
        .submit(DeviceIntent::StartPreview {
            request: RequestId(3),
            session: PreviewSession(1),
            kind: DeviceKind::Microphone,
        })
        .unwrap();
    wait_for(&runtime, Duration::from_secs(5), |s| {
        matches!(s.microphone_preview, PreviewState::Live { .. })
    });
    println!("READY_OS_REMOVE");
    let removed = wait_for(&runtime, Duration::from_secs(8), |s| {
        matches!(
            s.microphone_preview,
            PreviewState::Failed {
                error: DeviceError::Removed,
                ..
            }
        )
    });
    assert_eq!(removed.microphone_level, None);
    assert_eq!(
        runtime.selected_plan().saved.microphone,
        DeviceSelection::Specific(id)
    );
    shutdown(&runtime);
}

#[test]
#[ignore = "requires the virtual camera to have been removed, with no physical cameras"]
fn native_os_no_camera_is_unavailable() {
    let runtime = DeviceRuntime::start().unwrap();
    runtime
        .submit(DeviceIntent::StartPreview {
            request: RequestId(1),
            session: PreviewSession(1),
            kind: DeviceKind::Camera,
        })
        .unwrap();
    let absent = wait_for(&runtime, Duration::from_secs(5), |s| {
        matches!(
            s.camera_preview,
            PreviewState::Failed {
                error: DeviceError::Unavailable,
                ..
            }
        )
    });
    assert!(
        !absent
            .devices
            .as_slice()
            .iter()
            .any(|d| d.kind == DeviceKind::Camera)
    );
    assert!(runtime.shared.lock().frame.is_none());
    shutdown(&runtime);
}
