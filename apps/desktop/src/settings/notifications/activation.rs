//! Finite native reminder activation leases. The notification worker waits only
//! for submission; a bounded helper owns the later click and expires on logout.
use super::*;
use std::{
    io::{BufRead, BufReader, Write},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
};

const HOSTS: usize = 4;
static NEXT_LEASE: AtomicU64 = AtomicU64::new(1);

struct Host(Arc<Shared>);
impl Drop for Host {
    fn drop(&mut self) {
        let mut state = self.0.lock();
        state.activation_hosts = state.activation_hosts.saturating_sub(1);
    }
}

pub(super) fn deliver(
    shared: &Arc<Shared>,
    event: &NotificationEvent,
    sound: bool,
) -> Option<(DeliveryOutcome, DeliveryOutcome)> {
    if !cfg!(target_os = "macos") && !cfg!(target_os = "linux") {
        return None;
    }
    if cfg!(target_os = "linux") && !linux_daemon_supports(shared, event, "actions") {
        return None;
    }
    let locale = shared.lock().locale;
    let sound_file = (cfg!(target_os = "linux") && sound)
        .then(|| linux_sound_file(event.kind))
        .flatten()
        .filter(|_| linux_sound_supported(shared, event));
    let sound_available = !cfg!(target_os = "linux") || sound_file.is_some();
    let mut command = if cfg!(target_os = "macos") {
        let helper = super::super::native_process::mac_helper().ok()?;
        let lease = NEXT_LEASE
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .ok()?;
        let id = format!("nmt-{}-{}-{lease}", std::process::id(), event.request.0);
        let mut command = Command::new(helper);
        command.args([
            "notify",
            &id,
            &macos::hex(event.title.as_str()),
            &macos::hex(event.body.as_str()),
            "1",
            if sound { "message" } else { "silent" },
            "2",
            "0",
            "0",
            "0",
            match locale {
                Locale::English => "en",
                Locale::German => "de",
                Locale::Spanish => "es",
            },
        ]);
        command
    } else {
        let label = match locale {
            Locale::English => "Open",
            Locale::German => "Öffnen",
            Locale::Spanish => "Abrir",
        };
        linux_command_with_action(event, sound_file.as_deref(), Some(label))
    };
    {
        let mut state = shared.lock();
        if state.stopped || state.activation_hosts >= HOSTS {
            return None;
        }
        state.activation_hosts += 1;
    }
    let (ack_send, ack_receive) = mpsc::sync_channel(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancel_host = cancelled.clone();
    let host = Host(shared.clone());
    let event = event.clone();
    if std::thread::Builder::new()
        .name("talk-reminder-activation".into())
        .spawn(move || {
            let _host = host;
            run_host(
                &mut command,
                &_host.0,
                &event,
                sound_available,
                cancel_host,
                ack_send,
            );
        })
        .is_err()
    {
        return None;
    }
    match ack_receive.recv_timeout(Duration::from_secs(4)) {
        Ok(Ok((banner, sound_ok))) => Some((
            if banner {
                DeliveryOutcome::SubmittedToSystem
            } else {
                DeliveryOutcome::Failed(NotificationError::PermissionDenied)
            },
            if !sound {
                DeliveryOutcome::Suppressed
            } else if sound_ok {
                DeliveryOutcome::SubmittedToSystem
            } else {
                DeliveryOutcome::Failed(NotificationError::Unavailable)
            },
        )),
        Ok(Err(_)) | Err(_) => {
            cancelled.store(true, Ordering::Release);
            None
        }
    }
}

fn current(shared: &Shared, event: &NotificationEvent) -> bool {
    let state = shared.lock();
    !state.stopped
        && event
            .kind
            .scope()
            .is_some_and(|scope| state.sources.iter().any(|source| source.scope == scope))
}

pub(super) fn activate(shared: &Shared, event: &NotificationEvent) -> bool {
    if !event.kind.is_consistent() {
        return false;
    }
    let Some(activation @ NotificationActivation::OpenMessage { scope, .. }) =
        event.kind.activation()
    else {
        return false;
    };
    let mut state = shared.lock();
    if state.stopped
        || state.activations.len() >= 8
        || !state.sources.iter().any(|source| source.scope == scope)
    {
        return false;
    }
    state.activations.push_back(activation);
    true
}

fn run_host(
    command: &mut Command,
    shared: &Shared,
    event: &NotificationEvent,
    sound_available: bool,
    cancelled: Arc<AtomicBool>,
    ack: mpsc::SyncSender<Result<(bool, bool), NotificationError>>,
) {
    if check(shared, event, Effect::Banner) != Ok(true) {
        let _ = ack.try_send(Err(NotificationError::Cancelled));
        return;
    }
    let Ok(mut child) = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        let _ = ack.try_send(Err(NotificationError::Unavailable));
        return;
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return;
    };
    let (lines, receive) = mpsc::sync_channel(4);
    let reader = std::thread::Builder::new()
        .name("talk-reminder-output".into())
        .spawn(move || {
            let mut output = BufReader::with_capacity(128, stdout);
            loop {
                let mut line = Vec::with_capacity(64);
                match std::io::Read::take(&mut output, 65).read_until(b'\n', &mut line) {
                    Ok(0) => return,
                    Ok(_) if line.len() <= 64 && line.last() == Some(&b'\n') => {
                        if lines.try_send(String::from_utf8(line).ok()).is_err() {
                            return;
                        }
                    }
                    _ => {
                        let _ = lines.try_send(None);
                        return;
                    }
                }
            }
        });
    let Ok(reader) = reader else {
        let _ = child.kill();
        let _ = child.wait();
        return;
    };
    let start = Instant::now();
    let mut submitted = false;
    let mut exited = None;
    loop {
        if cancelled.load(Ordering::Acquire)
            || !current(shared, event)
            || start.elapsed() >= Duration::from_secs(42)
        {
            break;
        }
        if cfg!(target_os = "linux") && !submitted && start.elapsed() >= Duration::from_millis(150)
        {
            submitted = true;
            let _ = ack.try_send(Ok((true, sound_available)));
        }
        if let Ok(line) = receive.try_recv() {
            let Some(line) = line else { break };
            match line.trim() {
                "submitted\t1\t1" => {
                    submitted = true;
                    let _ = ack.try_send(Ok((true, true)));
                }
                "submitted\t1\t0" => {
                    submitted = true;
                    let _ = ack.try_send(Ok((true, false)));
                }
                "submitted\t0\t1" => {
                    submitted = true;
                    let _ = ack.try_send(Ok((false, true)));
                }
                "submitted\t0\t0" => {
                    submitted = true;
                    let _ = ack.try_send(Ok((false, false)));
                }
                "open" | "default" => {
                    if !submitted && cfg!(target_os = "linux") {
                        submitted = true;
                        let _ = ack.try_send(Ok((true, sound_available)));
                    }
                    if submitted {
                        activate(shared, event);
                    }
                    break;
                }
                "denied" | "failed" => break,
                _ => {}
            }
        }
        match child.try_wait() {
            Ok(Some(_))
                if exited.get_or_insert_with(Instant::now).elapsed()
                    >= Duration::from_millis(200) =>
            {
                break;
            }
            Ok(Some(_)) => {}
            Err(_) => break,
            Ok(None) => {}
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if !submitted {
        let _ = ack.try_send(Err(NotificationError::Failed));
    }
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(b"cancel\n");
    }
    let until = Instant::now() + Duration::from_millis(300);
    while child.try_wait().is_ok_and(|status| status.is_none()) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();
    let _ = reader.join();
}
