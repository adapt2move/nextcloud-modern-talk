use super::*;
use std::time::Instant;

#[test]
fn screen_helper_grant_does_not_claim_desktop_grant_before_restart() {
    let value = mac_screen_observation(PermissionStatus::Granted, false);
    assert_eq!(value.status, PermissionStatus::Unknown);
    assert_eq!(value.guidance, PermissionGuidance::RestartRequired);
    assert!(!value.can_request);
    assert!(!value.allows_device_preview());
    let denied_or_first = mac_screen_observation(PermissionStatus::Unknown, false);
    assert_eq!(denied_or_first.status, PermissionStatus::Unknown);
    assert_ne!(
        denied_or_first.guidance,
        PermissionGuidance::RestartRequired
    );
    let granted = mac_screen_observation(PermissionStatus::Granted, true);
    assert_eq!(granted.status, PermissionStatus::Granted);
    assert_ne!(granted.guidance, PermissionGuidance::RestartRequired);
}

#[test]
fn permission_owner_starts_without_prompt_and_rejects_unchecked_requests() {
    let owner = PermissionRuntime::start().unwrap();
    let first = owner.snapshot();
    assert_eq!(first.operation, PermissionOperation::Idle);
    for kind in [
        PermissionKind::Microphone,
        PermissionKind::Camera,
        PermissionKind::ScreenSharing,
    ] {
        assert_eq!(
            owner.submit(PermissionIntent {
                generation: first.generation,
                request: RequestId(1),
                kind,
                action: PermissionAction::Request
            }),
            Err(PermissionError::Unsupported)
        );
    }
    assert_eq!(
        owner.submit(PermissionIntent {
            generation: first.generation + 1,
            request: RequestId(1),
            kind: PermissionKind::Microphone,
            action: PermissionAction::Check
        }),
        Err(PermissionError::Stale)
    );
    owner.request_shutdown();
    let until = Instant::now() + Duration::from_secs(2);
    while owner.snapshot().runtime != RuntimeState::Stopped {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        owner.snapshot().microphone.status,
        PermissionStatus::Unknown
    );
}

#[test]
fn screen_observation_never_admits_capture_and_unknown_device_does_not_meter() {
    for status in [
        PermissionStatus::Unknown,
        PermissionStatus::NotDetermined,
        PermissionStatus::Granted,
        PermissionStatus::Denied,
        PermissionStatus::Restricted,
        PermissionStatus::PerSession,
        PermissionStatus::NotRequired,
        PermissionStatus::Unsupported,
        PermissionStatus::Unavailable,
    ] {
        let mut observation = PermissionObservation::unknown(PermissionKind::ScreenSharing);
        observation.status = status;
        assert!(!observation.allows_device_preview());
        observation.kind = PermissionKind::Microphone;
        assert_eq!(
            observation.allows_device_preview(),
            matches!(
                status,
                PermissionStatus::Granted | PermissionStatus::NotRequired
            )
        );
    }
}

#[test]
fn mac_status_mapping_preserves_refusal_restriction_and_unknown() {
    let owner = PermissionRuntime::start().unwrap();
    for (output, expected) in [
        ("granted\n", PermissionStatus::Granted),
        ("not-determined", PermissionStatus::NotDetermined),
        ("denied", PermissionStatus::Denied),
        ("restricted", PermissionStatus::Restricted),
        ("unknown", PermissionStatus::Unknown),
    ] {
        for kind in [PermissionKind::Microphone, PermissionKind::Camera] {
            let value = mac_observation(kind, helper_status(output).unwrap());
            assert_eq!(value.status, expected);
            assert_eq!(
                value.can_request,
                expected == PermissionStatus::NotDetermined
            );
            assert!(value.can_open_settings);
            assert_eq!(value.guidance, PermissionGuidance::SystemSettings);
            assert_eq!(
                value.allows_device_preview(),
                expected == PermissionStatus::Granted
            );
            if !value.can_request {
                *observation(&mut owner.shared.lock().snapshot, kind) = value;
                assert_eq!(
                    owner.submit(PermissionIntent {
                        generation: owner.snapshot().generation,
                        request: RequestId(1),
                        kind,
                        action: PermissionAction::Request,
                    }),
                    Err(PermissionError::Unsupported)
                );
                assert_eq!(owner.snapshot().operation, PermissionOperation::Idle);
            }
        }
    }
    for malformed in ["", "authorized", "denied\ngranted", "future-status"] {
        assert_eq!(helper_status(malformed), Err(PermissionError::Failed));
    }
}

#[test]
fn negative_screen_preflight_is_unknown_and_settings_never_confer_capture() {
    for (authorized, status) in [
        (false, PermissionStatus::Unknown),
        (true, PermissionStatus::Granted),
    ] {
        let value = mac_observation(PermissionKind::ScreenSharing, screen_status(authorized));
        assert_eq!(value.status, status);
        assert_eq!(value.can_request, !authorized);
        assert!(value.can_open_settings);
        assert_eq!(value.guidance, PermissionGuidance::RestartAfterGrant);
        assert!(!value.allows_device_preview());
    }
}

#[test]
fn cancellation_before_dispatch_never_checks_requests_or_opens_settings() {
    for kind in [
        PermissionKind::Microphone,
        PermissionKind::Camera,
        PermissionKind::ScreenSharing,
    ] {
        for action in [
            PermissionAction::Check,
            PermissionAction::Request,
            PermissionAction::OpenSystemSettings,
        ] {
            assert_eq!(
                platform(
                    PermissionIntent {
                        generation: 1,
                        request: RequestId(1),
                        kind,
                        action,
                    },
                    || true,
                ),
                Err(PermissionError::Cancelled)
            );
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn permission_helper_cancellation_and_timeout_reap_child_and_pipe_reader() {
    // A stopped synthetic child keeps its output pipe open until killed. Returning
    // from capture therefore also proves its owned reader was joined, not detached.
    for cancel in [true, false] {
        let directory = std::env::temp_dir().join(format!(
            "nmt-permission-helper-{}-{cancel}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();
        let pid_file = directory.join("pid");
        let result = native_process::capture(
            Command::new("/bin/sh")
                .args([
                    "-c",
                    "echo $$ > \"$1\"; kill -STOP \"$$\"",
                    "permission-fixture",
                ])
                .arg(&pid_file),
            Duration::from_secs(2),
            || {
                cancel
                    && std::fs::read_to_string(&pid_file)
                        .is_ok_and(|pid| pid.trim().parse::<u32>().is_ok())
            },
        );
        let pid = std::fs::read_to_string(&pid_file);
        std::fs::remove_dir_all(&directory).unwrap();
        assert_eq!(
            result,
            Err(if cancel {
                PermissionError::Cancelled
            } else {
                PermissionError::TimedOut
            })
        );
        let pid: u32 = pid.unwrap().trim().parse().unwrap();
        assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    }
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "Requires an owned Linux desktop/session bus; no capture starts"]
fn native_linux_permission_check_does_not_create_a_share_session() {
    let result = platform(
        PermissionIntent {
            generation: 1,
            request: RequestId(1),
            kind: PermissionKind::ScreenSharing,
            action: PermissionAction::Check,
        },
        || false,
    )
    .unwrap()
    .unwrap();
    assert!(matches!(
        result.status,
        PermissionStatus::PerSession | PermissionStatus::Unavailable
    ));
    assert!(!result.can_request && !result.allows_device_preview());
}
