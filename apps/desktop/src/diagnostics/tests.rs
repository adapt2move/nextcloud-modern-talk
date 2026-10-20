use super::*;
use std::fs;

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let path = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "talk-diagnostics-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        private::Directory::open(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn row(request: u64, outcome: Outcome) -> Event {
    Event {
        request,
        operation: Operation::Send,
        outcome,
        count: 0,
        error: None,
    }
}
fn output(root: &Temp) -> String {
    fs::read_to_string(root.0.join(FILES[0])).unwrap()
}

#[test]
fn rotation_retains_whole_records_in_three_bounded_files() {
    let root = Temp::new();
    let limit = 768;
    let mut sink = Sink::open(&root.0, limit).unwrap();
    for request in 1..=100 {
        sink.write(&encode(
            17,
            Record {
                at: request as u128,
                event: row(request, Outcome::Accepted),
            },
        ))
        .unwrap();
    }
    sink.flush().unwrap();
    let mut total = 0;
    for name in FILES {
        let bytes = fs::read(root.0.join(name)).unwrap();
        assert!(bytes.len() as u64 <= limit);
        total += bytes.len() as u64;
        assert!(bytes.ends_with(b"\n"));
        for line in std::str::from_utf8(&bytes).unwrap().lines() {
            assert!(line.starts_with("{\"timestamp_unix_ms\":"));
            assert!(line.ends_with('}'));
        }
    }
    assert!(total <= 3 * limit);
    assert_eq!(
        fs::read_dir(&root.0).unwrap().count(),
        4,
        "three logs plus the empty lockfile"
    );
    assert!(output(&root).contains("\"request\":100,"));
    assert!(
        !fs::read_to_string(root.0.join(FILES[2]))
            .unwrap()
            .contains("\"request\":1,")
    );
    drop(sink);
    let _restarted = Sink::open(&root.0, limit).unwrap();
    assert!(
        fs::read_to_string(root.0.join(FILES[1]))
            .unwrap()
            .contains("\"request\":100,")
    );
}

#[test]
fn second_writer_cannot_rotate_the_active_process_and_lock_is_released() {
    let root = Temp::new();
    let mut first = Sink::open(&root.0, MAX_BYTES).unwrap();
    first.write("{\"marker\":1}\n").unwrap();
    assert!(
        matches!(Sink::open(&root.0, MAX_BYTES), Err(e) if e.kind() == io::ErrorKind::WouldBlock)
    );
    assert_eq!(output(&root), "{\"marker\":1}\n");
    drop(first);
    assert!(Sink::open(&root.0, MAX_BYTES).is_ok());
}

#[test]
fn stalled_writer_has_finite_queue_and_shutdown_drains_admitted_records() {
    let root = Temp::new();
    let (handle, receiver) = channel();
    let (finished, completion) = mpsc::sync_channel(1);
    let sender = handle.clone();
    let producer = thread::spawn(move || {
        for request in 1..=CAPACITY as u64 + 17 {
            sender.record(row(request, Outcome::Accepted));
        }
        sender.shutdown(); // The queue is full: shutdown must not wait for a marker slot.
        finished.send(()).unwrap();
    });
    completion
        .recv_timeout(Duration::from_secs(2))
        .expect("submissions must return while the writer is stalled");
    producer.join().unwrap();
    assert_eq!(handle.state.dropped.load(Ordering::Relaxed), 17);
    run(
        Ok(root.0.clone()),
        receiver,
        handle.state.clone(),
        MAX_BYTES,
    );
    assert!(handle.state.stopped.load(Ordering::Acquire));
    let data = output(&root);
    assert_eq!(
        data.lines().count(),
        CAPACITY + 3,
        "start, counted overload, admitted events, stop"
    );
    assert!(data.contains("\"outcome\":\"dropped\",\"count\":17,"));
    assert!(data.contains(&format!("\"request\":{},", CAPACITY)));
    assert!(
        data.lines()
            .last()
            .unwrap()
            .contains("\"outcome\":\"stopped\"")
    );
}

#[test]
fn idle_shutdown_wakes_the_worker_and_open_or_write_failure_stops_cleanly() {
    let root = Temp::new();
    let (handle, receiver) = channel();
    let state = handle.state.clone();
    let path = root.0.clone();
    let (done, stopped) = mpsc::sync_channel(1);
    let worker = thread::spawn(move || {
        run(Ok(path), receiver, state, MAX_BYTES);
        done.send(()).unwrap();
    });
    handle.shutdown();
    stopped
        .recv_timeout(Duration::from_secs(2))
        .expect("idle shutdown must wake the writer");
    worker.join().unwrap();
    assert!(output(&root).contains("\"outcome\":\"stopped\""));
    for (path, limit) in [
        (Err(io::ErrorKind::PermissionDenied.into()), MAX_BYTES),
        (Ok(root.0.clone()), 1),
    ] {
        let (handle, receiver) = channel();
        run(path, receiver, handle.state.clone(), limit);
        assert!(handle.state.stopped.load(Ordering::Acquire));
        handle.record(row(7, Outcome::Accepted));
        handle.shutdown();
    }
}

#[test]
fn file_records_keep_correlation_version_and_only_the_closed_scalar_schema() {
    let root = Temp::new();
    let (handle, receiver) = channel();
    handle.record(row(73, Outcome::Started));
    let mut result = row(73, Outcome::Uncertain);
    result.error = Some(ErrorKind::Deadline);
    handle.record(result);
    handle.shutdown();
    run(
        Ok(root.0.clone()),
        receiver,
        handle.state.clone(),
        MAX_BYTES,
    );
    let data = output(&root);
    assert_eq!(data.matches("\"request\":73,").count(), 2);
    assert!(data.contains(&format!("\"version\":\"{}\"", env!("CARGO_PKG_VERSION"))));
    assert!(data.contains("\"error\":\"deadline\""));
    for line in data.lines() {
        assert_eq!(line.matches(':').count(), 9);
        assert!(!line.contains("http"));
        assert!(!line.contains('@'));
    }
}

#[cfg(unix)]
#[test]
fn private_files_reject_symlinks_hardlinks_and_broad_permissions_without_touching_target() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = Temp::new();
    let victim = root.0.join("synthetic-secret");
    fs::write(
        &victim,
        "synthetic credential and message must remain untouched",
    )
    .unwrap();
    fs::set_permissions(&victim, fs::Permissions::from_mode(0o600)).unwrap();
    let logs = root.0.join("logs");
    private::Directory::open(&logs).unwrap();
    for name in [LOCK, FILES[0]] {
        symlink(&victim, logs.join(name)).unwrap();
        assert!(Sink::open(&logs, MAX_BYTES).is_err());
        fs::remove_file(logs.join(name)).unwrap();
    }
    fs::hard_link(&victim, logs.join(FILES[0])).unwrap();
    assert!(Sink::open(&logs, MAX_BYTES).is_err());
    fs::remove_file(logs.join(FILES[0])).unwrap();
    fs::write(logs.join(FILES[0]), "public permissions").unwrap();
    fs::set_permissions(logs.join(FILES[0]), fs::Permissions::from_mode(0o644)).unwrap();
    assert!(Sink::open(&logs, MAX_BYTES).is_err());
    fs::remove_file(logs.join(FILES[0])).unwrap();
    let _sink = Sink::open(&logs, MAX_BYTES).unwrap();
    assert_eq!(
        fs::metadata(&logs).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(logs.join(FILES[0]))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        fs::read_to_string(&victim).unwrap(),
        "synthetic credential and message must remain untouched"
    );
    let alias = root.0.join("alias");
    symlink(&logs, &alias).unwrap();
    assert!(Sink::open(&alias, MAX_BYTES).is_err());
}
