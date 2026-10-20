//! Content-free local diagnostics. One bounded queue and one file owner; callers
//! never format backend data or perform filesystem I/O. Overload loses diagnostics
//! (and counts that loss), never chat work. Request numbers are process-local.
use std::{
    fs::File,
    io::{self, Write},
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use talk_core::ports::ErrorKind;

mod private;
#[cfg(test)]
mod tests;

const CAPACITY: usize = 256;
const MAX_BYTES: u64 = 2 * 1024 * 1024;
const FILES: [&str; 3] = [
    "diagnostics.jsonl",
    "diagnostics.1.jsonl",
    "diagnostics.2.jsonl",
];
const LOCK: &str = ".diagnostics.lock";
static LOGGER: OnceLock<Handle> = OnceLock::new();

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum Operation {
    Application,
    Connection,
    LiveConnection,
    History,
    Older,
    Context,
    Thread,
    Poll,
    Send,
}
impl Operation {
    fn name(self) -> &'static str {
        match self {
            Self::Application => "application",
            Self::Connection => "connection",
            Self::LiveConnection => "live_connection",
            Self::History => "history",
            Self::Older => "older",
            Self::Context => "context",
            Self::Thread => "thread",
            Self::Poll => "poll",
            Self::Send => "send",
        }
    }
}
#[derive(Clone, Copy)]
pub(crate) enum Outcome {
    Started,
    Stopped,
    Loading,
    Ready,
    Connected,
    Reconnecting,
    Internal,
    Failed,
    Uncertain,
    Accepted,
    Rejected,
    Received,
    Ignored,
    Offline,
    Recovered,
    Dropped,
}
impl Outcome {
    fn name(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Stopped => "stopped",
            Self::Loading => "loading",
            Self::Ready => "ready",
            Self::Connected => "connected",
            Self::Reconnecting => "reconnecting",
            Self::Internal => "internal",
            Self::Failed => "failed",
            Self::Uncertain => "uncertain",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Received => "received",
            Self::Ignored => "ignored",
            Self::Offline => "offline",
            Self::Recovered => "recovered",
            Self::Dropped => "dropped",
        }
    }
}
/// No strings, domain IDs, request bodies, replies or Debug values can enter the log.
#[derive(Clone, Copy)]
pub(crate) struct Event {
    pub request: u64,
    pub operation: Operation,
    pub outcome: Outcome,
    pub count: u64,
    pub error: Option<ErrorKind>,
}
impl Event {
    pub(crate) fn new(operation: Operation, outcome: Outcome) -> Self {
        Self {
            request: 0,
            operation,
            outcome,
            count: 0,
            error: None,
        }
    }
}
struct Record {
    at: u128,
    event: Event,
}
#[derive(Default)]
struct State {
    shutdown: AtomicBool,
    stopped: AtomicBool,
    dropped: AtomicU64,
}
#[derive(Clone)]
struct Handle {
    sender: SyncSender<Option<Record>>,
    state: Arc<State>,
}
impl Handle {
    fn shutdown(&self) {
        self.state.shutdown.store(true, Ordering::Release);
        // A full queue already wakes the worker. An empty queue needs one marker.
        let _ = self.sender.try_send(None);
    }
    fn record(&self, event: Event) {
        if self.state.shutdown.load(Ordering::Acquire) || self.state.stopped.load(Ordering::Acquire)
        {
            return;
        }
        match self.sender.try_send(Some(Record {
            at: timestamp(),
            event,
        })) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => (),
            Err(TrySendError::Full(_)) => {
                let _ = self
                    .state
                    .dropped
                    .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                        Some(n.saturating_add(1))
                    });
            }
        }
    }
}
pub(crate) fn record(event: Event) {
    if let Some(logger) = LOGGER.get() {
        logger.record(event);
    }
}
pub(crate) fn request_shutdown() {
    if let Some(logger) = LOGGER.get() {
        logger.shutdown();
    }
}
pub(crate) fn stopped() -> bool {
    LOGGER
        .get()
        .is_none_or(|logger| logger.state.stopped.load(Ordering::Acquire))
}

/// The process lifetime owns the worker. GPUI's normal quit hook drains it; this
/// bounded fallback also covers startup failure and platforms whose run returns.
/// A stalled OS filesystem cannot indefinitely delay process termination.
pub(crate) struct Session {
    handle: Option<Handle>,
    worker: Option<JoinHandle<()>>,
}
impl Drop for Session {
    fn drop(&mut self) {
        if let Some(handle) = &self.handle {
            handle.shutdown();
        }
        if let Some(worker) = self.worker.take() {
            let deadline = Instant::now() + Duration::from_millis(250);
            while !worker.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
    }
}
pub(crate) fn start() -> Session {
    let (handle, receiver) = channel();
    let state = handle.state.clone();
    match thread::Builder::new()
        .name("local-diagnostics".into())
        .spawn(move || {
            run(platform_path(), receiver, state, MAX_BYTES);
        }) {
        Ok(worker) => {
            let _ = LOGGER.set(handle.clone());
            Session {
                handle: Some(handle),
                worker: Some(worker),
            }
        }
        Err(_) => {
            unavailable();
            Session {
                handle: None,
                worker: None,
            }
        }
    }
}
fn channel() -> (Handle, Receiver<Option<Record>>) {
    let (sender, receiver) = mpsc::sync_channel(CAPACITY);
    (
        Handle {
            sender,
            state: Arc::new(State::default()),
        },
        receiver,
    )
}
fn unavailable() {
    // Intentionally omit paths and OS errors, which can contain usernames or data.
    let _ = writeln!(
        io::stderr().lock(),
        "Local diagnostics unavailable; chat continues without a diagnostic file."
    );
}
fn platform_path() -> io::Result<PathBuf> {
    let env_path = |name| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    #[cfg(target_os = "macos")]
    let path = env_path("HOME").map(|p| p.join("Library/Logs/NextCloud Modern Talk"));
    #[cfg(target_os = "windows")]
    let path = env_path("LOCALAPPDATA").map(|p| p.join("NextCloud Modern Talk/Logs"));
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let path = env_path("XDG_STATE_HOME")
        .or_else(|| env_path("HOME").map(|p| p.join(".local/state")))
        .map(|p| p.join("nextcloud-modern-talk/logs"));
    path.ok_or_else(|| io::ErrorKind::NotFound.into())
}
fn timestamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |t| t.as_millis())
}
fn run(
    path: io::Result<PathBuf>,
    receiver: Receiver<Option<Record>>,
    state: Arc<State>,
    limit: u64,
) {
    let result = (|| -> io::Result<()> {
        let mut sink = Sink::open(&path?, limit)?;
        let session = timestamp();
        sink.write(&encode(
            session,
            Record {
                at: session,
                event: Event::new(Operation::Application, Outcome::Started),
            },
        ))?;
        loop {
            let next = if state.shutdown.load(Ordering::Acquire) {
                receiver.try_recv().ok().flatten()
            } else {
                receiver.recv().ok().flatten()
            };
            let lost = state.dropped.swap(0, Ordering::Relaxed);
            if lost > 0 {
                let mut event = Event::new(Operation::Application, Outcome::Dropped);
                event.count = lost;
                sink.write(&encode(
                    session,
                    Record {
                        at: timestamp(),
                        event,
                    },
                ))?;
            }
            let Some(record) = next else {
                break;
            };
            sink.write(&encode(session, record))?;
        }
        sink.write(&encode(
            session,
            Record {
                at: timestamp(),
                event: Event::new(Operation::Application, Outcome::Stopped),
            },
        ))?;
        sink.flush()
    })();
    if result.is_err() {
        unavailable();
    }
    state.stopped.store(true, Ordering::Release);
}
#[cfg(test)]
pub(crate) fn test_line(event: Event) -> String {
    encode(1, Record { at: 1, event })
}
fn encode(session: u128, record: Record) -> String {
    let Event {
        request,
        operation,
        outcome,
        count,
        error,
    } = record.event;
    let error = error.map_or("none", error_name);
    format!(
        "{{\"timestamp_unix_ms\":{},\"session\":{session},\"pid\":{},\"version\":\"{}\",\"request\":{request},\"operation\":\"{}\",\"outcome\":\"{}\",\"count\":{count},\"error\":\"{error}\"}}\n",
        record.at,
        std::process::id(),
        env!("CARGO_PKG_VERSION"),
        operation.name(),
        outcome.name()
    )
}
fn error_name(error: ErrorKind) -> &'static str {
    match error {
        ErrorKind::Unconfigured => "unconfigured",
        ErrorKind::Unsupported => "unsupported",
        ErrorKind::Cancelled => "cancelled",
        ErrorKind::Deadline => "deadline",
        ErrorKind::Busy => "busy",
        ErrorKind::PermissionDenied => "permission_denied",
        ErrorKind::ReauthenticationRequired => "reauthentication_required",
        ErrorKind::Malformed => "malformed",
        ErrorKind::Uncertain => "uncertain",
        ErrorKind::StorageUnavailable => "storage_unavailable",
        ErrorKind::WrongScope => "wrong_scope",
        ErrorKind::Transport => "transport",
        ErrorKind::RedirectRejected => "redirect_rejected",
        ErrorKind::TooLarge => "too_large",
        ErrorKind::NotFound => "not_found",
        ErrorKind::Conflict => "conflict",
        ErrorKind::RateLimited => "rate_limited",
        ErrorKind::ServerUnavailable => "server_unavailable",
    }
}
struct Sink {
    directory: private::Directory,
    _lock: File,
    file: Option<File>,
    bytes: u64,
    limit: u64,
}
impl Sink {
    fn open(path: &std::path::Path, limit: u64) -> io::Result<Self> {
        let directory = private::Directory::open(path)?;
        let lock = directory.file(LOCK, false)?;
        lock.try_lock()
            .map_err(|_| io::Error::from(io::ErrorKind::WouldBlock))?;
        let mut result = Self {
            directory,
            _lock: lock,
            file: None,
            bytes: 0,
            limit,
        };
        // A new process starts a new file, avoiding appending to a partial crash record.
        result.rotate()?;
        Ok(result)
    }
    fn rotate(&mut self) -> io::Result<()> {
        self.flush()?;
        self.file.take();
        for name in FILES {
            self.directory.validate(name)?;
        }
        self.directory.remove(FILES[2])?;
        self.directory.rename(FILES[1], FILES[2])?;
        self.directory.rename(FILES[0], FILES[1])?;
        self.file = Some(self.directory.file(FILES[0], true)?);
        self.bytes = 0;
        Ok(())
    }
    fn write(&mut self, line: &str) -> io::Result<()> {
        let bytes = line.len() as u64;
        if bytes > self.limit {
            return Err(io::ErrorKind::InvalidData.into());
        }
        if self.bytes + bytes > self.limit {
            self.rotate()?;
        }
        self.file
            .as_mut()
            .ok_or(io::ErrorKind::NotConnected)?
            .write_all(line.as_bytes())?;
        self.bytes += bytes;
        Ok(())
    }
    fn flush(&mut self) -> io::Result<()> {
        if let Some(file) = &mut self.file {
            file.flush()?;
        }
        Ok(())
    }
}
