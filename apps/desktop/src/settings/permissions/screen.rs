//! Read-only Linux ScreenCast capability observation, never capture authority.
use ashpd::{desktop::screencast::Screencast, zbus};
use std::{
    future::Future,
    time::{Duration, Instant},
};
use talk_core::preferences::permissions::{
    PermissionError, PermissionGuidance, PermissionKind, PermissionObservation, PermissionStatus,
};

const CHECK_TIMEOUT: Duration = Duration::from_secs(3);
const CANCEL_INTERVAL: Duration = Duration::from_millis(20);

pub(super) fn check(
    cancelled: impl Fn() -> bool,
) -> Result<PermissionObservation, PermissionError> {
    observe(read_version(), cancelled, CHECK_TIMEOUT)
}

async fn read_version() -> Result<u32, ashpd::Error> {
    let portal = Screencast::new().await?;
    // ashpd 0.12.3's constructor defaults some failed version reads to 1.
    // Require a successful typed property read; constructing a proxy proves nothing.
    portal
        .get_property::<u32>("version")
        .await
        .map_err(Into::into)
}

fn observe(
    read: impl Future<Output = Result<u32, ashpd::Error>>,
    cancelled: impl Fn() -> bool,
    timeout: Duration,
) -> Result<PermissionObservation, PermissionError> {
    let deadline = Instant::now() + timeout;
    // PermissionRuntime admits one operation. Race the entire probe, including
    // bus connection and proxy creation; do not leave a worker behind on shutdown.
    // `or` polls the cancellation branch first and drops the losing future.
    // Dropping a property read cannot retract a sent read, but creates no session
    // or prompt to roll back. ashpd retains its shared bus connection, not capture.
    async_io::block_on(futures_lite::future::or(
        async {
            loop {
                if cancelled() {
                    return Err(PermissionError::Cancelled);
                }
                let now = Instant::now();
                if now >= deadline {
                    return Err(PermissionError::TimedOut);
                }
                async_io::Timer::at(deadline.min(now + CANCEL_INTERVAL)).await;
            }
        },
        async {
            let result = read.await;
            if cancelled() {
                return Err(PermissionError::Cancelled);
            }
            if Instant::now() >= deadline {
                return Err(PermissionError::TimedOut);
            }
            let status = match result {
                Ok(0) => PermissionStatus::Unavailable,
                Ok(_) => PermissionStatus::PerSession,
                Err(error) => error_status(error)?,
            };
            Ok(PermissionObservation {
                kind: PermissionKind::ScreenSharing,
                status,
                guidance: match status {
                    PermissionStatus::PerSession => PermissionGuidance::SelectForEachShare,
                    PermissionStatus::Unavailable => PermissionGuidance::BackendUnavailable,
                    _ => PermissionGuidance::CheckFirst,
                },
                can_request: false,
                can_open_settings: false,
            })
        },
    ))
}

fn error_status(error: ashpd::Error) -> Result<PermissionStatus, PermissionError> {
    match error {
        ashpd::Error::PortalNotFound(_) => Ok(PermissionStatus::Unavailable),
        ashpd::Error::Zbus(error) => bus_error_status(zbus::fdo::Error::from(error)),
        ashpd::Error::IO(error) => Ok(io_error_status(&error)),
        // No permission request is made here. A refused or malformed property
        // read is uncertainty about capability, not denial of screen consent.
        _ => Ok(PermissionStatus::Unknown),
    }
}

fn bus_error_status(error: zbus::fdo::Error) -> Result<PermissionStatus, PermissionError> {
    use zbus::fdo::Error;
    match error {
        Error::ServiceUnknown(_)
        | Error::NameHasNoOwner(_)
        | Error::UnknownObject(_)
        | Error::UnknownInterface(_)
        | Error::UnknownMethod(_)
        | Error::NoServer(_)
        | Error::Disconnected(_)
        | Error::FileNotFound(_)
        | Error::SpawnServiceNotFound(_) => Ok(PermissionStatus::Unavailable),
        Error::NoReply(_) | Error::Timeout(_) | Error::TimedOut(_) => {
            Err(PermissionError::TimedOut)
        }
        Error::ZBus(zbus::Error::InputOutput(error) | zbus::Error::Connection(error, _)) => {
            Ok(io_error_status(&error))
        }
        Error::ZBus(zbus::Error::Address(_) | zbus::Error::InterfaceNotFound) => {
            Ok(PermissionStatus::Unavailable)
        }
        // UnknownProperty (including a missing version), AccessDenied and type
        // errors must not become PerSession, Granted or a screen-permission Denied.
        _ => Ok(PermissionStatus::Unknown),
    }
}

fn io_error_status(error: &std::io::Error) -> PermissionStatus {
    use std::io::ErrorKind;
    match error.kind() {
        ErrorKind::NotFound
        | ErrorKind::ConnectionRefused
        | ErrorKind::ConnectionReset
        | ErrorKind::ConnectionAborted
        | ErrorKind::NotConnected
        | ErrorKind::BrokenPipe
        | ErrorKind::UnexpectedEof => PermissionStatus::Unavailable,
        _ => PermissionStatus::Unknown,
    }
}

#[cfg(test)]
mod tests;
