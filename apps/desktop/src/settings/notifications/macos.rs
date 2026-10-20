//! One app-bundled helper per current delivery; at most one reader and four lines.
//! Direct-call helpers live at most 45 seconds and disappear on revocation/EOF.
use super::*;
use std::{
    io::{BufRead, BufReader, Write},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
};
static LEASE: AtomicU64 = AtomicU64::new(1);

fn hex(value: &str) -> String {
    use std::fmt::Write;
    let mut result = String::with_capacity(value.len() * 2);
    for byte in value.bytes() {
        let _ = write!(result, "{byte:02x}");
    }
    result
}
fn allowed(shared: &Shared, event: &NotificationEvent, banner: bool, sound: bool) -> bool {
    (!banner || check(shared, event, Effect::Banner) == Ok(true))
        && (!sound || check(shared, event, Effect::Sound) == Ok(true))
}
fn sound_argument(enabled: bool, kind: NotificationKind) -> &'static str {
    // Sound selection is separate from current-call/action authority. An explicit
    // IncomingCalls test uses the ringtone while retaining a false call flag.
    if !enabled {
        "silent"
    } else if kind.category() == NotificationCategory::IncomingCalls {
        "incoming"
    } else {
        "message"
    }
}
pub(super) fn deliver(
    shared: &Shared,
    event: &NotificationEvent,
    banner: bool,
    sound: bool,
    dispatcher: Option<&dyn NotificationCallDispatcher>,
) -> (DeliveryOutcome, DeliveryOutcome) {
    let result = perform(shared, event, banner, sound, dispatcher);
    match result {
        Ok((banner_ok, sound_ok)) => (
            if !banner {
                DeliveryOutcome::Suppressed
            } else if banner_ok {
                DeliveryOutcome::SubmittedToSystem
            } else {
                DeliveryOutcome::Failed(NotificationError::PermissionDenied)
            },
            if !sound {
                DeliveryOutcome::Suppressed
            } else if sound_ok {
                DeliveryOutcome::SubmittedToSystem
            } else {
                DeliveryOutcome::Failed(NotificationError::PermissionDenied)
            },
        ),
        Err(error) => (
            if banner {
                outcome(Err(error))
            } else {
                DeliveryOutcome::Suppressed
            },
            if sound {
                outcome(Err(error))
            } else {
                DeliveryOutcome::Suppressed
            },
        ),
    }
}
fn perform(
    shared: &Shared,
    event: &NotificationEvent,
    banner: bool,
    sound: bool,
    dispatcher: Option<&dyn NotificationCallDispatcher>,
) -> Result<(bool, bool), NotificationError> {
    let helper =
        super::super::native_process::mac_helper().map_err(|_| NotificationError::Unavailable)?;
    if !allowed(shared, event, banner, sound) {
        return Err(NotificationError::Cancelled);
    }
    let call = match event.kind {
        NotificationKind::IncomingCall { call } => Some(call),
        _ => None,
    };
    let actions = if shared.lock().actions.len() < 8 {
        call.and_then(|call| dispatcher.map(|d| d.available_actions(call)))
            .unwrap_or_default()
    } else {
        NativeCallActions::default()
    };
    let locale = match shared.lock().locale {
        Locale::English => "en",
        Locale::German => "de",
        Locale::Spanish => "es",
    };
    let lease = LEASE
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
        .map_err(|_| NotificationError::Unavailable)?;
    let id = format!("nmt-{}-{}-{}", std::process::id(), event.request.0, lease);
    let bit = |value: bool| if value { "1" } else { "0" };
    let mut child = Command::new(helper)
        .args([
            "notify",
            &id,
            &hex(event.title.as_str()),
            &hex(event.body.as_str()),
            bit(banner),
            sound_argument(sound, event.kind),
            bit(call.is_some()),
            bit(actions.accept),
            bit(actions.decline),
            bit(matches!(event.kind, NotificationKind::Test { .. })),
            locale,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| NotificationError::Unavailable)?;
    let output = child.stdout.take().ok_or(NotificationError::Failed)?;
    let (send, receive) = mpsc::sync_channel(4);
    let reader = match std::thread::Builder::new()
        .name("talk-notification-actions".into())
        .spawn(move || {
            let mut output = BufReader::with_capacity(128, output);
            loop {
                let mut line = Vec::with_capacity(64);
                // A hostile/malformed helper cannot allocate an unbounded line.
                match std::io::Read::take(&mut output, 65).read_until(b'\n', &mut line) {
                    Ok(0) => return,
                    Ok(_) if line.len() <= 64 && line.last() == Some(&b'\n') => {
                        if send.try_send(String::from_utf8(line).ok()).is_err() {
                            return;
                        }
                    }
                    _ => {
                        let _ = send.try_send(None);
                        return;
                    }
                }
            }
        }) {
        Ok(reader) => reader,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(NotificationError::Unavailable);
        }
    };
    let deadline = Instant::now() + Duration::from_secs(if call.is_some() { 45 } else { 125 });
    let mut submitted = None;
    let mut exited = None;
    let result = loop {
        if !allowed(shared, event, banner, sound) {
            break Err(NotificationError::Cancelled);
        }
        if Instant::now() >= deadline {
            break submitted.ok_or(NotificationError::TimedOut);
        }
        if let Ok(line) = receive.try_recv() {
            let Some(line) = line else {
                break Err(NotificationError::Failed);
            };
            match line.trim() {
                "submitted\t1\t1" => submitted = Some((true, true)),
                "submitted\t1\t0" => submitted = Some((true, false)),
                "submitted\t0\t1" => submitted = Some((false, true)),
                "submitted\t0\t0" => submitted = Some((false, false)),
                "denied" => break Err(NotificationError::PermissionDenied),
                "failed" => break Err(NotificationError::Failed),
                action @ ("accept" | "decline") => {
                    let action = if action == "accept" {
                        NativeCallAction::Accept
                    } else {
                        NativeCallAction::Decline
                    };
                    let result = dispatch_action(shared, event, dispatcher, action);
                    if let Some(call) = call {
                        let mut state = shared.lock();
                        if state.actions.len() < 8 {
                            state.actions.push_back(NativeCallActionCompletion {
                                call,
                                action,
                                result,
                            });
                        }
                    }
                    break submitted.ok_or(NotificationError::Failed);
                }
                _ => break Err(NotificationError::Failed),
            }
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                // The pipe may still have its final acknowledgement; drain before
                // accepting exit. The native helper keeps a short ack grace period.
                if let Some(submitted) = submitted {
                    break Ok(submitted);
                }
                let finished = exited.get_or_insert_with(Instant::now);
                if finished.elapsed() >= Duration::from_millis(200) {
                    break Err(NotificationError::Failed);
                }
            }
            Ok(Some(_)) | Err(_) => break Err(NotificationError::Failed),
            _ => {}
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // Cooperative cancellation withdraws the notice. Kill/reap remains the bounded
    // fallback; expired actions remain inert even if the OS retained an old banner.
    if let Some(mut input) = child.stdin.take() {
        let _ = input.write_all(b"cancel\n");
    }
    let until = Instant::now() + Duration::from_secs(1);
    while child.try_wait().is_ok_and(|s| s.is_none()) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();
    let _ = reader.join();
    result
}
pub(super) fn dispatch_action(
    shared: &Shared,
    event: &NotificationEvent,
    dispatcher: Option<&dyn NotificationCallDispatcher>,
    action: NativeCallAction,
) -> Result<(), NotificationError> {
    let NotificationKind::IncomingCall { call } = event.kind else {
        return Err(NotificationError::Stale);
    };
    {
        let state = shared.lock();
        if state.stopped || state.actions.len() >= 8 {
            return Err(NotificationError::Busy);
        }
        state.eligible_source(event)?;
        if state
            .running
            .as_ref()
            .is_none_or(|r| r.cancelled || r.event.request != event.request)
        {
            return Err(NotificationError::Stale);
        }
    }
    let dispatcher = dispatcher.ok_or(NotificationError::Unavailable)?;
    let supported = dispatcher.available_actions(call);
    if !match action {
        NativeCallAction::Accept => supported.accept,
        NativeCallAction::Decline => supported.decline,
    } {
        return Err(NotificationError::Unavailable);
    }
    // The dispatcher owns the final atomic admission check; no lock here can make
    // an account/logout race authorize an operation in a different runtime.
    dispatcher.dispatch(call, action)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_dispatcher_and_non_call_actions_never_succeed() {
        let shared = Shared {
            state: Mutex::new(State::new(NotificationPreferences::default())),
            wake: Condvar::new(),
        };
        let event = NotificationEvent::test(
            RequestId(1),
            NotificationCategory::IncomingCalls,
            talk_core::bounded::Text::new("test").unwrap(),
            talk_core::bounded::Text::new("").unwrap(),
        );
        assert_eq!(sound_argument(true, event.kind), "incoming");
        assert_eq!(sound_argument(false, event.kind), "silent");
        assert_eq!(
            dispatch_action(&shared, &event, None, NativeCallAction::Accept),
            Err(NotificationError::Stale)
        );
    }
}
