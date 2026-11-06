//! Platform update boundary. Package inspection and D-Bus transactions have separate owners.
#[cfg(any(target_os = "linux", test))]
use super::UpdateError;
#[cfg(not(target_os = "linux"))]
use std::path::Path;
#[cfg(not(target_os = "linux"))]
use talk_core::lifecycle::Cancellation;

#[cfg(any(target_os = "linux", test))]
mod debian;
#[cfg(target_os = "linux")]
mod packagekit;
#[cfg(target_os = "linux")]
pub(super) use debian::verify_package;
#[cfg(target_os = "linux")]
pub(super) use packagekit::{available, install};

#[cfg(not(target_os = "linux"))]
pub(super) async fn available() -> Result<(), super::UpdateError> {
    Err(super::UpdateError::UnsupportedInstall)
}
#[cfg(not(target_os = "linux"))]
pub(super) async fn verify_package(
    _: &Path,
    _: &str,
    _: &Cancellation,
) -> Result<(), super::UpdateError> {
    Err(super::UpdateError::UnsupportedInstall)
}
#[cfg(not(target_os = "linux"))]
pub(super) async fn install(_: &Path) -> Result<(), super::UpdateError> {
    Err(super::UpdateError::UnsupportedInstall)
}
