//! Bounded waiter for the running application's native permission operation.
//! Dropping a probe cancels an unstarted request and ignores late callbacks;
//! an already displayed OS sheet is not claimed to have been rolled back.
use std::time::{Duration, Instant};
use talk_core::preferences::permissions::PermissionError;

pub(super) fn observe(
    kind: gpui::MacPermissionKind,
    request: bool,
    cancelled: impl Fn() -> bool,
) -> Result<gpui::MacPermissionStatus, PermissionError> {
    if cancelled() {
        return Err(PermissionError::Cancelled);
    }
    let operation = gpui::macos_permission(kind, request).map_err(mapped)?;
    let deadline = Instant::now() + Duration::from_secs(if request { 120 } else { 3 });
    loop {
        if cancelled() {
            return Err(PermissionError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(PermissionError::TimedOut);
        }
        if let Some(status) = operation.poll().map_err(mapped)? {
            return Ok(status);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn mapped(error: gpui::MacPermissionError) -> PermissionError {
    match error {
        gpui::MacPermissionError::Busy => PermissionError::Busy,
        gpui::MacPermissionError::Unavailable => PermissionError::Unavailable,
        gpui::MacPermissionError::Cancelled => PermissionError::Cancelled,
        gpui::MacPermissionError::Failed => PermissionError::Failed,
    }
}
