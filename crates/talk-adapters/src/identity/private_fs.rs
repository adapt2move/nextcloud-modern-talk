//! Validated profile directory and immutable private objects. All names are local IDs.
pub(super) mod binding;
#[cfg(any(target_os = "macos", all(test, unix)))]
mod macos_volume;
#[cfg(unix)]
#[path = "private_fs/unix.rs"]
mod platform;
#[cfg(target_os = "windows")]
#[path = "private_fs/windows.rs"]
mod platform;
pub(super) use platform::PrivateDir;
