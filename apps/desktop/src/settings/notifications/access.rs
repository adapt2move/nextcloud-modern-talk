//! One bounded authorization operation on the existing notification worker.
//! Checks and delivery never prompt. Only an admitted explicit Request can do so.
use super::*;

#[derive(Clone, Copy)]
pub(super) struct Command {
    request: RequestId,
    action: NotificationAccessAction,
    admitted: Instant,
}

pub(super) fn admit(
    state: &mut State,
    request: RequestId,
    action: NotificationAccessAction,
    now: Instant,
) -> Result<(), NotificationError> {
    if state.stopped {
        return Err(NotificationError::Closed);
    }
    if request <= state.last_access_request {
        return Err(NotificationError::Stale);
    }
    if matches!(
        state.access.operation,
        NotificationAccessOperation::Pending { .. }
    ) || state.running.is_some()
    {
        return Err(NotificationError::Busy);
    }
    if action == NotificationAccessAction::Request
        && state.access.authorization != NotificationAuthorization::NotDetermined
    {
        return Err(NotificationError::PermissionDenied);
    }
    if action == NotificationAccessAction::Request {
        for lane in 0..3 {
            if let Some(event) = state.pending[lane].take() {
                state.finish(
                    &event,
                    DeliveryOutcome::Cancelled,
                    DeliveryOutcome::Cancelled,
                );
            }
        }
    }
    state.last_access_request = request;
    state.access.operation = NotificationAccessOperation::Pending { request, action };
    state.access_pending = Some(Command {
        request,
        action,
        admitted: now,
    });
    Ok(())
}

pub(super) fn cancel(state: &mut State) {
    state.access_pending = None;
    if let NotificationAccessOperation::Pending { request, .. } = state.access.operation {
        state.access.operation = NotificationAccessOperation::Failed {
            request,
            error: NotificationError::Cancelled,
        };
    }
}

pub(super) fn complete(
    state: &mut State,
    command: Command,
    result: Result<Option<NotificationAuthorization>, NotificationError>,
) {
    if state.stopped
        || !matches!(state.access.operation, NotificationAccessOperation::Pending { request, .. } if request == command.request)
    {
        return;
    }
    match result {
        Ok(status) => {
            if let Some(status) = status {
                state.access.authorization = status;
            }
            state.access.operation = NotificationAccessOperation::Finished {
                request: command.request,
            };
        }
        Err(error) => {
            state.access.operation = NotificationAccessOperation::Failed {
                request: command.request,
                error,
            }
        }
    }
}

pub(super) fn perform(
    shared: &Shared,
    command: Command,
) -> Result<Option<NotificationAuthorization>, NotificationError> {
    if command.admitted.elapsed() >= Duration::from_secs(5) {
        return Err(NotificationError::TimedOut);
    }
    let cancelled = || shared.lock().stopped;
    if cancelled() {
        return Err(NotificationError::Cancelled);
    }
    if !cfg!(target_os = "macos") {
        return if command.action == NotificationAccessAction::Check && cfg!(target_os = "linux") {
            Ok(Some(NotificationAuthorization::NotRequired))
        } else {
            Err(NotificationError::Unavailable)
        };
    }
    if command.action == NotificationAccessAction::OpenSystemSettings {
        let mut process = std::process::Command::new("/usr/bin/open");
        process.args(["-b", "com.apple.systempreferences"]);
        super::super::native_process::launch(&mut process, Duration::from_secs(3), cancelled)
            .map_err(mapped)?;
        return Ok(None); // Opening Settings is not a grant.
    }
    let helper = match super::super::native_process::mac_helper() {
        Ok(helper) => helper,
        Err(_) => return Ok(Some(NotificationAuthorization::Unavailable)),
    };
    let requesting = command.action == NotificationAccessAction::Request;
    let output = super::super::native_process::capture(
        std::process::Command::new(helper).args([
            "permission",
            "notifications",
            if requesting { "request" } else { "check" },
        ]),
        Duration::from_secs(if requesting { 120 } else { 3 }),
        cancelled,
    )
    .map_err(mapped)?;
    decode(&output).map(Some)
}

fn mapped(error: talk_core::preferences::permissions::PermissionError) -> NotificationError {
    use talk_core::preferences::permissions::PermissionError;
    match error {
        PermissionError::Cancelled => NotificationError::Cancelled,
        PermissionError::TimedOut => NotificationError::TimedOut,
        PermissionError::Unavailable | PermissionError::Unsupported => {
            NotificationError::Unavailable
        }
        PermissionError::Closed => NotificationError::Closed,
        PermissionError::Busy => NotificationError::Busy,
        PermissionError::Stale => NotificationError::Stale,
        PermissionError::Failed => NotificationError::Failed,
    }
}
fn decode(output: &str) -> Result<NotificationAuthorization, NotificationError> {
    match output.trim() {
        "not-determined" => Ok(NotificationAuthorization::NotDetermined),
        "granted" => Ok(NotificationAuthorization::Granted),
        "denied" => Ok(NotificationAuthorization::Denied),
        "unknown" => Ok(NotificationAuthorization::Unknown),
        _ => Err(NotificationError::Failed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_permission_is_explicit_and_does_not_replay_pending_alerts_after_the_sheet() {
        let now = Instant::now();
        let mut state = State::new(NotificationPreferences::default());
        assert_eq!(
            admit(
                &mut state,
                RequestId(1),
                NotificationAccessAction::Request,
                now
            ),
            Err(NotificationError::PermissionDenied)
        );
        admit(
            &mut state,
            RequestId(1),
            NotificationAccessAction::Check,
            now,
        )
        .unwrap();
        let checking = state.access_pending.take().unwrap();
        assert_eq!(
            admit(
                &mut state,
                RequestId(2),
                NotificationAccessAction::Request,
                now
            ),
            Err(NotificationError::Busy)
        );
        complete(
            &mut state,
            checking,
            Ok(Some(NotificationAuthorization::NotDetermined)),
        );
        let event = NotificationEvent::test(
            RequestId(1),
            NotificationCategory::Messages,
            talk_core::bounded::Text::new("test").unwrap(),
            talk_core::bounded::Text::new("").unwrap(),
        );
        state.admit(event.clone(), now).unwrap();
        admit(
            &mut state,
            RequestId(2),
            NotificationAccessAction::Request,
            now,
        )
        .unwrap();
        assert!(state.pending.iter().all(Option::is_none));
        let completion = state.completions.pop_front().unwrap();
        assert_eq!(
            (completion.banner, completion.sound),
            (DeliveryOutcome::Cancelled, DeliveryOutcome::Cancelled)
        );
        assert_eq!(
            state.admit(
                NotificationEvent {
                    request: RequestId(2),
                    ..event
                },
                now
            ),
            Err(NotificationError::PermissionDenied)
        );
        let requesting = state.access_pending.take().unwrap();
        complete(
            &mut state,
            requesting,
            Ok(Some(NotificationAuthorization::Denied)),
        );
        assert_eq!(
            admit(
                &mut state,
                RequestId(3),
                NotificationAccessAction::Request,
                now
            ),
            Err(NotificationError::PermissionDenied)
        );
        admit(
            &mut state,
            RequestId(3),
            NotificationAccessAction::Check,
            now,
        )
        .unwrap();
        let checking = state.access_pending.take().unwrap();
        complete(
            &mut state,
            checking,
            Ok(Some(NotificationAuthorization::Granted)),
        );
        assert_eq!(
            state.access.authorization,
            NotificationAuthorization::Granted
        );
        assert!(
            state.pending.iter().all(Option::is_none),
            "granting permission cannot resurrect an old alert"
        );
    }

    #[test]
    fn stale_or_cancelled_completions_cannot_replace_a_newer_permission_observation() {
        let now = Instant::now();
        let mut state = State::new(NotificationPreferences::default());
        admit(
            &mut state,
            RequestId(1),
            NotificationAccessAction::Check,
            now,
        )
        .unwrap();
        let first = state.access_pending.take().unwrap();
        complete(
            &mut state,
            first,
            Ok(Some(NotificationAuthorization::Denied)),
        );
        admit(
            &mut state,
            RequestId(2),
            NotificationAccessAction::Check,
            now,
        )
        .unwrap();
        let second = state.access_pending.take().unwrap();
        complete(
            &mut state,
            first,
            Ok(Some(NotificationAuthorization::Granted)),
        );
        assert_eq!(
            state.access.authorization,
            NotificationAuthorization::Denied
        );
        state.stopped = true;
        cancel(&mut state);
        complete(
            &mut state,
            second,
            Ok(Some(NotificationAuthorization::Granted)),
        );
        assert_eq!(
            state.access.authorization,
            NotificationAuthorization::Denied
        );
        assert_eq!(
            state.access.operation,
            NotificationAccessOperation::Failed {
                request: RequestId(2),
                error: NotificationError::Cancelled
            }
        );
    }

    #[test]
    fn expired_requests_and_shutdown_never_start_a_late_os_prompt() {
        let shared = Shared {
            state: Mutex::new(State::new(NotificationPreferences::default())),
            wake: Condvar::new(),
        };
        let expired = Command {
            request: RequestId(1),
            action: NotificationAccessAction::Request,
            admitted: Instant::now() - Duration::from_secs(6),
        };
        assert_eq!(perform(&shared, expired), Err(NotificationError::TimedOut));
        shared.lock().stopped = true;
        let current = Command {
            admitted: Instant::now(),
            ..expired
        };
        assert_eq!(perform(&shared, current), Err(NotificationError::Cancelled));
        for invalid in ["granted\ndenied", "", "authorized", "private helper error"] {
            assert_eq!(decode(invalid), Err(NotificationError::Failed));
        }
    }
}
