//! Validated profile directory and immutable private objects. All names are local IDs.
#[cfg(unix)]
#[path = "private_fs/unix.rs"]
mod platform;
#[cfg(target_os = "windows")]
#[path = "private_fs/windows.rs"]
mod platform;
pub(super) use platform::PrivateDir;
