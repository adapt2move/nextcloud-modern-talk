//! Native local-file reads. One process-wide worker, no queue and no path-bearing errors.
//!
//! Cancellation/deadline stops observation immediately. A blocking OS operation may
//! still return later; its worker retains the permit until it exits, including when
//! the requesting window closes. No replacement worker can bypass that permit.
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use talk_core::ui::Text;

pub const MAX_CSV_BYTES: usize = 65_536;
pub const CSV_READ_DEADLINE: Duration = Duration::from_secs(10);
static CSV_WORKER_ACTIVE: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CsvReadError {
    Busy,
    TooLarge,
    InvalidUtf8,
    NotRegularFile,
    ReadFailed,
    Cancelled,
    TimedOut,
}
type Reply = Result<Text<MAX_CSV_BYTES>, CsvReadError>;

struct Permit;
impl Permit {
    fn acquire() -> Result<Self, CsvReadError> {
        CSV_WORKER_ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| Self)
            .map_err(|_| CsvReadError::Busy)
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        CSV_WORKER_ACTIVE.store(false, Ordering::Release);
    }
}

pub struct CsvRead {
    cancelled: Arc<AtomicBool>,
    deadline: Instant,
    result: mpsc::Receiver<Reply>,
    worker: Option<JoinHandle<()>>,
    reported: bool,
}
impl CsvRead {
    /// Reserves the single worker before spawning or allocating the read buffer.
    pub fn start(path: PathBuf) -> Result<Self, CsvReadError> {
        let permit = Permit::acquire()?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        let deadline = Instant::now() + CSV_READ_DEADLINE;
        let (tx, result) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("csv-file-read".into())
            .spawn(move || {
                let _permit = permit;
                let reply = read_path(&path, &flag, deadline);
                // Capacity is one and there is exactly one reply. A closed owner is
                // ordinary cancellation; never log the selected path or contents.
                let _ = tx.try_send(reply);
            })
            .map_err(|_| CsvReadError::ReadFailed)?;
        Ok(Self {
            cancelled,
            deadline,
            result,
            worker: Some(worker),
            reported: false,
        })
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    /// Nonblocking, exactly one result; cancellation wins over an unread success.
    pub fn try_result(&mut self) -> Option<Reply> {
        if self.reported {
            return None;
        }
        let result = match checkpoint(&self.cancelled, self.deadline) {
            Err(error) => Some(Err(error)),
            Ok(()) => match self.result.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(CsvReadError::ReadFailed)),
            },
        };
        if result.is_some() {
            self.reported = true;
            self.cancel();
        }
        result
    }
    /// A reported timeout/cancellation is not proof that a blocking OS call ended.
    pub fn is_finished(&self) -> bool {
        self.worker.as_ref().is_none_or(JoinHandle::is_finished)
    }
}
impl Drop for CsvRead {
    fn drop(&mut self) {
        self.cancel();
        if self.is_finished()
            && let Some(worker) = self.worker.take()
        {
            let _ = worker.join();
        }
        // An unfinished worker owns its permit and cancellation flag independently.
    }
}
fn checkpoint(cancelled: &AtomicBool, deadline: Instant) -> Result<(), CsvReadError> {
    if cancelled.load(Ordering::Acquire) {
        Err(CsvReadError::Cancelled)
    } else if Instant::now() >= deadline {
        Err(CsvReadError::TimedOut)
    } else {
        Ok(())
    }
}
fn read_path(path: &Path, cancelled: &AtomicBool, deadline: Instant) -> Reply {
    checkpoint(cancelled, deadline)?;
    let metadata = std::fs::metadata(path).map_err(|_| CsvReadError::ReadFailed)?;
    validate_metadata(&metadata)?;
    checkpoint(cancelled, deadline)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Prevent a file swapped for a FIFO between metadata and open from
        // holding a blocking reader. Recheck the actual opened descriptor below.
        options.custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32);
    }
    let mut file: File = options.open(path).map_err(|_| CsvReadError::ReadFailed)?;
    validate_metadata(&file.metadata().map_err(|_| CsvReadError::ReadFailed)?)?;
    read_utf8(&mut file, cancelled, deadline)
}
fn validate_metadata(metadata: &std::fs::Metadata) -> Result<(), CsvReadError> {
    if !metadata.is_file() {
        return Err(CsvReadError::NotRegularFile);
    }
    if metadata.len() > MAX_CSV_BYTES as u64 {
        return Err(CsvReadError::TooLarge);
    }
    Ok(())
}
fn read_utf8(reader: &mut impl Read, cancelled: &AtomicBool, deadline: Instant) -> Reply {
    checkpoint(cancelled, deadline)?;
    // One sentinel byte catches a file growing after metadata. Never read_to_end:
    // external size claims cannot trigger an unbounded buffer allocation.
    let mut bytes = vec![0; MAX_CSV_BYTES + 1];
    let mut used = 0;
    loop {
        checkpoint(cancelled, deadline)?;
        let end = (used + 4096).min(bytes.len());
        let count = reader
            .read(&mut bytes[used..end])
            .map_err(|_| CsvReadError::ReadFailed)?;
        checkpoint(cancelled, deadline)?;
        used += count;
        if used > MAX_CSV_BYTES {
            return Err(CsvReadError::TooLarge);
        }
        if count == 0 {
            break;
        }
    }
    bytes.truncate(used);
    let value = String::from_utf8(bytes).map_err(|_| CsvReadError::InvalidUtf8)?;
    let value = value.strip_prefix('\u{feff}').unwrap_or(&value);
    Text::new(&value.replace("\r\n", "\n")).map_err(|_| CsvReadError::TooLarge)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn read(bytes: &[u8]) -> Reply {
        read_utf8(
            &mut &*bytes,
            &AtomicBool::new(false),
            Instant::now() + CSV_READ_DEADLINE,
        )
    }
    #[test]
    fn utf8_bom_crlf_and_exact_byte_limit() {
        assert_eq!(
            read(b"\xef\xbb\xbfemail\r\na@example.test\r\n")
                .unwrap()
                .as_str(),
            "email\na@example.test\n"
        );
        assert!(read(&vec![b'a'; MAX_CSV_BYTES]).is_ok());
        assert_eq!(
            read(&vec![b'a'; MAX_CSV_BYTES + 1]),
            Err(CsvReadError::TooLarge)
        );
        assert_eq!(read(&[0xff]), Err(CsvReadError::InvalidUtf8));
    }
    #[test]
    fn cancellation_and_deadline_are_checked_before_reading() {
        struct NeverRead;
        impl Read for NeverRead {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                panic!("must not read");
            }
        }
        assert_eq!(
            read_utf8(
                &mut NeverRead,
                &AtomicBool::new(true),
                Instant::now() + CSV_READ_DEADLINE
            ),
            Err(CsvReadError::Cancelled)
        );
        assert_eq!(
            read_utf8(&mut NeverRead, &AtomicBool::new(false), Instant::now()),
            Err(CsvReadError::TimedOut)
        );
    }
    #[test]
    fn cancellation_during_read_discards_the_bytes() {
        struct CancelOnRead<'a>(&'a AtomicBool);
        impl Read for CancelOnRead<'_> {
            fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
                output[0] = b'a';
                self.0.store(true, Ordering::Release);
                Ok(1)
            }
        }
        let flag = AtomicBool::new(false);
        assert_eq!(
            read_utf8(
                &mut CancelOnRead(&flag),
                &flag,
                Instant::now() + CSV_READ_DEADLINE
            ),
            Err(CsvReadError::Cancelled)
        );
    }
    #[test]
    fn file_metadata_rejects_oversize_and_nonregular_files_before_buffer_allocation() {
        let directory = tempfile::tempdir().unwrap();
        let flag = AtomicBool::new(false);
        let deadline = Instant::now() + CSV_READ_DEADLINE;
        assert_eq!(
            read_path(directory.path(), &flag, deadline),
            Err(CsvReadError::NotRegularFile)
        );
        let path = directory.path().join("recipients.csv");
        let file = File::create(&path).unwrap();
        file.set_len((MAX_CSV_BYTES + 1) as u64).unwrap();
        assert_eq!(
            read_path(&path, &flag, deadline),
            Err(CsvReadError::TooLarge)
        );
        std::fs::write(&path, b"a@example.test").unwrap();
        assert_eq!(
            read_path(&path, &flag, deadline).unwrap().as_str(),
            "a@example.test"
        );
    }
    #[test]
    fn worker_permit_remains_held_until_actual_cleanup() {
        let permit = Permit::acquire().unwrap();
        assert!(matches!(Permit::acquire(), Err(CsvReadError::Busy)));
        let (started_tx, started_rx) = mpsc::sync_channel(1);
        let (stop_tx, stop_rx) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let _permit = permit;
            started_tx.send(()).unwrap();
            stop_rx.recv().unwrap();
        });
        started_rx.recv().unwrap();
        assert!(matches!(
            CsvRead::start(PathBuf::from("unused.csv")),
            Err(CsvReadError::Busy)
        ));
        stop_tx.send(()).unwrap();
        worker.join().unwrap();
        assert!(Permit::acquire().is_ok());
    }
}
