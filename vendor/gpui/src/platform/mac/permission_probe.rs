//! Finite lifetime for main-process macOS authorization callbacks.
//! Kept independent of native APIs so cancellation/admission is tested on Linux too.
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

/// An authorization observation from macOS, not an instruction to start capture.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MacPermissionStatus {
    /// The OS has not recorded a permission decision.
    NotDetermined,
    /// The OS currently permits the capability.
    Granted,
    /// The OS records a denial for this application.
    Denied,
    /// System policy restricts the capability independently of a user choice.
    Restricted,
    /// A screen request succeeded but this process still fails capture preflight.
    RestartRequired,
    /// No authoritative grant or denial was observed; this grants no permission.
    Unknown,
}
/// Failure to admit or complete a permission operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MacPermissionError {
    /// A callback of the same kind still owns its admission slot.
    Busy,
    /// The running process has no supported application bundle identity.
    Unavailable,
    /// The native API failed or the operation's result channel closed.
    Failed,
    /// The operation was cancelled before its result could be used.
    Cancelled,
}

// Even if an OS callback outlives a cancelled/timed-out waiter, it keeps its
// admission. Repeated checks cannot accumulate retained native callbacks.
static PENDING: [AtomicBool; 4] = [const { AtomicBool::new(false) }; 4];
struct Admission(usize);
impl Drop for Admission {
    fn drop(&mut self) {
        PENDING[self.0].store(false, Ordering::Release);
    }
}
pub(super) struct Probe {
    _admission: Admission,
    live: AtomicBool,
    send: Mutex<Option<mpsc::SyncSender<Result<MacPermissionStatus, MacPermissionError>>>>,
}
impl Probe {
    pub(super) fn live(&self) -> bool {
        self.live.load(Ordering::Acquire)
    }
    pub(super) fn finish(&self, value: Result<MacPermissionStatus, MacPermissionError>) {
        if let Some(send) = self.send.lock().unwrap_or_else(|e| e.into_inner()).take()
            && self.live.load(Ordering::Acquire)
        {
            let _ = send.try_send(value);
        }
    }
}
/// The waiter for one bounded native authorization callback.
///
/// Dropping this waiter suppresses late results. A retained OS callback continues
/// to own its admission slot until that callback is released.
pub struct MacPermissionOperation {
    probe: Arc<Probe>,
    receive: mpsc::Receiver<Result<MacPermissionStatus, MacPermissionError>>,
}
impl MacPermissionOperation {
    /// Observe the callback without blocking, returning `Ok(None)` while pending.
    ///
    /// A completion is delivered once. Retire the operation after a status or
    /// error; polling an already consumed result can return
    /// [`MacPermissionError::Failed`] because its result channel has closed.
    pub fn poll(&self) -> Result<Option<MacPermissionStatus>, MacPermissionError> {
        match self.receive.try_recv() {
            Ok(value) => value.map(Some),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => Err(MacPermissionError::Failed),
        }
    }
}
impl Drop for MacPermissionOperation {
    fn drop(&mut self) {
        self.probe.live.store(false, Ordering::Release);
    }
}

pub(super) fn admit(
    index: usize,
) -> Result<(MacPermissionOperation, Arc<Probe>), MacPermissionError> {
    PENDING[index]
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| MacPermissionError::Busy)?;
    let (send, receive) = mpsc::sync_channel(1);
    let probe = Arc::new(Probe {
        _admission: Admission(index),
        live: AtomicBool::new(true),
        send: Mutex::new(Some(send)),
    });
    Ok((
        MacPermissionOperation {
            probe: probe.clone(),
            receive,
        },
        probe,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn abandoned_callbacks_keep_their_admission_and_cannot_complete_a_new_probe() {
        let (waiter, callback) = admit(0).unwrap();
        assert!(matches!(admit(0), Err(MacPermissionError::Busy)));
        drop(waiter);
        assert!(
            !callback.live(),
            "queued native work sees cancellation before requesting"
        );
        assert!(
            matches!(admit(0), Err(MacPermissionError::Busy)),
            "a late OS callback still owns its slot"
        );
        callback.finish(Ok(MacPermissionStatus::Granted));
        drop(callback);
        let (waiter, callback) = admit(0).unwrap();
        callback.finish(Ok(MacPermissionStatus::Denied));
        callback.finish(Ok(MacPermissionStatus::Granted));
        assert_eq!(
            waiter.poll(),
            Ok(Some(MacPermissionStatus::Denied)),
            "only the first current completion is observable"
        );
        drop((waiter, callback));
        assert!(admit(0).is_ok());
    }
}
