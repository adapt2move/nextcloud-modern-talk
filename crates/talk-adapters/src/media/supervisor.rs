//! Worker-only diagnostic supervisor. Production capture/profile admission is
//! deliberately unavailable. Two process slots include draining generations.
//! All process I/O is safe Rust; no inherited raw FD or pointer ABI.
use super::wire::{Kind, Record, Scope};
use std::fs::{self, DirBuilder, File};
use std::io::{self, Read, Write};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

const QUEUE: usize = 16;
const MAX_LIFETIME: Duration = Duration::from_secs(30);
const STOP_ESCALATION: Duration = Duration::from_millis(200);
static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();

struct Registry {
    slots: [Option<Arc<Lifetime>>; 2],
    // Diagnostic-only lifetime tombstones. Failed startup also burns its scope;
    // dropping a helper cannot restart the absolute window for that call.
    used: [Option<Scope>; 8],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Busy,
    WrongScope,
    Stale,
    Io,
    Unavailable,
    Uncertain,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Exit {
    Running,
    /// Synthetic source shutdown only, never a physical device observation.
    DiagnosticDrained,
    TerminatedUnconfirmed,
    Failed,
}

struct Lifetime {
    scope: Scope,
    started: Instant,
    revoked: AtomicBool,
    reader_done: AtomicBool,
    drained: AtomicBool,
    result: Mutex<Exit>,
}

fn registry() -> &'static Mutex<Registry> {
    REGISTRY.get_or_init(|| {
        Mutex::new(Registry {
            slots: [None, None],
            used: [None; 8],
        })
    })
}

fn reserve(scope: Scope) -> Result<(usize, Arc<Lifetime>), Error> {
    let mut registry = registry().lock().map_err(|_| Error::Unavailable)?;
    if registry
        .slots
        .iter()
        .flatten()
        .any(|slot| slot.scope.0[..16] == scope.0[..16])
    {
        return Err(Error::Busy);
    }
    if registry.used.contains(&Some(scope)) {
        return Err(Error::Stale);
    }
    let used = registry
        .used
        .iter()
        .position(Option::is_none)
        .ok_or(Error::Busy)?;
    let index = registry
        .slots
        .iter()
        .position(Option::is_none)
        .ok_or(Error::Busy)?;
    let state = Arc::new(Lifetime {
        scope,
        started: Instant::now(),
        revoked: AtomicBool::new(false),
        reader_done: AtomicBool::new(false),
        drained: AtomicBool::new(false),
        result: Mutex::new(Exit::Running),
    });
    registry.used[used] = Some(scope);
    registry.slots[index] = Some(state.clone());
    Ok((index, state))
}

fn release(index: usize, state: &Arc<Lifetime>) {
    if let Ok(mut registry) = registry().lock()
        && registry.slots[index]
            .as_ref()
            .is_some_and(|entry| Arc::ptr_eq(entry, state))
    {
        registry.slots[index] = None;
    }
}

fn random_directory() -> io::Result<PathBuf> {
    let mut bytes = [0; 16];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    let mut name = String::from("nmt-media-");
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(name, "{b:02x}");
    }
    let path = std::env::temp_dir().join(name);
    // create(), not create_all(): never adopt an existing directory or symlink.
    DirBuilder::new().mode(0o700).create(&path)?;
    Ok(path)
}

/// An explicit synthetic-media laboratory lifetime. It cannot be constructed by
/// core/UI and confers no Nextcloud join/call authority. Its absolute lifetime
/// cannot be extended by diagnostic watchdog keepalives.
pub struct DiagnosticSession {
    state: Arc<Lifetime>,
    commands: SyncSender<Record>,
    events: Receiver<Record>,
    serial: u64,
    operation: u64,
}

/// Priority closure for the account/call owner. It never queues an ordinary
/// command or waits for the helper/renderer/network. Old handles name only the
/// original Lifetime, and cannot close a replacement generation.
#[derive(Clone)]
pub struct StopHandle(Arc<Lifetime>);

impl StopHandle {
    pub fn close(&self) {
        self.0.revoked.store(true, Ordering::Release);
    }
}

impl DiagnosticSession {
    pub fn stop_handle(&self) -> StopHandle {
        StopHandle(self.state.clone())
    }
    pub fn is_revoked(&self) -> bool {
        self.state.revoked.load(Ordering::Acquire)
    }
    /// Blocking startup belongs on a bounded owner worker, never GPUI. `create`
    /// must be a validated Create record with an explicit synthetic AV intent.
    pub fn start(helper: &Path, create: Record) -> Result<Self, Error> {
        if !helper.is_absolute()
            || create.kind != Kind::Create
            || create.payload().len() != 1
            || create.payload()[0] > 3
        {
            return Err(Error::Unavailable);
        }
        let (slot, state) = reserve(create.scope)?;
        match Self::spawn(helper, create, state.clone(), slot) {
            Ok(session) => Ok(session),
            Err(error) => {
                if error != Error::Uncertain {
                    release(slot, &state);
                }
                Err(error)
            }
        }
    }

    fn spawn(
        helper: &Path,
        create: Record,
        state: Arc<Lifetime>,
        slot: usize,
    ) -> Result<Self, Error> {
        let directory = random_directory().map_err(|_| Error::Io)?;
        let socket = directory.join("watchdog");
        let result = Self::spawn_in(helper, create, state, slot, &directory, &socket);
        if result
            .as_ref()
            .is_err_and(|error| *error != Error::Uncertain)
        {
            let _ = fs::remove_file(&socket);
            let _ = fs::remove_dir(&directory);
        }
        result
    }

    fn spawn_in(
        helper: &Path,
        create: Record,
        state: Arc<Lifetime>,
        slot: usize,
        directory: &Path,
        socket: &Path,
    ) -> Result<Self, Error> {
        let listener = UnixListener::bind(socket).map_err(|_| Error::Io)?;
        listener.set_nonblocking(true).map_err(|_| Error::Io)?;
        let mut child = Command::new(helper)
            .arg("--diagnostic-synthetic")
            .arg(socket)
            .env_clear()
            .current_dir(directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| Error::Io)?;
        let Some(mut stdin) = child.stdin.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Io);
        };
        let Some(mut stdout) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Io);
        };
        let serial = create.serial;
        let operation = create.operation;
        // Tiny initial record fits the empty pipe; no native work is admitted
        // until the private watchdog connection has established authority.
        if create.write(&mut stdin).is_err() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Io);
        }
        let until = Instant::now() + Duration::from_secs(3);
        let connection = loop {
            match listener.accept() {
                Ok((connection, _)) => break Ok(connection),
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < until =>
                {
                    match child.try_wait() {
                        Ok(Some(_)) => break Err(Error::Unavailable),
                        Err(_) => break Err(Error::Io),
                        Ok(None) => {}
                    }
                    thread::sleep(Duration::from_millis(2));
                }
                _ => break Err(Error::Unavailable),
            }
        };
        let mut connection = match connection {
            Ok(stream) => stream,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        if connection
            .set_write_timeout(Some(Duration::from_millis(10)))
            .is_err()
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Io);
        }
        let (commands, commands_rx) = mpsc::sync_channel::<Record>(QUEUE);
        let (events_tx, events) = mpsc::sync_channel::<Record>(QUEUE);
        let writer_state = state.clone();
        let writer = thread::Builder::new()
            .name("media-ipc-write".into())
            .stack_size(256 * 1024)
            .spawn(move || {
                loop {
                    if writer_state.revoked.load(Ordering::Acquire) {
                        break;
                    }
                    let record = match commands_rx.recv_timeout(Duration::from_millis(25)) {
                        Ok(record) => record,
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    };
                    if record.write(&mut stdin).is_err() {
                        writer_state.revoked.store(true, Ordering::Release);
                        break;
                    }
                }
            });
        let writer = match writer {
            Ok(join) => join,
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::Io);
            }
        };
        let reader_state = state.clone();
        let reader = thread::Builder::new()
            .name("media-ipc-read".into())
            .stack_size(256 * 1024)
            .spawn(move || {
                let mut serial = 0;
                while let Ok(record) = Record::read(&mut stdout) {
                    if record.scope != reader_state.scope
                        || record.serial <= serial
                        || (record.kind as u16) < 100
                    {
                        reader_state.revoked.store(true, Ordering::Release);
                        break;
                    }
                    serial = record.serial;
                    if record.kind == Kind::Drained {
                        reader_state.drained.store(true, Ordering::Release);
                    }
                    if events_tx.try_send(record).is_err() {
                        reader_state.revoked.store(true, Ordering::Release);
                        break;
                    }
                }
                reader_state.reader_done.store(true, Ordering::Release);
            });
        let reader = match reader {
            Ok(join) => join,
            Err(_) => {
                state.revoked.store(true, Ordering::Release);
                let _ = child.kill();
                let _ = child.wait();
                let _ = writer.join();
                return Err(Error::Io);
            }
        };
        let watchdog_state = state.clone();
        let directory = directory.to_path_buf();
        let socket = socket.to_path_buf();
        let process = Arc::new(Mutex::new(Some(child)));
        let watchdog_process = process.clone();
        // The watchdog owns the process handle directly: no ordinary IPC lock,
        // event receiver, renderer, account actor or remote leave can delay kill.
        let watchdog = thread::Builder::new()
            .name("media-watchdog".into())
            .stack_size(256 * 1024)
            .spawn(move || {
                let Some(mut child) = watchdog_process
                    .lock()
                    .ok()
                    .and_then(|mut value| value.take())
                else {
                    return;
                };
                let started = watchdog_state.started;
                let mut trigger = None;
                let mut heartbeat = 0u64;
                let mut killed = false;
                let mut last_heartbeat = Instant::now() - Duration::from_millis(30);
                let successful_exit = loop {
                    let now = Instant::now();
                    if now.duration_since(started) >= MAX_LIFETIME
                        || watchdog_state.reader_done.load(Ordering::Acquire)
                    {
                        watchdog_state.revoked.store(true, Ordering::Release);
                    }
                    if watchdog_state.revoked.load(Ordering::Acquire) {
                        let at = trigger.get_or_insert(now);
                        // Close priority channel once; EOF immediately revokes native
                        // authority. A blocked ordinary pipe is irrelevant.
                        let _ = connection.shutdown(std::net::Shutdown::Both);
                        if now.duration_since(*at) >= STOP_ESCALATION && !killed {
                            let _ = child.kill();
                            killed = true;
                        }
                    } else if now.duration_since(last_heartbeat) >= Duration::from_millis(25) {
                        heartbeat += 1;
                        let mut packet = [0; 80];
                        packet[..4].copy_from_slice(b"NMW1");
                        packet[4..68].copy_from_slice(&watchdog_state.scope.0);
                        packet[68..72].copy_from_slice(&100u32.to_be_bytes());
                        packet[72..].copy_from_slice(&heartbeat.to_be_bytes());
                        if connection.write_all(&packet).is_err() {
                            watchdog_state.revoked.store(true, Ordering::Release);
                        }
                        last_heartbeat = now;
                    }
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            break status.success();
                        }
                        Err(_) => {
                            let _ = child.kill();
                            let _ = child.wait();
                            break false;
                        }
                        Ok(None) => {}
                    }
                    thread::sleep(Duration::from_millis(5));
                };
                watchdog_state.revoked.store(true, Ordering::Release);
                let _ = connection.shutdown(std::net::Shutdown::Both);
                // Child exit closes pipes; these bounded workers must settle before
                // the old account slot is eligible for replacement.
                let _ = writer.join();
                let _ = reader.join();
                let outcome = if successful_exit && watchdog_state.drained.load(Ordering::Acquire) {
                    Exit::DiagnosticDrained
                } else if trigger.is_some() {
                    Exit::TerminatedUnconfirmed
                } else {
                    Exit::Failed
                };
                let _ = fs::remove_file(socket);
                let _ = fs::remove_dir(directory);
                release(slot, &watchdog_state);
                // A terminal observation is published only after retirement.
                // Otherwise a same-account replacement can race a reported drain
                // and see a still-reserved slot although all I/O has settled.
                if let Ok(mut result) = watchdog_state.result.lock() {
                    *result = outcome;
                }
            });
        if watchdog.is_err() {
            state.revoked.store(true, Ordering::Release);
            if let Ok(mut process) = process.lock()
                && let Some(mut child) = process.take()
            {
                let _ = child.kill();
                let _ = child.wait();
            }
            // Worker joins were transferred into the failed closure and are
            // detached. Keep this slot charged: settlement is uncertain.
            return Err(Error::Uncertain);
        }
        Ok(Self {
            state,
            commands,
            events,
            serial,
            operation,
        })
    }

    pub fn submit(&mut self, record: Record) -> Result<(), Error> {
        if record.scope != self.state.scope {
            return Err(Error::WrongScope);
        }
        if self.state.revoked.load(Ordering::Acquire)
            || record.serial <= self.serial
            || record.operation < self.operation
        {
            return Err(Error::Stale);
        }
        if record.kind == Kind::Create || (record.kind as u16) >= 100 {
            return Err(Error::Unavailable);
        }
        let serial = record.serial;
        let operation = record.operation;
        self.commands
            .try_send(record)
            .map_err(|error| match error {
                TrySendError::Full(_) => Error::Busy,
                TrySendError::Disconnected(_) => Error::Io,
            })?;
        self.serial = serial;
        self.operation = operation;
        Ok(())
    }
    pub fn receive(&self, timeout: Duration) -> Option<Record> {
        self.events
            .recv_timeout(timeout.min(Duration::from_millis(50)))
            .ok()
    }
    pub fn revoke(&self) {
        self.state.revoked.store(true, Ordering::Release);
    }
    pub fn outcome(&self) -> Exit {
        self.state
            .result
            .lock()
            .map(|value| *value)
            .unwrap_or(Exit::Failed)
    }
}

impl Drop for DiagnosticSession {
    fn drop(&mut self) {
        self.revoke();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slots_include_same_account_and_retiring_generations() {
        let one = Scope([1; 64]);
        let two = Scope([2; 64]);
        let three = Scope([3; 64]);
        let (i, a) = reserve(one).unwrap();
        let (j, b) = reserve(two).unwrap();
        assert!(matches!(reserve(three), Err(Error::Busy)));
        a.revoked.store(true, Ordering::Release);
        assert!(matches!(reserve(one), Err(Error::Busy)));
        release(i, &a);
        assert!(matches!(reserve(one), Err(Error::Stale)));
        let (k, c) = reserve(three).unwrap();
        release(j, &b);
        release(k, &c);
        // Exact equality, no ordering assumption for random generation bytes.
        for value in 4..=8 {
            let (index, state) = reserve(Scope([value; 64])).unwrap();
            release(index, &state);
        }
        assert!(matches!(reserve(one), Err(Error::Stale)));
        assert!(matches!(reserve(Scope([9; 64])), Err(Error::Busy)));
    }
}
